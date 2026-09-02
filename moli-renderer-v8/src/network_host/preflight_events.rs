use moli_fetch::ResponseHead;

use crate::{
    page_task_queue::RendererResourceCompletionSender,
    types::{
        AsyncSubresourceFetchEvent, AsyncSubresourceNetworkContext, SubresourceNetworkRecord,
        SubresourceResponseBody,
    },
};

#[derive(Clone)]
pub(in crate::network_host) struct CorsPreflightNetworkObserver {
    completion_tx: RendererResourceCompletionSender,
    context: AsyncSubresourceNetworkContext,
}

impl CorsPreflightNetworkObserver {
    pub(in crate::network_host) fn new(
        completion_tx: RendererResourceCompletionSender,
        context: AsyncSubresourceNetworkContext,
    ) -> Self {
        Self {
            completion_tx,
            context,
        }
    }

    pub(in crate::network_host) fn send_preflight_success(
        &self,
        request_url: url::Url,
        request_headers: Vec<(String, String)>,
        response: &ResponseHead,
    ) {
        self.send_record(
            SubresourceNetworkRecord::success_with_body(
                self.context.frame_id.clone(),
                self.context.document_url.clone(),
                request_url,
                "OPTIONS".to_owned(),
                request_headers,
                None,
                self.context.resource_type,
                response.request_cookie_report.clone(),
                response
                    .redirect_chain
                    .clone()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                response.final_url.clone(),
                response.status,
                response.headers.clone(),
                SubresourceResponseBody::from_bytes(Vec::new()),
                response.cookie_set_reports.clone(),
            )
            .with_from_cache(response.from_cache)
            .with_negotiated_http_version(response.negotiated_http_version),
        );
    }

    pub(in crate::network_host) fn send_preflight_failure(
        &self,
        request_url: url::Url,
        request_headers: Vec<(String, String)>,
        error_text: String,
    ) {
        self.send_record(SubresourceNetworkRecord::failure(
            self.context.frame_id.clone(),
            self.context.document_url.clone(),
            request_url,
            "OPTIONS".to_owned(),
            request_headers,
            None,
            self.context.resource_type,
            error_text,
        ));
    }

    fn send_record(&self, record: SubresourceNetworkRecord) {
        let _ = self.completion_tx.send_async_subresource_event(
            AsyncSubresourceFetchEvent::ObservedNetworkRecord(Box::new(record)),
        );
    }
}
