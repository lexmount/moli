use super::resource_request_started;
use std::sync::Arc;

use moli_page_types::{NavigationResponse, SubresourceResponseBody};

use crate::{
    network::loads::ResourceLoadLease,
    network::{ResourceResponseHead, ResourceTransfer},
};

/// A keepalive resource without a JS consumer keeps its request and resource lease until
/// transport completion. Its originating context observes the native receipts.
pub(crate) struct KeepaliveResource {
    pub(crate) network: Arc<ResourceTransfer>,
    load: ResourceLoadLease,
    // ServiceWorker streaming failures do not carry a final Response. Retain
    // the physical prefix here, just as the HTTP response collector does.
    stream: Arc<crate::network::ResourceResponseStream>,
}

impl KeepaliveResource {
    pub(crate) fn new(network: Arc<ResourceTransfer>, load: ResourceLoadLease) -> Arc<Self> {
        Arc::new(Self {
            stream: crate::network::ResourceResponseStream::for_load(
                network.clone(),
                &load,
                crate::types::SubresourceResourceType::CspReport,
            ),
            network,
            load,
        })
    }

    pub(crate) fn response_started(&self, head: ResourceResponseHead) {
        self.stream.response_started(head);
    }

    pub(crate) fn data_received(&self, bytes: &[u8]) {
        self.stream.data_received(bytes);
    }

    pub(crate) fn response_completed(&self, response: &NavigationResponse) {
        self.stream.finish_response();
        finish_keepalive_response(&self.network, response);
        self.load.finish();
    }

    pub(crate) fn fail(&self, message: String) {
        self.network.failed(&self.stream.failure(message));
        self.load.finish();
    }

    pub(crate) fn fetch(
        self: Arc<Self>,
        loader: crate::network::ResourceRequestClient,
        request: moli_fetch::Request,
        cancel: moli_fetch::FetchCancelHandle,
    ) {
        self.load.task_runner().spawn(async move {
            let result = loader
                .fetch_observed_script_text_with_cancel(request, cancel, self.network.as_ref())
                .await;
            self.network.complete(&result);
            self.load.finish();
        });
    }
}

fn finish_keepalive_response(network: &ResourceTransfer, response: &NavigationResponse) {
    network.body_completed(
        ResourceResponseHead {
            status_text: None,
            head: response.head(),
            network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
        },
        SubresourceResponseBody::from_navigation_response(response),
    );
}

pub(crate) fn keepalive_request_started(
    network: &crate::runtime::RendererNetworkRequest,
    info: &crate::types::PendingSubresourceFetchInfo,
) -> moli_page_types::SubresourceRequestStarted {
    resource_request_started(
        network,
        info,
        moli_page_types::SubresourceRequestInitiatorType::Script,
        true,
    )
}
