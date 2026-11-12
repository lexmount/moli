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
    output: PreflightNetworkOutput,
    frame_id: Option<String>,
    document_url: url::Url,
    resource_type: crate::types::SubresourceResourceType,
}

#[derive(Clone)]
enum PreflightNetworkOutput {
    Document(RendererResourceCompletionSender),
    Native {
        output: crate::network::interception_output::InterceptionOutput,
        source_document: crate::runtime::RendererDocumentLifecycleIdentity,
    },
}

impl CorsPreflightNetworkObserver {
    pub(in crate::network_host) fn new(
        completion_tx: RendererResourceCompletionSender,
        context: AsyncSubresourceNetworkContext,
    ) -> Self {
        Self {
            output: PreflightNetworkOutput::Document(completion_tx),
            frame_id: context.frame_id,
            document_url: context.document_url,
            resource_type: context.resource_type,
        }
    }

    pub(super) fn for_native_xhr(
        output: crate::network::interception_output::InterceptionOutput,
        source_document: crate::runtime::RendererDocumentLifecycleIdentity,
        info: &crate::types::PendingSubresourceFetchInfo,
    ) -> Self {
        Self {
            output: PreflightNetworkOutput::Native {
                output,
                source_document,
            },
            frame_id: info.frame_id.clone(),
            document_url: info.document_url.clone(),
            resource_type: info.resource_type,
        }
    }

    pub(in crate::network_host) fn send_preflight_success(
        &self,
        request_url: url::Url,
        request_headers: moli_fetch::RequestHeaders,
        response: &ResponseHead,
    ) {
        self.send_record(
            SubresourceNetworkRecord::success_with_body(
                self.frame_id.clone(),
                self.document_url.clone(),
                request_url,
                "OPTIONS".to_owned(),
                request_headers,
                None,
                self.resource_type,
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
        request_headers: moli_fetch::RequestHeaders,
        error_text: String,
    ) {
        self.send_record(SubresourceNetworkRecord::failure(
            self.frame_id.clone(),
            self.document_url.clone(),
            request_url,
            "OPTIONS".to_owned(),
            request_headers,
            None,
            self.resource_type,
            error_text,
        ));
    }

    fn send_record(&self, record: SubresourceNetworkRecord) {
        match &self.output {
            PreflightNetworkOutput::Document(completion) => {
                let _ = completion.send_async_subresource_event(
                    AsyncSubresourceFetchEvent::ObservedNetworkRecord(Box::new(record)),
                );
            }
            PreflightNetworkOutput::Native {
                output,
                source_document,
            } => {
                output.record_subresource_network(*source_document, record);
            }
        }
    }
}
