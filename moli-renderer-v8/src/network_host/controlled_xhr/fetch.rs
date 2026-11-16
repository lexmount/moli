use moli_fetch::{RequestCredentialsMode, RequestMode, ResponseHead};
use tokio::sync::mpsc;

use crate::{
    network::{ResourceInterceptionBody, interception_output::ResourceInterceptionContext},
    types::{
        NetworkBodySourceId, PendingSubresourceFetchInfo, ScriptNetworkOutputItem,
        SubresourceBodyFinished, SubresourceDataReceived, SubresourcePolicyContext,
        SubresourceResponseStarted,
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
    response: Option<
        Box<(
            PendingSubresourceFetchInfo,
            ResponseHead,
            ResourceInterceptionBody,
        )>,
    >,
    delivered: usize,
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
        load: crate::network::loads::ResourceLoadLease,
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
        }
    }

    pub(super) fn start(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        head: ResponseHead,
        network_request_headers: Option<Vec<(String, String)>>,
        body: ResourceInterceptionBody,
    ) -> Result<(), String> {
        super::super::validate_fetch_response_security_policy_with_body_classified(
            &self.request_origin,
            &head,
            &[],
            self.mode,
            self.credentials,
            self.policy,
            self.load.request_client().web_security_disabled(),
        )
        .map_err(|error| error.into_message())?;
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
                    .with_network_request_headers(network_request_headers.clone())
                    .with_negotiated_http_version(head.negotiated_http_version),
                ),
            ));
        let body_source_id = super::super::new_network_body_source_id();
        self.body_source_id = Some(body_source_id);
        let _ = self.completion.send(WorkerFetchEvent::StreamingStarted(
            WorkerFetchStreamingStarted {
                fetch_id: self.fetch_id,
                body_source_id,
                head: head.clone(),
            },
        ));
        self.response = Some(Box::new((info.clone(), head, body)));
        Ok(())
    }

    pub(super) fn chunk(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let handle = self
            .response
            .as_ref()
            .expect("response precedes body")
            .0
            .network_request_handle
            .unwrap();
        self.delivered += bytes.len();
        self.interception
            .publish_network(ScriptNetworkOutputItem::SubresourceDataReceived(
                SubresourceDataReceived::new(handle, bytes.len(), bytes.len()),
            ));
        // A detached JS consumer does not cancel a keepalive transport. Its
        // resource lease, rather than this delivery channel, owns cancellation.
        let _ = self.completion.send(WorkerFetchEvent::StreamingChunk(
            WorkerFetchStreamingChunk {
                body_source_id: self.body_source_id.unwrap(),
                bytes,
            },
        ));
        Ok(())
    }

    pub(super) fn finish(&mut self, result: Result<(), String>) {
        let (info, head, source) = *self
            .response
            .take()
            .expect("streamed response must have headers");
        let handle = info.network_request_handle.unwrap();
        let body = source.finish_capture(self.delivered);
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
                head,
                result: result.map(|()| body),
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
    ) {
        let super::ResourceResponsePolicy::Fetch(policy) = self.response_policy else {
            panic!("Worker fetch requires Fetch response policy");
        };
        let request_origin = self
            .request
            .browser_origin()
            .expect("native Worker Fetch has a browser origin")
            .clone();
        let mut stream = super::ControlledResourceStream::WorkerFetch(ControlledFetchStream::new(
            completion.clone(),
            fetch_id,
            self.interception.clone(),
            self.request.request_mode,
            self.request.credentials_mode,
            policy,
            request_origin.clone(),
            self.load.clone(),
        ));
        let result = self.run_inner(Some(&mut stream)).await;
        let result = self.normalize_cancellation(result);
        stream.policy_violations(std::mem::take(&mut self.csp_violations));
        if stream.body_source_id().is_some() {
            let super::ControlledResourceStream::WorkerFetch(stream) = &mut stream else {
                unreachable!()
            };
            stream.finish(result.map(|_| ()));
            self.load.finish();
            self.publish_completed();
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
        let _ = completion.send(WorkerFetchEvent::Completion(Box::new(
            crate::worker::WorkerFetchCompletion {
                fetch_id,
                result: record
                    .result
                    .map(|response| crate::worker::WorkerFetchResponse {
                        response: Box::new(response),
                        filter: self.response_filter,
                    }),
            },
        )));
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
