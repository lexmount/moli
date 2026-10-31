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
    /// Commit atomically. A failed batch is discarded without replay.
    fn write(&self, connection: &mut Connection) -> Result<()>;
}
#[derive(Clone, Copy)]
pub struct WritebackPolicy {
    pub delay: Duration,
    pub early_commit_operations: Option<usize>,
}
struct Pending<B> {
    batch: B,
    operations: usize,
    first_dirty: Option<tokio::time::Instant>,
    failure: Option<WritebackError>,
    closed: bool,
}
struct Shared<B> {
    pending: Mutex<Pending<B>>,
    changed: Notify,
}
/// Opening is part of the first database operation, on the SQL worker.
/// Merely flushing or closing an unused store must not create a database.
enum LazyConnection {
    Unopened(Box<dyn FnOnce() -> Result<Connection> + Send>),
    Open(Connection),
    Failed(WritebackError),
}
impl LazyConnection {
    fn get(&mut self) -> Result<&mut Connection> {
        if matches!(self, Self::Unopened(_)) {
            let Self::Unopened(open) = std::mem::replace(
                self,
                Self::Failed(WritebackError::new("database initialization interrupted")),
            ) else {
                unreachable!();
            };
            *self = match open() {
                Ok(connection) => Self::Open(connection),
                Err(error) => Self::Failed(error),
            };
        }
        match self {
            Self::Open(connection) => Ok(connection),
            Self::Failed(error) => Err(error.clone()),
            Self::Unopened(_) => unreachable!(),
        }
    }
    fn failure(&self) -> Option<WritebackError> {
        match self {
            Self::Failed(error) => Some(error.clone()),
            _ => None,
        }
    }
    fn close(self) -> Result<()> {
        match self {
            Self::Open(connection) => connection.close().map_err(|(_, error)| error.into()),
            Self::Unopened(_) => Ok(()),
            Self::Failed(error) => Err(error),
        }
    }
}
trait Query: Send {
    fn run(self: Box<Self>, connection: &mut LazyConnection);
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
    fn run(self: Box<Self>, connection: &mut LazyConnection) {
        let result = connection.get().and_then(self.query);
        let _ = self.reply.send(result);
    }
}
enum Message {
    Query(Box<dyn Query>),
    Flush { reply: mpsc::Sender<Result<()>> },
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
    /// Open and validate immediately, for callers such as explicit imports.
    pub fn open(
        path: PathBuf,
        mode: OpenMode,
        runtime: RuntimeHandle,
        policy: WritebackPolicy,
        initialize: impl FnOnce(&mut Connection) -> Result<()> + Send + 'static,
    ) -> Result<Self> {
        let owner = Self::deferred(path, mode, runtime, policy, initialize)?;
        owner.client.query(|_| Ok(()))?;
        Ok(owner)
    }
    /// Prepare a store without filesystem access. The first query or nonempty
    /// commit opens it; schema initialization and close also run in blocking work.
    pub fn deferred(
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
        let connection = LazyConnection::Unopened(Box::new(move || {
            if matches!(mode, OpenMode::Create)
                && let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(WritebackError::new)?;
            }
            let mut connection = crate::open_disk(&path, mode, ConnectionOptions::default())
                .map_err(WritebackError::new)?;
            initialize(&mut connection)?;
            Ok(connection)
        }));
        let (done_tx, done_rx) = mpsc::channel();
        let worker = runtime.clone();
        runtime
            .spawn(async move {
                let result = run(connection, receiver, shared.clone(), worker, policy).await;
                let mut pending = shared.pending.lock();
                pending.closed = true;
                if let Err(error) = &result {
                    pending.failure = Some(error.clone());
                }
                drop(pending);
                let _ = done_tx.send(result);
            })
            .map_err(WritebackError::new)?;
        Ok(Self {
            client,
            finished: Mutex::new(CloseReceipt {
                receiver: Some(done_rx),
                result: None,
            }),
        })
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
        mutation: impl FnOnce() -> std::result::Result<(T, B), E>,
    ) -> std::result::Result<T, E> {
        let mut pending = self.shared.pending.lock();
        if pending.closed {
            return Err(WritebackError::new("database is closed").into());
        }
        let (result, batch) = mutation()?;
        if !batch.is_empty() {
            pending.batch.merge(batch);
            pending.operations = pending.operations.saturating_add(1);
            pending
                .first_dirty
                .get_or_insert_with(tokio::time::Instant::now);
            self.shared.changed.notify_one();
        }
        Ok(result)
    }
    pub fn record(&self, batch: B) -> Result<()> {
        self.accept(|| Ok(((), batch)))
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
                .send(Message::Flush { reply: tx })
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
}

async fn commit<B: WriteBatch>(
    connection: LazyConnection,
    shared: &Shared<B>,
    runtime: &RuntimeHandle,
) -> Result<LazyConnection> {
    let batch = {
        let mut pending = shared.pending.lock();
        if pending.batch.is_empty() {
            return Ok(connection);
        }
        pending.operations = 0;
        pending.first_dirty = None;
        std::mem::take(&mut pending.batch)
    };
    let (connection, result) = runtime
        .blocking(move || {
            let mut connection = connection;
            let result = connection
                .get()
                .and_then(|connection| batch.write(connection));
            (connection, result)
        })
        .await
        .map_err(WritebackError::new)?;
    if let Err(error) = &result {
        tracing::warn!(%error, batch = std::any::type_name::<B>(), "SQLite persistence batch failed; the batch will not be retried");
    }
    shared.pending.lock().failure = result.err();
    Ok(connection)
}
async fn run<B: WriteBatch>(
    mut connection: LazyConnection,
    mut receiver: async_mpsc::UnboundedReceiver<Message>,
    shared: Arc<Shared<B>>,
    runtime: RuntimeHandle,
    policy: WritebackPolicy,
) -> Result<()> {
    loop {
        let deadline = {
            let pending = shared.pending.lock();
            if policy
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
                    if let Some(error) = connection.failure() {
                        shared.pending.lock().failure = Some(error);
                    }
                }
                Some(Message::Flush {reply}) => {
                    connection = commit(connection, &shared, &runtime).await?;
                    let result = shared.pending.lock().failure.clone().map_or(Ok(()), Err);
                    let _ = reply.send(result);
                }
                Some(Message::Close) | None => {
                    connection = commit(connection, &shared, &runtime).await?;
                    let result = shared.pending.lock().failure.clone().map_or(Ok(()), Err);
                    let closed = runtime.blocking(move || connection.close()).await.map_err(WritebackError::new)?;
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
