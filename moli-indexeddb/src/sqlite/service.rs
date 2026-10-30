//! Native command transport. Connections live exclusively in worker
//! state; a client reads only published metadata and never locks a SQL owner.

mod command;
pub use command::Command;
mod catalog;
pub use catalog::CatalogSnapshot;
mod connections;
pub use connections::{
    ConnectionNotification, DatabaseConnection, VersionChangeAcknowledgement,
    WeakDatabaseConnection,
};
mod clear;
pub use clear::{OwnerClear, OwnerClearRequest};

use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use moli_sqlite::{
    ConnectionOptions, Executor, ExecutorConfig, ExecutorError, ExecutorHandle, OpenMode,
};
use parking_lot::{Mutex, RwLock};

use super::*;
use crate::{RecordSnapshot, TransactionRequestHandle};

#[derive(Debug, Clone, Copy, Default)]
pub struct ServiceConfig {
    pub executor: ExecutorConfig,
    pub connection: ConnectionOptions,
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
    IdentityConflict,
    Capacity,
    Storage(IndexedDbError),
    Closed,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executor(error) => error.fmt(f),
            Self::IdentityConflict => {
                f.write_str("IndexedDB identity or path is already registered")
            }
            Self::Capacity => f.write_str("IndexedDB size or identifier range exhausted"),
            Self::Storage(error) => error.fmt(f),
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
    /// This command's transaction, if it remains active after reconciliation.
    /// Other disjoint readers/writers may still be active on the connection.
    pub active_transaction: Option<TransactionHandle>,
    pub result: Result<CommandResult, IndexedDbError>,
}

#[derive(Default)]
struct Published {
    closed: AtomicBool,
    deleting: AtomicBool,
    deleted: AtomicBool,
    failure: Mutex<Option<IndexedDbError>>,
    next_transaction: AtomicU64,
    /// Orders admission registration against retirement, without locking SQL.
    admission_gate: Mutex<()>,
    metadata: RwLock<Option<Arc<DatabaseMetadata>>>,
    connections: Mutex<BTreeMap<crate::DatabaseHandle, connections::ConnectionRecord>>,
    notifications: crate::ConnectionNotifications,
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
    catalog: Option<Arc<catalog::DiskCatalog>>,
    next_connection: Arc<AtomicU64>,
}

struct Entry {
    identity: Arc<DatabaseIdentity>,
    location: Location,
    state: Arc<Published>,
    database: Option<Database>,
    initialized: bool,
    failure: Option<IndexedDbError>,
    transactions: BTreeMap<TransactionHandle, StartedTransaction>,
    admissions: Arc<crate::TransactionRequestQueues>,
    catalog: Option<Arc<catalog::DiskCatalog>>,
    prepared: bool,
}

struct StartedTransaction {
    connection: Option<crate::DatabaseHandle>,
    admission: Option<TransactionRequestHandle>,
}

impl Entry {
    fn finish_admission(&mut self, transaction: TransactionHandle) {
        if let Some(started) = self.transactions.remove(&transaction)
            && let Some(admission) = started.admission
        {
            admission.finish();
        }
    }

    fn publish(&self) -> Arc<DatabaseMetadata> {
        let committed = self.database.as_ref().expect("connected owner").committed();
        if let Some(published) = self.state.metadata.read().as_ref()
            && Arc::ptr_eq(published, committed)
        {
            return published.clone();
        }
        let metadata = committed.clone();
        let previous = self.state.metadata.write().replace(metadata.clone());
        // Neither cloning nor destroying schema maps holds the publication lock.
        drop(previous);
        metadata
    }

    fn close(mut self, id: DatabaseId) -> Result<(), IndexedDbError> {
        let result = self.database.take().map_or(Ok(()), Database::close);
        if let Err(error) = &result {
            *self.state.failure.lock() = Some(error.clone());
        }
        // Rollback/close precede both admission release and closure publication.
        let mut connections = {
            let _gate = self.state.admission_gate.lock();
            self.state.closed.store(true, Ordering::Release);
            std::mem::take(&mut *self.state.connections.lock())
        };
        self.transactions.clear();
        for connection in connections.values_mut() {
            connection.mark_forced_closed();
        }
        let key = queue_key(id);
        self.admissions.cancel_database(&key.0, &key.1);
        for (handle, connection) in connections {
            connection.retire(&self.state.notifications, handle);
        }
        result
    }
}

