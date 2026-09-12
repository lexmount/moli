use std::sync::Arc;

use indexmap::IndexMap;

use super::{BrowserSequence, DocumentHandle, WorkerHandle};
use crate::page::{
    RendererNetworkOccurrence, RendererNetworkOutputItem, RendererNetworkSource,
    RendererNetworkSourceIdentity, ScriptNetworkOutputItem, SubresourceBodyFinished,
    SubresourceBodyFinishedResult, SubresourceNetworkRecord, SubresourceRequestStarted,
    SubresourceResponseStarted,
};

/// The native owner of a request, never a protocol Target. Worker execution-run
/// identity is retained separately in the exact renderer source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkOwner {
    Document(DocumentHandle),
    Worker(WorkerHandle),
}

impl NetworkOwner {
    pub fn context(self) -> super::BrowserContextId {
        match self {
            Self::Document(document) => document.web_contents().context(),
            Self::Worker(
                WorkerHandle::Dedicated { context, .. }
                | WorkerHandle::Shared { context, .. }
                | WorkerHandle::Service { context, .. },
            ) => context,
        }
    }
}

/// A committed resource occurrence from its physical Browser owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkOccurrence {
    pub owner: NetworkOwner,
    pub renderer: Arc<RendererNetworkOccurrence>,
}

/// A committed, single-use decision for a physical Worker request. The
/// observing Document supplies policy, never the request's execution owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerFetchPause {
    pub worker: WorkerHandle,
    pub document: DocumentHandle,
    pub sequence: BrowserSequence,
    pub pause: crate::page::RendererWorkerFetchPause,
    pub(super) renderer_document: super::RendererPageResidenceIdentity,
}

impl WorkerFetchPause {
    fn key(&self) -> NetworkRequestKey {
        worker_pause_key(&self.pause)
    }
}

fn worker_pause_key(pause: &crate::page::RendererWorkerFetchPause) -> NetworkRequestKey {
    (
        RendererNetworkSourceIdentity::Worker(pause.worker().clone()),
        NetworkRequestIdentity::Resource(pause.handle().get()),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkRequestState {
    Started(Arc<SubresourceRequestStarted>),
    Responding {
        request: Arc<SubresourceRequestStarted>,
        response: Arc<SubresourceResponseStarted>,
    },
    Completed {
        request: Arc<SubresourceRequestStarted>,
        response: Option<Arc<SubresourceResponseStarted>>,
        body: Arc<SubresourceBodyFinished>,
    },
    Recorded(SubresourceNetworkRecord),
    ChildDocument(Arc<crate::page::ChildFrameDocumentNetworkActivitySnapshot>),
}

impl NetworkRequestState {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Recorded(_) | Self::ChildDocument(_)
        )
    }

    fn retained_bytes(&self) -> usize {
        match self {
            Self::ChildDocument(response) => response.renderer_transport_charge_bytes(),
            Self::Recorded(record) => record.renderer_transport_charge_bytes(),
            Self::Completed {
                request,
                response,
                body,
            } => ScriptNetworkOutputItem::SubresourceRequestStarted(request.clone())
                .renderer_transport_charge_bytes()
                .saturating_add(
                    response
                        .as_ref()
                        .map(|response| {
                            ScriptNetworkOutputItem::SubresourceResponseStarted(response.clone())
                                .renderer_transport_charge_bytes()
                        })
                        .unwrap_or(0),
                )
                .saturating_add(
                    ScriptNetworkOutputItem::SubresourceBodyFinished(body.clone())
                        .renderer_transport_charge_bytes(),
                ),
            Self::Started(_) | Self::Responding { .. } => 0,
        }
    }
}

/// Current request state, not a second transcript of streamed chunks/messages.
/// Completed bodies share the existing memory/file-backed capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkRequestSnapshot {
    pub owner: NetworkOwner,
    pub renderer_source: RendererNetworkSource,
    pub sequence: BrowserSequence,
    pub state: NetworkRequestState,
}

