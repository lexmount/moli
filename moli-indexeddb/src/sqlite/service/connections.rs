//! Web connections share a physical database owner, but retire independently.
//! A pre-reserved control task makes owner teardown possible under data pressure.

use super::*;
use crate::{
    DatabaseCloseSignal, DatabaseHandle, VersionChangeBatch, VersionChangeCompletion,
    VersionChangeWake, connection_lifecycle::DatabaseConnectionLifecycle,
};

struct ConnectionStatus {
    close_pending: AtomicBool,
    closed: AtomicBool,
    forced_close: DatabaseCloseSignal,
    retire_requested: AtomicBool,
    retirement_in_flight: AtomicBool,
    retirement: Mutex<Option<TaskPermit>>,
    closing_acknowledgements: Mutex<Vec<VersionChangeCompletion>>,
    _reservation: MetadataReservation,
}

pub(super) struct ConnectionRecord {
    status: Arc<ConnectionStatus>,
    lifecycle: DatabaseConnectionLifecycle,
    wake: Option<VersionChangeWake>,
}

impl ConnectionRecord {
    pub(super) fn mark_forced_closed(&mut self) {
        self.wake = self.lifecycle.mark_forced_closed();
    }

    pub(super) fn retire(
        self,
        notifications: &crate::ConnectionNotifications,
        handle: DatabaseHandle,
    ) {
        let retirement = {
            let mut reservation = self.status.retirement.lock();
            self.status.closed.store(true, Ordering::Release);
            reservation.take()
        };
        drop(retirement);
        notifications.unregister(handle);
        let acknowledgements = std::mem::take(&mut *self.status.closing_acknowledgements.lock());
        drop(acknowledgements);
        if let Some(wake) = self.wake {
            wake();
        }
    }
}

/// The last owner releases its connection only after the database worker has
/// rolled back that connection's work. It never closes another connection's SQL
/// transaction, and it cannot delete the logical database or its generation.
#[derive(Clone)]
pub struct DatabaseConnection(Arc<ConnectionOwner>);

pub struct ConnectionNotification {
    pub old_version: u64,
    pub new_version: Option<u64>,
    pub completion: VersionChangeAcknowledgement,
}

/// Retain through the recipient's event microtask checkpoint. If that event
/// already requested close, its acknowledgement follows the worker's close
/// receipt, rather than racing it into a spurious blocked event.
pub struct VersionChangeAcknowledgement {
    status: Arc<ConnectionStatus>,
    completion: Option<VersionChangeCompletion>,
}

impl Drop for VersionChangeAcknowledgement {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            {
                let mut pending = self.status.closing_acknowledgements.lock();
                if self.status.retirement_in_flight.load(Ordering::Acquire)
                    && !self.status.closed.load(Ordering::Acquire)
                {
                    pending.push(completion);
                    return;
                }
            }
            drop(completion);
        }
    }
}

struct ConnectionOwner {
    client: DatabaseClient,
    handle: DatabaseHandle,
    status: Arc<ConnectionStatus>,
}

impl ServiceClient {
    pub fn queue_connection_request(
        &self,
        owner_key: &[u8],
        name: &str,
        wake: crate::ConnectionRequestWake,
    ) -> Result<crate::ConnectionRequestLease, IndexedDbError> {
        let registry = self.registry.lock();
        if registry.closed {
            return Err(IndexedDbError::InvalidState("SQL service is closed".into()));
        }
        registry.check_owner(owner_key)?;
        if owner_key
            .len()
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(name.len()))
            .is_none_or(|bytes| bytes > self.config.admission_bytes)
        {
            return Err(IndexedDbError::Resource(
                "IndexedDB connection request capacity exhausted".into(),
            ));
        }
        let owner = request_owner_key(owner_key)?;
        self.connection_requests.try_enqueue(&owner, name, wake)
    }
}

pub(super) fn request_owner_key(owner_key: &[u8]) -> Result<String, IndexedDbError> {
    let bytes = owner_key
        .len()
        .checked_mul(2)
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or_else(|| {
            IndexedDbError::Resource("IndexedDB connection owner size overflow".into())
        })?;
    // Hex preserves the entire opaque owner key, including binary and NUL.
    let mut owner = String::with_capacity(bytes);
    use std::fmt::Write;
    for byte in owner_key {
        write!(owner, "{byte:02x}").expect("writing to a String");
    }
    Ok(owner)
}

