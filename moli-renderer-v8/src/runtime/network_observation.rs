use std::sync::Arc;

use moli_page_types::{ChildFrameDocumentNetworkActivitySnapshot, ScriptNetworkOutputItem};
use parking_lot::Mutex;
use tokio::sync::watch;

use super::{RendererBrowserContextRuntimeId, RendererDocumentLifecycleIdentity};

/// Identity of a physical worker execution, not an Inspector target or session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RendererWorkerIdentity {
    Dedicated(u64),
    Shared(moli_shared_worker::SharedWorkerInstanceId),
    Service {
        version: u64,
        run: super::RendererServiceWorkerRunIdentity,
    },
}

impl RendererWorkerIdentity {
    pub fn unavailable_message(&self) -> &'static str {
        match self {
            Self::Dedicated(_) => "DedicatedWorkerRuntimeUnavailable",
            Self::Shared(_) => "SharedWorkerRuntimeUnavailable",
            Self::Service { .. } => "ServiceWorkerRuntimeUnavailable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererNetworkSource {
    Document {
        owner_local_host_id: super::RendererOwnerLocalHostId,
        document: RendererDocumentLifecycleIdentity,
    },
    Worker(RendererWorkerIdentity),
}

/// Request IDs are local to a physical producer. Page request admission spans
/// document.open; a Service Worker version must never span execution runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RendererNetworkSourceIdentity {
    Page {
        owner_local_host_id: super::RendererOwnerLocalHostId,
        page: super::PageId,
    },
    Worker(RendererWorkerIdentity),
}

impl RendererNetworkSource {
    pub fn identity(&self) -> RendererNetworkSourceIdentity {
        match self {
            Self::Document {
                owner_local_host_id,
                document,
            } => RendererNetworkSourceIdentity::Page {
                owner_local_host_id: *owner_local_host_id,
                page: document.document.page_id,
            },
            Self::Worker(worker) => RendererNetworkSourceIdentity::Worker(worker.clone()),
        }
    }

