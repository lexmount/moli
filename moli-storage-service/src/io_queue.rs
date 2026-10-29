use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, mpsc},
    thread,
};

use parking_lot::{Condvar, Mutex};

type StorageIoJob = Box<dyn FnOnce() + Send + 'static>;

/// Failure to enqueue work on the partition-owned storage IO sequence.
#[derive(Debug)]
pub enum StorageServiceDispatchError {
    /// The dedicated storage IO thread could not be created.
    WorkerSpawn(std::io::Error),
    /// The queue worker exited before accepting the task.
    QueueClosed,
    /// A worker cannot wait for a barrier queued behind its own operation.
    ReentrantDrain,
}

impl fmt::Display for StorageServiceDispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkerSpawn(error) => write!(f, "failed to start storage IO worker: {error}"),
            Self::QueueClosed => f.write_str("storage IO queue is closed"),
            Self::ReentrantDrain => f.write_str("storage IO worker cannot drain its own queue"),
        }
    }
}

impl Error for StorageServiceDispatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkerSpawn(error) => Some(error),
            Self::QueueClosed | Self::ReentrantDrain => None,
        }
    }
}

/// Failure produced while executing one accepted storage IO task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageServiceTaskError {
    /// The operation panicked. The queue catches it so later tasks can still run.
    Panicked,
}

impl fmt::Display for StorageServiceTaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panicked => f.write_str("storage IO task panicked"),
        }
    }
}

impl Error for StorageServiceTaskError {}

#[derive(Clone, Default)]
pub(crate) struct StorageIoQueue {
    state: Arc<Mutex<StorageIoQueueState>>,
}

#[derive(Clone, Default)]
pub(crate) struct StorageIoSequence {
    inner: Arc<StorageIoSequenceInner>,
}

#[derive(Default)]
struct StorageIoSequenceInner {
    state: Mutex<StorageIoSequenceState>,
    ready: Condvar,
}

#[derive(Default)]
struct StorageIoSequenceState {
    next_ticket: u64,
    serving: u64,
    cancelled: BTreeSet<u64>,
}

pub(crate) struct StorageIoReservation {
    inner: Arc<StorageIoSequenceInner>,
    ticket: Option<u64>,
}

pub(crate) struct StorageIoTurn {
    inner: Arc<StorageIoSequenceInner>,
    ticket: u64,
}

#[derive(Default)]
struct StorageIoQueueState {
    sender: Option<mpsc::Sender<StorageIoJob>>,
    worker: Option<thread::ThreadId>,
}

impl fmt::Debug for StorageIoQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let started = self.state.lock().sender.is_some();
        f.debug_struct("StorageIoQueue")
            .field("started", &started)
            .finish()
    }
}

impl fmt::Debug for StorageIoSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state.lock();
        f.debug_struct("StorageIoSequence")
            .field("next_ticket", &state.next_ticket)
            .field("serving", &state.serving)
            .field("cancelled", &state.cancelled.len())
            .finish()
    }
}

impl StorageIoSequence {
    pub(crate) fn reserve(&self) -> StorageIoReservation {
        let mut state = self.inner.state.lock();
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .expect("storage IO ticket space exhausted");
        StorageIoReservation {
            inner: self.inner.clone(),
            ticket: Some(ticket),
        }
    }

    pub(crate) fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _turn = self.reserve().enter();
        operation()
    }
}

impl StorageIoReservation {
    pub(crate) fn enter(mut self) -> StorageIoTurn {
        let ticket = self.ticket.take().expect("storage IO ticket already used");
        let mut state = self.inner.state.lock();
        while state.serving != ticket {
            self.inner.ready.wait(&mut state);
        }
        drop(state);
        StorageIoTurn {
            inner: self.inner.clone(),
            ticket,
        }
    }
}

impl Drop for StorageIoReservation {
    fn drop(&mut self) {
        let Some(ticket) = self.ticket.take() else {
            return;
        };
        let mut state = self.inner.state.lock();
        state.cancelled.insert(ticket);
        advance_cancelled_storage_tickets(&mut state);
        self.inner.ready.notify_all();
    }
}

impl Drop for StorageIoTurn {
    fn drop(&mut self) {
        let mut state = self.inner.state.lock();
        debug_assert_eq!(state.serving, self.ticket);
        state.serving = state
            .serving
            .checked_add(1)
            .expect("storage IO ticket space exhausted");
        advance_cancelled_storage_tickets(&mut state);
        self.inner.ready.notify_all();
    }
}

