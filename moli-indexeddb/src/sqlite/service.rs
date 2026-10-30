//! Native command transport. Connections live exclusively in worker
//! state; a client reads only published metadata and never locks a SQL owner.

mod blob;
mod command;
mod entry;
mod maintenance;
mod owner;
pub use command::Command;
use owner::{DatabaseOwner, Owner};
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
    time::{Duration, Instant},
};

use moli_sqlite::{ConnectionOptions, OpenMode, Runtime, RuntimeError, RuntimeHandle};
use parking_lot::{Mutex, RwLock};

use super::*;
use crate::{RecordSnapshot, TransactionRequestHandle};

#[derive(Debug, Clone, Copy, Default)]
pub struct ServiceConfig {
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
    Runtime(RuntimeError),
    IdentityConflict,
    Capacity,
    Storage(IndexedDbError),
    Closed,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(f),
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
impl From<RuntimeError> for DispatchError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

#[derive(Debug)]
pub enum CommandResult {
    Metadata(Arc<DatabaseMetadata>),
    Connection(DatabaseConnection),
    Prepared(PreparedWrite),
    Key(Key),
    Records(Vec<RecordSnapshot>),
    RecordPage(RecordPage),
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
    owner: Arc<Owner>,
    admissions: Arc<crate::TransactionRequestQueues>,
    catalog: Option<Arc<catalog::DiskCatalog>>,
    next_connection: Arc<AtomicU64>,
}

struct Entry {
    identity: Arc<DatabaseIdentity>,
    location: Location,
    state: Arc<Published>,
    database: Option<Database>,
    blobs: Arc<super::blobs::BlobSnapshots>,
    initialized: bool,
    failure: Option<IndexedDbError>,
    transactions: BTreeMap<TransactionHandle, StartedTransaction>,
    admissions: Arc<crate::TransactionRequestQueues>,
    catalog: Option<Arc<catalog::DiskCatalog>>,
    prepared: bool,
    idle_since: Option<Instant>,
    request_owner: String,
    connection_requests: Arc<crate::ConnectionRequestQueues>,
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

    fn is_idle(&self) -> bool {
        matches!(self.location, Location::Disk { .. })
            && self.database.is_some()
            && self.transactions.is_empty()
            && self.state.connections.lock().is_empty()
            && !self.state.deleting.load(Ordering::Acquire)
            && !self
                .connection_requests
                .has_requests(&self.request_owner, &self.identity.name)
    }

    fn update_idle(&mut self, now: Instant) {
        if self.is_idle() {
            self.idle_since.get_or_insert(now);
        } else {
            self.idle_since = None;
        }
    }

    fn idle_expired(&mut self, now: Instant) -> bool {
        self.update_idle(now);
        self.idle_since
            .is_some_and(|since| now.saturating_duration_since(since) >= maintenance::IDLE_GRACE)
    }

    fn close_if_idle(&mut self, now: Instant) -> Result<(), IndexedDbError> {
        if !self.idle_expired(now) {
            return Ok(());
        }
        let database = self.database.take().expect("idle physical connection");
        self.idle_since = None;
        let result = database.close();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
            *self.state.failure.lock() = Some(error.clone());
        }
        result
    }