struct Lane {
    entries: BTreeMap<DatabaseId, Entry>,
}

#[derive(Default)]
struct Registry {
    closed: bool,
    next_id: u64,
    entries: BTreeMap<(Vec<u8>, String), DatabaseClient>,
    clearing: BTreeMap<clear::ClearScope, clear::ClearingOwner>,
}

pub struct Service {
    executor: Executor,
    client: ServiceClient,
}

/// A nonblocking namespace handle. Unlike Service, it does not own worker
/// threads and cannot keep storage alive after its partition shuts down.
#[derive(Clone)]
pub struct ServiceClient {
    executor: ExecutorHandle,
    registry: Arc<Mutex<Registry>>,
    lanes: Arc<[Arc<Mutex<Lane>>]>,
    config: ServiceConfig,
    admissions: Arc<crate::TransactionRequestQueues>,
    catalog: Option<Arc<catalog::DiskCatalog>>,
    next_connection: Arc<AtomicU64>,
    connection_requests: Arc<crate::ConnectionRequestQueues>,
}

impl Service {
    pub fn new(config: ServiceConfig) -> Result<Self, DispatchError> {
        let executor = Executor::new(config.executor)?;
        let lanes = (0..config.executor.workers)
            .map(|_| {
                Arc::new(Mutex::new(Lane {
                    entries: BTreeMap::new(),
                }))
            })
            .collect::<Vec<_>>()
            .into();
        let client = ServiceClient {
            executor: executor.handle(),
            registry: Arc::new(Mutex::new(Registry::default())),
            lanes,
            config,
            catalog: None,
            next_connection: Arc::new(AtomicU64::new(0)),
            connection_requests: Arc::new(crate::ConnectionRequestQueues::default()),
            admissions: Arc::new(crate::TransactionRequestQueues::default()),
        };
        Ok(Self { executor, client })
    }

    pub fn client(&self) -> ServiceClient {
        self.client.clone()
    }
}

impl std::ops::Deref for Service {
    type Target = ServiceClient;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl ServiceClient {
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
        registry.reap_closed();
        // Move the caller's input into the lookup key, avoiding allocation even
        // when reopening a registered database.
        let DatabaseIdentity {
            owner_key,
            name,
            generation,
        } = identity;
        let key = (owner_key, name);
        registry
            .check_owner(&key.0)
            .map_err(DispatchError::Storage)?;
        if let Some(existing) = registry.entries.get(&key) {
            if existing.identity.generation != generation
                || !same_location(&existing.location, &location)
            {
                return Err(DispatchError::IdentityConflict);
            }
            if existing.is_closed() {
                return Err(DispatchError::Closed);
            }
            let client = existing.clone();
            client.submit_metadata(completion)?;
            return Ok(client);
        }
        self.register_client(&mut registry, key, generation, location, None, completion)
    }