fn advance_cancelled_storage_tickets(state: &mut StorageIoSequenceState) {
    while state.cancelled.remove(&state.serving) {
        state.serving = state
            .serving
            .checked_add(1)
            .expect("storage IO ticket space exhausted");
    }
}

impl StorageIoQueue {
    /// The caller has stopped producers. Wait only on the partition owner,
    /// never on this queue's worker or a V8 event loop.
    pub(crate) fn drain(&self) -> Result<(), StorageServiceDispatchError> {
        let sender = {
            let state = self.state.lock();
            if state.worker == Some(thread::current().id()) {
                return Err(StorageServiceDispatchError::ReentrantDrain);
            }
            let Some(sender) = &state.sender else {
                return Ok(());
            };
            sender.clone()
        };
        let (done, completed) = mpsc::channel();
        sender
            .send(Box::new(move || {
                let _ = done.send(());
            }))
            .map_err(|_| StorageServiceDispatchError::QueueClosed)?;
        completed
            .recv()
            .map_err(|_| StorageServiceDispatchError::QueueClosed)
    }

    pub(crate) fn dispatch<T, Operation, Completion>(
        &self,
        operation: Operation,
        completion: Completion,
    ) -> Result<(), StorageServiceDispatchError>
    where
        T: Send + 'static,
        Operation: FnOnce() -> T + Send + 'static,
        Completion: FnOnce(Result<T, StorageServiceTaskError>) + Send + 'static,
    {
        let job = move || {
            let result = catch_unwind(AssertUnwindSafe(operation))
                .map_err(|_| StorageServiceTaskError::Panicked);
            completion(result);
        };
        self.dispatch_job(Box::new(job))
    }

    fn dispatch_job(&self, job: StorageIoJob) -> Result<(), StorageServiceDispatchError> {
        let sender = self.sender()?;
        sender
            .send(job)
            .map_err(|_| StorageServiceDispatchError::QueueClosed)
    }

    fn sender(&self) -> Result<mpsc::Sender<StorageIoJob>, StorageServiceDispatchError> {
        let mut state = self.state.lock();
        if let Some(sender) = &state.sender {
            return Ok(sender.clone());
        }

        let (sender, receiver) = mpsc::channel::<StorageIoJob>();
        let worker = thread::Builder::new()
            .name("moli-storage-io".to_owned())
            .spawn(move || run_storage_io_queue(receiver))
            .map_err(StorageServiceDispatchError::WorkerSpawn)?;
        state.worker = Some(worker.thread().id());
        state.sender = Some(sender.clone());
        Ok(sender)
    }
}

fn run_storage_io_queue(receiver: mpsc::Receiver<StorageIoJob>) {
    while let Ok(job) = receiver.recv() {
        // An operation panic is normally converted to StorageServiceTaskError
        // inside `dispatch`. Catch completion callback panics as well so one
        // faulty client cannot terminate the partition's storage IO sequence.
        let _ = catch_unwind(AssertUnwindSafe(job));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn receive<T>(receiver: &mpsc::Receiver<T>) -> T {
        receiver.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    #[test]
    fn draining_unused_queue_does_not_start_a_worker() {
        let queue = StorageIoQueue::default();
        queue.drain().unwrap();
        assert!(queue.state.lock().sender.is_none());
    }

    #[test]
    fn drain_waits_for_the_accepted_operation_and_its_completion() {
        let queue = StorageIoQueue::default();
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let (completion, completed) = mpsc::channel();
        queue
            .dispatch(
                move || {
                    entered.send(()).unwrap();
                    receive(&resume);
                },
                move |result| {
                    result.unwrap();
                    completion.send(()).unwrap();
                },
            )
            .unwrap();
        receive(&waiting);
        let (done, drained) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = done.send(queue.drain());
        });
        assert!(matches!(drained.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).unwrap();
        receive(&drained).unwrap();
        receive(&completed);
        worker.join().unwrap();
    }

    #[test]
    fn drain_rejects_a_worker_waiting_for_itself_and_survives_panicked_work() {
        let queue = StorageIoQueue::default();
        let nested = queue.clone();
        let (sender, receiver) = mpsc::channel();
        queue
            .dispatch(
                move || nested.drain(),
                move |result| {
                    let _ = sender.send(result);
                },
            )
            .unwrap();
        assert!(matches!(
            receive(&receiver).unwrap(),
            Err(StorageServiceDispatchError::ReentrantDrain)
        ));
        queue
            .dispatch(
                || panic!("accepted storage operation failed"),
                |result| {
                    assert_eq!(result, Err(StorageServiceTaskError::Panicked));
                },
            )
            .unwrap();
        queue.drain().unwrap();
    }
}
