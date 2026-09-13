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

    #[test]
    fn document_source_retirement_keeps_one_lease_and_releases_context_registration() {
        let reporter =
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(24));
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let observed = inputs.clone();
        reporter.install_handler(move |input| observed.lock().push(input));
        let owner = crate::RendererOwnerLocalHostId::new_for_testing(3);
        let document = crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(
            crate::PageId::new_for_testing(5),
        )
        .identity();
        let captured = reporter.for_document(owner, document);
        let request = captured.start_request().unwrap();
        let duplicate_capture = reporter.for_document(owner, document);
        reporter.close_source(owner, document.document.page_id);
        reporter.close_source(owner, document.document.page_id);
        assert!(captured.start_request().is_none());
        assert!(duplicate_capture.start_request().is_none());
        assert!(
            inputs.lock().is_empty(),
            "repeat retirement cannot bypass the admitted request"
        );
        drop(request);
        assert!(matches!(
            inputs.lock().as_slice(),
            [RendererNetworkInput::SourceClosed { .. }]
        ));
        assert!(
            reporter.documents.lock().is_empty(),
            "Context cannot retain a closed Page registration"
        );
        drop((captured, duplicate_capture));
        assert_eq!(
            inputs.lock().len(),
            1,
            "captures do not own request completion"
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
    parent_request: Option<moli_page_types::SubresourceNetworkRequestHandle>,
    committed: watch::Sender<Option<(u64, RendererNetworkSource)>>,
}

impl RendererNetworkCommit {
    /// A physical preflight inherits only its still-admitted parent's authority.
    pub fn parent_request(&self) -> Option<moli_page_types::SubresourceNetworkRequestHandle> {
        self.parent_request
    }

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
    requests: Arc<Mutex<NetworkSourceState>>,
}

#[derive(Debug)]
enum NetworkSourceState {
    Active(usize),
    Retiring(std::num::NonZeroUsize),
    Closed,
}

/// One admitted request keeps only its native source, never a Page, Worker or VM.
/// Clones share the same request identity and release one source lease together.
#[derive(Clone, Debug)]
pub(crate) struct RendererNetworkRequest {
    lease: Arc<NetworkRequestInner>,
    handle: moli_page_types::SubresourceNetworkRequestHandle,
}

#[derive(Debug)]
struct NetworkRequestInner {
    reporter: RendererNetworkReporter,
    source: RendererNetworkSource,
    requests: Arc<Mutex<NetworkSourceState>>,
    admitted_handle: moli_page_types::SubresourceNetworkRequestHandle,
}

impl RendererNetworkRequest {
    #[cfg(test)]
    pub(crate) fn unobserved_for_test() -> Self {
        RendererWorkerNetworkReporter::unobserved_for_test()
            .start_request()
            .unwrap()
    }

    fn start(
        reporter: RendererNetworkReporter,
        source: RendererNetworkSource,
        requests: Arc<Mutex<NetworkSourceState>>,
        handle: moli_page_types::SubresourceNetworkRequestHandle,
    ) -> Option<Self> {
        {
            let mut state = requests.lock();
            let NetworkSourceState::Active(count) = &mut *state else {
                return None;
            };
            *count = count
                .checked_add(1)
                .expect("network request count exhausted");
        }
        Some(Self {
            lease: Arc::new(NetworkRequestInner {
                reporter,
                source,
                requests,
                admitted_handle: handle,
            }),
            handle,
        })
    }

    pub(crate) fn handle(&self) -> moli_page_types::SubresourceNetworkRequestHandle {
        self.handle
    }

    pub(crate) fn report(&self, item: ScriptNetworkOutputItem) -> RendererNetworkObservation {
        self.lease.reporter.report_source(
            self.lease.source.clone(),
            item,
            (self.handle != self.lease.admitted_handle).then_some(self.lease.admitted_handle),
        )
    }

    /// A physical preflight belongs to an already-admitted request, including
    /// a keepalive continuing after its Worker retires. It shares that lease,
    /// while ordinary clones keep their original physical request identity.
    pub(crate) fn preflight(&self) -> Self {
        Self {
            lease: self.lease.clone(),
            handle: moli_page_types::SubresourceNetworkRequestHandle::allocate(),
        }
    }
}

#[cfg(test)]
mod worker_request_tests {
    use super::*;

    #[test]
    fn admitted_preflight_keeps_retired_source_until_last_descendant_finishes() {
        let reporter =
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(1));
        let closed = Arc::new(Mutex::new(Vec::new()));
        let observed = closed.clone();
        reporter.install_handler(move |input| {
            if let RendererNetworkInput::SourceClosed { source, .. } = input {
                observed.lock().push(source);
            }
        });
        let identity =
            RendererWorkerIdentity::Shared(moli_shared_worker::SharedWorkerInstanceId::from_u64(1));
        let source = RendererWorkerNetworkReporter::new(reporter, identity.clone());
        let parent = source.start_request().unwrap();
        source.close_source();
        assert!(
            source.start_request().is_none(),
            "retirement rejects new top-level requests"
        );
        let preflight = parent.preflight();
        assert_ne!(preflight.handle(), parent.handle());
        assert_eq!(preflight.clone().handle(), preflight.handle());
        drop(parent);
        assert!(
            closed.lock().is_empty(),
            "admitted preflight still owns its source permission"
        );
        let redirected_preflight = preflight.preflight();
        assert_ne!(redirected_preflight.handle(), preflight.handle());
        drop(preflight);
        source.close_source();
        assert!(
            closed.lock().is_empty(),
            "a redirect cannot lose the original source lease"
        );
        drop(redirected_preflight);
        assert_eq!(
            *closed.lock(),
            [RendererNetworkSourceIdentity::Worker(identity)]
        );
    }
}