    pub fn document(
        &self,
    ) -> Option<(
        super::RendererOwnerLocalHostId,
        RendererDocumentLifecycleIdentity,
    )> {
        match self {
            Self::Document {
                owner_local_host_id,
                document,
            } => Some((*owner_local_host_id, *document)),
            Self::Worker(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererNetworkOutputItem {
    Resource(Arc<ScriptNetworkOutputItem>),
    ChildDocument(Arc<ChildFrameDocumentNetworkActivitySnapshot>),
    WorkerFetch {
        policy_document: Option<(
            super::RendererOwnerLocalHostId,
            super::RendererDocumentToken,
        )>,
        pause: super::RendererWorkerFetchPause,
    },
}

impl From<ScriptNetworkOutputItem> for RendererNetworkOutputItem {
    fn from(item: ScriptNetworkOutputItem) -> Self {
        Self::Resource(Arc::new(item))
    }
}

impl RendererNetworkOutputItem {
    pub fn renderer_transport_charge_bytes(&self) -> usize {
        match self {
            Self::Resource(item) => item.renderer_transport_charge_bytes(),
            Self::ChildDocument(item) => item.renderer_transport_charge_bytes(),
            Self::WorkerFetch { pause, .. } => pause.renderer_transport_charge_bytes(),
        }
    }
}

/// One physical producer occurrence, shared by native input and source FIFO.
/// A complete-only diagnostic remains complete-only; it does not invent a start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererNetworkOccurrence {
    pub runtime: RendererBrowserContextRuntimeId,
    pub source: RendererNetworkSource,
    pub item: RendererNetworkOutputItem,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_source_retirement_waits_for_exact_request_leases_not_reporter_clones() {
        let reporter =
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(23));
        let inputs = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let pending = inputs.clone();
        reporter.install_handler(move |input| pending.lock().push_back(input));
        let worker =
            RendererWorkerNetworkReporter::new(reporter, RendererWorkerIdentity::Dedicated(41));
        let first = worker.start_request().unwrap();
        let same_request = first.clone();
        let second = worker.start_request().unwrap();
        assert_ne!(first.handle(), second.handle());
        worker.close_source();
        worker.close_source();
        assert!(worker.start_request().is_none());
        assert!(inputs.lock().is_empty());
        drop(first);
        drop(second);
        assert!(
            inputs.lock().is_empty(),
            "a cloned request still owns the original transfer"
        );
        same_request.report(ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(
            moli_page_types::SubresourceBodyFinished::failed(
                same_request.handle(),
                "detached transport failed".into(),
            ),
        )));
        assert!(matches!(
            inputs.lock().pop_front(),
            Some(RendererNetworkInput::Observation(_))
        ));
        drop(same_request);
        assert!(matches!(
            inputs.lock().pop_front(),
            Some(RendererNetworkInput::SourceClosed {
                source: RendererNetworkSourceIdentity::Worker(RendererWorkerIdentity::Dedicated(
                    41
                )),
                ..
            })
        ));
        worker.close_source();
        drop(worker);
        assert!(
            inputs.lock().is_empty(),
            "reporter/drop paths must not repeat source close"
        );
    }

    #[tokio::test]
    async fn native_network_receipts_require_exact_commit_and_source_close_follows_the_fifo() {
        let runtime = RendererBrowserContextRuntimeId::new_for_testing(19);
        let reporter = RendererNetworkReporter::new(runtime);
        let owner = crate::RendererOwnerLocalHostId::new_for_testing(11);
        let document = crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(
            crate::PageId::new_for_testing(7),
        )
        .identity();
        let item = ScriptNetworkOutputItem::SubresourceBodyFinished(std::sync::Arc::new(
            moli_page_types::SubresourceBodyFinished::failed(
                moli_page_types::SubresourceNetworkRequestHandle::new(1),
                "failed".into(),
            ),
        ));
        assert!(
            reporter
                .report(owner, document, item.clone())
                .committed()
                .await
                .is_none()
        );
        let inputs = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let pending = inputs.clone();
        reporter.install_handler(move |input| pending.lock().push_back(input));
        let rejected = reporter.report(owner, document, item.clone());
        let accepted = reporter.report(owner, document, item.clone());
        let clone = accepted.clone();
        reporter.close_source(owner, document.document.page_id);
        drop(inputs.lock().pop_front().unwrap());
        assert!(rejected.committed().await.is_none());
        let RendererNetworkInput::Observation(input) = inputs.lock().pop_front().unwrap() else {
            panic!("receipt must precede closure");
        };
        input.commit(
            47,
            RendererNetworkSource::Document {
                owner_local_host_id: owner,
                document,
            },
        );
        for observation in [accepted, clone] {
            let committed = observation.committed().await.unwrap();
            assert_eq!(committed.browser_sequence(), 47);
            assert_eq!(committed.occurrence().item, item.clone().into());
        }
        assert!(
            matches!(inputs.lock().pop_front(), Some(RendererNetworkInput::SourceClosed { runtime: actual, source: RendererNetworkSourceIdentity::Page { owner_local_host_id, page } }) if actual == runtime && owner_local_host_id == owner && page == document.document.page_id)
        );
        assert!(inputs.lock().is_empty());
    }
}

#[derive(Clone, Debug)]
pub struct RendererNetworkObservation {
    occurrence: Arc<RendererNetworkOccurrence>,
    committed: watch::Receiver<Option<(u64, RendererNetworkSource)>>,
}

impl PartialEq for RendererNetworkObservation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.occurrence, &other.occurrence)
    }
}
impl Eq for RendererNetworkObservation {}

#[derive(Clone, Debug)]
pub struct RendererCommittedNetworkObservation {
    occurrence: Arc<RendererNetworkOccurrence>,
    browser_sequence: u64,
}

impl RendererCommittedNetworkObservation {
    pub fn occurrence(&self) -> &RendererNetworkOccurrence {
        &self.occurrence
    }
    pub fn browser_sequence(&self) -> u64 {
        self.browser_sequence
    }
}

