use std::{
    fmt,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle, ThreadId},
};

use parking_lot::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct ExecutorConfig {
    pub workers: usize,
    pub max_in_flight_bytes: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            workers: thread::available_parallelism()
                .map_or(1, usize::from)
                .min(4),
            max_in_flight_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum ExecutorError {
    InvalidConfiguration,
    Capacity,
    Closed,
    Spawn(std::io::Error),
    WorkerPanicked,
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => f.write_str("invalid SQLite executor configuration"),
            Self::Capacity => f.write_str("SQLite executor capacity exhausted"),
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
    in_flight_bytes: usize,
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

/// One byte budget for queued input, work in progress, undelivered results and
/// retained cursor data. Lifecycle work never needs a reservation or task slot.
pub struct InFlightData {
    shared: Arc<Shared>,
    bytes: usize,
}

impl Executor {
    pub fn new(config: ExecutorConfig) -> Result<Self, ExecutorError> {
        if config.workers == 0 || config.max_in_flight_bytes == 0 {
            return Err(ExecutorError::InvalidConfiguration);
        }
        let mut executor = Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(QueueState {
                    closed: false,
                    in_flight_bytes: 0,
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
    /// Already delivered data may outlive shutdown and retain its byte budget.
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

    pub fn try_reserve(&self, bytes: usize) -> Result<InFlightData, ExecutorError> {
        let mut state = self.shared.state.lock();
        if state.closed {
            return Err(ExecutorError::Closed);
        }
        if bytes
            > self
                .shared
                .config
                .max_in_flight_bytes
                .saturating_sub(state.in_flight_bytes)
        {
            return Err(ExecutorError::Capacity);
        }
        state.in_flight_bytes += bytes;
        Ok(InFlightData {
            shared: self.shared.clone(),
            bytes,
        })
    }

    /// Equal keys run FIFO on one worker. Accepted commands run independently
    /// of result consumption; shutdown takes the same gate and follows them.
    /// Storage owners use this directly for commit, abort, cancel and close.
    pub fn submit(
        &self,
        key: u64,
        task: impl FnOnce() + Send + 'static,
    ) -> Result<(), ExecutorError> {
        let sent = {
            let state = self.shared.state.lock();
            if state.closed {
                return Err(ExecutorError::Closed);
            }
            let lane = (key % self.shared.config.workers as u64) as usize;
            state.senders[lane].send(Message::Run(Box::new(task)))
        };
        // A failed send owns the task; drop it outside the gate so any retained
        // data can return its bytes without reentering the locked state.
        sent.map_err(|_| ExecutorError::Closed)
    }

    pub fn in_flight_bytes(&self) -> usize {
        self.shared.state.lock().in_flight_bytes
    }

    pub fn worker_count(&self) -> usize {
        self.shared.config.workers
    }
}

impl InFlightData {
    pub fn belongs_to(&self, handle: &ExecutorHandle) -> bool {
        Arc::ptr_eq(&self.shared, &handle.shared)
    }

    pub fn reserved_bytes(&self) -> usize {
        self.bytes
    }

    /// Failed growth leaves the original budget intact. Shrinking still works
    /// after shutdown, since delivered data can outlive its SQL worker.
    pub fn try_resize(&mut self, bytes: usize) -> Result<(), ExecutorError> {
        let mut state = self.shared.state.lock();
        if bytes > self.bytes {
            if state.closed {
                return Err(ExecutorError::Closed);
            }
            if bytes - self.bytes
                > self
                    .shared
                    .config
                    .max_in_flight_bytes
                    .saturating_sub(state.in_flight_bytes)
            {
                return Err(ExecutorError::Capacity);
            }
        }
        state.in_flight_bytes = state.in_flight_bytes - self.bytes + bytes;
        self.bytes = bytes;
        Ok(())
    }

    pub fn submit(
        self,
        key: u64,
        task: impl FnOnce(InFlightData) + Send + 'static,
    ) -> Result<(), ExecutorError> {
        let handle = ExecutorHandle {
            shared: self.shared.clone(),
        };
        handle.submit(key, move || task(self))
    }
}

impl Drop for InFlightData {
    fn drop(&mut self) {
        self.shared.state.lock().in_flight_bytes -= self.bytes;
    }
}

#[cfg(test)]
mod tests;
