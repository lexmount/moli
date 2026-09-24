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
    runtime::{RendererNetworkObservation, RendererNetworkRequest},
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
    Finished(SubresourceNetworkRequestHandle),
}

impl ResourceTransferState {
    fn handle(&self) -> SubresourceNetworkRequestHandle {
        match self {
            Self::Requested(network) | Self::Responding(network) => network.handle(),
            Self::Finished(handle) => *handle,
        }
    }
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
        self.observe(network.report(item));
    }

    pub(crate) fn observe(&self, observation: RendererNetworkObservation) {
        (self.observer)(observation);
    }

    fn record_response(
        network: &RendererNetworkRequest,
        response: ResourceResponseHead,
        observer: &mut impl FnMut(RendererNetworkObservation),
    ) {
        let ResourceResponseHead {
            head,
            status_text,
            network_request_headers,
        } = response;
        let response = moli_page_types::SubresourceResponseStarted::new(
            network.handle(),
            head.redirect_chain.into_iter().map(Into::into).collect(),
            head.final_url,
            head.status,
            head.headers,
            head.cookie_set_reports,
        )
        .with_status_text(status_text)
        .with_request_cookie_report(head.request_cookie_report)
        .with_from_cache(head.from_cache)
        .with_negotiated_http_version(head.negotiated_http_version)
        .with_network_request_headers(network_request_headers);
        observer(
            network.report(ScriptNetworkOutputItem::SubresourceResponseStarted(
                Arc::new(response),
            )),
        );
    }

    pub(crate) fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.state.lock().handle()
    }

    /// Completion releases the source lease. A late continuation must not
    /// acquire it again, even while another consumer still retains the handle.
    pub(crate) fn request(&self) -> Option<RendererNetworkRequest> {
        match &*self.state.lock() {
            ResourceTransferState::Requested(network)
            | ResourceTransferState::Responding(network) => Some(network.clone()),
            ResourceTransferState::Finished(_) => None,
        }
    }

    pub(crate) fn data_received(&self, bytes: usize) {
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
            ResourceTransferState::Finished(_) => {}
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
                status_text: None,
                head: response.head(),
                network_request_headers: response
                    .network_request_extra_info()
                    .map(|info| info.headers.clone()),
            },
            SubresourceResponseBody::from_fetch_response(response),
        );
    }

    pub(crate) fn body_completed(&self, head: ResourceResponseHead, body: SubresourceResponseBody) {
        self.body_completed_with(head, body, |observation| self.observe(observation));
    }

    /// A VM-owned completion records its receipts in the current owner turn,
    /// before its callback can retire that owner. I/O uses the stored observer.
    pub(crate) fn body_completed_with(
        &self,
        head: ResourceResponseHead,
        body: SubresourceResponseBody,
        observer: impl FnMut(RendererNetworkObservation),
    ) {
        self.complete_with(|_| Ok((head, body)), observer);
    }

    pub(crate) fn failed(&self, error: &ResourceResponseFailure) {
        self.failed_with(error, |observation| self.observe(observation));
    }

    pub(crate) fn failed_with(
        &self,
        error: &ResourceResponseFailure,
        observer: impl FnMut(RendererNetworkObservation),
    ) {
        self.complete_with(|_| Err(error.clone()), observer);
    }

    /// Claim terminal permission before checking the result. A winning response
    /// can admit dependent work before publishing its terminal; a late response
    /// cannot run policy side effects after cancellation has already won.
    pub(crate) fn complete_with(
        &self,
        result: impl FnOnce(
            &RendererNetworkRequest,
        ) -> Result<
            (ResourceResponseHead, SubresourceResponseBody),
            ResourceResponseFailure,
        >,
        mut observer: impl FnMut(RendererNetworkObservation),
    ) {
        let previous = {
            let mut state = self.state.lock();
            let finished = ResourceTransferState::Finished(state.handle());
            std::mem::replace(&mut *state, finished)
        };
        let network = match &previous {
            ResourceTransferState::Requested(network)
            | ResourceTransferState::Responding(network) => network,
            ResourceTransferState::Finished(_) => return,
        };
        let body = match result(network) {
            Ok((head, body)) => match previous {
                ResourceTransferState::Requested(_) => {
                    Self::record_response(network, head, &mut observer);
                    SubresourceBodyFinished::ready(network.handle(), body)
                }
                ResourceTransferState::Responding(_) => {
                    SubresourceBodyFinished::ready_after_streaming(network.handle(), body)
                }
                ResourceTransferState::Finished(_) => unreachable!(),
            },
            Err(ResourceResponseFailure::Request(message)) => {
                SubresourceBodyFinished::failed(network.handle(), message)
            }
            Err(ResourceResponseFailure::Network { message, context }) => {
                SubresourceBodyFinished::failed_with_network_context(
                    network.handle(),
                    message,
                    context,
                )
            }
            Err(ResourceResponseFailure::PartialBody {
                message,
                response,
                body,
            }) => {
                if matches!(previous, ResourceTransferState::Requested(_)) {
                    Self::record_response(network, response.as_ref().clone(), &mut observer);
                }
                SubresourceBodyFinished::failed_with_partial_body(network.handle(), message, body)
            }
        };
        observer(
            network.report(ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(
                body,
            ))),
        );
    }
}

impl ResourceResponseObserver for ResourceTransfer {
    fn response_started(&self, response: Arc<ResourceResponseHead>) {
        let mut state = self.state.lock();
        let finished = ResourceTransferState::Finished(state.handle());
        match std::mem::replace(&mut *state, finished) {
            ResourceTransferState::Requested(network) => {
                Self::record_response(&network, response.as_ref().clone(), &mut |observation| {
                    self.observe(observation);
                });
                *state = ResourceTransferState::Responding(network);
            }
            ResourceTransferState::Responding(_) => {
                panic!("one resource response head per consumer")
            }
            ResourceTransferState::Finished(_) => {}
        }
    }

    fn data_received(&self, bytes: &[u8]) {
        ResourceTransfer::data_received(self, bytes.len());
    }

    fn cancelled(&self, failure: &ResourceResponseFailure) {
        self.failed(failure);
    }
}

impl Drop for ResourceTransfer {
    fn drop(&mut self) {
        if !matches!(self.state.get_mut(), ResourceTransferState::Finished(_)) {
            self.failed(&ResourceResponseFailure::Request(
                "Resource load cancelled".into(),
            ));
        }
    }
}
