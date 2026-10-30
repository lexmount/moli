//! A file owns one FIFO; the partition owns the executor. Logical mutation
//! acceptance never waits for SQLite. Pending data is moved, not copied, into
//! an immutable commit, so its acknowledgement cannot clear newer changes.
use crate::{ConnectionOptions, OpenMode, RuntimeHandle};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};
use tokio::sync::{Notify, mpsc as async_mpsc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritebackError(Arc<str>);
impl WritebackError {
    pub fn new(message: impl fmt::Display) -> Self {
        Self(message.to_string().into())
    }
}
impl fmt::Display for WritebackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for WritebackError {}
impl From<rusqlite::Error> for WritebackError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(error)
    }
}
type Result<T> = std::result::Result<T, WritebackError>;

pub trait WriteBatch: Default + Send + 'static {
    fn is_empty(&self) -> bool;
    /// Compose chronological deltas: self happened before newer.
    fn merge(&mut self, newer: Self);
    /// An error can leave commit publication uncertain. Do not elide a later
    /// delete merely because an older put was originally a new row.
    fn mark_commit_uncertain(&mut self) {}
    /// Must be atomic and idempotent; a failed batch can be explicitly retried.
    fn write(&self, connection: &mut Connection) -> Result<()>;
}
#[derive(Clone, Copy)]
pub struct WritebackPolicy {
    pub delay: Duration,
    pub early_commit_operations: Option<usize>,
}
#[derive(Clone, Copy)]
pub enum FailureAdmission {
    Retain,
    Reject,
}
struct Pending<B> {
    batch: B,
    accepted: u64,
    committed: u64,
    operations: usize,
    first_dirty: Option<tokio::time::Instant>,
    failure: Option<WritebackError>,
    closed: bool,
}
struct Shared<B> {
    pending: Mutex<Pending<B>>,
    changed: Notify,
}
trait Query: Send {
    fn run(self: Box<Self>, connection: &mut Connection);
}
struct QueryJob<F, T> {
    query: F,
    reply: mpsc::Sender<Result<T>>,
}
impl<F, T> Query for QueryJob<F, T>
where
    F: FnOnce(&mut Connection) -> Result<T> + Send,
    T: Send,
{
    fn run(self: Box<Self>, connection: &mut Connection) {
        let _ = self.reply.send((self.query)(connection));
    }
}
enum Message {
    Query(Box<dyn Query>),
    Flush {
        through: u64,
        reply: mpsc::Sender<Result<()>>,
    },
    Close,
}
pub struct WritebackClient<B> {
    shared: Arc<Shared<B>>,
    sender: async_mpsc::UnboundedSender<Message>,
    runtime: RuntimeHandle,
}
impl<B> Clone for WritebackClient<B> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            sender: self.sender.clone(),
            runtime: self.runtime.clone(),
        }
    }
}
#[derive(Default)]
struct CloseReceipt {
    receiver: Option<mpsc::Receiver<Result<()>>>,
    result: Option<Result<()>>,
}
pub struct WritebackOwner<B> {
    client: WritebackClient<B>,
    finished: Mutex<CloseReceipt>,
}
impl<B: WriteBatch> WritebackOwner<B> {
    /// Native initialization, before opening V8. Every connection operation,
    /// including schema initialization and final close, runs in blocking work.
    pub fn open(
        path: PathBuf,
        mode: OpenMode,
        runtime: RuntimeHandle,
        policy: WritebackPolicy,
        initialize: impl FnOnce(&mut Connection) -> Result<()> + Send + 'static,
    ) -> Result<Self> {
        if runtime.is_current() {
            return Err(WritebackError::new(
                "cannot open a database from its SQL executor",
            ));
        }
        let (sender, receiver) = async_mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending {
                batch: B::default(),
                accepted: 0,
                committed: 0,
                operations: 0,
                first_dirty: None,
                failure: None,
                closed: false,
            }),
            changed: Notify::new(),
        });
        let client = WritebackClient {
            shared: shared.clone(),
            sender,
            runtime: runtime.clone(),
        };
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = runtime.clone();
        runtime
            .spawn(async move {
                let opened = worker
                    .blocking(move || {
                        let mut connection =
                            crate::open_disk(&path, mode, ConnectionOptions::default())
                                .map_err(WritebackError::new)?;
                        initialize(&mut connection)?;
                        Ok(connection)
                    })
                    .await
                    .map_err(WritebackError::new)
                    .and_then(|r| r);
                let result = match opened {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        run(connection, receiver, shared.clone(), worker, policy).await
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        Err(error)
                    }
                };
                let mut pending = shared.pending.lock();
                pending.closed = true;
                if let Err(error) = &result {
                    pending.failure = Some(error.clone());
                }
                drop(pending);
                let _ = done_tx.send(result);
            })
            .map_err(WritebackError::new)?;
        ready_rx
            .recv()
            .map_err(|_| WritebackError::new("database initialization stopped"))??;
        Ok(Self {
            client,
            finished: Mutex::new(CloseReceipt {
                receiver: Some(done_rx),
                result: None,
            }),
        })
    }
    /// Native startup inspection. Unidentified existing files are opened
    /// read-only, before any journal/locking pragma or schema initialization.
    pub fn inspect<T: Send + 'static>(
        path: PathBuf,
        runtime: RuntimeHandle,
        inspect: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let worker = runtime.clone();
        runtime
            .wait(async move {
                worker
                    .blocking(move || {
                        let connection = Connection::open_with_flags(
                            path,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                        )?;
                        let result = inspect(&connection)?;
                        connection
                            .close()
                            .map_err(|(_, error)| WritebackError::new(error))?;
                        Ok(result)
                    })
                    .await
                    .map_err(WritebackError::new)?
            })
            .map_err(WritebackError::new)?
    }

    pub fn client(&self) -> WritebackClient<B> {
        self.client.clone()
    }
    pub fn close(&self) -> Result<()> {
        if self.client.runtime.is_current() {
            return Err(WritebackError::new(
                "cannot close a database from its SQL executor",
            ));
        }
        let mut finished = self.finished.lock();
        if let Some(result) = &finished.result {
            return result.clone();
        }
        let receipt = finished.receiver.take().expect("unobserved close receipt");
        self.client.shared.pending.lock().closed = true;
        let _ = self.client.sender.send(Message::Close);
        let result = receipt
            .recv()
            .map_err(|_| WritebackError::new("database stopped before its close receipt"))
            .and_then(|r| r);
        finished.result = Some(result.clone());
        result
    }
}
impl<B> Drop for WritebackOwner<B> {
    fn drop(&mut self) {
        // The native partition closes explicitly and observes the receipt.
        // Drop still seals admission if startup unwinds before that boundary.
        self.client.shared.pending.lock().closed = true;
        let _ = self.client.sender.send(Message::Close);
        if !self.client.runtime.is_current()
            && let Some(receipt) = self.finished.get_mut().receiver.take()
        {
            let _ = receipt.recv();
        }
    }
}
impl<B: WriteBatch> WritebackClient<B> {
    /// Publish the native cache and accepted delta under the same admission
    /// gate. The closure never enters V8 or performs SQL.
    pub fn accept<T, E: From<WritebackError>>(
        &self,
        admission: FailureAdmission,
        mutation: impl FnOnce() -> std::result::Result<(T, B), E>,
    ) -> std::result::Result<T, E> {
        let mut pending = self.shared.pending.lock();
        if pending.closed {
            return Err(WritebackError::new("database is closed").into());
        }
        if let FailureAdmission::Reject = admission
            && let Some(error) = &pending.failure
        {
            return Err(error.clone().into());
        }
        let next = pending
            .accepted
            .checked_add(1)
            .ok_or_else(|| E::from(WritebackError::new("database sequence exhausted")))?;
        let (result, batch) = mutation()?;
        if !batch.is_empty() {
            pending.batch.merge(batch);
            pending.accepted = next;
            pending.operations = pending.operations.saturating_add(1);
            pending
                .first_dirty
                .get_or_insert_with(tokio::time::Instant::now);
            self.shared.changed.notify_one();
        }
        Ok(result)
    }
    pub fn record(&self, batch: B, admission: FailureAdmission) -> Result<()> {
        self.accept(admission, || Ok(((), batch)))
    }
    /// Only the cold-load native bridge uses this on a V8 owner. Completion
    /// comes directly from the SQL worker; it cannot depend on a JS task.
    pub fn query<T: Send + 'static>(
        &self,
        query: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        if self.runtime.is_current() {
            return Err(WritebackError::new(
                "cannot synchronously query the SQL executor from itself",
            ));
        }
        let (tx, rx) = mpsc::channel();
        {
            let pending = self.shared.pending.lock();
            if pending.closed {
                return Err(WritebackError::new("database is closed"));
            }
            self.sender
                .send(Message::Query(Box::new(QueryJob { query, reply: tx })))
                .map_err(WritebackError::new)?;
        }
        rx.recv()
            .map_err(|_| WritebackError::new("database stopped before its query reply"))?
    }
    pub fn request_flush(&self) {
        let pending = self.shared.pending.lock();
        if pending.closed {
            return;
        }
        let (reply, _) = mpsc::channel();
        let _ = self.sender.send(Message::Flush {
            through: pending.accepted,
            reply,
        });
    }
    pub fn flush(&self) -> Result<()> {
        if self.runtime.is_current() {
            return Err(WritebackError::new(
                "cannot synchronously flush the SQL executor from itself",
            ));
        }
        let (tx, rx) = mpsc::channel();
        {
            let pending = self.shared.pending.lock();
            if pending.closed {
                return Err(WritebackError::new("database is closed"));
            }
            self.sender
                .send(Message::Flush {
                    through: pending.accepted,
                    reply: tx,
                })
                .map_err(WritebackError::new)?;
        }
        rx.recv()
            .map_err(|_| WritebackError::new("database stopped before its flush receipt"))?
    }
    pub fn ensure_open(&self) -> Result<()> {
        if self.shared.pending.lock().closed {
            Err(WritebackError::new("database is closed"))
        } else {
            Ok(())
        }
    }
    pub fn failure(&self) -> Option<WritebackError> {
        self.shared.pending.lock().failure.clone()
    }
}

