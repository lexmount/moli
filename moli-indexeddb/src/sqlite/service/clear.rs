//! An owner fence spans preparation, worker cleanup and FIFO retirement.
//! Only execute waits; preparing the bounded native payload performs no I/O.

use super::*;
use std::sync::mpsc;

pub(super) struct ClearingOwner {
    active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ClearScope {
    Owner(Vec<u8>),
    Prefix(Vec<u8>),
}

impl ClearScope {
    fn bytes(&self) -> &Vec<u8> {
        match self {
            Self::Owner(bytes) | Self::Prefix(bytes) => bytes,
        }
    }

    pub(super) fn matches(&self, owner: &[u8]) -> bool {
        match self {
            Self::Owner(bytes) => bytes == owner,
            Self::Prefix(bytes) => owner.starts_with(bytes),
        }
    }

    pub(super) fn overlaps_prefix(&self, prefix: &[u8]) -> bool {
        match self {
            Self::Owner(bytes) => bytes.starts_with(prefix),
            Self::Prefix(bytes) => bytes.starts_with(prefix) || prefix.starts_with(bytes),
        }
    }

    fn covers(&self, other: &Self) -> bool {
        match self {
            Self::Owner(_) => self == other,
            Self::Prefix(bytes) => other.bytes().starts_with(bytes),
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        match other {
            Self::Owner(bytes) => self.matches(bytes),
            Self::Prefix(bytes) => self.overlaps_prefix(bytes),
        }
    }
}

impl Registry {
    pub(super) fn check_owner(&self, owner: &[u8]) -> Result<(), IndexedDbError> {
        if self.clearing.keys().any(|scope| scope.matches(owner)) {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB owner is being cleared".into(),
            ));
        }
        Ok(())
    }
}

struct Targets {
    scope: ClearScope,
    fifo_owner: String,
    clients: Vec<DatabaseClient>,
}

/// Prepared without SQL or filesystem access. Move this to a storage worker;
/// `execute` must never run on V8 or one of this service's SQL workers.
///
/// Dropping an unfinished clear keeps the owner fenced. Retrying the same
/// owner can finish it; failed or canceled cleanup never silently permits reopen.
pub struct OwnerClear {
    service: ServiceClient,
    targets: Arc<Targets>,
    finished: bool,
}

/// A queued maintenance request owns its payload without fencing yet.
/// The storage owner can discard an already-completed duplicate before prepare.
pub struct OwnerClearRequest {
    service: ServiceClient,
    scope: ClearScope,
}

impl OwnerClearRequest {
    pub fn execute(self) -> Result<(), IndexedDbError> {
        self.service.prepare_clear_scope(self.scope)?.execute()
    }
}

impl ServiceClient {
    pub fn request_clear_owner(&self, owner: Vec<u8>) -> Result<OwnerClearRequest, IndexedDbError> {
        self.request_clear_scope(ClearScope::Owner(owner))
    }

    fn request_clear_scope(&self, scope: ClearScope) -> Result<OwnerClearRequest, IndexedDbError> {
        if self.executor.is_worker_thread() {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB clear cannot run on a SQL worker".into(),
            ));
        }
        if matches!(&scope, ClearScope::Owner(owner) if owner.is_empty()) {
            return Err(IndexedDbError::Data(
                "empty IndexedDB owner identity".into(),
            ));
        }
        if self.registry.lock().closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        Ok(OwnerClearRequest {
            service: self.clone(),
            scope,
        })
    }

    pub fn prepare_clear_owner(&self, owner: Vec<u8>) -> Result<OwnerClear, IndexedDbError> {
        let request = self.request_clear_owner(owner)?;
        self.prepare_clear_scope(request.scope)
    }

    pub fn prepare_clear_owners_with_prefix(
        &self,
        prefix: Vec<u8>,
    ) -> Result<OwnerClear, IndexedDbError> {
        let request = self.request_clear_scope(ClearScope::Prefix(prefix))?;
        self.prepare_clear_scope(request.scope)
    }

    fn prepare_clear_scope(&self, scope: ClearScope) -> Result<OwnerClear, IndexedDbError> {
        if self.executor.is_worker_thread() {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB clear cannot run on a SQL worker".into(),
            ));
        }
        let mut registry = self.registry.lock();
        if registry.closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        if registry.clearing.iter().any(|(existing, state)| {
            scope.overlaps(existing) && (state.active || !scope.covers(existing))
        }) {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB overlapping owner clear is already active or requires a broader retry"
                    .into(),
            ));
        }
        registry.reap_closed();
        let mut clients = Vec::new();
        for ((key, _), client) in &registry.entries {
            if !scope.matches(key) {
                continue;
            }
            if matches!(&client.location, Location::Disk { .. }) && client.catalog.is_none() {
                return Err(IndexedDbError::InvalidState(
                    "only catalog generations can be cleared".into(),
                ));
            }
            clients.push(client.clone());
        }
        let fifo_owner = connections::request_owner_key(scope.bytes())?;
        // A broader retry replaces abandoned, fully covered fences atomically.
        registry
            .clearing
            .retain(|existing, _| existing == &scope || !scope.covers(existing));
        registry
            .clearing
            .insert(scope.clone(), ClearingOwner { active: true });
        drop(registry);
        // Do not hold the registry while ordering against native admissions.
        for client in &clients {
            let _gate = client.state.admission_gate.lock();
            client.state.deleting.store(true, Ordering::Release);
        }
        Ok(OwnerClear {
            service: self.clone(),
            targets: Arc::new(Targets {
                scope,
                fifo_owner,
                clients,
            }),
            finished: false,
        })
    }
}