impl NetworkRequestSnapshot {
    pub fn output_items(&self) -> Vec<RendererNetworkOutputItem> {
        let items = match &self.state {
            NetworkRequestState::ChildDocument(response) => {
                return vec![RendererNetworkOutputItem::ChildDocument(response.clone())];
            }
            NetworkRequestState::Started(request) => {
                vec![ScriptNetworkOutputItem::SubresourceRequestStarted(
                    request.clone(),
                )]
            }
            NetworkRequestState::Responding { request, response } => vec![
                ScriptNetworkOutputItem::SubresourceRequestStarted(request.clone()),
                ScriptNetworkOutputItem::SubresourceResponseStarted(response.clone()),
            ],
            NetworkRequestState::Completed {
                request,
                response,
                body,
            } => {
                let mut items = vec![ScriptNetworkOutputItem::SubresourceRequestStarted(
                    request.clone(),
                )];
                items.extend(
                    response
                        .iter()
                        .cloned()
                        .map(ScriptNetworkOutputItem::SubresourceResponseStarted),
                );
                items.push(ScriptNetworkOutputItem::SubresourceBodyFinished(
                    body.clone(),
                ));
                items
            }
            NetworkRequestState::Recorded(record) => {
                vec![ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                    record.clone(),
                ))]
            }
        };
        items.into_iter().map(Into::into).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum NetworkRequestIdentity {
    Resource(u64),
    ChildDocument(String),
}

pub(super) type NetworkRequestKey = (RendererNetworkSourceIdentity, NetworkRequestIdentity);

pub(super) fn request_key(occurrence: &RendererNetworkOccurrence) -> Option<NetworkRequestKey> {
    let source = occurrence.source.identity();
    let item = match &occurrence.item {
        RendererNetworkOutputItem::Resource(item) => item,
        RendererNetworkOutputItem::WorkerFetch { pause, .. } => {
            return Some(worker_pause_key(pause));
        }
        RendererNetworkOutputItem::ChildDocument(response) => {
            return Some((
                source,
                NetworkRequestIdentity::ChildDocument(response.loader_id.clone()),
            ));
        }
    };
    let handle = match item.as_ref() {
        ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => record.request_handle()?,
        ScriptNetworkOutputItem::SubresourceRequestStarted(request)
        | ScriptNetworkOutputItem::SubresourceRequestUpdated(request) => request.handle(),
        ScriptNetworkOutputItem::SubresourceResponseStarted(response) => response.handle(),
        ScriptNetworkOutputItem::SubresourceDataReceived(data) => data.handle(),
        ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(message) => message.handle(),
        ScriptNetworkOutputItem::SubresourceBodyFinished(body) => body.handle(),
        ScriptNetworkOutputItem::WebSocketNetworkEvent(_)
        | ScriptNetworkOutputItem::WebSocketLifecycleEvent(_) => return None,
    };
    Some((source, NetworkRequestIdentity::Resource(handle.get())))
}

#[derive(Default)]
pub(super) struct NetworkRequests {
    entries: IndexMap<NetworkRequestKey, NetworkRequestSnapshot>,
    worker_pauses: IndexMap<NetworkRequestKey, WorkerFetchPause>,
}

impl Drop for NetworkRequests {
    fn drop(&mut self) {
        for pause in self.worker_pauses.values() {
            pause.pause.release();
        }
    }
}

