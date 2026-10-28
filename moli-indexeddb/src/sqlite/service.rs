//! Bounded native command transport. Connections live exclusively in worker
//! state; a client reads only published metadata and never locks a SQL owner.

mod command;
pub use command::Command;

use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use moli_sqlite::{
    ConnectionOptions, Executor, ExecutorConfig, ExecutorError, ExecutorHandle, OpenMode,
};
pub use moli_sqlite::{TaskKind, TaskPermit};
use parking_lot::{Mutex, RwLock};

use super::*;
use crate::{RecordSnapshot, TransactionRequestHandle, read_budget::ReadBudget};

#[derive(Debug, Clone, Copy)]
pub struct ServiceConfig {
    pub executor: ExecutorConfig,
    pub connection: ConnectionOptions,
    pub connections_per_worker: usize,
    pub databases: usize,
    pub admissions: usize,
    pub admission_bytes: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            executor: ExecutorConfig::default(),
            connection: ConnectionOptions::default(),
            connections_per_worker: 8,
            databases: 1024,
            admissions: 1024,
            admission_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Location {
    Memory,
    /// Create is used only inside an exclusively allocated generation directory.
    Disk {
        path: PathBuf,
        mode: OpenMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DatabaseId(u64);

#[derive(Debug)]
pub enum DispatchError {
    Executor(ExecutorError),
    InvalidReservation,
    IdentityConflict,
    Capacity,
    Closed,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executor(error) => error.fmt(f),
            Self::InvalidReservation => f.write_str("invalid IndexedDB task reservation"),
            Self::IdentityConflict => {
                f.write_str("IndexedDB identity or path is already registered")
            }
            Self::Capacity => f.write_str("IndexedDB database capacity exhausted"),
            Self::Closed => f.write_str("IndexedDB SQL service is closed"),
        }
    }
}

impl std::error::Error for DispatchError {}
impl From<ExecutorError> for DispatchError {
    fn from(error: ExecutorError) -> Self {
        Self::Executor(error)
    }
}

#[derive(Debug)]
pub enum CommandResult {
    Metadata(Arc<DatabaseMetadata>),
    Prepared(PreparedWrite),
    Key(Key),
    Records(Vec<RecordSnapshot>),
    Count(u64),
    Unit,
}

pub struct Completion {
    pub database: DatabaseId,
    pub generation: [u8; 16],
    pub result: Result<CommandResult, IndexedDbError>,
    pub permit: TaskPermit,
}

struct ResultBudget<'a> {
    permit: &'a mut TaskPermit,
    available: u64,
}

impl ReadBudget for ResultBudget<'_> {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        if let Some(remaining) = self.available.checked_sub(bytes) {
            self.available = remaining;
            return Ok(());
        }
        let additional = usize::try_from(bytes - self.available)
            .ok()
            .and_then(|bytes| self.permit.reserved_bytes().checked_add(bytes))
            .ok_or_else(|| IndexedDbError::Resource("IndexedDB result size overflow".into()))?;
        self.permit
            .try_resize(additional)
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        self.available = 0;
        Ok(())
    }

    fn refund(&mut self, bytes: u64) {
        self.available += bytes;
    }
}

#[derive(Default)]
struct Published {
    closed: AtomicBool,
    next_transaction: AtomicU64,
    /// Orders admission registration against retirement, without locking SQL.
    admission_gate: Mutex<()>,
    metadata: RwLock<Option<Arc<DatabaseMetadata>>>,
    pending: AtomicUsize,
}

#[derive(Clone)]
pub struct DatabaseClient {
    id: DatabaseId,
    identity: Arc<DatabaseIdentity>,
    location: Location,
    state: Arc<Published>,
    executor: ExecutorHandle,
    lane: Arc<Mutex<Lane>>,
    config: ServiceConfig,
    admissions: Arc<crate::TransactionRequestQueues>,
}

