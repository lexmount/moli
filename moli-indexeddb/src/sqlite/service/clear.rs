//! An owner fence spans preparation, worker cleanup and FIFO retirement.
//! Only execute waits; preparing the bounded native payload performs no I/O.

use super::*;
use std::sync::mpsc;

pub(super) struct ClearingOwner {
    active: bool,
    _reservation: MetadataReservation,
}

impl Registry {
    pub(super) fn check_owner(&self, owner: &[u8]) -> Result<(), IndexedDbError> {
        if self.clearing.contains_key(owner) {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB owner is being cleared".into(),
            ));
        }
        Ok(())
    }
}

struct Targets {
    owner: Vec<u8>,
    fifo_owner: String,
    clients: Vec<DatabaseClient>,
    _reservation: MetadataReservation,
}

/// Prepared without SQL or filesystem access. Move this to a storage worker;
/// `execute` must never run on V8 or one of this service's SQL workers.
///
/// Dropping an unfinished clear keeps the owner fenced. Retrying the same
/// owner can finish it; failed or canceled cleanup never silently permits reopen.
pub struct OwnerClear {
    service: ServiceClient,
    targets: Arc<Targets>,
    permit: Option<TaskPermit>,
    finished: bool,
}

/// A queued maintenance request retains control capacity without fencing yet.
/// The storage owner can discard an already-completed duplicate before prepare.
pub struct OwnerClearRequest {
    service: ServiceClient,
    owner: Vec<u8>,
    permit: TaskPermit,
}

impl OwnerClearRequest {
    pub fn execute(self) -> Result<(), IndexedDbError> {
        self.service
            .prepare_clear_owner_reserved(self.owner, self.permit)?
            .execute()
    }
}

impl ServiceClient {
    pub fn reserve_clear_owner(&self, owner: Vec<u8>) -> Result<OwnerClearRequest, IndexedDbError> {
        if self.executor.is_worker_thread() {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB clear cannot run on a SQL worker".into(),
            ));
        }
        if owner.is_empty() {
            return Err(IndexedDbError::Data(
                "empty IndexedDB owner identity".into(),
            ));
        }
        if self.registry.lock().closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        let permit = self
            .executor
            .try_reserve(
                TaskKind::Control,
                1024usize.saturating_add(owner.capacity()),
            )
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        Ok(OwnerClearRequest {
            service: self.clone(),
            owner,
            permit,
        })
    }

    pub fn prepare_clear_owner(&self, owner: Vec<u8>) -> Result<OwnerClear, IndexedDbError> {
        let request = self.reserve_clear_owner(owner)?;
        self.prepare_clear_owner_reserved(request.owner, request.permit)
    }

    fn prepare_clear_owner_reserved(
        &self,
        owner: Vec<u8>,
        permit: TaskPermit,
    ) -> Result<OwnerClear, IndexedDbError> {
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
        if registry
            .clearing
            .get(&owner)
            .is_some_and(|state| state.active)
        {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB owner clear is already active".into(),
            ));
        }
        registry.reap_closed();
        let mut count = 0usize;
        let mut bytes = owner
            .capacity()
            .saturating_add(owner.len().saturating_mul(2))
            .saturating_add(4096);
        for ((key, _), client) in &registry.entries {
            if key != &owner {
                continue;
            }
            count += 1;
            bytes = bytes.saturating_add(std::mem::size_of::<DatabaseClient>() + 512);
            if let Location::Disk { path, .. } = &client.location {
                if client.catalog.is_none() {
                    return Err(IndexedDbError::InvalidState(
                        "only catalog generations can be cleared".into(),
                    ));
                }
                bytes = bytes.saturating_add(path.capacity().saturating_mul(2));
            }
        }
        let mut reservation = self.metadata.reservation();
        reservation.try_resize(bytes)?;
        let mut fence_reservation = None;
        if !registry.clearing.contains_key(&owner) {
            let mut lease = self.metadata.reservation();
            lease.try_resize(
                owner
                    .capacity()
                    .saturating_add(metadata::map_entry_bytes::<Vec<u8>, ClearingOwner>() as usize),
            )?;
            fence_reservation = Some(lease);
        }
        let fifo_owner = connections::request_owner_key(&owner)?;
        let mut clients = Vec::with_capacity(count);
        clients.extend(
            registry
                .entries
                .iter()
                .filter(|((key, _), _)| key == &owner)
                .map(|(_, client)| client.clone()),
        );
        if let Some(lease) = fence_reservation {
            registry.clearing.insert(
                owner.clone(),
                ClearingOwner {
                    active: true,
                    _reservation: lease,
                },
            );
        } else {
            registry
                .clearing
                .get_mut(&owner)
                .expect("existing clear")
                .active = true;
        }
        drop(registry);
        // Do not hold the registry while ordering against native admissions.
        for client in &clients {
            let _gate = client.state.admission_gate.lock();
            client.state.deleting.store(true, Ordering::Release);
        }
        Ok(OwnerClear {
            service: self.clone(),
            targets: Arc::new(Targets {
                owner,
                fifo_owner,
                clients,
                _reservation: reservation,
            }),
            permit: Some(permit),
            finished: false,
        })
    }
}

impl OwnerClear {
    /// Blocking storage-worker operation. One control permit travels through
    /// all SQL lanes, independent of data capacity and the database count.
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
            self.permit
                .take()
                .expect("clear control reservation")
                .submit(index as u64, move |permit| {
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
                    let _ = sender.send((permit, error));
                })
                .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
            let (permit, error) = receiver
                .recv()
                .map_err(|_| IndexedDbError::Io("IndexedDB clear worker stopped".into()))?;
            self.permit = Some(permit);
            if first_error.is_none() {
                first_error = error;
            }
        }
        self.service
            .connection_requests
            .cancel_owner(&self.targets.fifo_owner);
        let mut registry = self.service.registry.lock();
        if first_error.is_none() {
            registry
                .entries
                .retain(|(owner, _), _| owner != &self.targets.owner);
            registry.clearing.remove(&self.targets.owner);
        } else {
            registry
                .clearing
                .get_mut(&self.targets.owner)
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
                .get_mut(&self.targets.owner)
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