async fn commit<B: WriteBatch>(
    connection: Connection,
    shared: &Shared<B>,
    runtime: &RuntimeHandle,
) -> Result<Connection> {
    let taken = {
        let mut pending = shared.pending.lock();
        if pending.batch.is_empty() {
            pending.committed = pending.accepted;
            pending.operations = 0;
            pending.first_dirty = None;
            pending.failure = None;
            return Ok(connection);
        }
        let batch = std::mem::take(&mut pending.batch);
        let operations = std::mem::take(&mut pending.operations);
        let since = pending.first_dirty.take();
        (batch, pending.accepted, operations, since)
    };
    let (batch, through, operations, since) = taken;
    let (connection, batch, result) = runtime
        .blocking(move || {
            let mut connection = connection;
            let result = batch.write(&mut connection);
            (connection, batch, result)
        })
        .await
        .map_err(WritebackError::new)?;
    let mut pending = shared.pending.lock();
    match result {
        Ok(()) => {
            pending.committed = through;
            pending.failure = None;
        }
        Err(error) => {
            let mut batch = batch;
            batch.mark_commit_uncertain();
            batch.merge(std::mem::take(&mut pending.batch));
            pending.batch = batch;
            pending.operations = pending.operations.saturating_add(operations);
            pending.first_dirty = since.or(pending.first_dirty);
            pending.failure = Some(error);
        }
    }
    Ok(connection)
}
async fn run<B: WriteBatch>(
    mut connection: Connection,
    mut receiver: async_mpsc::UnboundedReceiver<Message>,
    shared: Arc<Shared<B>>,
    runtime: RuntimeHandle,
    policy: WritebackPolicy,
) -> Result<()> {
    loop {
        let deadline = {
            let pending = shared.pending.lock();
            if pending.failure.is_some() {
                None
            } else if policy
                .early_commit_operations
                .is_some_and(|limit| pending.operations >= limit)
            {
                Some(tokio::time::Instant::now())
            } else {
                pending.first_dirty.map(|since| since + policy.delay)
            }
        };
        tokio::select! {
            message = receiver.recv() => match message {
                Some(Message::Query(query)) => {
                    connection = runtime.blocking(move || { query.run(&mut connection); connection }).await.map_err(WritebackError::new)?;
                }
                Some(Message::Flush {through, reply}) => {
                    if shared.pending.lock().committed < through {
                        connection = commit(connection, &shared, &runtime).await?;
                    }
                    let result = {
                        let pending=shared.pending.lock();
                        if pending.committed>=through {Ok(())}
                        else {Err(pending.failure.clone().unwrap_or_else(||WritebackError::new("flush did not cover its accepted sequence")))}
                    };
                    let _ = reply.send(result);
                }
                Some(Message::Close) | None => {
                    connection = commit(connection, &shared, &runtime).await?;
                    let result = shared.pending.lock().failure.clone().map_or(Ok(()), Err);
                    let closed = runtime.blocking(move || connection.close().map_err(|(_, error)| WritebackError::new(error))).await.map_err(WritebackError::new)?;
                    return result.and(closed);
                }
            },
            _ = shared.changed.notified() => {},
            _ = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => { connection = commit(connection, &shared, &runtime).await?; }
        }
    }
}

#[cfg(test)]
mod tests;