impl RendererNetworkObservation {
    #[cfg(test)]
    pub(crate) fn worker_record_for_test(&self) -> &moli_page_types::SubresourceNetworkRecord {
        let RendererNetworkOutputItem::Resource(item) = &self.occurrence.item else {
            panic!("expected a Worker resource")
        };
        let ScriptNetworkOutputItem::SubresourceNetworkRecord(record) = item.as_ref() else {
            panic!("expected a complete Worker record")
        };
        record
    }

    pub async fn committed(mut self) -> Option<RendererCommittedNetworkObservation> {
        loop {
            if let Some((browser_sequence, source)) = self.committed.borrow_and_update().clone() {
                if self.occurrence.source != source {
                    Arc::make_mut(&mut self.occurrence).source = source;
                }
                return Some(RendererCommittedNetworkObservation {
                    occurrence: self.occurrence,
                    browser_sequence,
                });
            }
            self.committed.changed().await.ok()?;
        }
    }

    pub(crate) fn item(&self) -> &RendererNetworkOutputItem {
        &self.occurrence.item
    }
}

/// The fetch result and its native receipt travel together through child
/// completion and load delivery, without a second raw protocol result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererChildDocumentNetworkObservation(RendererNetworkObservation);

impl RendererChildDocumentNetworkObservation {
    pub(crate) fn new(
        runtime: &super::RendererBrowserContextRuntime,
        source: (
            super::RendererOwnerLocalHostId,
            RendererDocumentLifecycleIdentity,
        ),
        activity: ChildFrameDocumentNetworkActivitySnapshot,
    ) -> Self {
        Self(runtime.report_network(
            source.0,
            source.1,
            RendererNetworkOutputItem::ChildDocument(Arc::new(activity)),
        ))
    }

    pub(crate) fn activity(&self) -> &ChildFrameDocumentNetworkActivitySnapshot {
        let RendererNetworkOutputItem::ChildDocument(activity) = self.0.item() else {
            unreachable!("child network constructor fixes its payload kind");
        };
        activity
    }

    pub(crate) fn into_observation(self) -> RendererNetworkObservation {
        self.0
    }

    pub async fn committed(self) -> Option<RendererCommittedNetworkObservation> {
        self.0.committed().await
    }

    #[cfg(test)]
    pub(crate) fn unobserved_for_test(response: ChildFrameDocumentNetworkActivitySnapshot) -> Self {
        let reporter =
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(1));
        let document = super::RendererDocumentLifecycleJournalHandle::new_initial(
            super::PageId::new_for_testing(1),
        )
        .identity();
        Self(reporter.report(
            super::RendererOwnerLocalHostId::new_for_testing(1),
            document,
            RendererNetworkOutputItem::ChildDocument(Arc::new(response)),
        ))
    }
}

/// Only the native owner may acknowledge input. Dropping it rejects the FIFO
/// observation, including shutdown and input from a revoked reservation.
pub enum RendererNetworkInput {
    Observation(RendererNetworkCommit),
    SourceClosed {
        runtime: RendererBrowserContextRuntimeId,
        source: RendererNetworkSourceIdentity,
    },
}

pub struct RendererNetworkCommit {
    pub occurrence: Arc<RendererNetworkOccurrence>,
    committed: watch::Sender<Option<(u64, RendererNetworkSource)>>,
}

impl RendererNetworkCommit {
    pub fn commit(self, browser_sequence: u64, source: RendererNetworkSource) {
        assert_ne!(browser_sequence, 0, "native occurrence needs a sequence");
        self.committed
            .send_replace(Some((browser_sequence, source)));
    }
}

impl Drop for RendererNetworkCommit {
    fn drop(&mut self) {
        if self.committed.borrow().is_none()
            && let RendererNetworkOutputItem::WorkerFetch { pause, .. } = &self.occurrence.item
        {
            // The source journal can outlive a rejected native input. Its
            // observation must not retain an unowned request decision.
            pause.release();
        }
    }
}

