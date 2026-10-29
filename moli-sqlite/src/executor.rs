use std::{
    fmt,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle, ThreadId},
};

use parking_lot::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct ExecutorConfig {
    pub workers: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            workers: thread::available_parallelism()
                .map_or(1, usize::from)
                .min(4),
        }
    }
}

#[derive(Debug)]
pub enum ExecutorError {
    InvalidConfiguration,
    Closed,
    Spawn(std::io::Error),
    WorkerPanicked,
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => f.write_str("invalid SQLite executor configuration"),
            Self::Closed => f.write_str("SQLite executor is closed"),
            Self::Spawn(error) => write!(f, "failed to start SQLite worker: {error}"),
            Self::WorkerPanicked => f.write_str("SQLite worker panicked"),
        }
    }
}

impl std::error::Error for ExecutorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            _ => None,
        }
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

enum Message {
    Run(Job),
    Shutdown(Option<Job>),
}

struct QueueState {
    closed: bool,
    senders: Vec<mpsc::Sender<Message>>,
    worker_threads: Vec<ThreadId>,
}

struct Shared {
    config: ExecutorConfig,
    state: Mutex<QueueState>,
}

/// The partition owns this object and explicitly shuts it down off the V8
/// thread. Cloneable handles never own/join worker threads.
pub struct Executor {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct ExecutorHandle {
    shared: Arc<Shared>,
}

impl Executor {
    pub fn new(config: ExecutorConfig) -> Result<Self, ExecutorError> {
        if config.workers == 0 {
            return Err(ExecutorError::InvalidConfiguration);
        }
        let mut executor = Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(QueueState {
                    closed: false,
                    senders: Vec::with_capacity(config.workers),
                    worker_threads: Vec::with_capacity(config.workers),
                }),
            }),
            workers: Vec::with_capacity(config.workers),
        };
        for index in 0..config.workers {
            let (sender, receiver) = mpsc::channel();
            let worker = thread::Builder::new()
                .name(format!("moli-sqlite-{index}"))
                .spawn(move || {
                    while let Ok(message) = receiver.recv() {
                        match message {
                            Message::Run(job) => job(),
                            Message::Shutdown(finalize) => {
                                if let Some(finalize) = finalize {
                                    finalize();
                                }
                                break;
                            }
                        }
                    }
                })
                .map_err(ExecutorError::Spawn)?;
            {
                let mut state = executor.shared.state.lock();
                state.senders.push(sender);
                state.worker_threads.push(worker.thread().id());
            }
            executor.workers.push(worker);
        }
        Ok(executor)
    }

    pub fn handle(&self) -> ExecutorHandle {
        ExecutorHandle {
            shared: self.shared.clone(),
        }
    }

    /// Reject new submissions, drain accepted work, then join all workers.
    pub fn shutdown(&mut self) -> Result<(), ExecutorError> {
        self.shutdown_impl(None)
    }

    /// One finalizer per worker, ordered after accepted commands and independent
    /// of data still retained by consumers. Storage owners roll back and close
    /// their connections on the worker before it exits.
    pub fn shutdown_with(
        &mut self,
        finalize: impl Fn(usize) + Send + Sync + 'static,
    ) -> Result<(), ExecutorError> {
        self.shutdown_impl(Some(Arc::new(finalize)))
    }

    fn shutdown_impl(
        &mut self,
        finalize: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    ) -> Result<(), ExecutorError> {
        {
            let mut state = self.shared.state.lock();
            if !state.closed {
                state.closed = true;
                // Sending under the same gate as submit orders shutdown after
                // every accepted job, including concurrent submitters.
                for (index, sender) in state.senders.iter().enumerate() {
                    let job = finalize.as_ref().map(|finalize| {
                        let finalize = finalize.clone();
                        Box::new(move || finalize(index)) as Job
                    });
                    let _ = sender.send(Message::Shutdown(job));
                }
            }
        }
        let mut failed = false;
        for worker in self.workers.drain(..) {
            failed |= worker.join().is_err();
        }
        if failed {
            Err(ExecutorError::WorkerPanicked)
        } else {
            Ok(())
        }
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl ExecutorHandle {
    /// Blocking maintenance must not wait for this executor from its own lane.
    pub fn is_worker_thread(&self) -> bool {
        self.shared
            .state
            .lock()
            .worker_threads
            .contains(&thread::current().id())
    }

    /// Equal keys run FIFO on one worker. Accepted commands run independently
    /// of result consumption; shutdown takes the same gate and follows them.
    /// Storage owners use this directly for commit, abort, cancel and close.
    pub fn submit(
        &self,
        key: u64,
        task: impl FnOnce() + Send + 'static,
    ) -> Result<(), ExecutorError> {
        let state = self.shared.state.lock();
        if state.closed {
            return Err(ExecutorError::Closed);
        }
        let lane = (key % self.shared.config.workers as u64) as usize;
        let sent = state.senders[lane].send(Message::Run(Box::new(task)));
        drop(state);
        sent.map_err(|_| ExecutorError::Closed)
    }

    pub fn worker_count(&self) -> usize {
        self.shared.config.workers
    }
}

#[cfg(test)]
mod tests;
