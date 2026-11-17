use std::sync::Arc;

use moli_fetch::{RequestCredentialsMode, RequestMode, ResponseHead};
use tokio::sync::mpsc;

use crate::{
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
    load: crate::network::loads::ResourceLoadLease,
    mode: RequestMode,
    credentials: RequestCredentialsMode,
    policy: SubresourcePolicyContext,
    pub(super) body_source_id: Option<NetworkBodySourceId>,
    response: Option<Box<StreamedFetchResponse>>,
    delivered: usize,
    timing: crate::worker::WorkerResourceTiming,
    response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    event_source_parser: Option<crate::network_host::EventSourceParser>,
}

struct StreamedFetchResponse {
    handle: SubresourceNetworkRequestHandle,
    head: ResponseHead,
    body: ResourceInterceptionBody,
    needs_orb_body_validation: bool,
}

impl ControlledFetchStream {
    fn with_event_source_parser(
        mut self,
        parser: Option<crate::network_host::EventSourceParser>,
    ) -> Self {
        self.event_source_parser = parser;
        self
    }
    pub(super) fn set_response_filter(
        &mut self,
        filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    ) {
        self.response_filter = filter;
    }
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
        load: crate::network::loads::ResourceLoadLease,
        timing: crate::worker::WorkerResourceTiming,
    ) -> Self {
        Self {
            completion,
            request_origin,
            load,
            fetch_id,
            interception,
            mode,
            credentials,
            policy,
            body_source_id: None,
            response: None,
            delivered: 0,
            timing,
            response_filter: None,
            event_source_parser: None,
        }
    }

    pub(super) fn start(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        head: ResponseHead,
        network_request_headers: Option<Vec<(String, String)>>,
        body: ResourceInterceptionBody,
    ) -> Result<(), String> {
        let skip_fetch_security_validation = self
            .response_filter
            .as_ref()
            .is_some_and(|filter| filter.service_worker_metadata().is_some());
        if skip_fetch_security_validation {
            self.timing.begin_synthetic_response();
        }
        self.timing.observe_head(&head);
        if !skip_fetch_security_validation {
            super::super::validate_fetch_response_headers(
                &self.request_origin,
                &head,
                self.mode,
                self.credentials,
                self.policy,
                self.load.request_client().web_security_disabled(),
            )?;
        }
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
            self.response_filter.as_ref(),
            self.load.request_client().web_security_disabled(),
        ));
        let web_security_disabled = self.load.request_client().web_security_disabled();
        let needs_orb_body_validation = !skip_fetch_security_validation
            && !web_security_disabled
            && super::super::fetch_response_needs_orb_body_validation(
                &self.request_origin,
                &head.final_url,
                &head.headers,
                self.mode,
            );
        self.timing.select_native_response(
            &head,
            Arc::clone(&metadata),
            self.response_filter.clone(),
            None,
            skip_fetch_security_validation,
        );
        let _ = self
            .completion
            .send(WorkerFetchEvent::StreamingStarted(Box::new(
                WorkerFetchStreamingStarted {
                    fetch_id: self.fetch_id,
                    body_source_id,
                    head: head.clone(),
                    metadata: Arc::clone(&metadata),
                },
            )));
        self.response = Some(Box::new(StreamedFetchResponse {
            handle,
            head,
            body,
            needs_orb_body_validation,
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
        if let Some(parser) = &mut self.event_source_parser {
            let messages = parser.push(&bytes);
            let _ = self.completion.send(WorkerFetchEvent::EventSourceState {
                fetch_id: self.fetch_id,
                body_source_id: self.body_source_id.unwrap(),
                last_event_id: parser.last_event_id().to_owned(),
                reconnect_delay_ms: parser.reconnect_delay_ms(),
            });
            for message in messages {
                let _ = self.completion.send(WorkerFetchEvent::EventSourceMessage {
                    fetch_id: self.fetch_id,
                    body_source_id: self.body_source_id.unwrap(),
                    message,
                });
            }
        } else if !self
            .response
            .as_ref()
            .expect("response precedes body")
            .needs_orb_body_validation
        {
            let _ = self.completion.send(WorkerFetchEvent::StreamingChunk(
                WorkerFetchStreamingChunk {
                    fetch_id: self.fetch_id,
                    body_source_id: self.body_source_id.unwrap(),
                    bytes,
                },
            ));
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, mut result: Result<(), String>) {
        let response = *self
            .response
            .take()
            .expect("streamed response must have headers");
        let handle = response.handle;
        let body = response.body.finish_capture(self.delivered);
        let mut opaque_response_blocked = false;
        if result.is_ok() && response.needs_orb_body_validation {
            match super::super::validated_opaque_response_body(&response.head.headers, &body) {
                Ok(_) => {}
                Err(super::super::FetchResponseSecurityViolation::OpaqueResponseBlocked(_)) => {
                    opaque_response_blocked = true;
                }
                Err(error) => result = Err(error.into_message()),
            }
        }
        if result.is_ok() && response.needs_orb_body_validation && !opaque_response_blocked {
            match body.try_bytes() {
                Ok(bytes) => {
                    let _ = self.completion.send(WorkerFetchEvent::StreamingChunk(
                        WorkerFetchStreamingChunk {
                            fetch_id: self.fetch_id,
                            body_source_id: self.body_source_id.unwrap(),
                            bytes: bytes.into_owned(),
                        },
                    ));
                }
                Err(error) => result = Err(error.to_string()),
            }
        }
        if opaque_response_blocked {
            self.timing.discard_response();
        }
        self.timing.complete();
        let terminal = match &result {
            Ok(()) if opaque_response_blocked => SubresourceBodyFinished::failed_with_partial_body(
                handle,
                super::super::ABORTED_ERROR_TEXT.to_owned(),
                body.clone(),
            ),
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
                result,
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
        integrity: String,
        timing: crate::worker::WorkerResourceTiming,
        event_source_parser: Option<crate::network_host::EventSourceParser>,
    ) {
        let super::ResourceResponsePolicy::Fetch(policy) = self.response_policy else {
            panic!("Worker fetch requires Fetch response policy");
        };
        let request_origin = self
            .request
            .browser_origin()
            .expect("native Worker Fetch has a browser origin")
            .clone();
        let web_security_disabled = self.load.request_client().web_security_disabled();
        let mut stream = super::ControlledResourceStream::WorkerFetch(Box::new(
            ControlledFetchStream::new(
                completion.clone(),
                fetch_id,
                self.native_interception().clone(),
                self.request.request_mode,
                self.effective_credentials_mode(),
                policy,
                request_origin.clone(),
                self.load.clone(),
                timing.clone(),
            )
            .with_event_source_parser(event_source_parser),
        ));
        let result = self
            .run_inner(
                (integrity.is_empty()
                    && self.info.resource_type != crate::types::SubresourceResourceType::Font)
                    .then_some(&mut stream),
            )
            .await;
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
                self.effective_credentials_mode(),
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
        if !integrity.is_empty()
            && let Ok(delivered) = &result
        {
            let head = delivered.response.head();
            let filter = self
                .response_filter
                .clone()
                .or_else(|| {
                    super::super::network_response_filter(
                        self.request
                            .request_origin()
                            .expect("Worker fetch has a requesting origin"),
                        &head,
                        self.request.request_mode,
                        self.request.redirect_mode(),
                        web_security_disabled,
                    )
                })
                .unwrap_or_else(|| crate::types::FetchResponseType::Basic.into());
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
        let mut record = super::result::XhrLoadResult::new(self.info.clone(), result, headers);
        if let Err(error) = &record.result {
            record.network_record = failure_record(&self.info, network_error(error));
        } else if blocked_opaque {
            record.network_record =
                failure_record(&self.info, super::super::ABORTED_ERROR_TEXT.to_owned());
        }
        self.load.finish();
        self.native_interception()
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
            let (head, body) = response.into_body();
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
            if blocked_opaque {
                timing.discard_response();
            } else {
                let synthetic = delivery
                    .as_ref()
                    .is_some_and(|filter| filter.service_worker_metadata().is_some());
                timing.select_native_response(
                    &head,
                    Arc::new(metadata.clone()),
                    delivery.clone(),
                    Some(body_size),
                    synthetic,
                );
            }
            crate::worker::WorkerFetchResponse {
                head,
                body,
                metadata,
            }
        });
        if result.is_err() {
            timing.discard_response();
        }
        timing.complete();
        let _ = completion.send(WorkerFetchEvent::Completion(Box::new(
            crate::worker::WorkerFetchCompletion {
                fetch_id,
                body_is_null: record.body_is_null,
                result,
            },
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