type NetworkHandler = Box<dyn Fn(RendererNetworkInput) + Send + Sync>;

/// A producer bound before a Worker thread starts. Its parent carries only the
/// returned receipt, and cannot change the owner or commit the request itself.
#[derive(Clone, Debug)]
pub(crate) struct RendererWorkerNetworkReporter {
    reporter: RendererNetworkReporter,
    source: RendererWorkerIdentity,
    requests: Arc<Mutex<WorkerNetworkSourceState>>,
}

#[derive(Debug)]
enum WorkerNetworkSourceState {
    Active(usize),
    Retiring(std::num::NonZeroUsize),
    Closed,
}

/// One admitted request keeps only its native source, never a Worker or VM.
/// Clones share the same request identity and release one source lease together.
#[derive(Clone, Debug)]
pub(crate) struct RendererWorkerNetworkRequest(Arc<WorkerNetworkRequestInner>);

#[derive(Debug)]
struct WorkerNetworkRequestInner {
    source: RendererWorkerNetworkReporter,
    handle: moli_page_types::SubresourceNetworkRequestHandle,
}

impl RendererWorkerNetworkRequest {
    pub(crate) fn handle(&self) -> moli_page_types::SubresourceNetworkRequestHandle {
        self.0.handle
    }

    pub(crate) fn report(&self, item: ScriptNetworkOutputItem) -> RendererNetworkObservation {
        self.0.source.report_item(item)
    }
}

impl Drop for WorkerNetworkRequestInner {
    fn drop(&mut self) {
        let mut state = self.source.requests.lock();
        let close = match &mut *state {
            WorkerNetworkSourceState::Active(count) => {
                *count = count.checked_sub(1).expect("admitted request source lease");
                false
            }
            WorkerNetworkSourceState::Retiring(count) => {
                if let Some(remaining) = std::num::NonZeroUsize::new(count.get() - 1) {
                    *count = remaining;
                    false
                } else {
                    *state = WorkerNetworkSourceState::Closed;
                    true
                }
            }
            WorkerNetworkSourceState::Closed => unreachable!("live request outlasts source close"),
        };
        drop(state);
        if close {
            self.source
                .reporter
                .close_producer(RendererNetworkSourceIdentity::Worker(
                    self.source.source.clone(),
                ));
        }
    }
}

impl RendererWorkerNetworkReporter {
    pub(crate) fn report_pause(
        &self,
        pause: super::RendererWorkerFetchPause,
        policy_document: Option<(
            super::RendererOwnerLocalHostId,
            super::RendererDocumentToken,
        )>,
    ) -> RendererNetworkObservation {
        self.reporter.report_source(
            RendererNetworkSource::Worker(self.source.clone()),
            RendererNetworkOutputItem::WorkerFetch {
                policy_document,
                pause,
            },
        )
    }

    pub(crate) fn identity(&self) -> &RendererWorkerIdentity {
        &self.source
    }

    pub(crate) fn new(reporter: RendererNetworkReporter, source: RendererWorkerIdentity) -> Self {
        Self {
            reporter,
            source,
            requests: Arc::new(Mutex::new(WorkerNetworkSourceState::Active(0))),
        }
    }

    pub(crate) fn start_request(&self) -> Option<RendererWorkerNetworkRequest> {
        let mut state = self.requests.lock();
        let WorkerNetworkSourceState::Active(count) = &mut *state else {
            return None;
        };
        *count = count
            .checked_add(1)
            .expect("Worker request count exhausted");
        Some(RendererWorkerNetworkRequest(Arc::new(
            WorkerNetworkRequestInner {
                source: self.clone(),
                handle: moli_page_types::SubresourceNetworkRequestHandle::allocate(),
            },
        )))
    }

