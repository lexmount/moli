//! A database owns its state, not a thread. One mailbox serializes all access.

use std::panic::{AssertUnwindSafe, catch_unwind};
use tokio::sync::{mpsc, oneshot, watch};

use super::*;

type Work = Box<dyn Job>;
pub(super) type CloseReceipt = watch::Receiver<Option<Result<(), IndexedDbError>>>;

enum Message {
    Run(Work),
    Maintain {
        now: Instant,
        completion: oneshot::Sender<Result<(), IndexedDbError>>,
    },
    Stop,
}

pub(super) struct Owner {
    sender: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    state: Arc<Published>,
    completion: CloseReceipt,
    #[cfg(test)]
    maintenance_submitted: AtomicU64,
}

/// Moves into a blocking task for exactly one command, then returns to its
/// async mailbox. No connection borrow survives that task.
pub(super) struct DatabaseOwner {
    pub(super) id: DatabaseId,
    pub(super) config: ServiceConfig,
    pub(super) entry: Option<Entry>,
    state: Arc<Published>,
    terminal_failure: Option<IndexedDbError>,
}

trait Job: Send {
    fn execute(self: Box<Self>, owner: &mut DatabaseOwner) -> bool;
    fn reject(self: Box<Self>, error: IndexedDbError);
}

struct Task<F, C> {
    run: F,
    completion: C,
}

impl<F, C, T> Job for Task<F, C>
where
    F: FnOnce(&mut DatabaseOwner) -> Result<T, IndexedDbError> + Send,
    C: FnOnce(Result<T, IndexedDbError>) + Send,
{
    fn execute(self: Box<Self>, owner: &mut DatabaseOwner) -> bool {
        let Self { run, completion } = *self;
        let mut failed = false;
        let result = match catch_unwind(AssertUnwindSafe(|| run(owner))) {
            Ok(result) => result,
            Err(_) => {
                failed = true;
                Err(owner.fail("IndexedDB database command panicked"))
            }
        };
        if let Some(entry) = owner.entry.as_mut() {
            entry.update_idle(Instant::now());
        }
        if catch_unwind(AssertUnwindSafe(|| completion(result))).is_err() {
            owner.fail("IndexedDB completion callback panicked");
            failed = true;
        }
        failed
    }

    fn reject(self: Box<Self>, error: IndexedDbError) {
        let _ = catch_unwind(AssertUnwindSafe(|| (self.completion)(Err(error))));
    }
}

impl Owner {
    pub(super) fn start(
        runtime: RuntimeHandle,
        id: DatabaseId,
        config: ServiceConfig,
        state: Arc<Published>,
        entry: impl FnOnce(std::sync::Weak<Self>) -> Entry,
    ) -> Result<Arc<Self>, DispatchError> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (completion, receipt) = watch::channel(None);
        let owner = Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            state: state.clone(),
            completion: receipt,
            #[cfg(test)]
            maintenance_submitted: AtomicU64::new(0),
        });
        let database = DatabaseOwner {
            id,
            config,
            entry: Some(entry(Arc::downgrade(&owner))),
            state,
            terminal_failure: None,
        };
        let endpoint = Arc::downgrade(&owner);
        let worker = runtime.clone();
        runtime.spawn(async move {
            run(database, receiver, endpoint, worker, completion).await;
        })?;
        Ok(owner)
    }

    pub(super) fn submit<T: Send + 'static>(
        &self,
        run: impl FnOnce(&mut DatabaseOwner) -> Result<T, IndexedDbError> + Send + 'static,
        completion: impl FnOnce(Result<T, IndexedDbError>) + Send + 'static,
    ) -> Result<(), DispatchError> {
        let sent = {
            let sender = self.sender.lock();
            sender
                .as_ref()
                .ok_or(DispatchError::Closed)?
                .send(Message::Run(Box::new(Task { run, completion })))
        };
        // A rejected closure may own the last logical connection and enqueue
        // its retirement on Drop. Destroy it after releasing the submit gate.
        sent.map_err(|_| DispatchError::Closed)
    }

    pub(super) fn maintain(
        &self,
        now: Instant,
    ) -> Result<oneshot::Receiver<Result<(), IndexedDbError>>, DispatchError> {
        #[cfg(test)]
        self.maintenance_submitted.fetch_add(1, Ordering::Release);
        let (completion, receipt) = oneshot::channel();
        let sent = {
            let sender = self.sender.lock();
            sender
                .as_ref()
                .ok_or(DispatchError::Closed)?
                .send(Message::Maintain { now, completion })
        };
        sent.map_err(|_| DispatchError::Closed)?;
        Ok(receipt)
    }

    pub(super) fn stop(&self) -> CloseReceipt {
        // The single final receipt describes owner cleanup, independently of
        // already-delivered request errors such as a failed trash rename.
        let sender = self.sender.lock().take();
        if let Some(sender) = sender {
            let _ = sender.send(Message::Stop);
        }
        self.completion.clone()
    }

    #[cfg(test)]
    pub(super) fn maintenance_submitted(&self) -> u64 {
        self.maintenance_submitted.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn is_stopping(&self) -> bool {
        self.sender.lock().is_none()
    }
}

