use moli_fetch::{FetchCancelHandle, Request, ResponseHead};
use std::sync::Arc;

use crate::{
    network::{ResourceRequestClient, ResourceTransfer},
    runtime::{RendererNetworkObservation, RendererNetworkRequest},
    types::SubresourceResourceType,
};

/// Physical OPTIONS requests inherit the source of the accepted request.
/// The observer delivers receipts on that request's existing execution route.
#[derive(Clone)]
pub(crate) struct CorsPreflightNetworkObserver {
    pub(crate) request: RendererNetworkRequest,
    pub(crate) observer: Arc<dyn Fn(RendererNetworkObservation) + Send + Sync>,
    pub(crate) frame_id: Option<String>,
    pub(crate) resource_type: SubresourceResourceType,
    pub(crate) keepalive: bool,
}

impl CorsPreflightNetworkObserver {
    pub(in crate::network_host) async fn fetch(
        &self,
        loader: &ResourceRequestClient,
        request: Request,
        cancel: Option<FetchCancelHandle>,
    ) -> Result<ResponseHead, String> {
        let observer = self.observer.clone();
        let (network, started) = ResourceTransfer::start(
            self.request.dependent_request(),
            move |event| observer(event),
            |network| {
                moli_page_types::SubresourceRequestStarted::new(
                    network.handle(),
                    self.frame_id.clone(),
                    request
                        .cookie_context
                        .initiator_url
                        .clone()
                        .expect("a CORS preflight has an initiator"),
                    request.url.clone(),
                    request.method.clone(),
                    request.request_headers.clone(),
                    None,
                    self.resource_type,
                    moli_page_types::SubresourceRequestInitiatorType::Script,
                    None,
                )
                .with_keepalive(self.keepalive)
            },
        );
        (self.observer)(started);
        let result = loader
            .fetch_observed_script_text_with_cancel(
                request,
                cancel.unwrap_or_default(),
                network.as_ref(),
            )
            .await;
        network.complete(&result);
        result
            .map(|response| response.head())
            .map_err(|error| error.to_string())
    }
}
