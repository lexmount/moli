use std::sync::Arc;

use moli_fetch::{RequestCredentialsMode, RequestMode, ResponseHead};
use tokio::sync::mpsc;

use crate::{
    context_bootstrap::FetchResourceTiming,
    network::{ResourceInterceptionBody, interception_output::ResourceInterceptionContext},
    types::{
        NetworkBodySourceId, PendingSubresourceFetchInfo, ScriptNetworkOutputItem,
        SubresourceBodyFinished, SubresourceDataReceived, SubresourceNetworkRequestHandle,
        SubresourcePolicyContext, SubresourceResponseStarted,
    },
    worker::{
        WorkerFetchEvent, WorkerFetchStreamingChunk, WorkerFetchStreamingFinished,
        WorkerFetchStreamingStarted,
    },
};

/// JS delivery and the Network observer consume the same native response;
/// neither needs the creator Page to resume a Worker command.
pub(crate) struct ControlledFetchStream {
    completion: mpsc::UnboundedSender<WorkerFetchEvent>,
    fetch_id: u32,
    interception: ResourceInterceptionContext,
    request_origin: moli_url::WebOrigin,
    request_url: Arc<url::Url>,
    load: crate::network::loads::ResourceLoadLease,
    mode: RequestMode,
    credentials: RequestCredentialsMode,
    policy: SubresourcePolicyContext,
    resource_timing: FetchResourceTiming,
    pub(super) body_source_id: Option<NetworkBodySourceId>,
    response: Option<Box<StreamedFetchResponse>>,
    delivered: usize,
    timing: crate::worker::WorkerResourceTiming,
}

struct StreamedFetchResponse {
    handle: SubresourceNetworkRequestHandle,
    head: ResponseHead,
    body: ResourceInterceptionBody,
    metadata: Arc<moli_fetch_types::FetchResponseMetadata>,
}

impl ControlledFetchStream {
    pub(super) fn policy_violations(
        &self,
        violations: Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>,
    ) {
        if !violations.is_empty() {
            let _ = self
                .completion
                .send(WorkerFetchEvent::PolicyViolations(violations));
        }
    }

    pub(super) fn new(
        completion: mpsc::UnboundedSender<WorkerFetchEvent>,
        fetch_id: u32,
        interception: ResourceInterceptionContext,
        mode: RequestMode,
        credentials: RequestCredentialsMode,
        policy: SubresourcePolicyContext,
        request_origin: moli_url::WebOrigin,
        request_url: Arc<url::Url>,
        load: crate::network::loads::ResourceLoadLease,
        resource_timing: FetchResourceTiming,
        timing: crate::worker::WorkerResourceTiming,
    ) -> Self {
        Self {
            completion,
            request_origin,
            request_url,
            load,
            fetch_id,
            interception,
            mode,
            credentials,
            policy,
            resource_timing,
            body_source_id: None,
            response: None,
            delivered: 0,
            timing,
        }
    }

    pub(super) fn start(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        head: ResponseHead,
        network_request_headers: Option<Vec<(String, String)>>,
        body: ResourceInterceptionBody,
    ) -> Result<(), String> {
        self.timing.observe_head(&head);
        super::super::validate_fetch_response_headers(
            &self.request_origin,
            &head,
            self.mode,
            self.credentials,
            self.policy,
            self.load.request_client().web_security_disabled(),
        )?;
        let handle = info
            .network_request_handle
            .expect("native Fetch owns its request handle");
        self.interception
            .publish_network(ScriptNetworkOutputItem::SubresourceResponseStarted(
                Box::new(
                    SubresourceResponseStarted::new(
                        handle,
                        head.redirect_chain
                            .clone()
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                        head.final_url.clone(),
                        head.status,
                        head.headers.clone(),
                        head.cookie_set_reports.clone(),
                    )
                    .with_status_text(head.status_text.clone())
                    .with_from_cache(head.from_cache)
                    .with_network_request_extra_info((network_request_headers.clone()).map(
                        |headers| moli_fetch::NetworkRequestExtraInfo {
                            headers,
                            cookie_report: head.request_cookie_report.clone().unwrap_or_default(),
                        },
                    ))
                    .with_negotiated_http_version(head.negotiated_http_version),
                ),
            ));
        let body_source_id = super::super::new_network_body_source_id();
        self.body_source_id = Some(body_source_id);
        let metadata = Arc::new(super::super::fetch_response_metadata(
            &self.request_origin,
            &head,
            self.mode,
            None,
            self.load.request_client().web_security_disabled(),
        ));
        let mut observable_head = head.clone();
        observable_head.headers = super::super::filter_cors_exposed_response_headers(
            &self.request_origin,
            &head,
            self.credentials,
            self.load.request_client().web_security_disabled(),
        );
        let _ = self.completion.send(WorkerFetchEvent::StreamingStarted(
            WorkerFetchStreamingStarted {
                fetch_id: self.fetch_id,
                body_source_id,
                head: observable_head,
                metadata: Arc::clone(&metadata),
            },
        ));
        self.response = Some(Box::new(StreamedFetchResponse {
            handle,
            head,
            body,
            metadata,
        }));
        Ok(())
    }

