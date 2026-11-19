use crate::runtime::WorkerInspectorError;
use anyhow::Result;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

/// One native session incarnation. Wire ids may be reused after attach; tasks
/// and channels retain this token so retired work cannot address its successor.
#[derive(Clone)]
pub(crate) struct WorkerInspectorSessionLifetime(
    pub(super) Arc<WorkerInspectorSessionLifetimeState>,
);

pub(super) struct WorkerInspectorSessionLifetimeState {
    pub(super) active: AtomicBool,
    native_created: AtomicBool,
    detachment: Mutex<WorkerInspectorSessionCompletion>,
    retirement: Mutex<WorkerInspectorSessionCompletion>,
}

#[derive(Default)]
struct WorkerInspectorSessionCompletion {
    result: Option<std::result::Result<(), WorkerInspectorError>>,
    waiters: Vec<oneshot::Sender<Result<()>>>,
}

impl WorkerInspectorSessionCompletion {
    fn complete(&mut self, result: std::result::Result<(), WorkerInspectorError>) {
        if self.result.is_some() {
            return;
        }
        self.result = Some(result);
        for waiter in self.waiters.drain(..) {
            let _ = waiter.send(result.map_err(Into::into));
        }
    }

    fn subscribe(&mut self) -> oneshot::Receiver<Result<()>> {
        let (tx, rx) = oneshot::channel();
        if let Some(result) = self.result {
            let _ = tx.send(result.map_err(Into::into));
        } else {
            self.waiters.push(tx);
        }
        rx
    }
}

impl WorkerInspectorSessionLifetime {
    pub(super) fn new() -> Self {
        Self(Arc::new(WorkerInspectorSessionLifetimeState {
            active: AtomicBool::new(true),
            native_created: AtomicBool::new(false),
            detachment: Mutex::new(WorkerInspectorSessionCompletion::default()),
            retirement: Mutex::new(WorkerInspectorSessionCompletion::default()),
        }))
    }

    pub(crate) fn is_active(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
    }
    pub(crate) fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn mark_native_created(&self) {
        self.0.native_created.store(true, Ordering::Release);
    }
    pub(crate) fn native_was_created(&self) -> bool {
        self.0.native_created.load(Ordering::Acquire)
    }

    pub(crate) fn complete_retirement(
        &self,
        result: std::result::Result<(), WorkerInspectorError>,
    ) {
        self.0.retirement.lock().complete(result);
    }

    #[cfg(test)]
    pub(crate) fn retirement(&self) -> oneshot::Receiver<Result<()>> {
        self.0.retirement.lock().subscribe()
    }

    pub(crate) fn complete_detachment(
        &self,
        result: std::result::Result<(), WorkerInspectorError>,
    ) {
        self.0.detachment.lock().complete(result);
    }

    pub(super) fn detachment(&self) -> oneshot::Receiver<Result<()>> {
        self.0.detachment.lock().subscribe()
    }
}
