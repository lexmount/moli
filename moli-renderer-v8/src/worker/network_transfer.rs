use moli_page_types::{
    ScriptNetworkOutputItem, SubresourceBodyFinished, SubresourceDataReceived,
    SubresourceNetworkRequestHandle, SubresourceRequestStarted, SubresourceResponseBody,
};
use parking_lot::Mutex;
use std::sync::Arc;

use super::{
    WorkerNetworkObserver,
    global_scope::{publish_worker_network_item, record_worker_fetch_response},
};
use crate::{
    network::{
        ResourceResponseFailure, ResourceResponseHead, ResourceResponseObserver,
        ResourceResponseResult,
    },
    runtime::{RendererWorkerNetworkReporter, RendererWorkerNetworkRequest},
};

/// One resource consumer's publication and completion permission. Work retains
/// this request, never its VM or parent pump. The last consumer settles the
/// request; its Context load or shared cache owns transport cancellation.
pub(crate) struct WorkerResourceTransfer {
    state: Mutex<WorkerResourceTransferState>,
    observer: WorkerNetworkObserver,
}

enum WorkerResourceTransferState {
    Requested(RendererWorkerNetworkRequest),
    Responding(RendererWorkerNetworkRequest),
    Finished,
}

impl WorkerResourceTransfer {
    pub(super) fn start(
        source: &RendererWorkerNetworkReporter,
        observer: WorkerNetworkObserver,
        request: impl FnOnce(&RendererWorkerNetworkRequest) -> SubresourceRequestStarted,
    ) -> Option<Arc<Self>> {
        let network = source.start_request()?;
        Some(Self::from_request(network, observer, request))
    }

    fn from_request(
        network: RendererWorkerNetworkRequest,
        observer: WorkerNetworkObserver,
        request: impl FnOnce(&RendererWorkerNetworkRequest) -> SubresourceRequestStarted,
    ) -> Arc<Self> {
        publish_worker_network_item(
            &observer,
            &network,
            ScriptNetworkOutputItem::SubresourceRequestStarted(Arc::new(request(&network))),
        );
        Arc::new(Self {
            state: Mutex::new(WorkerResourceTransferState::Requested(network)),
            observer,
        })
    }

    pub(crate) fn preflight(
        parent: &RendererWorkerNetworkRequest,
        observer: WorkerNetworkObserver,
        request: &moli_fetch::Request,
        keepalive: bool,
    ) -> Arc<Self> {
        let resource_type = match request.browser_request_metadata() {
            Some(moli_fetch::BrowserRequestMetadata::Xhr) => {
                moli_page_types::SubresourceResourceType::Xhr
            }
            _ => moli_page_types::SubresourceResourceType::Fetch,
        };
        Self::from_request(parent.preflight(), observer, |network| {
            super::global_scope::worker_request_started(
                network,
                request
                    .cookie_context
                    .initiator_url
                    .as_ref()
                    .expect("a CORS preflight has an initiator"),
                &request.url,
                &request.method,
                &request.request_headers,
                &None,
                resource_type,
            )
            .with_keepalive(keepalive)
        })
    }

    pub(super) fn handle(&self) -> SubresourceNetworkRequestHandle {
        match &*self.state.lock() {
            WorkerResourceTransferState::Requested(network)
            | WorkerResourceTransferState::Responding(network) => network.handle(),
            WorkerResourceTransferState::Finished => {
                panic!("completed request has no continuation")
            }
        }
    }

    pub(super) fn update_request(
        &self,
        request: impl FnOnce(&RendererWorkerNetworkRequest) -> SubresourceRequestStarted,
    ) {
        let state = self.state.lock();
        if let WorkerResourceTransferState::Requested(network) = &*state {
            publish_worker_network_item(
                &self.observer,
                network,
                ScriptNetworkOutputItem::SubresourceRequestUpdated(Arc::new(request(network))),
            );
        }
    }

    pub(crate) fn complete(&self, result: &ResourceResponseResult) {
        match result {
            Ok(response) => self.response_completed(response),
            Err(error) => self.failed(error),
        }
    }

    pub(crate) fn response_completed(&self, response: &moli_fetch::Response) {
        self.body_completed(
            ResourceResponseHead {
                head: response.head(),
                network_request_headers: response
                    .network_request_extra_info()
                    .map(|info| info.headers.clone()),
            },
            SubresourceResponseBody::from_fetch_response(response),
        );
    }

    pub(crate) fn body_completed(&self, head: ResourceResponseHead, body: SubresourceResponseBody) {
        let previous = std::mem::replace(
            &mut *self.state.lock(),
            WorkerResourceTransferState::Finished,
        );
        let (network, body) = match previous {
            WorkerResourceTransferState::Requested(network) => {
                record_worker_fetch_response(
                    &self.observer,
                    &network,
                    head.head,
                    head.network_request_headers,
                );
                let body = SubresourceBodyFinished::ready(network.handle(), body);
                (network, body)
            }
            WorkerResourceTransferState::Responding(network) => {
                let body = SubresourceBodyFinished::ready_after_streaming(network.handle(), body);
                (network, body)
            }
            WorkerResourceTransferState::Finished => return,
        };
        publish_worker_network_item(
            &self.observer,
            &network,
            ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(body)),
        );
    }

    pub(crate) fn failed(&self, error: &ResourceResponseFailure) {
        let previous = std::mem::replace(
            &mut *self.state.lock(),
            WorkerResourceTransferState::Finished,
        );
        let network = match previous {
            WorkerResourceTransferState::Requested(network) => {
                if let ResourceResponseFailure::PartialBody { response, .. } = error {
                    record_worker_fetch_response(
                        &self.observer,
                        &network,
                        response.head.clone(),
                        response.network_request_headers.clone(),
                    );
                }
                network
            }
            WorkerResourceTransferState::Responding(network) => network,
            WorkerResourceTransferState::Finished => return,
        };
        let body = match error {
            ResourceResponseFailure::Request(message) => {
                SubresourceBodyFinished::failed(network.handle(), message.clone())
            }
            ResourceResponseFailure::PartialBody { message, body, .. } => {
                SubresourceBodyFinished::failed_with_partial_body(
                    network.handle(),
                    message.clone(),
                    body.clone(),
                )
            }
        };
        publish_worker_network_item(
            &self.observer,
            &network,
            ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(body)),
        );
    }
}

