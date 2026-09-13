use std::sync::Arc;

use moli_page_types::{NavigationResponse, SubresourceResponseBody, SubresourceResponseBodyWriter};
use parking_lot::Mutex;

use crate::{
    network::loads::ResourceLoadLease,
    network::{
        ResourceResponseFailure, ResourceResponseHead, ResourceResponseObserver, ResourceTransfer,
    },
    page_task_queue::RendererResourceCompletionSender,
    types::{AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent},
};

/// A report without a JS consumer keeps its request and resource lease until
/// transport completion. The Page is only an observer of its native receipts.
pub(crate) struct CspReportResource {
    pub(crate) network: Arc<ResourceTransfer>,
    load: ResourceLoadLease,
    // ServiceWorker streaming failures do not carry a final Response. Retain
    // the physical prefix here, just as the HTTP response collector does.
    stream: Mutex<Option<(Arc<ResourceResponseHead>, SubresourceResponseBodyWriter)>>,
}

impl CspReportResource {
    pub(crate) fn new(network: Arc<ResourceTransfer>, load: ResourceLoadLease) -> Arc<Self> {
        Arc::new(Self {
            network,
            load,
            stream: Mutex::new(None),
        })
    }

    pub(crate) fn response_started(&self, head: moli_fetch::ResponseHead) {
        let response = Arc::new(ResourceResponseHead {
            head,
            network_request_headers: None,
        });
        self.network.response_started(response.clone());
        *self.stream.lock() = Some((
            response,
            SubresourceResponseBodyWriter::with_disk_pool(self.load.request_client().disk_pool()),
        ));
    }

    pub(crate) fn data_received(&self, bytes: &[u8]) {
        if let Some((_, body)) = &mut *self.stream.lock() {
            body.append(bytes);
        }
        self.network.data_received(bytes.len());
    }

    pub(crate) fn response_completed(&self, response: &NavigationResponse) {
        finish_report_response(&self.network, response);
        self.stream.lock().take();
        self.load.finish();
    }

    pub(crate) fn fail(&self, message: String) {
        let error = match self.stream.lock().take() {
            Some((response, body)) => ResourceResponseFailure::PartialBody {
                message,
                response,
                body: body.finish(),
            },
            None => ResourceResponseFailure::Request(message),
        };
        self.network.failed(&error);
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

/// Buffered intercepted results still belong to the original pending request.
/// If its Page route retires before delivery, the result itself settles the
/// native request rather than losing the physical response with the VM.
pub(crate) struct CompletedCspReport {
    network: Arc<ResourceTransfer>,
    completion: Option<AsyncSubresourceFetchCompletion>,
}

impl std::fmt::Debug for CompletedCspReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletedCspReport")
            .field("completion", &self.completion)
            .finish_non_exhaustive()
    }
}

impl CompletedCspReport {
    pub(crate) fn new(
        network: Arc<ResourceTransfer>,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Self {
        Self {
            network,
            completion: Some(completion),
        }
    }

    pub(crate) fn internal_id(&self) -> u64 {
        self.completion
            .as_ref()
            .expect("unclaimed report result")
            .internal_id
    }

    pub(crate) fn into_completion(mut self) -> AsyncSubresourceFetchCompletion {
        self.completion
            .take()
            .expect("report result is claimed once")
    }
}

impl Drop for CompletedCspReport {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            finish_report_result(&self.network, &completion.result.into_result());
        }
    }
}

pub(crate) fn send_report_completion(
    sender: &RendererResourceCompletionSender,
    network: Arc<ResourceTransfer>,
    completion: AsyncSubresourceFetchCompletion,
) {
    let _ = sender.send_async_subresource_event(AsyncSubresourceFetchEvent::CspReport(Box::new(
        CompletedCspReport::new(network, completion),
    )));
}

pub(crate) fn finish_report_result(
    network: &ResourceTransfer,
    result: &Result<NavigationResponse, String>,
) {
    match result {
        Ok(response) => finish_report_response(network, response),
        Err(message) => network.failed(&ResourceResponseFailure::Request(message.clone())),
    }
}

fn finish_report_response(network: &ResourceTransfer, response: &NavigationResponse) {
    network.body_completed(
        ResourceResponseHead {
            head: response.head(),
            network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
        },
        SubresourceResponseBody::from_navigation_response(response),
    );
}

pub(crate) async fn fetch_buffered_csp_report(
    loader: &crate::network::ResourceRequestClient,
    request: moli_fetch::Request,
    cancel: moli_fetch::FetchCancelHandle,
    network: &ResourceTransfer,
) -> Result<NavigationResponse, String> {
    let observed = loader
        .fetch_raw_stream_with_cancel_and_network_metadata(request, cancel)
        .await
        .map_err(|error| error.to_string())?;
    let headers = observed
        .request_observation()
        .map(|request| request.headers().to_vec());
    match crate::network::resource_response::collect_observed_response(observed, None).await {
        Ok(response) => {
            Ok(NavigationResponse::from(response).with_network_request_headers(headers))
        }
        Err(error) => {
            // No response decision is possible on a broken transport. Preserve
            // its head and prefix before delivering the error to the pending request.
            network.failed(&error);
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};

    #[test]
    fn buffered_report_result_survives_a_closed_route_and_a_claim_defers_completion() {
        for closed_route in [false, true] {
            let source = crate::runtime::RendererWorkerNetworkReporter::unobserved_for_test();
            let records = Arc::new(Mutex::new(Vec::new()));
            let observed = records.clone();
            let url = url::Url::parse("data:text/plain,physical").unwrap();
            let (network, started) = ResourceTransfer::start(
                source.start_request().unwrap(),
                move |receipt| {
                    let crate::runtime::RendererNetworkOutputItem::Resource(item) = receipt.item()
                    else {
                        panic!("resource receipt")
                    };
                    observed.lock().push(item.clone());
                },
                |request| {
                    moli_page_types::SubresourceRequestStarted::new(
                        request.handle(),
                        None,
                        url.clone(),
                        url.clone(),
                        "POST".into(),
                        moli_fetch::RequestHeaders::default(),
                        None,
                        moli_page_types::SubresourceResourceType::CspReport,
                        moli_page_types::SubresourceRequestInitiatorType::Script,
                        None,
                    )
                },
            );
            let crate::runtime::RendererNetworkOutputItem::Resource(started) = started.item()
            else {
                panic!("resource admission")
            };
            records.lock().push(started.clone());
            let response =
                NavigationResponse::from(crate::network_host::local_url_response(&url).unwrap());
            let completion = AsyncSubresourceFetchCompletion {
                internal_id: 1,
                request_url: url,
                request_method: "POST".into(),
                request_headers: moli_fetch::RequestHeaders::default(),
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: Ok(response).into(),
            };
            if closed_route {
                send_report_completion(
                    &RendererResourceCompletionSender::closed_for_test(),
                    network.clone(),
                    completion,
                );
            } else {
                let completion =
                    CompletedCspReport::new(network.clone(), completion).into_completion();
                assert_eq!(
                    records.lock().len(),
                    1,
                    "the claim transfers the decision to its pending request"
                );
                finish_report_result(&network, &completion.result.into_result());
            }
            drop(network);
            let records = records.lock();
            assert_eq!(
                records.len(),
                3,
                "one admission, physical head and terminal"
            );
            let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = records[2].as_ref() else {
                panic!("terminal last")
            };
            let SubresourceBodyFinishedResult::Ready(body) = body.result() else {
                panic!("retain the actual completed response")
            };
            assert_eq!(body.clone_body_bytes(), b"physical");
        }
    }
}