impl OwnerClear {
    /// Blocking storage-worker operation. Each lane performs its cleanup after
    /// accepted commands, independently of retained data and database count.
    pub fn execute(mut self) -> Result<(), IndexedDbError> {
        if self.service.executor.is_worker_thread() {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB clear cannot run on a SQL worker".into(),
            ));
        }
        let mut first_error = None;
        for (index, lane) in self.service.lanes.iter().enumerate() {
            let lane = lane.clone();
            let targets = self.targets.clone();
            let workers = self.service.lanes.len() as u64;
            let (sender, receiver) = mpsc::channel();
            self.service
                .executor
                .submit(index as u64, move || {
                    let mut error = None;
                    let mut lane = lane.lock();
                    for client in &targets.clients {
                        if client.id.0 % workers != index as u64 {
                            continue;
                        }
                        if let Err(failure) = clear_database(&mut lane, client) {
                            error.get_or_insert(failure);
                        }
                    }
                    drop(lane);
                    let _ = sender.send(error);
                })
                .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
            let error = receiver
                .recv()
                .map_err(|_| IndexedDbError::Io("IndexedDB clear worker stopped".into()))?;
            if first_error.is_none() {
                first_error = error;
            }
        }
        match &self.targets.scope {
            ClearScope::Owner(_) => self
                .service
                .connection_requests
                .cancel_owner(&self.targets.fifo_owner),
            ClearScope::Prefix(_) => self
                .service
                .connection_requests
                .cancel_owner_prefix(&self.targets.fifo_owner),
        }
        let mut registry = self.service.registry.lock();
        if first_error.is_none() {
            registry
                .entries
                .retain(|(owner, _), _| !self.targets.scope.matches(owner));
            registry.clearing.remove(&self.targets.scope);
        } else {
            registry
                .clearing
                .get_mut(&self.targets.scope)
                .expect("clear fence")
                .active = false;
        }
        self.finished = true;
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for OwnerClear {
    fn drop(&mut self) {
        if !self.finished
            && let Some(state) = self
                .service
                .registry
                .lock()
                .clearing
                .get_mut(&self.targets.scope)
        {
            state.active = false;
        }
    }
}

fn clear_database(lane: &mut Lane, client: &DatabaseClient) -> Result<(), IndexedDbError> {
    if client.state.deleted.load(Ordering::Acquire) {
        return Ok(());
    }
    let result = (|| {
        if let Some(entry) = lane.entries.remove(&client.id) {
            entry.close(client.id)?;
        }
        if !client.is_retired() {
            return Err(corruption("IndexedDB clear lost its database owner"));
        }
        if let (Some(catalog), Location::Disk { path, .. }) = (&client.catalog, &client.location) {
            catalog.retire(path)?;
        }
        Ok(())
    })();
    match &result {
        Ok(()) => {
            *client.state.failure.lock() = None;
            client.state.deleted.store(true, Ordering::Release);
        }
        Err(error) => *client.state.failure.lock() = Some(error.clone()),
    }
    result
}