impl DatabaseClient {
    pub fn open_connection(
        &self,
        wake: VersionChangeWake,
    ) -> Result<DatabaseConnection, IndexedDbError> {
        // Include the sparse registry nodes, notification registration and Arc
        // headers. The slot is retained until the worker confirms retirement.
        let bytes = metadata::map_entry_bytes::<DatabaseHandle, ConnectionRecord>() as usize
            + std::mem::size_of::<ConnectionOwner>()
            + std::mem::size_of::<ConnectionStatus>()
            + 1024;
        let reservation = self.try_reserve_metadata(bytes)?;
        let retirement = self
            .executor
            .try_reserve(TaskKind::Control, 512)
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        let _gate = self.state.admission_gate.lock();
        if self.is_closed() {
            return Err(IndexedDbError::InvalidState(
                "SQL database is closed".into(),
            ));
        }
        let raw = self
            .next_connection
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| IndexedDbError::Resource("SQL connection handles exhausted".into()))?
            + 1;
        let handle = DatabaseHandle::from_raw(raw);
        let mut lifecycle = DatabaseConnectionLifecycle::default();
        let status = Arc::new(ConnectionStatus {
            close_pending: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            forced_close: lifecycle.observe(wake.clone()),
            retire_requested: AtomicBool::new(false),
            retirement_in_flight: AtomicBool::new(false),
            retirement: Mutex::new(Some(retirement)),
            closing_acknowledgements: Mutex::new(Vec::new()),
            _reservation: reservation,
        });
        self.state.connections.lock().insert(
            handle,
            ConnectionRecord {
                status: status.clone(),
                lifecycle,
                wake: None,
            },
        );
        self.state
            .notifications
            .register(handle, String::new(), wake);
        Ok(DatabaseConnection(Arc::new(ConnectionOwner {
            client: self.clone(),
            handle,
            status,
        })))
    }

    pub fn has_open_connections(&self) -> bool {
        self.state.notifications.has_connections("")
    }

    /// The caller holds the head open/delete request until this algorithm ends.
    /// Each recipient acknowledges only after its own event and microtasks.
    pub fn begin_version_change(
        &self,
        old_version: u64,
        new_version: Option<u64>,
        wake: VersionChangeWake,
    ) -> VersionChangeBatch {
        self.state
            .notifications
            .begin_version_change("", old_version, new_version, wake)
    }
}

impl DatabaseConnection {
    pub fn handle(&self) -> DatabaseHandle {
        self.0.handle
    }

    pub fn client(&self) -> &DatabaseClient {
        &self.0.client
    }

    pub fn is_closed(&self) -> bool {
        self.0.status.close_pending.load(Ordering::Acquire)
            || self.0.status.closed.load(Ordering::Acquire)
            || self.0.client.is_closed()
    }

    pub fn is_forced_closed(&self) -> bool {
        self.0.status.forced_close.is_forced_closed()
    }

    pub fn close_signal(&self) -> DatabaseCloseSignal {
        self.0.status.forced_close.clone()
    }

    /// Refuse new transactions immediately, retaining all accepted admissions
    /// and keeping the connection visible as an upgrade/delete blocker.
    pub fn mark_close_pending(&self) {
        let _gate = self.0.client.state.admission_gate.lock();
        self.0.status.close_pending.store(true, Ordering::Release);
        self.0
            .client
            .state
            .notifications
            .mark_close_pending(self.0.handle);
    }

    pub fn queue_transaction_start(
        &self,
        stores: &[String],
        mode: TransactionMode,
        wake: crate::TransactionRequestWake,
    ) -> Result<crate::TransactionRequestLease, IndexedDbError> {
        let _gate = self.0.client.state.admission_gate.lock();
        if self.is_closed() {
            return Err(IndexedDbError::InvalidState(
                "SQL connection is closing".into(),
            ));
        }
        self.0
            .client
            .queue_transaction_start_locked(self.0.handle, stores, mode, wake)
    }

    pub fn take_notification(&self) -> Option<ConnectionNotification> {
        let notification = self
            .0
            .client
            .state
            .notifications
            .take_notification(self.0.handle)?;
        Some(ConnectionNotification {
            old_version: notification.old_version,
            new_version: notification.new_version,
            completion: VersionChangeAcknowledgement {
                status: self.0.status.clone(),
                completion: Some(notification.completion),
            },
        })
    }

    /// Stop an owner even when undelivered notification tasks retain aliases.
    /// Forced-close publication still happens only after worker rollback.
    pub fn retire(&self) -> Result<(), DispatchError> {
        self.0
            .status
            .retire_requested
            .store(true, Ordering::Release);
        self.mark_close_pending();
        let permit = self.0.status.retirement.lock().take();
        if let Some(permit) = permit {
            self.0
                .status
                .retirement_in_flight
                .store(true, Ordering::Release);
            retire(
                &self.0.client,
                self.0.handle,
                permit,
                true,
                self.0.status.clone(),
                |_, _| {},
            )?;
        }
        Ok(())
    }

