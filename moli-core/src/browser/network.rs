use std::sync::Arc;

use indexmap::IndexMap;

use super::{BrowserSequence, DocumentHandle};
use crate::page::{
    RendererDocumentLifecycleIdentity, RendererNetworkOccurrence, ScriptNetworkOutputItem,
    SubresourceBodyFinished, SubresourceBodyFinishedResult, SubresourceNetworkRecord,
    SubresourceRequestStarted, SubresourceResponseStarted,
};

/// A committed resource occurrence. Its source is a physical Browser Document,
/// including a reserved Document before commit, never a protocol Target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkOccurrence {
    pub document: DocumentHandle,
    pub renderer: Arc<RendererNetworkOccurrence>,
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
}

impl NetworkRequestState {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Recorded(_))
    }

    fn retained_bytes(&self) -> usize {
        match self {
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
    pub document: DocumentHandle,
    pub renderer_document: RendererDocumentLifecycleIdentity,
    pub sequence: BrowserSequence,
    pub state: NetworkRequestState,
}

impl NetworkRequestSnapshot {
    pub fn output_items(&self) -> Vec<ScriptNetworkOutputItem> {
        match &self.state {
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
        }
    }
}

pub(super) type NetworkRequestKey = (super::RendererPageResidenceIdentity, u64);

pub(super) fn request_key(occurrence: &RendererNetworkOccurrence) -> Option<NetworkRequestKey> {
    let handle = match occurrence.item.as_ref() {
        ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => record.request_handle()?,
        ScriptNetworkOutputItem::SubresourceRequestStarted(request) => request.handle(),
        ScriptNetworkOutputItem::SubresourceResponseStarted(response) => response.handle(),
        ScriptNetworkOutputItem::SubresourceDataReceived(data) => data.handle(),
        ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(message) => message.handle(),
        ScriptNetworkOutputItem::SubresourceBodyFinished(body) => body.handle(),
        ScriptNetworkOutputItem::WebSocketNetworkEvent(_)
        | ScriptNetworkOutputItem::WebSocketLifecycleEvent(_) => return None,
    };
    Some((
        super::RendererPageResidenceIdentity::from_parts(
            occurrence.owner_local_host_id,
            occurrence.document.document.page_id,
        ),
        handle.get(),
    ))
}

#[derive(Default)]
pub(super) struct NetworkRequests {
    entries: IndexMap<NetworkRequestKey, NetworkRequestSnapshot>,
}

impl NetworkRequests {
    pub(super) fn get(&self, key: NetworkRequestKey) -> Option<&NetworkRequestSnapshot> {
        self.entries.get(&key)
    }

    pub(super) fn snapshots(&self) -> impl Iterator<Item = NetworkRequestSnapshot> + '_ {
        self.entries.values().cloned()
    }

    pub(super) fn close_source(
        &mut self,
        page: super::RendererPageResidenceIdentity,
    ) -> Option<DocumentHandle> {
        let mut document = None;
        self.entries.retain(|(source, _), entry| {
            if *source == page {
                document = Some(entry.document);
                false
            } else {
                true
            }
        });
        document
    }

    pub(super) fn commit(
        &mut self,
        document: DocumentHandle,
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
        let state = match occurrence.item.as_ref() {
            ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                if previous.is_some() {
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
                    NetworkRequestState::Completed { .. } | NetworkRequestState::Recorded(_) => {
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
        };
        let renderer_document =
            previous.map_or(occurrence.document, |entry| entry.renderer_document);
        let completed = state.is_terminal();
        if completed {
            self.entries.shift_remove(&key);
        }
        self.entries.insert(
            key,
            NetworkRequestSnapshot {
                document,
                renderer_document,
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
                    evicted.push(*key);
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
        item: ScriptNetworkOutputItem,
    ) -> RendererNetworkOccurrence {
        RendererNetworkOccurrence {
            runtime: crate::RendererBrowserContextRuntimeId::new_for_testing(3),
            owner_local_host_id: crate::RendererOwnerLocalHostId::new_for_testing(11),
            document,
            item: Arc::new(item),
        }
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
        assert!(!requests.commit(document, &body, BrowserSequence::allocate()));
        assert!(requests.commit(document, &start, BrowserSequence::allocate()));
        let NetworkRequestState::Started(stored) =
            &requests.get(request_key(&start).unwrap()).unwrap().state
        else {
            panic!("request must be started");
        };
        assert!(Arc::ptr_eq(stored, &request));
        assert!(!requests.commit(document, &start, BrowserSequence::allocate()));
        assert!(requests.commit(document, &body, BrowserSequence::allocate()));
        assert!(!requests.commit(document, &body, BrowserSequence::allocate()));
        assert!(matches!(
            requests.snapshots().next().unwrap().state,
            NetworkRequestState::Completed { .. }
        ));
        assert_eq!(
            requests.close_source(request_key(&start).unwrap().0),
            Some(document)
        );
        assert_eq!(requests.snapshots().count(), 0);
        assert!(!requests.commit(document, &body, BrowserSequence::allocate()));
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
            assert!(requests.commit(document, &event, BrowserSequence::allocate()));
        }
        assert_eq!(requests.snapshots().count(), 256);
        assert!(
            requests
                .get((
                    super::super::RendererPageResidenceIdentity::from_parts(
                        crate::RendererOwnerLocalHostId::new_for_testing(11),
                        renderer.document.page_id
                    ),
                    1
                ))
                .is_none()
        );
        assert!(
            requests
                .get((
                    super::super::RendererPageResidenceIdentity::from_parts(
                        crate::RendererOwnerLocalHostId::new_for_testing(11),
                        renderer.document.page_id
                    ),
                    258
                ))
                .is_some()
        );
        for snapshot in requests.snapshots() {
            assert!(matches!(
                snapshot.output_items().as_slice(),
                [ScriptNetworkOutputItem::SubresourceNetworkRecord(_)]
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
        second_event.owner_local_host_id = crate::RendererOwnerLocalHostId::new_for_testing(12);
        let mut requests = NetworkRequests::default();
        assert!(requests.commit(first, &first_event, BrowserSequence::allocate()));
        assert!(requests.commit(second, &second_event, BrowserSequence::allocate()));
        assert_eq!(requests.snapshots().count(), 2);
        assert_eq!(
            requests.close_source(request_key(&first_event).unwrap().0),
            Some(first)
        );
        assert_eq!(requests.snapshots().next().unwrap().document, second);
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
                document,
                &occurrence(renderer, item.clone()),
                BrowserSequence::allocate()
            ));
        }
        let snapshot = requests.snapshots().next().unwrap();
        assert_eq!(snapshot.output_items(), items);
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