impl DatabaseOwner {
    pub(super) fn entry(&mut self) -> Result<&mut Entry, IndexedDbError> {
        self.entry
            .as_mut()
            .ok_or_else(|| IndexedDbError::InvalidState("stale IndexedDB database client".into()))
    }

    pub(super) fn close(&mut self) -> Result<(), IndexedDbError> {
        let result = self
            .entry
            .as_mut()
            .map_or(Ok(()), |entry| entry.close(self.id, self.config));
        self.entry = None;
        if let Err(error) = &result {
            self.terminal_failure.get_or_insert(error.clone());
        }
        result
    }

    fn fail(&mut self, message: &str) -> IndexedDbError {
        let error = IndexedDbError::Io(message.into());
        *self.state.failure.lock() = Some(error.clone());
        self.terminal_failure.get_or_insert(error.clone());
        let _ = catch_unwind(AssertUnwindSafe(|| self.close()));
        error
    }
}

async fn run(
    mut database: DatabaseOwner,
    mut receiver: mpsc::UnboundedReceiver<Message>,
    endpoint: std::sync::Weak<Owner>,
    runtime: RuntimeHandle,
    completion: watch::Sender<Option<Result<(), IndexedDbError>>>,
) {
    let state = database.state.clone();
    let entry = database.entry.as_ref().expect("initial database owner");
    let admissions = entry.admissions.clone();
    let blobs = entry.blobs.clone();
    let id = database.id;
    while let Some(message) = receiver.recv().await {
        let message = match message {
            Message::Maintain { now, completion } => {
                // The owner can inspect its own idle state without any SQL.
                // Do not allocate a blocking task for an already-closed file.
                if database
                    .entry
                    .as_mut()
                    .is_none_or(|entry| !entry.idle_expired(now))
                {
                    let _ = completion.send(Ok(()));
                    continue;
                }
                Message::Run(Box::new(Task {
                    run: move |owner: &mut DatabaseOwner| owner.entry()?.close_if_idle(now),
                    completion: move |result| {
                        let _ = completion.send(result);
                    },
                }))
            }
            message => message,
        };
        let result = runtime
            .blocking(move || {
                let stop = match catch_unwind(AssertUnwindSafe(|| match message {
                    Message::Run(job) => job.execute(&mut database) || database.entry.is_none(),
                    Message::Maintain { .. } => {
                        unreachable!("maintenance was resolved before blocking")
                    }
                    Message::Stop => {
                        let _ = database.close();
                        true
                    }
                })) {
                    Ok(stop) => stop,
                    Err(_) => {
                        database.fail("IndexedDB owner cleanup panicked");
                        true
                    }
                };
                (database, stop)
            })
            .await;
        match result {
            Ok((returned, stop)) => {
                database = returned;
                if stop {
                    break;
                }
            }
            Err(error) => {
                // Command and callback panics are caught on the blocking
                // worker. A JoinError is terminal too; never replay a command.
                let failure = IndexedDbError::Io(error.to_string());
                let failed = failure.clone();
                let _ = runtime
                    .blocking(move || {
                        // JoinError means the blocking closure and its connection
                        // have already dropped. Retire published handles even
                        // though its exclusive Entry could not be returned.
                        blobs.fail(&failed);
                        *state.failure.lock() = Some(failed);
                        let mut connections = {
                            let _gate = state.admission_gate.lock();
                            state.closed.store(true, Ordering::Release);
                            std::mem::take(&mut *state.connections.lock())
                        };
                        for connection in connections.values_mut() {
                            connection.mark_forced_closed();
                        }
                        let key = queue_key(id);
                        admissions.cancel_database(&key.0, &key.1);
                        for (handle, connection) in connections {
                            connection.retire(&state.notifications, handle);
                        }
                    })
                    .await;
                finish_queue(receiver, endpoint, &runtime).await;
                let _ = completion.send(Some(Err(failure)));
                return;
            }
        }
    }
    let result = if database.entry.is_some() {
        runtime
            .blocking(move || {
                if catch_unwind(AssertUnwindSafe(|| database.close())).is_err() {
                    database.fail("IndexedDB owner cleanup panicked");
                }
                database.terminal_failure.map_or(Ok(()), Err)
            })
            .await
            .map_err(|error| IndexedDbError::Io(error.to_string()))
            .and_then(|result| result)
    } else {
        database.terminal_failure.map_or(Ok(()), Err)
    };
    finish_queue(receiver, endpoint, &runtime).await;
    let _ = completion.send(Some(result));
}