    fn close(&mut self, id: DatabaseId, config: ServiceConfig) -> Result<(), IndexedDbError> {
        // An idle physical connection may already be closed. Reopen only when
        // live Blob objects still need an immutable copy before retirement.
        if self.blobs.has_live() {
            if self.database.is_none()
                && let Location::Disk { path, .. } = &self.location
            {
                let reopened = moli_sqlite::open_disk(path, OpenMode::Existing, config.connection)
                    .map_err(|error| IndexedDbError::Io(error.to_string()))
                    .and_then(|connection| {
                        Database::open_shared(connection, self.identity.clone())
                    });
                match reopened {
                    Ok(database) => self.database = Some(database),
                    Err(error) => self.blobs.fail(&error),
                }
            }
            if let Some(database) = &self.database {
                self.blobs.preserve_all(database);
            }
        }
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

#[derive(Default)]
struct Registry {
    closed: bool,
    next_id: u64,
    entries: BTreeMap<(Vec<u8>, crate::IndexedDbName), DatabaseClient>,
    clearing: BTreeMap<clear::ClearScope, clear::ClearingOwner>,
}

pub struct Service {
    runtime: Option<Runtime>,
    maintenance: Option<maintenance::Maintenance>,
    client: ServiceClient,
}

/// A nonblocking namespace handle. Unlike Service, it does not own worker
/// threads and cannot keep storage alive after its partition shuts down.
#[derive(Clone)]
pub struct ServiceClient {
    runtime: RuntimeHandle,
    registry: Arc<Mutex<Registry>>,
    config: ServiceConfig,
    admissions: Arc<crate::TransactionRequestQueues>,
    catalog: Option<Arc<catalog::DiskCatalog>>,
    next_connection: Arc<AtomicU64>,
    connection_requests: Arc<crate::ConnectionRequestQueues>,
}

impl Service {
    pub fn new(config: ServiceConfig) -> Result<Self, DispatchError> {
        let runtime = Runtime::new()?;
        let client = ServiceClient {
            runtime: runtime.handle(),
            registry: Arc::new(Mutex::new(Registry::default())),
            config,
            catalog: None,
            next_connection: Arc::new(AtomicU64::new(0)),
            connection_requests: Arc::new(crate::ConnectionRequestQueues::default()),
            admissions: Arc::new(crate::TransactionRequestQueues::default()),
        };
        let maintenance = maintenance::Maintenance::start(
            client.runtime.clone(),
            Arc::downgrade(&client.registry),
        )?;
        Ok(Self {
            runtime: Some(runtime),
            maintenance: Some(maintenance),
            client,
        })
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
        self.register_client(
            &mut registry,
            key,
            generation,
            location,
            None,
            None,
            completion,
        )
    }

    fn register_client(
        &self,
        registry: &mut Registry,
        key: (Vec<u8>, crate::IndexedDbName),
        generation: [u8; 16],
        location: Location,
        catalog: Option<Arc<catalog::DiskCatalog>>,
        recovered: Option<Arc<DatabaseMetadata>>,
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
        let request_owner =
            connections::request_owner_key(&key.0).map_err(DispatchError::Storage)?;
        let initialized = recovered.is_some();
        let previous = recovered.or_else(|| {
            registry
                .entries
                .get(&key)
                .and_then(|client| client.committed())
        });
        let state = Arc::new(Published {
            metadata: RwLock::new(previous),
            ..Default::default()
        });
        let owner = Owner::start(
            self.runtime.clone(),
            id,
            self.config,
            state.clone(),
            |owner| Entry {
                identity: identity.clone(),
                location: location.clone(),
                state: state.clone(),
                database: None,
                blobs: super::blobs::BlobSnapshots::new(Arc::new(blob::DatabaseBlobReader {
                    owner,
                    runtime: self.runtime.clone(),
                })),
                initialized,
                idle_since: None,
                request_owner,
                connection_requests: self.connection_requests.clone(),
                failure: None,
                transactions: BTreeMap::new(),
                admissions: self.admissions.clone(),
                catalog: catalog.clone(),
                prepared: !matches!(
                    location,
                    Location::Disk {
                        mode: OpenMode::Create,
                        ..
                    }
                ),
            },
        )?;
        let client = DatabaseClient {
            id,
            identity,
            location,
            state,
            owner,
            admissions: self.admissions.clone(),
            catalog,
            next_connection: self.next_connection.clone(),
        };
        client.submit_metadata(completion)?;
        registry.entries.insert(key, client.clone());
        Ok(client)
    }
}

impl Service {
    /// Native host boundary, off V8 and off this runtime. Seal every database
    /// FIFO, drain accepted work and final rollback/close, then join the runtime.
    pub fn shutdown(&mut self) -> Result<(), IndexedDbError> {
        if self.client.runtime.is_current() {
            return Err(IndexedDbError::InvalidState(
                "cannot shut down SQLite from its own runtime".into(),
            ));
        }
        let Some(mut runtime) = self.runtime.take() else {
            return Ok(());
        };
        let maintained = if let Some(maintenance) = self.maintenance.take() {
            maintenance.stop(&runtime.handle())
        } else {
            Ok(())
        };
        let owners = {
            let mut registry = self.registry.lock();
            registry.closed = true;
            registry
                .entries
                .values()
                .map(|client| client.owner.clone())
                .collect::<Vec<_>>()
        };
        let receipts = owners.iter().map(|owner| owner.stop()).collect::<Vec<_>>();
        let drained = runtime
            .handle()
            .wait(async move {
                let mut failure = None;
                for mut receipt in receipts {
                    let result = receipt
                        .wait_for(|result| result.is_some())
                        .await
                        .map_err(|_| {
                            IndexedDbError::Io(
                                "SQLite owner stopped before its close receipt".into(),
                            )
                        })
                        .and_then(|result| result.as_ref().expect("completed owner").clone());
                    if let Err(error) = result {
                        failure.get_or_insert(error);
                    }
                }
                failure.map_or(Ok(()), Err)
            })
            .map_err(|error| IndexedDbError::Io(error.to_string()))
            .and_then(|result| result);
        let stopped = runtime
            .shutdown()
            .map_err(|error| IndexedDbError::Io(error.to_string()));
        self.connection_requests.shutdown();
        maintained.and(drained).and(stopped)
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
        stores: &[crate::IndexedDbName],
        mode: TransactionMode,
        wake: crate::TransactionRequestWake,
    ) -> Result<crate::TransactionRequestLease, IndexedDbError> {
        let _gate = self.state.admission_gate.lock();
        self.queue_transaction_start_locked(connection, stores, mode, wake)
    }

    fn queue_transaction_start_locked(
        &self,
        connection: crate::DatabaseHandle,
        stores: &[crate::IndexedDbName],
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
    /// SQLite ownership has ended. The owner receipt also waits for any
    /// remaining catalog work and completion callbacks in its terminal command.
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
        let submitted = self.owner.submit(
            move |owner| Ok(owner.run(command, connection, preflight)),
            move |result| {
                let (result, active_transaction) =
                    result.unwrap_or_else(|error| (Err(error), None));
                completion(Completion {
                    database: id,
                    generation,
                    active_transaction,
                    result,
                });
            },
        );
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
        self.owner.submit(
            |owner| {
                let config = owner.config;
                let result = owner.entry()?.metadata(config);
                let active = owner
                    .entry
                    .as_ref()
                    .and_then(|entry| entry.database.as_ref())
                    .and_then(Database::active_transaction);
                Ok((result, active))
            },
            move |result| {
                let (result, active_transaction) =
                    result.unwrap_or_else(|error| (Err(error), None));
                completion(Completion {
                    database: id,
                    generation,
                    active_transaction,
                    result,
                });
            },
        )?;
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

#[cfg(test)]
mod tests;