struct Pending(Arc<Published>);
impl Pending {
    fn new(state: Arc<Published>) -> Self {
        state.pending.fetch_add(1, Ordering::AcqRel);
        Self(state)
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Entry {
    identity: Arc<DatabaseIdentity>,
    location: Location,
    state: Arc<Published>,
    database: Option<Database>,
    initialized: bool,
    failure: Option<IndexedDbError>,
    admission: Option<TransactionRequestHandle>,
    admissions: Arc<crate::TransactionRequestQueues>,
    last_used: u64,
}

impl Entry {
    fn finish_admission(&mut self) {
        if let Some(admission) = self.admission.take() {
            admission.finish();
        }
    }

    fn publish(&self) -> Arc<DatabaseMetadata> {
        let committed = self.database.as_ref().expect("connected owner").committed();
        if let Some(published) = self.state.metadata.read().as_ref()
            && published.as_ref() == committed
        {
            return published.clone();
        }
        let metadata = Arc::new(committed.clone());
        let previous = self.state.metadata.write().replace(metadata.clone());
        // Neither cloning nor destroying schema maps holds the publication lock.
        drop(previous);
        metadata
    }

    fn close(mut self, id: DatabaseId) -> Result<(), IndexedDbError> {
        let result = self.database.take().map_or(Ok(()), Database::close);
        // Rollback/close precede both admission release and closure publication.
        {
            let _gate = self.state.admission_gate.lock();
            self.state.closed.store(true, Ordering::Release);
        }
        self.admission = None;
        let key = queue_key(id);
        self.admissions.cancel_database(&key.0, &key.1);
        result
    }
}

#[derive(Default)]
struct Lane {
    entries: BTreeMap<DatabaseId, Entry>,
    clock: u64,
}

#[derive(Default)]
struct Registry {
    closed: bool,
    next_id: u64,
    entries: BTreeMap<(Vec<u8>, String), DatabaseClient>,
}

pub struct Service {
    executor: Executor,
    registry: Mutex<Registry>,
    lanes: Vec<Arc<Mutex<Lane>>>,
    config: ServiceConfig,
    admissions: Arc<crate::TransactionRequestQueues>,
}

impl Service {
    pub fn new(config: ServiceConfig) -> Result<Self, DispatchError> {
        if config.connections_per_worker == 0
            || config.databases == 0
            || config.admissions == 0
            || config.admission_bytes == 0
        {
            return Err(DispatchError::Capacity);
        }
        let executor = Executor::new(config.executor)?;
        let lanes = (0..config.executor.workers)
            .map(|_| Arc::new(Mutex::new(Lane::default())))
            .collect();
        Ok(Self {
            executor,
            registry: Mutex::new(Registry::default()),
            lanes,
            config,
            admissions: Arc::new(crate::TransactionRequestQueues::serial_bounded(
                config.admissions,
                config.admission_bytes,
            )),
        })
    }

    /// Registration and the initial command enqueue share a gate, so a second
    /// client cannot overtake database initialization. The callback only sends a
    /// native completion to its owner; it must not enter V8 on this worker.
    pub fn open(
        &self,
        identity: DatabaseIdentity,
        location: Location,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<DatabaseClient, DispatchError> {
        let mut registry = self.registry.lock();
        if registry.closed {
            return Err(DispatchError::Closed);
        }
        registry.entries.retain(|_, client| !client.is_closed());
        let key = (identity.owner_key.clone(), identity.name.clone());
        if let Some(existing) = registry.entries.get(&key) {
            if existing.identity.as_ref() != &identity
                || !same_location(&existing.location, &location)
            {
                return Err(DispatchError::IdentityConflict);
            }
            let client = existing.clone();
            client.submit_metadata(completion)?;
            return Ok(client);
        }
        if registry.entries.len() >= self.config.databases {
            return Err(DispatchError::Capacity);
        }
        if let Location::Disk { path, .. } = &location && registry.entries.values().any(|client| matches!(&client.location, Location::Disk { path: other, .. } if other == path)) { return Err(DispatchError::IdentityConflict); }
        registry.next_id = registry
            .next_id
            .checked_add(1)
            .ok_or(DispatchError::Capacity)?;
        let id = DatabaseId(registry.next_id);
        let index = (id.0 % self.config.executor.workers as u64) as usize;
        let client = DatabaseClient {
            id,
            identity: Arc::new(identity),
            location,
            state: Arc::new(Published::default()),
            executor: self.executor.handle(),
            lane: self.lanes[index].clone(),
            config: self.config,
            admissions: self.admissions.clone(),
        };
        let input = client
            .identity
            .owner_key
            .capacity()
            .saturating_add(client.identity.name.capacity())
            .saturating_add(std::mem::size_of::<Entry>());
        let permit = client
            .executor
            .try_reserve(TaskKind::Data, input.saturating_add(1024))?;
        let entry = Entry {
            identity: client.identity.clone(),
            location: client.location.clone(),
            state: client.state.clone(),
            database: None,
            initialized: false,
            failure: None,
            admission: None,
            admissions: self.admissions.clone(),
            last_used: 0,
        };
        let lane = client.lane.clone();
        let config = client.config;
        let generation = client.identity.generation;
        let pending = Pending::new(client.state.clone());
        permit.submit(id.0, move |permit| {
            let result = {
                let mut lane = lane.lock();
                lane.entries.insert(id, entry);
                lane.connect(id, config)
                    .map(|_| CommandResult::Metadata(lane.entries[&id].publish()))
            };
            drop(pending);
            completion(Completion {
                database: id,
                generation,
                result,
                permit,
            });
        })?;
        registry.entries.insert(key, client.clone());
        Ok(client)
    }

    /// Call off the V8 owner thread. Cleanup has one reserved finalizer per
    /// worker and succeeds independently of undelivered result permits.
    pub fn shutdown(&mut self) -> Result<(), IndexedDbError> {
        self.registry.lock().closed = true;
        let lanes = self.lanes.clone();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let failures = errors.clone();
        self.executor
            .shutdown_with(move |index| {
                let entries = std::mem::take(&mut lanes[index].lock().entries);
                for (id, entry) in entries {
                    if let Err(error) = entry.close(id) {
                        failures.lock().push(error);
                    }
                }
            })
            .map_err(|error| IndexedDbError::Io(error.to_string()))?;
        let error = errors.lock().pop();
        error.map_or(Ok(()), Err)
    }

    pub fn queue_usage(&self) -> moli_sqlite::QueueUsage {
        self.executor.handle().usage()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl DatabaseClient {
    /// Handles are unique for this database registration, including across
    /// renderer realms. They are never persisted or reused after completion.
    pub fn allocate_transaction_handle(&self) -> Result<TransactionHandle, IndexedDbError> {
        if self.is_closed() {
            return Err(IndexedDbError::InvalidState(
                "SQL database is closed".into(),
            ));
        }
        self.state
            .next_transaction
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map(|value| TransactionHandle::from_raw(value + 1))
            .map_err(|_| IndexedDbError::Resource("SQL transaction handles exhausted".into()))
    }

    pub fn queue_transaction_start(
        &self,
        connection: crate::DatabaseHandle,
        stores: &[String],
        mode: TransactionMode,
        wake: crate::TransactionRequestWake,
    ) -> Result<crate::TransactionRequestLease, IndexedDbError> {
        let _gate = self.state.admission_gate.lock();
        if self.is_closed() || mode == TransactionMode::VersionChange || stores.is_empty() {
            return Err(IndexedDbError::InvalidState(
                "regular transaction requires a live database and nonempty scope".into(),
            ));
        }
        let metadata = self
            .committed()
            .filter(|metadata| metadata.version != 0)
            .ok_or_else(|| IndexedDbError::InvalidState("database is not initialized".into()))?;
        for name in stores {
            if !metadata.stores.contains_key(name) {
                return Err(IndexedDbError::NotFound(format!(
                    "object store `{name}` was not found"
                )));
            }
        }
        self.admissions.try_enqueue(
            queue_key(self.id),
            connection,
            stores.iter().cloned().collect(),
            mode,
            wake,
        )
    }
    pub fn id(&self) -> DatabaseId {
        self.id
    }
    pub fn identity(&self) -> &DatabaseIdentity {
        &self.identity
    }
    pub fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::Acquire)
    }
    pub fn committed(&self) -> Option<Arc<DatabaseMetadata>> {
        self.state.metadata.read().clone()
    }

    /// The owner may retain additional call-time snapshot bytes in this permit
    /// across prepare/serialize/finish round trips.
    pub fn try_reserve(&self, kind: TaskKind, bytes: usize) -> Result<TaskPermit, DispatchError> {
        if self.is_closed() {
            return Err(DispatchError::Closed);
        }
        Ok(self.executor.try_reserve(kind, bytes)?)
    }

    /// Result bytes are an initial reservation, not a per-request size limit.
    /// Queries grow the same permit before allocation, up to the shared budget.
    pub fn submit(
        &self,
        command: Command,
        result_bytes: usize,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        let bytes = command
            .input_bytes()
            .checked_add(result_bytes)
            .ok_or(DispatchError::Capacity)?;
        let permit = self.try_reserve(command.kind(), bytes)?;
        self.submit_reserved(permit, command, result_bytes, completion)
    }

    pub fn submit_reserved(
        &self,
        permit: TaskPermit,
        command: Command,
        result_bytes: usize,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        if self.is_closed() {
            return Err(DispatchError::Closed);
        }
        if !permit.belongs_to(&self.executor)
            || permit.kind() != command.kind()
            || command
                .input_bytes()
                .checked_add(result_bytes)
                .is_none_or(|bytes| bytes > permit.reserved_bytes())
        {
            return Err(DispatchError::InvalidReservation);
        }
        let id = self.id;
        let generation = self.identity.generation;
        let lane = self.lane.clone();
        let config = self.config;
        let pending = Pending::new(self.state.clone());
        permit.submit(id.0, move |mut permit| {
            let result = lane.lock().run(
                id,
                command,
                &mut ResultBudget {
                    permit: &mut permit,
                    available: result_bytes as u64,
                },
                config,
            );
            drop(pending);
            completion(Completion {
                database: id,
                generation,
                result,
                permit,
            });
        })?;
        Ok(())
    }

    fn submit_metadata(
        &self,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        let permit = self.try_reserve(TaskKind::Data, 1024)?;
        let id = self.id;
        let generation = self.identity.generation;
        let lane = self.lane.clone();
        let config = self.config;
        let pending = Pending::new(self.state.clone());
        permit.submit(id.0, move |permit| {
            let result = {
                let mut lane = lane.lock();
                lane.connect(id, config)
                    .map(|_| CommandResult::Metadata(lane.entries[&id].publish()))
            };
            drop(pending);
            completion(Completion {
                database: id,
                generation,
                result,
                permit,
            });
        })?;
        Ok(())
    }
}

fn same_location(left: &Location, right: &Location) -> bool {
    match (left, right) {
        (Location::Memory, Location::Memory) => true,
        (Location::Disk { path: left, .. }, Location::Disk { path: right, .. }) => left == right,
        _ => false,
    }
}

fn queue_key(id: DatabaseId) -> (String, String) {
    // Registrations share a queue only while they share the same live client.
    // An old close cannot cancel admissions created by a later registration,
    // even when the database name or persistent generation is reused on reopen.
    (id.0.to_string(), String::new())
}

impl Lane {
    fn connect(&mut self, id: DatabaseId, config: ServiceConfig) -> Result<(), IndexedDbError> {
        let entry = self.entries.get(&id).ok_or_else(|| {
            IndexedDbError::InvalidState("stale IndexedDB database client".into())
        })?;
        if entry.state.closed.load(Ordering::Acquire) {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB database is retired".into(),
            ));
        }
        if let Some(error) = &entry.failure {
            return Err(error.clone());
        }
        if entry.database.is_some() {
            return Ok(());
        }
        let opened = self
            .entries
            .values()
            .filter(|entry| entry.database.is_some())
            .count();
        if opened >= config.connections_per_worker {
            let idle = self
                .entries
                .iter()
                .filter(|(key, entry)| {
                    **key != id
                        && matches!(entry.location, Location::Disk { .. })
                        && entry.state.pending.load(Ordering::Acquire) == 0
                        && entry
                            .database
                            .as_ref()
                            .is_some_and(|database| !database.is_transaction_active())
                })
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| *id);
            let Some(idle) = idle else {
                return Err(IndexedDbError::Resource(
                    "all SQLite connections are pinned or hold private in-memory databases".into(),
                ));
            };
            self.entries
                .get_mut(&idle)
                .expect("idle database")
                .database
                .take()
                .expect("connected database")
                .close()?;
        }
        let entry = self.entries.get_mut(&id).expect("registered database");
        let result = (|| {
            let (connection, create) = match &entry.location {
                Location::Memory if !entry.initialized => {
                    (moli_sqlite::open_private_memory(config.connection), true)
                }
                Location::Memory => {
                    return Err(IndexedDbError::InvalidState(
                        "private memory database cannot be recreated".into(),
                    ));
                }
                Location::Disk { path, mode } => {
                    let mode = if entry.initialized {
                        OpenMode::Existing
                    } else {
                        *mode
                    };
                    (
                        moli_sqlite::open_disk(path, mode, config.connection),
                        matches!(mode, OpenMode::Create),
                    )
                }
            };
            let connection = connection.map_err(|error| IndexedDbError::Io(error.to_string()))?;
            if create {
                Database::create(connection, (*entry.identity).clone())
            } else {
                Database::open(connection, (*entry.identity).clone())
            }
        })();
        match result {
            Ok(database) => {
                if let Some(previous) = entry.state.metadata.read().as_ref()
                    && previous.as_ref() != database.committed()
                {
                    let error = corruption("IndexedDB changed outside its database owner");
                    entry.failure = Some(error.clone());
                    return Err(error);
                }
                entry.database = Some(database);
                entry.initialized = true;
                entry.publish();
                Ok(())
            }
            Err(error) => {
                entry.failure = Some(error.clone());
                Err(error)
            }
        }
    }