    pub(super) fn chunk(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let handle = self
            .response
            .as_ref()
            .expect("response precedes body")
            .handle;
        self.delivered += bytes.len();
        self.timing.observe_chunk(bytes.len());
        self.interception
            .publish_network(ScriptNetworkOutputItem::SubresourceDataReceived(
                SubresourceDataReceived::new(handle, bytes.len(), bytes.len()),
            ));
        // A detached JS consumer does not cancel a keepalive transport. Its
        // resource lease, rather than this delivery channel, owns cancellation.
        let _ = self.completion.send(WorkerFetchEvent::StreamingChunk(
            WorkerFetchStreamingChunk {
                fetch_id: self.fetch_id,
                body_source_id: self.body_source_id.unwrap(),
                bytes,
            },
        ));
        Ok(())
    }

    pub(super) fn finish(&mut self, result: Result<(), String>) {
        self.timing.complete();
        let response = *self
            .response
            .take()
            .expect("streamed response must have headers");
        let handle = response.handle;
        let body = response.body.finish_capture(self.delivered);
        let terminal = match &result {
            Ok(()) => SubresourceBodyFinished::ready_after_streaming(handle, body.clone()),
            Err(error) => SubresourceBodyFinished::failed_with_partial_body(
                handle,
                network_error(error),
                body.clone(),
            ),
        };
        self.interception
            .publish_network(ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(
                terminal,
            )));
        let _ = self.completion.send(WorkerFetchEvent::StreamingFinished(
            WorkerFetchStreamingFinished {
                fetch_id: self.fetch_id,
                body_source_id: self.body_source_id.unwrap(),
                result: result.map(|()| {
                    self.resource_timing.completed(
                        &self.request_url,
                        &response.head,
                        body.len(),
                        &response.metadata,
                        None,
                    )
                }),
            },
        ));
    }
}

fn network_error(error: &str) -> String {
    if super::super::is_cors_policy_failure_message(error) {
        super::super::FAILED_ERROR_TEXT.to_owned()
    } else {
        error.to_owned()
    }
}

