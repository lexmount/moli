use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

use super::{RendererBrowserContextRuntimeId, RendererSharedWorkerTargetInfo};

/// Execution facts, without Inspector commands, sessions or protocol target IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererWorkerLifecycle {
    SharedCreated(RendererSharedWorkerTargetInfo),
    SharedDestroyed(moli_shared_worker::SharedWorkerInstanceId),
}

/// The source FIFO carries an observation of the Browser commit, never its
/// decision authority. A missing/dropped Browser consumer cannot acknowledge it.
#[derive(Clone, Debug)]
pub struct RendererWorkerLifecycleObservation {
    runtime: RendererBrowserContextRuntimeId,
    lifecycle: Arc<RendererWorkerLifecycle>,
    committed: watch::Receiver<Option<u64>>,
}

impl PartialEq for RendererWorkerLifecycleObservation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.lifecycle, &other.lifecycle)
    }
}

impl Eq for RendererWorkerLifecycleObservation {}

/// A complete occurrence already committed by Browser. `browser_sequence` is
/// supplied by that owner; it is not a renderer output cursor or turn sequence.
#[derive(Clone, Debug)]
pub struct RendererCommittedWorkerLifecycle {
    runtime: RendererBrowserContextRuntimeId,
    browser_sequence: u64,
    lifecycle: Arc<RendererWorkerLifecycle>,
}

impl RendererCommittedWorkerLifecycle {
    pub fn runtime(&self) -> RendererBrowserContextRuntimeId {
        self.runtime
    }
    pub fn browser_sequence(&self) -> u64 {
        self.browser_sequence
    }
    pub fn lifecycle(&self) -> &RendererWorkerLifecycle {
        &self.lifecycle
    }
}

impl RendererWorkerLifecycleObservation {
    pub async fn committed(mut self) -> Option<RendererCommittedWorkerLifecycle> {
        loop {
            let sequence = *self.committed.borrow_and_update();
            if let Some(browser_sequence) = sequence {
                return Some(RendererCommittedWorkerLifecycle {
                    runtime: self.runtime,
                    browser_sequence,
                    lifecycle: self.lifecycle,
                });
            }
            self.committed.changed().await.ok()?;
        }
    }

    pub(crate) fn lifecycle(&self) -> &RendererWorkerLifecycle {
        &self.lifecycle
    }
}

/// Move-only acknowledgement owned by the Browser input consumer. Dropping a
/// stale Context input rejects its FIFO observation without an extra cancel flag.
pub struct RendererWorkerLifecycleInput {
    pub runtime: RendererBrowserContextRuntimeId,
    pub lifecycle: Arc<RendererWorkerLifecycle>,
    committed: watch::Sender<Option<u64>>,
}

impl RendererWorkerLifecycleInput {
    pub fn commit(self, browser_sequence: u64) {
        assert_ne!(
            browser_sequence, 0,
            "Browser occurrence must have a sequence"
        );
        self.committed.send_replace(Some(browser_sequence));
    }
}

type WorkerLifecycleHandler = Box<dyn Fn(RendererWorkerLifecycleInput) + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RendererWorkerLifecycleReporter {
    runtime: RendererBrowserContextRuntimeId,
    handler: Arc<Mutex<Option<WorkerLifecycleHandler>>>,
}

impl std::fmt::Debug for RendererWorkerLifecycleReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererWorkerLifecycleReporter")
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl RendererWorkerLifecycleReporter {
    pub(crate) fn new(runtime: RendererBrowserContextRuntimeId) -> Self {
        Self {
            runtime,
            handler: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn runtime(&self) -> RendererBrowserContextRuntimeId {
        self.runtime
    }

    pub(crate) fn install_handler(
        &self,
        handler: impl Fn(RendererWorkerLifecycleInput) + Send + Sync + 'static,
    ) {
        let mut slot = self.handler.lock();
        assert!(
            slot.is_none(),
            "one runtime can only have one Browser lifecycle owner"
        );
        *slot = Some(Box::new(handler));
    }

    pub(crate) fn report(
        &self,
        lifecycle: RendererWorkerLifecycle,
    ) -> RendererWorkerLifecycleObservation {
        let lifecycle = Arc::new(lifecycle);
        let (committed, observation) = watch::channel(None);
        let input = RendererWorkerLifecycleInput {
            runtime: self.runtime,
            lifecycle: lifecycle.clone(),
            committed,
        };
        // Enqueue under the reporter lock: concurrent producers in this Context
        // cannot reorder native transitions before the Browser consumes them.
        if let Some(handler) = self.handler.lock().as_ref() {
            handler(input);
        }
        RendererWorkerLifecycleObservation {
            runtime: self.runtime,
            lifecycle,
            committed: observation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn worker_lifecycle_observations_require_their_exact_native_acknowledgement() {
        let reporter = RendererWorkerLifecycleReporter::new(
            RendererBrowserContextRuntimeId::new_for_testing(31),
        );
        let fact = RendererWorkerLifecycle::SharedDestroyed(
            moli_shared_worker::SharedWorkerInstanceId::from_u64(7),
        );
        assert!(reporter.report(fact.clone()).committed().await.is_none());
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let pending = inputs.clone();
        reporter.install_handler(move |input| pending.lock().push(input));
        let rejected = reporter.report(fact.clone());
        let accepted = reporter.report(fact.clone());
        let clone = accepted.clone();
        assert!(accepted.committed.borrow().is_none());
        let accepted_input = inputs.lock().pop().unwrap();
        let rejected_input = inputs.lock().pop().unwrap();
        accepted_input.commit(43);
        drop(rejected_input);
        assert!(rejected.committed().await.is_none());
        for observation in [accepted, clone] {
            let committed = observation.committed().await.unwrap();
            assert_eq!(committed.runtime(), reporter.runtime());
            assert_eq!(committed.browser_sequence(), 43);
            assert_eq!(committed.lifecycle(), &fact);
        }
    }
}