impl Drop for NetworkRequestInner {
    fn drop(&mut self) {
        let mut state = self.requests.lock();
        let close = match &mut *state {
            NetworkSourceState::Active(count) => {
                *count = count.checked_sub(1).expect("admitted request source lease");
                false
            }
            NetworkSourceState::Retiring(count) => {
                if let Some(remaining) = std::num::NonZeroUsize::new(count.get() - 1) {
                    *count = remaining;
                    false
                } else {
                    *state = NetworkSourceState::Closed;
                    true
                }
            }
            NetworkSourceState::Closed => unreachable!("live request outlasts source close"),
        };
        drop(state);
        if close {
            let source = self.source.identity();
            self.reporter.documents.lock().remove(&source);
            self.reporter.close_producer(source);
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
            None,
        )
    }

    pub(crate) fn identity(&self) -> &RendererWorkerIdentity {
        &self.source
    }

    pub(crate) fn new(reporter: RendererNetworkReporter, source: RendererWorkerIdentity) -> Self {
        Self {
            reporter,
            source,
            requests: Arc::new(Mutex::new(NetworkSourceState::Active(0))),
        }
    }

    pub(crate) fn start_request(&self) -> Option<RendererNetworkRequest> {
        RendererNetworkRequest::start(
            self.reporter.clone(),
            RendererNetworkSource::Worker(self.source.clone()),
            self.requests.clone(),
            moli_page_types::SubresourceNetworkRequestHandle::allocate(),
        )
    }

    pub(crate) fn close_source(&self) {
        close_network_requests(
            &self.reporter,
            RendererNetworkSourceIdentity::Worker(self.source.clone()),
            &self.requests,
        );
    }

    #[cfg(test)]
    pub(crate) fn unobserved_for_test() -> Self {
        Self::new(
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(1)),
            RendererWorkerIdentity::Shared(moli_shared_worker::SharedWorkerInstanceId::from_u64(1)),
        )
    }
}

/// Captured with the original Document, before any asynchronous continuation.
#[derive(Clone, Debug)]
pub(crate) struct RendererDocumentNetworkReporter {
    reporter: RendererNetworkReporter,
    source: RendererNetworkSource,
    requests: Arc<Mutex<NetworkSourceState>>,
}

impl RendererDocumentNetworkReporter {
    pub(crate) fn start_request(&self) -> Option<RendererNetworkRequest> {
        self.start_request_with_handle(moli_page_types::SubresourceNetworkRequestHandle::allocate())
    }

    pub(crate) fn start_request_with_handle(
        &self,
        handle: moli_page_types::SubresourceNetworkRequestHandle,
    ) -> Option<RendererNetworkRequest> {
        RendererNetworkRequest::start(
            self.reporter.clone(),
            self.source.clone(),
            self.requests.clone(),
            handle,
        )
    }
}

fn close_network_requests(
    reporter: &RendererNetworkReporter,
    source: RendererNetworkSourceIdentity,
    requests: &Mutex<NetworkSourceState>,
) -> bool {
    let close = {
        let mut state = requests.lock();
        let NetworkSourceState::Active(count) = *state else {
            return false;
        };
        *state = match std::num::NonZeroUsize::new(count) {
            Some(count) => NetworkSourceState::Retiring(count),
            None => NetworkSourceState::Closed,
        };
        count == 0
    };
    if close {
        reporter.close_producer(source);
    }
    close
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
    documents: Arc<
        Mutex<
            std::collections::HashMap<
                RendererNetworkSourceIdentity,
                Arc<Mutex<NetworkSourceState>>,
            >,
        >,
    >,
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
            documents: Default::default(),
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
            None,
        )
    }

    fn report_source(
        &self,
        source: RendererNetworkSource,
        item: impl Into<RendererNetworkOutputItem>,
        parent_request: Option<moli_page_types::SubresourceNetworkRequestHandle>,
    ) -> RendererNetworkObservation {
        let occurrence = Arc::new(RendererNetworkOccurrence {
            runtime: self.runtime,
            source,
            item: item.into(),
        });
        let (committed, observation) = watch::channel(None);
        let input = RendererNetworkInput::Observation(RendererNetworkCommit {
            occurrence: occurrence.clone(),
            parent_request,
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
        let source = RendererNetworkSourceIdentity::Page {
            owner_local_host_id,
            page,
        };
        let requests = self.documents.lock().get(&source).cloned();
        if let Some(requests) = requests {
            if close_network_requests(self, source.clone(), &requests) {
                self.documents.lock().remove(&source);
            }
        } else {
            self.close_producer(source);
        }
    }

    pub(crate) fn for_document(
        &self,
        owner_local_host_id: super::RendererOwnerLocalHostId,
        document: RendererDocumentLifecycleIdentity,
    ) -> RendererDocumentNetworkReporter {
        let source = RendererNetworkSource::Document {
            owner_local_host_id,
            document,
        };
        let requests = self
            .documents
            .lock()
            .entry(source.identity())
            .or_insert_with(|| Arc::new(Mutex::new(NetworkSourceState::Active(0))))
            .clone();
        RendererDocumentNetworkReporter {
            reporter: self.clone(),
            source,
            requests,
        }
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