impl super::ControlledResourceLoad {
    pub(crate) async fn run_fetch(
        mut self,
        completion: mpsc::UnboundedSender<WorkerFetchEvent>,
        fetch_id: u32,
        resource_timing: FetchResourceTiming,
        integrity: String,
        timing: crate::worker::WorkerResourceTiming,
    ) {
        // Network/CDP correlation follows redirects; Resource Timing names
        // the original request, including when interception rewrites its URL.
        let request_url = Arc::new(self.info.url.clone());
        let super::ResourceResponsePolicy::Fetch(policy) = self.response_policy else {
            panic!("Worker fetch requires Fetch response policy");
        };
        let request_origin = self
            .request
            .browser_origin()
            .expect("native Worker Fetch has a browser origin")
            .clone();
        let web_security_disabled = self.load.request_client().web_security_disabled();
        let mut stream = super::ControlledResourceStream::WorkerFetch(ControlledFetchStream::new(
            completion.clone(),
            fetch_id,
            self.interception.clone(),
            self.request.request_mode,
            self.request.credentials_mode,
            policy,
            request_origin.clone(),
            Arc::clone(&request_url),
            self.load.clone(),
            resource_timing.clone(),
            timing.clone(),
        ));
        let result = self.run_inner((integrity.is_empty() && self.info.resource_type != crate::types::SubresourceResourceType::Font).then_some(&mut stream)).await;
        let result = self.normalize_cancellation(result);
        stream.policy_violations(std::mem::take(&mut self.csp_violations));
        if stream.body_source_id().is_some() {
            let super::ControlledResourceStream::WorkerFetch(stream) = &mut stream else {
                unreachable!()
            };
            stream.finish(result.map(|_| ()));
            self.load.finish();
            self.publish_completed();
            let _ = completion.send(WorkerFetchEvent::ResourceTiming { fetch_id, timing });
            return;
        }
        let (mut result, headers) = match result {
            Ok((Some(response), headers)) => (Ok(response), headers),
            Ok((None, _)) => unreachable!("no stream was started"),
            Err(error) => (Err(error), None),
        };
        let mut blocked_opaque = false;
        if self.response_filter.is_none()
            && let Ok(delivered) = &result
        {
            match super::super::validate_fetch_response_security_policy_with_body_classified(
                &request_origin,
                &delivered.response.head(),
                delivered.response.body_bytes(),
                self.request.request_mode,
                self.request.credentials_mode,
                policy,
                self.load.request_client().web_security_disabled(),
            ) {
                Ok(()) => {}
                Err(super::super::FetchResponseSecurityViolation::OpaqueResponseBlocked(_)) => {
                    blocked_opaque = true
                }
                Err(error) => result = Err(error.into_message()),
            }
        }
        if !integrity.is_empty() && let Ok(delivered) = &result {
            let head = delivered.response.head();
            let filter = self.response_filter.clone().or_else(|| {
                super::super::network_response_filter(
                    self.request.request_origin().expect("Worker fetch has a requesting origin"),
                    &head,
                    self.request.request_mode,
                    self.request.redirect_mode(),
                )
            }).unwrap_or_else(|| crate::types::FetchResponseType::Basic.into());
            if let Err(error) = super::super::validate_fetch_response_integrity(
                &integrity,
                &self.request.method,
                head.status,
                &filter,
                delivered.response.body_bytes(),
            ) {
                result = Err(error);
            }
        }
        if let Ok(delivered) = &result {
            let head = delivered.response.head();
            timing.observe_head(&head);
            timing.observe_chunk(delivered.response.body_bytes().len());
            if let Some(filter) = self.response_filter.clone() {
                timing.select_response(&head, filter, delivered.response.body_bytes().len(), true);
            }
        } else {
            timing.discard_response();
        }
        timing.complete();
        let mut record = super::result::XhrLoadResult::new(self.info.clone(), result, headers);
        if let Err(error) = &record.result {
            record.network_record = failure_record(&self.info, network_error(error));
        } else if blocked_opaque {
            record.network_record =
                failure_record(&self.info, super::super::ABORTED_ERROR_TEXT.to_owned());
        }
        self.load.finish();
        self.interception
            .record_subresource_network(record.network_record);
        self.publish_completed();
        // A manual response remains an opaque redirect even if interception
        // removed Location. Select the delivery once before creating metadata.
        let delivery = self.response_filter.take().or_else(|| {
            (self.request.redirect_mode() == moli_fetch::RequestRedirectMode::Manual
                && record
                    .result
                    .as_ref()
                    .is_ok_and(|response| matches!(response.status, 301 | 302 | 303 | 307 | 308)))
            .then_some(crate::types::FetchResponseType::OpaqueRedirect.into())
        });
        let result = record.result.map(|response| {
            let body_size = response.body_bytes().len();
            let (mut head, body) = response.into_body();
            let mut metadata = super::super::fetch_response_metadata(
                &request_origin,
                &head,
                self.request.request_mode,
                delivery.as_ref(),
                web_security_disabled,
            );
            let body = if blocked_opaque {
                metadata.response_type = crate::types::FetchResponseType::Opaque;
                metadata.timing_allow_passed = false;
                moli_fetch::ResponseBody::materialized_bytes(Vec::new())
            } else {
                body
            };
            let resource_timing = resource_timing.completed(
                &request_url,
                &head,
                body_size,
                &metadata,
                delivery.as_ref(),
            );
            if !delivery.as_ref().is_some_and(|filter| filter.is_readable()) {
                head.headers = super::super::filter_cors_exposed_response_headers(
                    &request_origin,
                    &head,
                    self.request.credentials_mode,
                    web_security_disabled,
                );
            }
            crate::worker::WorkerFetchResponse {
                head,
                body,
                metadata,
                resource_timing,
            }
        });
        let _ = completion.send(WorkerFetchEvent::Completion(Box::new(
            crate::worker::WorkerFetchCompletion { fetch_id, result },
        )));
        let _ = completion.send(WorkerFetchEvent::ResourceTiming { fetch_id, timing });
    }
}

fn failure_record(
    info: &PendingSubresourceFetchInfo,
    error: String,
) -> crate::types::SubresourceNetworkRecord {
    crate::types::SubresourceNetworkRecord::failure(
        info.frame_id.clone(),
        info.document_url.clone(),
        info.url.clone(),
        info.method.clone(),
        info.request_headers.clone(),
        info.request_body.clone(),
        info.resource_type,
        error,
    )
    .with_request_handle(info.network_request_handle.expect("native Fetch identity"))
    .with_request_body_bytes(info.request_body_bytes.clone())
}