async fn finish_queue(
    mut receiver: mpsc::UnboundedReceiver<Message>,
    endpoint: std::sync::Weak<Owner>,
    runtime: &RuntimeHandle,
) {
    let failure = if let Some(owner) = endpoint.upgrade() {
        owner.sender.lock().take();
        owner.state.failure.lock().clone()
    } else {
        None
    };
    receiver.close();
    if receiver.is_empty() {
        return;
    }
    // All accepted senders have been sealed. Rejection callbacks and large
    // queued payload destruction stay off the sole async scheduler too.
    let _ = runtime
        .blocking(move || {
            while let Ok(message) = receiver.try_recv() {
                match message {
                    Message::Run(job) => job.reject(failure.clone().unwrap_or_else(|| {
                        IndexedDbError::InvalidState("IndexedDB database owner is closed".into())
                    })),
                    Message::Stop => (),
                    Message::Maintain { completion, .. } => {
                        let _ = completion.send(Err(failure.clone().unwrap_or_else(|| {
                            IndexedDbError::InvalidState("IndexedDB owner is closed".into())
                        })));
                    }
                }
            }
        })
        .await;
}

impl DatabaseOwner {
    pub(super) fn run(
        &mut self,
        command: Command,
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
        let mut result = self.run_command(command, connection, preflight);
        let Some(database) = self
            .entry
            .as_mut()
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
                *self
                    .entry
                    .as_ref()
                    .expect("connected entry")
                    .state
                    .failure
                    .lock() = Some(error.clone());
                let _ = self.close();
                return (Err(error), None);
            }
        };
        let entry = self.entry.as_mut().expect("reconciled entry");
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
        command: Command,
        connection: Option<crate::DatabaseHandle>,
        preflight: Result<(), IndexedDbError>,
    ) -> Result<CommandResult, IndexedDbError> {
        let id = self.id;
        let config = self.config;
        if matches!(command, Command::Close | Command::DeleteDatabase) {
            let deleting = matches!(command, Command::DeleteDatabase);
            let entry = self.entry()?;
            let state = entry.state.clone();
            let catalog = entry.catalog.clone();
            let location = entry.location.clone();
            let result = self.close().and_then(|()| {
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
        self.entry()?
            .run_command(id, command, config, connection, preflight)
    }
}
