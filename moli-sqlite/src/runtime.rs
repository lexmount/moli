//! One async scheduler; SQLite work runs in Tokio's separate blocking pool.

use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, mpsc},
};

use parking_lot::Mutex;
use tokio::{
    runtime::{Builder, Handle},
    task::{JoinError, JoinSet},
};

#[derive(Debug)]
pub enum RuntimeError {
    Closed,
    Start(std::io::Error),
    Task(JoinError),
    OwnRuntime,
    Stopped,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("SQLite runtime is closed"),
            Self::Start(error) => write!(f, "failed to start SQLite runtime: {error}"),
            Self::Task(error) => write!(f, "SQLite task failed: {error}"),
            Self::OwnRuntime => {
                f.write_str("cannot synchronously wait for the SQLite runtime from itself")
            }
            Self::Stopped => f.write_str("SQLite runtime stopped before delivering its result"),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Start(error) => Some(error),
            Self::Task(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Tasks {
    closed: bool,
    running: JoinSet<()>,
    failure: Option<JoinError>,
}

/// Owned by the storage partition. Callers stop their producers and mailboxes
/// before shutdown; shutdown joins every accepted async task and its blocking
/// work. Cloneable handles do not own the Tokio runtime.
pub struct Runtime {
    runtime: Option<tokio::runtime::Runtime>,
    handle: RuntimeHandle,
}

#[derive(Clone)]
pub struct RuntimeHandle {
    handle: Handle,
    tasks: Arc<Mutex<Tasks>>,
}

impl Runtime {
    pub fn new() -> Result<Self, RuntimeError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("moli-sqlite")
            .enable_time()
            .build()
            .map_err(RuntimeError::Start)?;
        let handle = RuntimeHandle {
            handle: runtime.handle().clone(),
            tasks: Arc::new(Mutex::new(Tasks::default())),
        };
        Ok(Self {
            runtime: Some(runtime),
            handle,
        })
    }

    pub fn handle(&self) -> RuntimeHandle {
        self.handle.clone()
    }

    /// Native startup/shutdown boundary only; never call on a V8 owner or this
    /// runtime's own async/blocking workers. All accepted work joins before
    /// the runtime infrastructure is released. Inside a Tokio host, only that
    /// final infrastructure teardown is nonblocking; it is never the proof
    /// that database commands or connection cleanup have completed.
    pub fn shutdown(&mut self) -> Result<(), RuntimeError> {
        if self.runtime.is_none() {
            return Ok(());
        }
        if self.handle.is_current() {
            return Err(RuntimeError::OwnRuntime);
        }
        let (mut running, mut failure) = {
            let mut tasks = self.handle.tasks.lock();
            tasks.closed = true;
            (std::mem::take(&mut tasks.running), tasks.failure.take())
        };
        let (sender, receiver) = mpsc::channel();
        self.handle.handle.spawn(async move {
            while let Some(result) = running.join_next().await {
                if let Err(error) = result {
                    failure.get_or_insert(error);
                }
            }
            let _ = sender.send(failure.map_or(Ok(()), |error| Err(RuntimeError::Task(error))));
        });
        let result = receiver.recv().map_err(|_| RuntimeError::Stopped)?;
        let runtime = self
            .runtime
            .take()
            .expect("runtime is present until shutdown");
        if Handle::try_current().is_ok() {
            // All accepted async tasks (and their awaited blocking work) have
            // already joined. Only empty runtime infrastructure remains. Do
            // not queue destruction onto the host's blocking pool: this may
            // already be its only blocking worker.
            runtime.shutdown_background();
        } else {
            drop(runtime);
        }

        result
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl RuntimeHandle {
    pub fn is_current(&self) -> bool {
        Handle::try_current().is_ok_and(|current| current.id() == self.handle.id())
    }

    pub fn spawn(
        &self,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), RuntimeError> {
        // Share Tokio's task machinery across owner loops. Individual blocking
        // SQL operations retain their concrete types.
        self.spawn_task(Box::pin(task))
    }

    fn spawn_task(
        &self,
        task: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) -> Result<(), RuntimeError> {
        let mut tasks = self.tasks.lock();
        if tasks.closed {
            return Err(RuntimeError::Closed);
        }
        while let Some(result) = tasks.running.try_join_next() {
            if let Err(error) = result {
                tasks.failure.get_or_insert(error);
            }
        }
        tasks.running.spawn_on(task, &self.handle);
        Ok(())
    }

    /// For work belonging to an accepted async task. Await it before that task
    /// ends, so shutdown cannot leave an owner or live connection behind.
    pub async fn blocking<T: Send + 'static>(
        &self,
        task: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, RuntimeError> {
        self.handle
            .spawn_blocking(task)
            .await
            .map_err(RuntimeError::Task)
    }

    /// Bridge for synchronous native startup/maintenance APIs. Production SQL
    /// clients enqueue instead; owner loops never wait synchronously.
    pub fn wait<T: Send + 'static>(
        &self,
        task: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, RuntimeError> {
        if self.is_current() {
            return Err(RuntimeError::OwnRuntime);
        }
        let (sender, receiver) = mpsc::channel();
        self.spawn(async move {
            let _ = sender.send(task.await);
        })?;
        receiver.recv().map_err(|_| RuntimeError::Stopped)
    }
}

#[cfg(test)]
mod tests;