    fn register_client(
        &self,
        registry: &mut Registry,
        key: (Vec<u8>, String),
        generation: [u8; 16],
        location: Location,
        catalog: Option<Arc<catalog::DiskCatalog>>,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<DatabaseClient, DispatchError> {
        if let Location::Disk { path, .. } = &location && registry.entries.iter().any(|(other_key, client)| other_key != &key && matches!(&client.location, Location::Disk { path: other, .. } if other == path)) { return Err(DispatchError::IdentityConflict); }
        let identity = Arc::new(DatabaseIdentity {
            owner_key: key.0.clone(),
            name: key.1.clone(),
            generation,
        });
        registry.next_id = registry
            .next_id
            .checked_add(1)
            .ok_or(DispatchError::Capacity)?;
        let id = DatabaseId(registry.next_id);
        let index = (id.0 % self.config.executor.workers as u64) as usize;
        let previous = registry
            .entries
            .get(&key)
            .and_then(|client| client.committed());
        let client = DatabaseClient {
            id,
            identity,
            location,
            state: Arc::new(Published {
                metadata: RwLock::new(previous),
                ..Default::default()
            }),
            executor: self.executor.clone(),
            lane: self.lanes[index].clone(),
            config: self.config,
            admissions: self.admissions.clone(),
            catalog: catalog.clone(),
            next_connection: self.next_connection.clone(),
        };
        let entry = Entry {
            identity: client.identity.clone(),
            location: client.location.clone(),
            state: client.state.clone(),
            database: None,
            initialized: false,
            failure: None,
            transactions: BTreeMap::new(),
            admissions: self.admissions.clone(),
            catalog,
            prepared: !matches!(
                client.location,
                Location::Disk {
                    mode: OpenMode::Create,
                    ..
                }
            ),
        };
        let lane = client.lane.clone();
        let config = client.config;
        let generation = client.identity.generation;
        client.executor.submit(id.0, move || {
            let result = {
                let mut lane = lane.lock();
                lane.entries.insert(id, entry);
                lane.connect(id, config)
                    .map(|_| CommandResult::Metadata(lane.entries[&id].publish()))
            };
            completion(Completion {
                database: id,
                generation,
                active_transaction: None,
                result,
            });
        })?;
        registry.entries.insert(key, client.clone());
        Ok(client)
    }
}

impl Service {
    /// Call off the V8 owner thread. Each worker finalizes its connections after
    /// accepted commands, independently of retained result data.
    pub fn shutdown(&mut self) -> Result<(), IndexedDbError> {
        self.registry.lock().closed = true;
        let lanes = self.lanes.clone();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let failures = errors.clone();
        let close_lane = Arc::new(move |index: usize| {
            let entries = std::mem::take(&mut lanes[index].lock().entries);
            for (id, entry) in entries {
                if let Err(error) = entry.close(id) {
                    failures.lock().push(error);
                }
            }
        });
        let finalize = close_lane.clone();
        let stopped = self
            .executor
            .shutdown_with(move |index| finalize(index))
            .map_err(|error| IndexedDbError::Io(error.to_string()));
        if stopped.is_err() {
            // In an unwind build a failed worker cannot run its finalizer.
            // All workers have joined, so shutdown's off-V8 thread can safely
            // close the remaining owners even when old clients retain lanes.
            for index in 0..self.lanes.len() {
                close_lane(index);
            }
        }
        // All workers have joined even if one panicked. Pending factory
        // requests still need a terminal wake on that error path.
        self.connection_requests.shutdown();
        stopped?;
        let error = errors.lock().pop();
        error.map_or(Ok(()), Err)
    }
}

impl ServiceClient {
    pub fn is_closed(&self) -> bool {
        self.registry.lock().closed
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
        self.queue_transaction_start_locked(connection, stores, mode, wake)
    }

