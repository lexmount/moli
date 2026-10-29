use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle, ThreadId},
};

use parking_lot::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub tasks: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutorConfig {
    pub workers: usize,
    pub data: Capacity,
    pub control: Capacity,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            workers: thread::available_parallelism()
                .map_or(1, usize::from)
                .min(4),
            data: Capacity {
                tasks: 256,
                bytes: 64 * 1024 * 1024,
            },
            control: Capacity {
                tasks: 64,
                bytes: 256 * 1024,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Data,
    /// Commit, abort, close, and other bounded lifecycle work only.
    Control,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueUsage {
    pub data_tasks: usize,
    pub data_bytes: usize,
    pub control_tasks: usize,
    pub control_bytes: usize,
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

type Task = Box<dyn FnOnce(TaskPermit) + Send + 'static>;

struct PendingJob {
    key: u64,
    permit: TaskPermit,
    task: Task,
}

struct QueueState {
    closed: bool,
    usage: QueueUsage,
    pending: VecDeque<PendingJob>,
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

enum TaskSlot {
    Reserved,
    Waiting,
    /// Memory reserved for owner retirement, independent of ordinary slots.
    Cleanup,
}

/// Accounts for input, work in progress, and the undelivered result. Move this
/// permit into the completion message; release it only after consumption or
/// cancellation. Reserve a byte bound before retaining the command's payload.
pub struct TaskPermit {
    shared: Arc<Shared>,
    kind: TaskKind,
    bytes: usize,
    overhead: usize,
    slot: TaskSlot,
}

impl QueueState {
    fn tasks(&self, kind: TaskKind) -> usize {
        match kind {
            TaskKind::Data => self.usage.data_tasks,
            TaskKind::Control => self.usage.control_tasks,
        }
    }

    fn count(&mut self, kind: TaskKind, add: bool) {
        let tasks = match kind {
            TaskKind::Data => &mut self.usage.data_tasks,
            TaskKind::Control => &mut self.usage.control_tasks,
        };
        if add {
            *tasks += 1;
        } else {
            *tasks -= 1;
        }
    }

    fn dispatch_pending(&mut self, config: ExecutorConfig) -> Vec<Message> {
        let mut blocked = BTreeSet::new();
        let mut waiting = VecDeque::new();
        let mut failed = Vec::new();
        while let Some(mut job) = self.pending.pop_front() {
            let capacity = match job.permit.kind {
                TaskKind::Data => config.data.tasks,
                TaskKind::Control => config.control.tasks,
            };
            // Preserve database FIFO across task kinds. Other databases and
            // the control lane may progress when data results occupy all slots.
            // Shutdown drains accepted work without waiting on result consumers;
            // the byte budget continues to bound all retained jobs and results.
            if blocked.contains(&job.key)
                || (!self.closed
                    && !matches!(job.permit.slot, TaskSlot::Cleanup)
                    && self.tasks(job.permit.kind) >= capacity)
            {
                blocked.insert(job.key);
                waiting.push_back(job);
                continue;
            }
            if !matches!(job.permit.slot, TaskSlot::Cleanup) {
                self.count(job.permit.kind, true);
                job.permit.slot = TaskSlot::Reserved;
            }
            let lane = (job.key % config.workers as u64) as usize;
            let message = Message::Run(Box::new(move || (job.task)(job.permit)));
            if let Err(error) = self.senders[lane].send(message) {
                failed.push(error.0);
            }
        }
        self.pending = waiting;
        failed
    }
}

impl Executor {
    pub fn new(config: ExecutorConfig) -> Result<Self, ExecutorError> {
        if config.workers == 0
            || config.data.tasks == 0
            || config.data.bytes == 0
            || config.control.tasks == 0
            || config.control.bytes == 0
        {
            return Err(ExecutorError::InvalidConfiguration);
        }
        let mut executor = Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(QueueState {
                    closed: false,
                    usage: QueueUsage::default(),
                    pending: VecDeque::new(),
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
    /// Storage owners enqueue connection cleanup before calling this method.
    /// Already delivered results may outlive shutdown and retain their permits.
    pub fn shutdown(&mut self) -> Result<(), ExecutorError> {
        self.shutdown_impl(None)
    }

    /// One finalizer per worker, ordered after accepted commands and independent
    /// of task permits still retained by consumers. This fixed-size shutdown
    /// capacity lets storage owners roll back/close connections on their worker
    /// even when both normal queues are full.
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
        let discarded = {
            let mut state = self.shared.state.lock();
            let mut discarded = Vec::new();
            if !state.closed {
                state.closed = true;
                discarded = state.dispatch_pending(self.shared.config);
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
            discarded
        };
        drop(discarded);
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

    pub fn try_reserve(&self, kind: TaskKind, bytes: usize) -> Result<TaskPermit, ExecutorError> {
        self.reserve(kind, bytes, false)
    }

    /// Retain a queued request without occupying an execution/result slot.
    /// Snapshot bytes and queue overhead are charged immediately. On submit,
    /// temporary slot exhaustion waits asynchronously until a consumer releases
    /// capacity; it never blocks the owner or a SQL worker.
    pub fn reserve_pending(
        &self,
        kind: TaskKind,
        bytes: usize,
    ) -> Result<TaskPermit, ExecutorError> {
        self.reserve(kind, bytes, true)
    }

    /// Reserve bounded lifecycle memory in advance. Abort/close must remain
    /// runnable even when all ordinary control results are retained. Cleanup
    /// work shares the byte budget and worker FIFO, but needs no ordinary slot.
    pub fn reserve_cleanup(&self, bytes: usize) -> Result<TaskPermit, ExecutorError> {
        let mut permit = self.reserve(TaskKind::Control, bytes, true)?;
        permit.slot = TaskSlot::Cleanup;
        Ok(permit)
    }

    fn reserve(
        &self,
        kind: TaskKind,
        bytes: usize,
        pending: bool,
    ) -> Result<TaskPermit, ExecutorError> {
        let overhead = if pending {
            std::mem::size_of::<PendingJob>() + 256
        } else {
            0
        };
        let charged = bytes.checked_add(overhead).ok_or(ExecutorError::Capacity)?;
        let mut state = self.shared.state.lock();
        if state.closed {
            return Err(ExecutorError::Closed);
        }
        let (capacity, tasks, used_bytes) = match kind {
            TaskKind::Data => (
                self.shared.config.data,
                state.usage.data_tasks,
                state.usage.data_bytes,
            ),
            TaskKind::Control => (
                self.shared.config.control,
                state.usage.control_tasks,
                state.usage.control_bytes,
            ),
        };
        if (!pending && tasks >= capacity.tasks)
            || charged > capacity.bytes.saturating_sub(used_bytes)
        {
            return Err(ExecutorError::Capacity);
        }
        match kind {
            TaskKind::Data => {
                state.usage.data_tasks += usize::from(!pending);
                state.usage.data_bytes += charged;
            }
            TaskKind::Control => {
                state.usage.control_tasks += usize::from(!pending);
                state.usage.control_bytes += charged;
            }
        }
        Ok(TaskPermit {
            shared: self.shared.clone(),
            kind,
            bytes,
            overhead,
            slot: if pending {
                TaskSlot::Waiting
            } else {
                TaskSlot::Reserved
            },
        })
    }

    pub fn usage(&self) -> QueueUsage {
        self.shared.state.lock().usage
    }

    pub fn worker_count(&self) -> usize {
        self.shared.config.workers
    }
}

impl TaskPermit {
    pub fn kind(&self) -> TaskKind {
        self.kind
    }

    pub fn belongs_to(&self, handle: &ExecutorHandle) -> bool {
        Arc::ptr_eq(&self.shared, &handle.shared)
    }

    pub fn reserved_bytes(&self) -> usize {
        self.bytes
    }

    /// The owner has consumed the completion. Lazy cursor pages may keep its
    /// bytes alive, but must not occupy an execution/result slot until GC.
    /// Resubmitting this permit acquires a slot again through the pending queue.
    pub fn release_execution_slot(&mut self) {
        let discarded = {
            let mut state = self.shared.state.lock();
            if matches!(self.slot, TaskSlot::Reserved) {
                state.count(self.kind, false);
                self.slot = TaskSlot::Waiting;
            }
            state.dispatch_pending(self.shared.config)
        };
        drop(discarded);
    }

    /// Resize before retaining additional input or result buffers. Failed growth
    /// leaves the original reservation intact; shrinking also works after close.
    pub fn try_resize(&mut self, bytes: usize) -> Result<(), ExecutorError> {
        let mut state = self.shared.state.lock();
        let (capacity, used) = match self.kind {
            TaskKind::Data => (self.shared.config.data.bytes, state.usage.data_bytes),
            TaskKind::Control => (self.shared.config.control.bytes, state.usage.control_bytes),
        };
        if bytes > self.bytes {
            if state.closed {
                return Err(ExecutorError::Closed);
            }
            if bytes - self.bytes > capacity.saturating_sub(used) {
                return Err(ExecutorError::Capacity);
            }
        }
        match self.kind {
            TaskKind::Data => state.usage.data_bytes = used - self.bytes + bytes,
            TaskKind::Control => state.usage.control_bytes = used - self.bytes + bytes,
        }
        self.bytes = bytes;
        Ok(())
    }

    /// Equal keys run FIFO on one worker. A transaction can retain its database
    /// connection between commands while that worker serves other databases.
    pub fn submit(
        self,
        database_key: u64,
        task: impl FnOnce(TaskPermit) + Send + 'static,
    ) -> Result<(), ExecutorError> {
        let shared = self.shared.clone();
        let mut permit = self;
        let discarded = {
            let mut state = shared.state.lock();
            if state.closed {
                return Err(ExecutorError::Closed);
            }
            if matches!(permit.slot, TaskSlot::Reserved) {
                state.count(permit.kind, false);
                permit.slot = TaskSlot::Waiting;
            }
            state.pending.push_back(PendingJob {
                key: database_key,
                permit,
                task: Box::new(task),
            });
            state.dispatch_pending(shared.config)
        };
        let failed = !discarded.is_empty();
        drop(discarded);
        if failed {
            Err(ExecutorError::Closed)
        } else {
            Ok(())
        }
    }
}

impl Drop for TaskPermit {
    fn drop(&mut self) {
        let discarded = {
            let mut state = self.shared.state.lock();
            if matches!(self.slot, TaskSlot::Reserved) {
                state.count(self.kind, false);
            }
            match self.kind {
                TaskKind::Data => state.usage.data_bytes -= self.bytes + self.overhead,
                TaskKind::Control => state.usage.control_bytes -= self.bytes + self.overhead,
            }
            state.dispatch_pending(self.shared.config)
        };
        // Failed sends own permits; dropping them under the gate would reenter it.
        drop(discarded);
    }
}

#[cfg(test)]
mod tests;