impl NetworkRequests {
    pub(super) fn worker_pauses(&self) -> impl Iterator<Item = WorkerFetchPause> + '_ {
        self.worker_pauses
            .values()
            .filter(|entry| entry.pause.is_available())
            .cloned()
    }

    pub(super) fn worker_pause(
        &self,
        pause: &crate::page::RendererWorkerFetchPause,
    ) -> Option<WorkerFetchPause> {
        self.worker_pauses
            .get(&worker_pause_key(pause))
            .filter(|entry| entry.pause == *pause && pause.is_available())
            .cloned()
    }

    pub(super) fn pause_worker(&mut self, pause: WorkerFetchPause) {
        if self
            .worker_pauses
            .get(&pause.key())
            .is_some_and(|old| old.pause == pause.pause)
        {
            return;
        }
        if let Some(old) = self.worker_pauses.insert(pause.key(), pause) {
            old.pause.invalidate();
        }
    }

    pub(super) fn start_worker_decision(
        &mut self,
        pause: &WorkerFetchPause,
        decision: crate::page::WorkerFetchDecision,
    ) -> Result<crate::page::PendingWorkerFetchDecision, String> {
        let key = pause.key();
        if self.worker_pauses.get(&key) != Some(pause) {
            return Err("Worker request pause is no longer available".into());
        }
        let pending = pause.pause.start_decision(decision)?;
        self.worker_pauses.shift_remove(&key);
        Ok(pending)
    }

    pub(super) fn release_worker_pauses_for_document(&mut self, document: DocumentHandle) {
        self.worker_pauses.retain(|_, entry| {
            if entry.document != document {
                return true;
            }
            entry.pause.release();
            false
        });
    }

    pub(super) fn retire_worker_pauses(&mut self, worker: WorkerHandle) {
        self.worker_pauses.retain(|_, entry| {
            if entry.worker != worker {
                return true;
            }
            entry.pause.invalidate();
            false
        });
    }

    pub(super) fn get(&self, key: &NetworkRequestKey) -> Option<&NetworkRequestSnapshot> {
        self.entries.get(key)
    }

    pub(super) fn snapshots(&self) -> impl Iterator<Item = NetworkRequestSnapshot> + '_ {
        self.entries.values().cloned()
    }

    pub(super) fn close_source(
        &mut self,
        producer: &RendererNetworkSourceIdentity,
    ) -> Option<NetworkOwner> {
        let mut owner = None;
        self.worker_pauses.retain(|(source, _), entry| {
            if source == producer {
                owner = Some(NetworkOwner::Worker(entry.worker));
                entry.pause.invalidate();
                return false;
            }
            if let RendererNetworkSourceIdentity::Page {
                owner_local_host_id,
                page,
            } = producer
                && entry.renderer_document
                    == super::RendererPageResidenceIdentity::from_parts(*owner_local_host_id, *page)
            {
                entry.pause.release();
                return false;
            }
            true
        });
        self.entries.retain(|(source, _), entry| {
            if source == producer {
                owner = Some(entry.owner);
                false
            } else {
                true
            }
        });
        owner
    }

    pub(super) fn commit(
        &mut self,
        owner: NetworkOwner,
        occurrence: &RendererNetworkOccurrence,
        sequence: BrowserSequence,
    ) -> bool {
        let Some(key) = request_key(occurrence) else {
            // WebSocket frames are transient observations, not HTTP snapshots.
            return true;
        };
        let previous = self.entries.get(&key);
        if previous.is_some_and(|entry| entry.state.is_terminal()) {
            return false;
        }
        let state = match &occurrence.item {
            RendererNetworkOutputItem::WorkerFetch { .. } => {
                unreachable!("decision admission is separate from Network state")
            }
            RendererNetworkOutputItem::ChildDocument(response) => {
                NetworkRequestState::ChildDocument(response.clone())
            }
            RendererNetworkOutputItem::Resource(item) => match item.as_ref() {
                ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                    if previous.is_some() {
                        return false;
                    }
                    NetworkRequestState::Started(request.clone())
                }
                ScriptNetworkOutputItem::SubresourceRequestUpdated(request) => {
                    let Some(NetworkRequestSnapshot {
                        state: NetworkRequestState::Started(previous),
                        ..
                    }) = previous
                    else {
                        return false;
                    };
                    if request == previous {
                        return false;
                    }
                    NetworkRequestState::Started(request.clone())
                }
                ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
                    let Some(NetworkRequestSnapshot {
                        state: NetworkRequestState::Started(request),
                        ..
                    }) = previous
                    else {
                        return false;
                    };
                    NetworkRequestState::Responding {
                        request: request.clone(),
                        response: response.clone(),
                    }
                }
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
                    let Some(previous) = previous else {
                        return false;
                    };
                    let (request, response) = match &previous.state {
                        NetworkRequestState::Started(request) => (request.clone(), None),
                        NetworkRequestState::Responding { request, response } => {
                            (request.clone(), Some(response.clone()))
                        }
                        NetworkRequestState::Completed { .. }
                        | NetworkRequestState::Recorded(_)
                        | NetworkRequestState::ChildDocument(_) => {
                            unreachable!()
                        }
                    };
                    if response.is_none()
                        && matches!(body.result(), SubresourceBodyFinishedResult::Ready(_))
                    {
                        return false;
                    }
                    // Preserve the real terminal: diagnostic failure records lose
                    // the response headers and partially captured streaming body.
                    NetworkRequestState::Completed {
                        request,
                        response,
                        body: body.clone(),
                    }
                }
                ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                    NetworkRequestState::Recorded((**record).clone())
                }
                ScriptNetworkOutputItem::SubresourceDataReceived(_)
                | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_) => {
                    let Some(previous) = previous else {
                        return false;
                    };
                    previous.state.clone()
                }
                ScriptNetworkOutputItem::WebSocketNetworkEvent(_)
                | ScriptNetworkOutputItem::WebSocketLifecycleEvent(_) => unreachable!(),
            },
        };
        let renderer_source = previous.map_or_else(
            || occurrence.source.clone(),
            |entry| entry.renderer_source.clone(),
        );
        let completed = state.is_terminal();
        if completed {
            if let Some(pause) = self.worker_pauses.shift_remove(&key) {
                pause.pause.invalidate();
            }
            self.entries.shift_remove(&key);
        }
        self.entries.insert(
            key,
            NetworkRequestSnapshot {
                owner,
                renderer_source,
                sequence,
                state,
            },
        );
        if completed {
            self.trim_completed();
        }
        true
    }

    fn trim_completed(&mut self) {
        // This is recovery retention, not authority to cancel live requests.
        // Bound both completed count and retained payload, independently of CDP.
        let mut count = 0;
        let mut bytes = 0usize;
        let mut evicted = Vec::new();
        for (key, entry) in self.entries.iter().rev() {
            if entry.state.is_terminal() {
                count += 1;
                bytes = bytes.saturating_add(entry.state.retained_bytes());
                if count > 256 || bytes > 16 * 1024 * 1024 {
                    evicted.push(key.clone());
                }
            }
        }
        for key in evicted {
            self.entries.shift_remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{BrowserContextId, DocumentId, WebContentsHandle, WebContentsId};
    use crate::page::RendererDocumentLifecycleIdentity;
    use crate::page::{
        SubresourceBodyFinished, SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType,
        SubresourceResourceType,
    };

    fn source() -> (DocumentHandle, RendererDocumentLifecycleIdentity) {
        let page = crate::PageId::new_for_testing(17);
        (
            DocumentHandle::new(
                WebContentsHandle::new(BrowserContextId::allocate(), WebContentsId::allocate()),
                DocumentId::allocate(),
            ),
            RendererDocumentLifecycleIdentity {
                frame: crate::page::RendererFrameToken { page_id: page },
                document: crate::page::RendererDocumentToken::new_for_testing(page, 1),
                epoch: crate::page::RendererLifecycleEpoch(1),
            },
        )
    }

    fn occurrence(
        document: RendererDocumentLifecycleIdentity,
        item: impl Into<RendererNetworkOutputItem>,
    ) -> RendererNetworkOccurrence {
        RendererNetworkOccurrence {
            runtime: crate::RendererBrowserContextRuntimeId::new_for_testing(3),
            source: RendererNetworkSource::Document {
                owner_local_host_id: crate::RendererOwnerLocalHostId::new_for_testing(11),
                document,
            },
            item: item.into(),
        }
    }

    #[test]
    fn native_child_network_retention_is_shared_bounded_and_scoped_to_its_renderer() {
        use crate::page::{
            ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentNetworkResponse,
            ChildFrameDocumentNetworkSnapshot, SubresourceResponseBody,
        };
        let (document, renderer) = source();
        let response = Arc::new(ChildFrameDocumentNetworkActivitySnapshot {
            frame_id: "child".into(),
            parent_frame_id: None,
            loader_id: "loader".into(),
            snapshot: ChildFrameDocumentNetworkSnapshot {
                request_url: "https://example.test/child".into(),
                request_method: "GET".into(),
                request_headers: Vec::new(),
                response: Ok(ChildFrameDocumentNetworkResponse {
                    final_url: "https://example.test/child".into(),
                    status: 200,
                    response_headers: Vec::new(),
                    encoded_data_length: 4,
                    response_body: Some(SubresourceResponseBody::from_bytes(b"body".to_vec())),
                    from_cache: false,
                }),
            },
        });
        let input = occurrence(
            renderer,
            RendererNetworkOutputItem::ChildDocument(response.clone()),
        );
        let mut requests = NetworkRequests::default();
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &input,
            BrowserSequence::allocate()
        ));
        assert!(!requests.commit(
            NetworkOwner::Document(document),
            &input,
            BrowserSequence::allocate()
        ));
        let snapshot = requests.snapshots().next().unwrap();
        let items = snapshot.output_items();
        let [RendererNetworkOutputItem::ChildDocument(stored)] = items.as_slice() else {
            panic!("child completion must not fabricate another start");
        };
        assert!(Arc::ptr_eq(stored, &response));
        let mut peer = input.clone();
        peer.source = RendererNetworkSource::Document {
            owner_local_host_id: crate::RendererOwnerLocalHostId::new_for_testing(12),
            document: renderer,
        };
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &peer,
            BrowserSequence::allocate()
        ));
        assert_eq!(
            requests.snapshots().count(),
            2,
            "local Page and loader IDs may collide across owners"
        );
        assert_eq!(
            requests.close_source(&request_key(&input).unwrap().0),
            Some(NetworkOwner::Document(document))
        );
        assert_eq!(requests.snapshots().count(), 1);
        for id in 0..258 {
            let mut response = response.as_ref().clone();
            response.loader_id = format!("loader-{id}");
            assert!(requests.commit(
                NetworkOwner::Document(document),
                &occurrence(
                    renderer,
                    RendererNetworkOutputItem::ChildDocument(Arc::new(response))
                ),
                BrowserSequence::allocate()
            ));
        }
        assert_eq!(requests.snapshots().count(), 256);
        let mut large = response.as_ref().clone();
        large.loader_id = "large".into();
        large.snapshot.response.as_mut().unwrap().response_body =
            Some(SubresourceResponseBody::from_bytes(vec![
                b'x';
                16 * 1024 * 1024
                    + 1
            ]));
        let large = occurrence(
            renderer,
            RendererNetworkOutputItem::ChildDocument(Arc::new(large)),
        );
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &large,
            BrowserSequence::allocate()
        ));
        assert!(
            requests.get(&request_key(&large).unwrap()).is_none(),
            "oversized completed response must not escape the retained-byte cap"
        );
        let mut failed = response.as_ref().clone();
        failed.loader_id = "failed".into();
        failed.snapshot.response = Err("connection closed before headers".into());
        let failure = occurrence(
            renderer,
            RendererNetworkOutputItem::ChildDocument(Arc::new(failed.clone())),
        );
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &failure,
            BrowserSequence::allocate()
        ));
        assert!(!requests.commit(
            NetworkOwner::Document(document),
            &failure,
            BrowserSequence::allocate()
        ));
        assert!(requests.get(&request_key(&failure).unwrap()).is_some());
        failed.loader_id = "oversized-failure".into();
        failed.snapshot.response = Err("x".repeat(16 * 1024 * 1024 + 1));
        let oversized_failure = occurrence(
            renderer,
            RendererNetworkOutputItem::ChildDocument(Arc::new(failed)),
        );
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &oversized_failure,
            BrowserSequence::allocate()
        ));
        assert!(
            requests
                .get(&request_key(&oversized_failure).unwrap())
                .is_none(),
            "failure diagnostics must obey the same retained-byte cap"
        );
    }

    #[test]
    fn native_network_request_update_preserves_one_admission_and_current_recovery_metadata() {
        let (document, renderer) = source();
        let owner = NetworkOwner::Document(document);
        let handle = SubresourceNetworkRequestHandle::new(19);
        let request = |method: &str, body: &[u8]| {
            Arc::new(
                SubresourceRequestStarted::new(
                    handle,
                    None,
                    "https://example.test/".parse().unwrap(),
                    "https://example.test/request".parse().unwrap(),
                    method.into(),
                    vec![("x-method".into(), method.into())],
                    None,
                    SubresourceResourceType::Fetch,
                    SubresourceRequestInitiatorType::Script,
                    None,
                )
                .with_request_body_bytes(Some(body.to_vec())),
            )
        };
        let original = request("POST", b"original");
        let updated = request("PATCH", &[0, 128, 255]);
        let start = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceRequestStarted(original.clone()),
        );
        let update = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceRequestUpdated(updated.clone()),
        );
        let mut requests = NetworkRequests::default();
        assert!(
            !requests.commit(owner, &update, BrowserSequence::allocate()),
            "metadata cannot admit a request"
        );
        assert!(requests.commit(owner, &start, BrowserSequence::allocate()));
        assert!(requests.commit(owner, &update, BrowserSequence::allocate()));
        assert!(!requests.commit(owner, &update, BrowserSequence::allocate()));
        let snapshot = requests.snapshots().next().unwrap();
        let NetworkRequestState::Started(current) = snapshot.state else {
            panic!("the request remains in its original stage")
        };
        assert!(Arc::ptr_eq(&current, &updated));
        assert_eq!(requests.snapshots().count(), 1);
        assert_eq!(
            original.method(),
            "POST",
            "the original occurrence remains immutable"
        );
        let response = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceResponseStarted(Arc::new(
                SubresourceResponseStarted::new(
                    handle,
                    Vec::new(),
                    updated.url().clone(),
                    200,
                    Vec::new(),
                    Vec::new(),
                ),
            )),
        );
        assert!(requests.commit(owner, &response, BrowserSequence::allocate()));
        let stale = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceRequestUpdated(original),
        );
        assert!(
            !requests.commit(owner, &stale, BrowserSequence::allocate()),
            "a response seals request metadata"
        );
        let body = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(
                SubresourceBodyFinished::ready(
                    handle,
                    crate::page::SubresourceResponseBody::from_bytes(b"ok".to_vec()),
                ),
            )),
        );
        assert!(requests.commit(owner, &body, BrowserSequence::allocate()));
        assert!(!requests.commit(owner, &stale, BrowserSequence::allocate()));
        let snapshot = requests.snapshots().next().unwrap();
        let NetworkRequestState::Completed { request, .. } = &snapshot.state else {
            panic!("one native terminal")
        };
        assert!(Arc::ptr_eq(request, &updated));
        assert_eq!(
            snapshot.output_items().len(),
            3,
            "recovery keeps current facts, not the override history"
        );
    }

    #[test]
    fn native_network_state_shares_request_facts_and_rejects_duplicate_or_unadmitted_completion() {
        let (document, renderer) = source();
        let handle = SubresourceNetworkRequestHandle::new(1);
        let request = Arc::new(SubresourceRequestStarted::new(
            handle,
            None,
            "https://example.test/".parse().unwrap(),
            "https://example.test/fetch".parse().unwrap(),
            "GET".into(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            SubresourceRequestInitiatorType::Script,
            None,
        ));
        let start = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceRequestStarted(request.clone()),
        );
        let body = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceBodyFinished(std::sync::Arc::new(
                SubresourceBodyFinished::failed(handle, "net::ERR_ABORTED".into()),
            )),
        );
        let mut requests = NetworkRequests::default();
        assert!(!requests.commit(
            NetworkOwner::Document(document),
            &body,
            BrowserSequence::allocate()
        ));
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &start,
            BrowserSequence::allocate()
        ));
        let NetworkRequestState::Started(stored) =
            &requests.get(&request_key(&start).unwrap()).unwrap().state
        else {
            panic!("request must be started");
        };
        assert!(Arc::ptr_eq(stored, &request));
        assert!(!requests.commit(
            NetworkOwner::Document(document),
            &start,
            BrowserSequence::allocate()
        ));
        assert!(requests.commit(
            NetworkOwner::Document(document),
            &body,
            BrowserSequence::allocate()
        ));
        assert!(!requests.commit(
            NetworkOwner::Document(document),
            &body,
            BrowserSequence::allocate()
        ));
        assert!(matches!(
            requests.snapshots().next().unwrap().state,
            NetworkRequestState::Completed { .. }
        ));
        assert_eq!(
            requests.close_source(&request_key(&start).unwrap().0),
            Some(NetworkOwner::Document(document))
        );
        assert_eq!(requests.snapshots().count(), 0);
        assert!(!requests.commit(
            NetworkOwner::Document(document),
            &body,
            BrowserSequence::allocate()
        ));
    }

    #[test]
    fn native_network_complete_only_recovery_is_bounded_and_does_not_fabricate_a_start() {
        let (document, renderer) = source();
        let mut requests = NetworkRequests::default();
        for id in 1..=258 {
            let record = SubresourceNetworkRecord::failure(
                None,
                "https://example.test/".parse().unwrap(),
                "https://example.test/resource".parse().unwrap(),
                "GET".into(),
                Vec::new(),
                None,
                SubresourceResourceType::Script,
                "net::ERR_ABORTED".into(),
            )
            .with_request_handle(SubresourceNetworkRequestHandle::new(id));
            let event = occurrence(
                renderer,
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
            );
            assert!(requests.commit(
                NetworkOwner::Document(document),
                &event,
                BrowserSequence::allocate()
            ));
        }
        assert_eq!(requests.snapshots().count(), 256);
        assert!(
            requests
                .get(&(
                    RendererNetworkSourceIdentity::Page {
                        owner_local_host_id: crate::RendererOwnerLocalHostId::new_for_testing(11),
                        page: renderer.document.page_id
                    },
                    NetworkRequestIdentity::Resource(1)
                ))
                .is_none()
        );
        assert!(
            requests
                .get(&(
                    RendererNetworkSourceIdentity::Page {
                        owner_local_host_id: crate::RendererOwnerLocalHostId::new_for_testing(11),
                        page: renderer.document.page_id
                    },
                    NetworkRequestIdentity::Resource(258)
                ))
                .is_some()
        );
        for snapshot in requests.snapshots() {
            assert!(matches!(
                snapshot.output_items().as_slice(),
                [RendererNetworkOutputItem::Resource(item)] if matches!(item.as_ref(), ScriptNetworkOutputItem::SubresourceNetworkRecord(_))
            ));
        }
    }

    #[test]
    fn native_network_equal_local_page_and_request_ids_do_not_alias_between_renderer_owners() {
        let (first, renderer) = source();
        let second =
            DocumentHandle::new(first.web_contents(), crate::browser::DocumentId::allocate());
        let request = SubresourceNetworkRecord::failure(
            None,
            "https://example.test/".parse().unwrap(),
            "https://example.test/resource".parse().unwrap(),
            "GET".into(),
            Vec::new(),
            None,
            SubresourceResourceType::Script,
            "net::ERR_ABORTED".into(),
        )
        .with_request_handle(SubresourceNetworkRequestHandle::new(1));
        let first_event = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(request)),
        );
        let mut second_event = first_event.clone();
        second_event.source = RendererNetworkSource::Document {
            owner_local_host_id: crate::RendererOwnerLocalHostId::new_for_testing(12),
            document: renderer,
        };
        let mut requests = NetworkRequests::default();
        assert!(requests.commit(
            NetworkOwner::Document(first),
            &first_event,
            BrowserSequence::allocate()
        ));
        assert!(requests.commit(
            NetworkOwner::Document(second),
            &second_event,
            BrowserSequence::allocate()
        ));
        assert_eq!(requests.snapshots().count(), 2);
        assert_eq!(
            requests.close_source(&request_key(&first_event).unwrap().0),
            Some(NetworkOwner::Document(first))
        );
        assert_eq!(
            requests.snapshots().next().unwrap().owner,
            NetworkOwner::Document(second)
        );
    }

    #[test]
    fn native_worker_network_equal_handles_are_scoped_by_kind_and_physical_run() {
        use crate::page::{RendererServiceWorkerRunIdentity, RendererWorkerIdentity};
        let (document, renderer) = source();
        let request = SubresourceNetworkRecord::failure(
            None,
            "https://example.test/".parse().unwrap(),
            "https://example.test/probe".parse().unwrap(),
            "GET".into(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "net::ERR_ABORTED".into(),
        )
        .with_request_handle(SubresourceNetworkRequestHandle::new(1));
        let page = occurrence(
            renderer,
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(request)),
        );
        let mut inputs = vec![(NetworkOwner::Document(document), page.clone())];
        for worker in [
            RendererWorkerIdentity::Shared(moli_shared_worker::SharedWorkerInstanceId::from_u64(1)),
            RendererWorkerIdentity::Service {
                version: 1,
                run: RendererServiceWorkerRunIdentity::fresh(),
            },
            RendererWorkerIdentity::Service {
                version: 1,
                run: RendererServiceWorkerRunIdentity::fresh(),
            },
            RendererWorkerIdentity::Dedicated(1),
        ] {
            let handle = match &worker {
                RendererWorkerIdentity::Dedicated(instance) => WorkerHandle::Dedicated {
                    context: document.web_contents().context(),
                    instance: *instance,
                },
                RendererWorkerIdentity::Shared(instance) => WorkerHandle::Shared {
                    context: document.web_contents().context(),
                    instance: *instance,
                },
                RendererWorkerIdentity::Service { version, .. } => WorkerHandle::Service {
                    context: document.web_contents().context(),
                    version: *version,
                },
            };
            let mut input = page.clone();
            input.source = RendererNetworkSource::Worker(worker);
            inputs.push((NetworkOwner::Worker(handle), input));
        }
        let mut requests = NetworkRequests::default();
        for (owner, input) in &inputs {
            assert!(requests.commit(*owner, input, BrowserSequence::allocate()));
            assert!(!requests.commit(*owner, input, BrowserSequence::allocate()));
        }
        assert_eq!(requests.snapshots().count(), 5);
        // Closing one physical run must not evict its successor, a Shared
        // Worker with the same local number, or the creating Page.
        assert_eq!(
            requests.close_source(&inputs[2].1.source.identity()),
            Some(inputs[2].0)
        );
        assert_eq!(requests.snapshots().count(), 4);
        for index in [0, 1, 3, 4] {
            assert!(
                requests
                    .get(&request_key(&inputs[index].1).unwrap())
                    .is_some()
            );
        }
    }

    #[test]
    fn native_network_failed_stream_snapshot_retains_exact_headers_and_partial_body() {
        let (document, renderer) = source();
        let handle = SubresourceNetworkRequestHandle::new(1);
        let request = Arc::new(SubresourceRequestStarted::new(
            handle,
            None,
            "https://example.test/".parse().unwrap(),
            "https://example.test/stream".parse().unwrap(),
            "GET".into(),
            Vec::new(),
            None,
            SubresourceResourceType::Xhr,
            SubresourceRequestInitiatorType::Script,
            None,
        ));
        let response = Arc::new(SubresourceResponseStarted::new(
            handle,
            Vec::new(),
            "https://example.test/stream".parse().unwrap(),
            200,
            vec![("x-native-header".into(), "retained".into())],
            Vec::new(),
        ));
        let body = Arc::new(SubresourceBodyFinished::failed_with_partial_body(
            handle,
            "net::ERR_ABORTED".into(),
            crate::page::SubresourceResponseBody::from_bytes(b"partial".to_vec()),
        ));
        let items = [
            ScriptNetworkOutputItem::SubresourceRequestStarted(request.clone()),
            ScriptNetworkOutputItem::SubresourceResponseStarted(response.clone()),
            ScriptNetworkOutputItem::SubresourceBodyFinished(body.clone()),
        ];
        let mut requests = NetworkRequests::default();
        for item in &items {
            assert!(requests.commit(
                NetworkOwner::Document(document),
                &occurrence(renderer, item.clone()),
                BrowserSequence::allocate()
            ));
        }
        let snapshot = requests.snapshots().next().unwrap();
        assert_eq!(
            snapshot.output_items(),
            items.map(RendererNetworkOutputItem::from)
        );
        let NetworkRequestState::Completed {
            request: stored_request,
            response: Some(stored_response),
            body: stored_body,
        } = snapshot.state
        else {
            panic!("failed stream must keep its staged facts");
        };
        assert!(Arc::ptr_eq(&stored_request, &request));
        assert!(Arc::ptr_eq(&stored_response, &response));
        assert!(Arc::ptr_eq(&stored_body, &body));
        let SubresourceBodyFinishedResult::FailedWithPartialBody { partial_body, .. } =
            stored_body.result()
        else {
            panic!("partial capture must survive native recovery");
        };
        assert_eq!(partial_body.clone_body_bytes(), b"partial");
    }
}
