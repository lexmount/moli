use moli_page_types::{
    ScriptNetworkOutputItem, SubresourceBodyFinished, SubresourceDataReceived,
    SubresourceNetworkRequestHandle, SubresourceRequestStarted, SubresourceResponseBody,
};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::{
    network::{
        ResourceResponseFailure, ResourceResponseHead, ResourceResponseObserver,
        ResourceResponseResult,
    },
    runtime::RendererNetworkRequest,
};

/// One resource consumer's publication and completion permission. Work retains
/// this request, never its VM or parent pump. The last consumer settles the
/// request; its Context load or shared cache owns transport cancellation.
pub(crate) struct ResourceTransfer {
    state: Mutex<ResourceTransferState>,
    observer: Box<dyn Fn(crate::runtime::RendererNetworkObservation) + Send + Sync>,
}

enum ResourceTransferState {
    Requested(RendererNetworkRequest),
    Responding(RendererNetworkRequest),
    Finished,
}

impl ResourceTransfer {
    /// Return the first receipt to the admitting owner for its request-admission
    /// turn. Later I/O uses the captured observer.
    #[must_use]
    pub(crate) fn start(
        network: RendererNetworkRequest,
        observer: impl Fn(crate::runtime::RendererNetworkObservation) + Send + Sync + 'static,
        request: impl FnOnce(&RendererNetworkRequest) -> SubresourceRequestStarted,
    ) -> (Arc<Self>, crate::runtime::RendererNetworkObservation) {
        let started = network.report(ScriptNetworkOutputItem::SubresourceRequestStarted(
            Arc::new(request(&network)),
        ));
        let transfer = Arc::new(Self {
            state: Mutex::new(ResourceTransferState::Requested(network)),
            observer: Box::new(observer),
        });
        (transfer, started)
    }

    fn publish(&self, network: &RendererNetworkRequest, item: ScriptNetworkOutputItem) {
        (self.observer)(network.report(item));
    }

    fn record_response(
        &self,
        network: &RendererNetworkRequest,
        head: moli_fetch::ResponseHead,
        network_request_headers: Option<Vec<(String, String)>>,
    ) {
        let response = moli_page_types::SubresourceResponseStarted::new(
            network.handle(),
            head.redirect_chain.into_iter().map(Into::into).collect(),
            head.final_url,
            head.status,
            head.headers,
            head.cookie_set_reports,
        )
        .with_request_cookie_report(head.request_cookie_report)
        .with_from_cache(head.from_cache)
        .with_negotiated_http_version(head.negotiated_http_version)
        .with_network_request_headers(network_request_headers);
        self.publish(
            network,
            ScriptNetworkOutputItem::SubresourceResponseStarted(Arc::new(response)),
        );
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.request().handle()
    }

    pub(crate) fn request(&self) -> RendererNetworkRequest {
        match &*self.state.lock() {
            ResourceTransferState::Requested(network)
            | ResourceTransferState::Responding(network) => network.clone(),
            ResourceTransferState::Finished => {
                panic!("completed request has no continuation")
            }
        }
    }

    pub(crate) fn update_request(
        &self,
        request: impl FnOnce(&RendererNetworkRequest) -> SubresourceRequestStarted,
    ) {
        let state = self.state.lock();
        if let ResourceTransferState::Requested(network) = &*state {
            self.publish(
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
        let previous = std::mem::replace(&mut *self.state.lock(), ResourceTransferState::Finished);
        let (network, body) = match previous {
            ResourceTransferState::Requested(network) => {
                self.record_response(&network, head.head, head.network_request_headers);
                let body = SubresourceBodyFinished::ready(network.handle(), body);
                (network, body)
            }
            ResourceTransferState::Responding(network) => {
                let body = SubresourceBodyFinished::ready_after_streaming(network.handle(), body);
                (network, body)
            }
            ResourceTransferState::Finished => return,
        };
        self.publish(
            &network,
            ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(body)),
        );
    }

    pub(crate) fn failed(&self, error: &ResourceResponseFailure) {
        let previous = std::mem::replace(&mut *self.state.lock(), ResourceTransferState::Finished);
        let network = match previous {
            ResourceTransferState::Requested(network) => {
                if let ResourceResponseFailure::PartialBody { response, .. } = error {
                    self.record_response(
                        &network,
                        response.head.clone(),
                        response.network_request_headers.clone(),
                    );
                }
                network
            }
            ResourceTransferState::Responding(network) => network,
            ResourceTransferState::Finished => return,
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
        self.publish(
            &network,
            ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(body)),
        );
    }
}

impl ResourceResponseObserver for ResourceTransfer {
    fn response_started(&self, response: Arc<ResourceResponseHead>) {
        let mut state = self.state.lock();
        match std::mem::replace(&mut *state, ResourceTransferState::Finished) {
            ResourceTransferState::Requested(network) => {
                self.record_response(
                    &network,
                    response.head.clone(),
                    response.network_request_headers.clone(),
                );
                *state = ResourceTransferState::Responding(network);
            }
            ResourceTransferState::Responding(_) => {
                panic!("one resource response head per consumer")
            }
            ResourceTransferState::Finished => {}
        }
    }

    fn data_received(&self, bytes: usize) {
        let state = self.state.lock();
        match &*state {
            ResourceTransferState::Responding(network) => self.publish(
                network,
                ScriptNetworkOutputItem::SubresourceDataReceived(SubresourceDataReceived::new(
                    network.handle(),
                    bytes,
                    bytes,
                )),
            ),
            ResourceTransferState::Requested(_) => {
                panic!("resource data must follow its response head")
            }
            ResourceTransferState::Finished => {}
        }
    }
}

impl Drop for ResourceTransfer {
    fn drop(&mut self) {
        if !matches!(self.state.get_mut(), ResourceTransferState::Finished) {
            self.failed(&ResourceResponseFailure::Request(
                "Resource load cancelled".into(),
            ));
        }
    }
}