    pub fn begin_upgrade(
        &self,
        transaction: TransactionHandle,
        version: u64,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<(), DispatchError> {
        let command = Command::Begin {
            transaction,
            mode: TransactionMode::VersionChange,
            stores: Default::default(),
            version: Some(version),
            admission: None,
        };
        let permit = self
            .0
            .client
            .try_reserve(command.kind(), command.input_bytes())?;
        let _gate = self.0.client.state.admission_gate.lock();
        if self.is_closed() {
            return Err(DispatchError::Closed);
        }
        self.0.client.submit_reserved_owned(
            permit,
            command,
            0,
            Some(self.0.handle),
            Ok(()),
            completion,
        )
    }

    /// Call after accepted frontend transactions have delivered their terminal
    /// events. A busy connection stays registered; failed close returns its
    /// reserved control slot so teardown and a later retry remain possible.
    pub fn close(
        &self,
        completion: impl FnOnce(Result<(), IndexedDbError>) + Send + 'static,
    ) -> Result<(), DispatchError> {
        self.mark_close_pending();
        let permit = self
            .0
            .status
            .retirement
            .lock()
            .take()
            .ok_or(DispatchError::Closed)?;
        let owner = self.0.clone();
        self.0
            .status
            .retirement_in_flight
            .store(true, Ordering::Release);
        retire(
            &self.0.client,
            self.0.handle,
            permit,
            false,
            self.0.status.clone(),
            move |result, permit| {
                if result.is_err() && !owner.status.closed.load(Ordering::Acquire) {
                    let retry = {
                        let mut retirement = owner.status.retirement.lock();
                        if owner.status.closed.load(Ordering::Acquire) {
                            drop(permit);
                            None
                        } else if owner.status.retire_requested.load(Ordering::Acquire) {
                            Some(permit)
                        } else {
                            *retirement = Some(permit);
                            owner
                                .status
                                .retirement_in_flight
                                .store(false, Ordering::Release);
                            None
                        }
                    };
                    if let Some(permit) = retry {
                        let _ = retire(
                            &owner.client,
                            owner.handle,
                            permit,
                            true,
                            owner.status.clone(),
                            |_, _| {},
                        );
                    } else {
                        let acknowledgements =
                            std::mem::take(&mut *owner.status.closing_acknowledgements.lock());
                        drop(acknowledgements);
                    }
                } else {
                    drop(permit);
                }
                // An internal callback alias must not outlive the receipt:
                // dropping the caller's last connection after a failed close
                // must enqueue retirement before subsequently submitted work.
                drop(owner);
                completion(result);
            },
        )
    }
}

impl Drop for ConnectionOwner {
    fn drop(&mut self) {
        if self.status.closed.load(Ordering::Acquire) {
            return;
        }
        let permit = self.status.retirement.lock().take();
        if let Some(permit) = permit {
            self.status
                .retirement_in_flight
                .store(true, Ordering::Release);
            let _ = retire(
                &self.client,
                self.handle,
                permit,
                true,
                self.status.clone(),
                |_, _| {},
            );
        }
    }
}

fn retire(
    client: &DatabaseClient,
    handle: DatabaseHandle,
    permit: TaskPermit,
    forced: bool,
    status: Arc<ConnectionStatus>,
    completion: impl FnOnce(Result<(), IndexedDbError>, TaskPermit) + Send + 'static,
) -> Result<(), DispatchError> {
    let lane = client.lane.clone();
    let id = client.id;
    // No Data reservation and no synchronous SQL/lane lock on the owner thread.
    permit.submit(id.0, move |permit| {
        let result = lane.lock().retire_connection(
            id,
            handle,
            forced || status.retire_requested.load(Ordering::Acquire),
        );
        completion(result, permit);
    })?;
    Ok(())
}

impl Lane {
    fn retire_connection(
        &mut self,
        id: DatabaseId,
        handle: DatabaseHandle,
        forced: bool,
    ) -> Result<(), IndexedDbError> {
        let Some(entry) = self.entries.get_mut(&id) else {
            // Physical retirement already completed rollback and signalled all
            // connections. This task may have been queued immediately after it.
            return Ok(());
        };
        let owned_transaction = entry.connection == Some(handle);
        if forced
            && owned_transaction
            && let Some(database) = entry.database.as_mut()
            && let Some(transaction) = database.active_transaction()
        {
            if let Err(error) = database.abort(transaction) {
                let entry = self.entries.remove(&id).expect("registered database");
                *entry.state.failure.lock() = Some(error.clone());
                let _ = entry.close(id);
                return Err(error);
            }
            entry.publish();
        }
        if !forced
            && (entry.connection == Some(handle)
                || entry.admissions.has_connection_requests(handle))
        {
            return Err(IndexedDbError::InvalidState(
                "SQL connection still has accepted transactions".into(),
            ));
        }
        let record = {
            let _gate = entry.state.admission_gate.lock();
            entry.state.connections.lock().remove(&handle)
        };
        if let Some(mut record) = record {
            if forced {
                record.mark_forced_closed();
                // Rollback and close signals precede cancellation wakes and
                // the next transaction's admission. Never roll back its owner.
                entry.admissions.cancel_connection(handle);
                if owned_transaction {
                    entry.finish_admission();
                }
            }
            record.retire(&entry.state.notifications, handle);
        }
        Ok(())
    }
}