impl ResourceResponseObserver for WorkerResourceTransfer {
    fn response_started(&self, response: Arc<ResourceResponseHead>) {
        let mut state = self.state.lock();
        match std::mem::replace(&mut *state, WorkerResourceTransferState::Finished) {
            WorkerResourceTransferState::Requested(network) => {
                record_worker_fetch_response(
                    &self.observer,
                    &network,
                    response.head.clone(),
                    response.network_request_headers.clone(),
                );
                *state = WorkerResourceTransferState::Responding(network);
            }
            WorkerResourceTransferState::Responding(_) => {
                panic!("one resource response head per consumer")
            }
            WorkerResourceTransferState::Finished => {}
        }
    }

    fn data_received(&self, bytes: usize) {
        let state = self.state.lock();
        match &*state {
            WorkerResourceTransferState::Responding(network) => publish_worker_network_item(
                &self.observer,
                network,
                ScriptNetworkOutputItem::SubresourceDataReceived(SubresourceDataReceived::new(
                    network.handle(),
                    bytes,
                    bytes,
                )),
            ),
            WorkerResourceTransferState::Requested(_) => {
                panic!("resource data must follow its response head")
            }
            WorkerResourceTransferState::Finished => {}
        }
    }
}

impl Drop for WorkerResourceTransfer {
    fn drop(&mut self) {
        if !matches!(self.state.get_mut(), WorkerResourceTransferState::Finished) {
            self.failed(&ResourceResponseFailure::Request(
                "Worker resource load cancelled".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::global_scope::worker_request_started;

    #[test]
    fn worker_resource_terminal_is_single_use_with_or_without_streaming() {
        use crate::runtime::RendererNetworkOutputItem;
        use crate::worker::WorkerToParentMessage;
        use moli_page_types::SubresourceBodyFinishedResult;

        for streamed in [false, true] {
            for failed in [false, true] {
                let source = RendererWorkerNetworkReporter::unobserved_for_test();
                let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
                let url = url::Url::parse("data:text/javascript,//ok").unwrap();
                let response = crate::network_host::local_url_response(&url).unwrap();
                let head = Arc::new(ResourceResponseHead {
                    head: response.head(),
                    network_request_headers: None,
                });
                let transfer = WorkerResourceTransfer::start(
                    &source,
                    WorkerNetworkObserver::Channel(send.downgrade()),
                    |network| {
                        worker_request_started(
                            network,
                            &url,
                            &url,
                            "GET",
                            &moli_fetch::RequestHeaders::default(),
                            &None,
                            moli_page_types::SubresourceResourceType::Script,
                        )
                    },
                )
                .unwrap();
                if streamed {
                    transfer.response_started(head.clone());
                    transfer.data_received(2);
                    if !failed {
                        transfer.data_received(2);
                    }
                }
                if failed {
                    transfer.failed(&ResourceResponseFailure::PartialBody {
                        message: "truncated".into(),
                        response: head.clone(),
                        body: SubresourceResponseBody::from_bytes(b"//".to_vec()),
                    });
                } else {
                    transfer.response_completed(&response);
                }
                // Late callbacks and the final lease drop cannot duplicate or
                // change the committed terminal result.
                transfer.failed(&ResourceResponseFailure::Request("late".into()));
                transfer.response_completed(&response);
                transfer.response_started(head);
                transfer.data_received(2);
                drop(transfer);
                let mut items = Vec::new();
                while let Ok(WorkerToParentMessage::Network(observation)) = receive.try_recv() {
                    let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                        panic!("script producer must publish resource facts")
                    };
                    items.push(item.clone());
                }
                assert_eq!(
                    items.len(),
                    if streamed {
                        if failed { 4 } else { 5 }
                    } else {
                        3
                    }
                );
                let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = items[0].as_ref()
                else {
                    panic!("start first")
                };
                let ScriptNetworkOutputItem::SubresourceResponseStarted(head) = items[1].as_ref()
                else {
                    panic!("one real response head before terminal")
                };
                assert_eq!(head.handle(), start.handle());
                let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) =
                    items.last().unwrap().as_ref()
                else {
                    panic!("terminal last")
                };
                assert_eq!(terminal.handle(), start.handle());
                match (failed, terminal.result()) {
                    (false, SubresourceBodyFinishedResult::Ready(body)) => {
                        assert_eq!(body.clone_body_bytes(), b"//ok");
                        assert_eq!(terminal.data_was_streamed(), streamed);
                    }
                    (
                        true,
                        SubresourceBodyFinishedResult::FailedWithPartialBody {
                            error_text,
                            partial_body,
                        },
                    ) => {
                        assert_eq!(error_text, "truncated");
                        assert_eq!(partial_body.clone_body_bytes(), b"//");
                    }
                    result => panic!("the first terminal outcome must win: {result:?}"),
                }
            }
        }
    }
}