    fn queue_transaction_start_locked(
        &self,
        connection: crate::DatabaseHandle,
        stores: &[String],
        mode: TransactionMode,
        wake: crate::TransactionRequestWake,
    ) -> Result<crate::TransactionRequestLease, IndexedDbError> {
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
        Ok(self.admissions.enqueue(
            queue_key(self.id),
            connection,
            stores.iter().cloned().collect(),
            mode,
            wake,
        ))
    }
    pub fn id(&self) -> DatabaseId {
        self.id
    }
    pub fn identity(&self) -> &DatabaseIdentity {
        &self.identity
    }
    pub fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::Acquire) || self.state.deleting.load(Ordering::Acquire)
    }
    /// Unlike a pending deletion, retirement proves worker rollback and close.
    pub fn is_retired(&self) -> bool {
        self.state.closed.load(Ordering::Acquire)
    }
    pub fn committed(&self) -> Option<Arc<DatabaseMetadata>> {
        self.state.metadata.read().clone()
    }

    pub fn submit(
        &self,
        command: Command,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        self.submit_checked(command, Ok(()), completion)
    }

    /// A failed external preflight is reconciled on the SQL worker too: an
    /// inline-key SAVEPOINT must roll back before the request error is delivered.
    pub fn submit_checked(
        &self,
        command: Command,
        preflight: Result<(), IndexedDbError>,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        if preflight.is_err()
            && !matches!(
                command,
                Command::Write { .. } | Command::FinishWrite { .. } | Command::Commit { .. }
            )
        {
            return Err(DispatchError::Storage(IndexedDbError::InvalidState(
                "unsupported SQL preflight rejection".into(),
            )));
        }
        self.submit_owned(command, None, preflight, completion)
    }

    fn submit_owned(
        &self,
        command: Command,
        connection: Option<crate::DatabaseHandle>,
        preflight: Result<(), IndexedDbError>,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        let deleting = matches!(command, Command::DeleteDatabase);
        let _gate = deleting.then(|| self.state.admission_gate.lock());
        if self.is_closed() {
            return Err(DispatchError::Closed);
        }
        if deleting && self.catalog.is_none() && matches!(self.location, Location::Disk { .. }) {
            return Err(DispatchError::Storage(IndexedDbError::InvalidState(
                "only catalog generations can be deleted".into(),
            )));
        }
        if deleting {
            self.state.deleting.store(true, Ordering::Release);
        }
        let id = self.id;
        let generation = self.identity.generation;
        let lane = self.lane.clone();
        let config = self.config;
        let submitted = self.executor.submit(id.0, move || {
            let (result, active_transaction) =
                lane.lock().run(id, command, config, connection, preflight);
            completion(Completion {
                database: id,
                generation,
                active_transaction,
                result,
            });
        });
        if submitted.is_err() && deleting {
            self.state.deleting.store(false, Ordering::Release);
        }
        submitted?;
        Ok(())
    }

    fn submit_metadata(
        &self,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        let id = self.id;
        let generation = self.identity.generation;
        let lane = self.lane.clone();
        let config = self.config;
        self.executor.submit(id.0, move || {
            let (result, active_transaction) = {
                let mut lane = lane.lock();
                let result = lane
                    .connect(id, config)
                    .map(|_| CommandResult::Metadata(lane.entries[&id].publish()));
                let active = lane
                    .entries
                    .get(&id)
                    .and_then(|entry| entry.database.as_ref())
                    .and_then(Database::active_transaction);
                (result, active)
            };
            completion(Completion {
                database: id,
                generation,
                active_transaction,
                result,
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
                    if let Some(catalog) = &entry.catalog
                        && !entry.prepared
                    {
                        catalog.prepare(path, &entry.identity)?;
                        entry.prepared = true;
                    }
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
            let mut connection =
                connection.map_err(|error| IndexedDbError::Io(error.to_string()))?;
            if create {
                Database::initialize(&mut connection, &entry.identity)?;
                // Loading schema can fail after the creating schema was
                // committed. A retry must open that disk generation, not try
                // to create a second schema over it. Unpublished private memory
                // is disposable until its first successful open.
                if matches!(entry.location, Location::Disk { .. }) {
                    entry.initialized = true;
                }
                if let (Some(catalog), Location::Disk { path, .. }) =
                    (&entry.catalog, &entry.location)
                {
                    catalog.finish_preparation(path, &connection)?;
                }
            }
            Database::open_shared(connection, entry.identity.clone())
        })();
        match result {
            Ok(database) => {
                if let Some(previous) = entry.state.metadata.read().as_ref()
                    && previous.as_ref() != database.committed().as_ref()
                {
                    let error = corruption("IndexedDB changed outside its database owner");
                    entry.failure = Some(error.clone());
                    *entry.state.failure.lock() = Some(error.clone());
                    return Err(error);
                }
                entry.database = Some(database);
                entry.initialized = true;
                entry.publish();
                Ok(())
            }
            Err(error) => {
                if !matches!(error, IndexedDbError::Resource(_)) {
                    entry.failure = Some(error.clone());
                    *entry.state.failure.lock() = Some(error.clone());
                }
                Err(error)
            }
        }
    }

    fn run(
        &mut self,
        id: DatabaseId,
        command: Command,
        config: ServiceConfig,
        connection: Option<crate::DatabaseHandle>,
        preflight: Result<(), IndexedDbError>,
    ) -> (
        Result<CommandResult, IndexedDbError>,
        Option<TransactionHandle>,
    ) {
        let transaction = command.transaction();
        let ends_transaction = matches!(
            command,
            Command::Commit { .. }
                | Command::Abort { .. }
                | Command::Close
                | Command::DeleteDatabase
        );
        let mut result = self.run_command(id, command, config, connection, preflight);
        let Some(database) = self
            .entries
            .get_mut(&id)
            .and_then(|entry| entry.database.as_mut())
        else {
            return (result, None);
        };
        match database.reconcile_transaction() {
            Ok(_) => (),
            Err(error) => {
                // No owned transaction remains, but SQLite could not confirm
                // rollback. Closing on this worker makes the outcome terminal;
                // never hand the same unowned connection to a new transaction.
                let entry = self.entries.remove(&id).expect("connected entry");
                *entry.state.failure.lock() = Some(error.clone());
                let _ = entry.close(id);
                return (Err(error), None);
            }
        };
        let entry = self.entries.get_mut(&id).expect("reconciled entry");
        let database = entry.database.as_ref().expect("reconciled owner");
        let active = transaction.filter(|handle| database.transaction_is_active(*handle));
        let finished: Vec<_> = entry
            .transactions
            .keys()
            .copied()
            .filter(|handle| !database.transaction_is_active(*handle))
            .collect();
        if !ends_transaction && transaction.is_some() && active != transaction && result.is_ok() {
            result = Err(IndexedDbError::TransactionInactive(
                "SQL transaction ended while executing a request".into(),
            ));
        }
        // Publish a writer's committed state before releasing any admissions.
        let metadata = entry.publish();
        for handle in finished {
            entry.finish_admission(handle);
        }
        if active.is_none() && matches!(result, Ok(CommandResult::Unit)) {
            result = Ok(CommandResult::Metadata(metadata));
        }
        (result, active)
    }

    fn run_command(
        &mut self,
        id: DatabaseId,
        command: Command,
        config: ServiceConfig,
        connection: Option<crate::DatabaseHandle>,
        preflight: Result<(), IndexedDbError>,
    ) -> Result<CommandResult, IndexedDbError> {
        if matches!(command, Command::Close | Command::DeleteDatabase) {
            let deleting = matches!(command, Command::DeleteDatabase);
            let entry = self.entries.remove(&id).ok_or_else(|| {
                IndexedDbError::InvalidState("stale IndexedDB database client".into())
            })?;
            let state = entry.state.clone();
            let catalog = entry.catalog.clone();
            let location = entry.location.clone();
            let result = entry.close(id).and_then(|()| {
                if deleting
                    && let (Some(catalog), Location::Disk { path, .. }) = (catalog, location)
                {
                    catalog.retire(&path)?;
                }
                Ok(())
            });
            if deleting {
                match &result {
                    Ok(()) => state.deleted.store(true, Ordering::Release),
                    Err(error) => *state.failure.lock() = Some(error.clone()),
                }
            }
            result?;
            return Ok(CommandResult::Unit);
        }
        self.connect(id, config)?;
        let entry = self.entries.get_mut(&id).expect("connected database");
        let database = entry.database.as_mut().expect("connected owner");
        if let Err(error) = preflight {
            match command {
                Command::Write { transaction, .. } => {
                    database.ready(transaction)?;
                }
                Command::FinishWrite {
                    transaction,
                    request,
                    ..
                } => {
                    database.cancel_write(transaction, request)?;
                }
                Command::Commit { transaction, .. } => {
                    database.abort(transaction)?;
                }
                _ => {
                    return Err(IndexedDbError::InvalidState(
                        "unsupported SQL preflight rejection".into(),
                    ));
                }
            }
            return Err(error);
        }
        match command {
            Command::Begin {
                transaction,
                mode,
                stores,
                version,
                admission,
            } => {
                if let Some(connection) = connection
                    && !entry.state.connections.lock().contains_key(&connection)
                {
                    return Err(IndexedDbError::InvalidState(
                        "SQL connection is closed".into(),
                    ));
                }
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
                        entry.transactions.insert(
                            transaction,
                            StartedTransaction {
                                connection: connection.or_else(|| {
                                    admission.as_ref().map(TransactionRequestHandle::database)
                                }),
                                admission,
                            },
                        );
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
            Command::RenameStore {
                transaction,
                name,
                new_name,
            } => database
                .rename_object_store(transaction, &name, &new_name)
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
            Command::RenameIndex {
                transaction,
                store,
                name,
                new_name,
            } => database
                .rename_index(transaction, &store, &name, &new_name)
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
                .query(transaction, &store, &query)
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
            Command::Commit { transaction, quota } => {
                let result = database
                    .commit_with_quota(transaction, quota)
                    .map(|_| CommandResult::Unit);
                if result.is_err() && database.is_transaction_active() {
                    let _ = database.abort(transaction);
                }
                result
            }
            Command::Abort { transaction } => {
                database.abort(transaction).map(|_| CommandResult::Unit)
            }
            Command::Close | Command::DeleteDatabase => {
                unreachable!("close handled without reopening")
            }
        }
    }
}

#[cfg(test)]
mod tests;