    pub(crate) fn report(
        &self,
        mut record: moli_page_types::SubresourceNetworkRecord,
    ) -> RendererNetworkObservation {
        if record.request_handle().is_none() {
            record = record
                .with_request_handle(moli_page_types::SubresourceNetworkRequestHandle::allocate());
        }
        self.report_item(ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            record,
        )))
    }

    pub(crate) fn report_item(&self, item: ScriptNetworkOutputItem) -> RendererNetworkObservation {
        self.reporter
            .report_source(RendererNetworkSource::Worker(self.source.clone()), item)
    }

    pub(crate) fn close_source(&self) {
        let mut state = self.requests.lock();
        let mut close = false;
        if let WorkerNetworkSourceState::Active(count) = *state {
            *state = match std::num::NonZeroUsize::new(count) {
                Some(count) => WorkerNetworkSourceState::Retiring(count),
                None => {
                    close = true;
                    WorkerNetworkSourceState::Closed
                }
            };
        }
        drop(state);
        if close {
            self.reporter
                .close_producer(RendererNetworkSourceIdentity::Worker(self.source.clone()));
        }
    }

    #[cfg(test)]
    pub(crate) fn unobserved_for_test() -> Self {
        Self::new(
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(1)),
            RendererWorkerIdentity::Shared(moli_shared_worker::SharedWorkerInstanceId::from_u64(1)),
        )
    }
}

impl PartialEq for RendererWorkerNetworkReporter {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && Arc::ptr_eq(&self.reporter.handler, &other.reporter.handler)
    }
}
impl Eq for RendererWorkerNetworkReporter {}

#[derive(Clone)]
pub(crate) struct RendererNetworkReporter {
    runtime: RendererBrowserContextRuntimeId,
    handler: Arc<Mutex<Option<NetworkHandler>>>,
}

impl std::fmt::Debug for RendererNetworkReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererNetworkReporter")
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl RendererNetworkReporter {
    pub(crate) fn runtime(&self) -> RendererBrowserContextRuntimeId {
        self.runtime
    }

    pub(crate) fn new(runtime: RendererBrowserContextRuntimeId) -> Self {
        Self {
            runtime,
            handler: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn install_handler(
        &self,
        handler: impl Fn(RendererNetworkInput) + Send + Sync + 'static,
    ) {
        let mut slot = self.handler.lock();
        assert!(slot.is_none(), "one runtime has one native Network owner");
        *slot = Some(Box::new(handler));
    }

    pub(crate) fn report(
        &self,
        owner_local_host_id: super::RendererOwnerLocalHostId,
        document: RendererDocumentLifecycleIdentity,
        item: impl Into<RendererNetworkOutputItem>,
    ) -> RendererNetworkObservation {
        self.report_source(
            RendererNetworkSource::Document {
                owner_local_host_id,
                document,
            },
            item,
        )
    }

    pub(crate) fn report_source(
        &self,
        source: RendererNetworkSource,
        item: impl Into<RendererNetworkOutputItem>,
    ) -> RendererNetworkObservation {
        let occurrence = Arc::new(RendererNetworkOccurrence {
            runtime: self.runtime,
            source,
            item: item.into(),
        });
        let (committed, observation) = watch::channel(None);
        let input = RendererNetworkInput::Observation(RendererNetworkCommit {
            occurrence: occurrence.clone(),
            committed,
        });
        if let Some(handler) = self.handler.lock().as_ref() {
            handler(input);
        }
        RendererNetworkObservation {
            occurrence,
            committed: observation,
        }
    }

    pub(crate) fn close_source(
        &self,
        owner_local_host_id: super::RendererOwnerLocalHostId,
        page: super::PageId,
    ) {
        self.close_producer(RendererNetworkSourceIdentity::Page {
            owner_local_host_id,
            page,
        });
    }

    pub(crate) fn close_producer(&self, source: RendererNetworkSourceIdentity) {
        if let Some(handler) = self.handler.lock().as_ref() {
            handler(RendererNetworkInput::SourceClosed {
                runtime: self.runtime,
                source,
            });
        }
    }
}