    fn run(
        &mut self,
        id: DatabaseId,
        command: Command,
        result_budget: &mut dyn ReadBudget,
        config: ServiceConfig,
    ) -> Result<CommandResult, IndexedDbError> {
        if matches!(command, Command::Close) {
            let entry = self.entries.remove(&id).ok_or_else(|| {
                IndexedDbError::InvalidState("stale IndexedDB database client".into())
            })?;
            entry.close(id)?;
            return Ok(CommandResult::Unit);
        }
        self.connect(id, config)?;
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(&id).expect("connected database");
        entry.last_used = self.clock;
        let database = entry.database.as_mut().expect("connected owner");
        let result = match command {
            Command::Begin {
                transaction,
                mode,
                stores,
                version,
                admission,
            } => {
                if mode != TransactionMode::VersionChange {
                    let key = queue_key(id);
                    let Some(request) = admission.as_ref() else {
                        return Err(IndexedDbError::InvalidState(
                            "regular SQL transactions require admission".into(),
                        ));
                    };
                    if !request.belongs_to(&entry.admissions)
                        || !request.matches_key(&key.0, &key.1)
                        || request.mode() != mode
                        || request
                            .store_names()
                            .into_iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            != stores
                        || !request.claim_start()
                    {
                        return Err(IndexedDbError::InvalidState(
                            "invalid or unready transaction admission".into(),
                        ));
                    }
                } else if admission.is_some() {
                    return Err(IndexedDbError::InvalidState(
                        "upgrade transaction cannot reuse regular admission".into(),
                    ));
                }
                match database.begin(transaction, mode, stores, version) {
                    Ok(()) => {
                        entry.admission = admission;
                        Ok(CommandResult::Unit)
                    }
                    Err(error) => {
                        if let Some(admission) = admission {
                            admission.finish();
                        }
                        Err(error)
                    }
                }
            }
            Command::CreateStore {
                transaction,
                name,
                options,
            } => database
                .create_object_store(transaction, &name, options)
                .map(|_| CommandResult::Unit),
            Command::DeleteStore { transaction, name } => database
                .delete_object_store(transaction, &name)
                .map(|_| CommandResult::Unit),
            Command::CreateIndex {
                transaction,
                store,
                name,
                options,
            } => database
                .create_index(transaction, &store, &name, options)
                .map(|_| CommandResult::Unit),
            Command::DeleteIndex {
                transaction,
                store,
                name,
            } => database
                .delete_index(transaction, &store, &name)
                .map(|_| CommandResult::Unit),
            Command::PrepareWrite {
                transaction,
                store,
                add_only,
            } => database
                .prepare_write(transaction, &store, None, add_only)
                .map(CommandResult::Prepared),
            Command::Write {
                transaction,
                store,
                key,
                value,
                indexes,
                add_only,
                quota,
            } => database
                .write(
                    transaction,
                    &store,
                    RecordWrite {
                        key,
                        value,
                        indexes,
                        add_only,
                        quota,
                    },
                )
                .map(CommandResult::Key),
            Command::FinishWrite {
                transaction,
                request,
                value,
                indexes,
                quota,
            } => database
                .finish_write(transaction, request, value, indexes, quota)
                .map(CommandResult::Key),
            Command::CancelWrite {
                transaction,
                request,
            } => database
                .cancel_write(transaction, request)
                .map(|_| CommandResult::Unit),
            Command::BackfillIndex {
                transaction,
                store,
                name,
                primary_key,
                keys,
            } => database
                .backfill_index_record(transaction, &store, &name, &primary_key, keys)
                .map(|_| CommandResult::Unit),
            Command::Query {
                transaction,
                store,
                query,
            } => database
                .query_budgeted(transaction, &store, &query, result_budget)
                .map(CommandResult::Records),
            Command::Count {
                transaction,
                store,
                source,
                range,
            } => database
                .query_count(transaction, &store, &source, &range)
                .map(CommandResult::Count),
            Command::Delete {
                transaction,
                store,
                range,
            } => database
                .delete_range(transaction, &store, &range)
                .map(|_| CommandResult::Unit),
            Command::Commit { transaction } => {
                let result = database.commit(transaction).map(|_| CommandResult::Unit);
                if result.is_err() && database.is_transaction_active() {
                    let _ = database.abort(transaction);
                }
                result
            }
            Command::Abort { transaction } => {
                database.abort(transaction).map(|_| CommandResult::Unit)
            }
            Command::Close => unreachable!("close handled without reopening"),
        };
        if !database.is_transaction_active() {
            // COMMIT publication precedes releasing admission and posting the
            // completion, even if the V8 receiver has already disappeared.
            let metadata = entry.publish();
            entry.finish_admission();
            if result.is_ok() {
                return Ok(match result {
                    Ok(CommandResult::Unit) => CommandResult::Metadata(metadata),
                    Ok(result) => result,
                    Err(_) => unreachable!(),
                });
            }
        }
        result
    }
}

#[cfg(test)]
mod tests;
