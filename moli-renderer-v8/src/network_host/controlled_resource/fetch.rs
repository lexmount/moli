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
    mode: RequestMode,
    credentials: RequestCredentialsMode,
    policy: SubresourcePolicyContext,
    started: Option<Box<StartedFetchStream>>,
}

struct StartedFetchStream {
    info: PendingSubresourceFetchInfo,
    head: ResponseHead,
    delivered: usize,
    delivery: WorkerFetchStreamingChunk,
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
    ) -> Self {
        Self {
            completion,
            fetch_id,
            interception,
            mode,
            credentials,
            policy,
            started: None,
        }
    }

    pub(super) fn body_source_id(&self) -> Option<NetworkBodySourceId> {
        self.started
            .as_ref()
            .map(|started| started.delivery.body_source_id)
    }

    pub(super) fn start(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        head: ResponseHead,
        network_request_headers: Option<Vec<(String, String)>>,
        body: ResourceInterceptionBody,
    ) -> Result<(), String> {
        super::super::validate_fetch_response_security_policy_with_body_classified(
            &info.document_url,
            &head,
            &[],
            self.mode,
            self.credentials,
            self.policy,
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
                    .with_request_cookie_report(head.request_cookie_report.clone())
                    .with_status_text(head.status_text.clone())
                    .with_from_cache(head.from_cache)
                    .with_network_request_headers(network_request_headers.clone())
                    .with_negotiated_http_version(head.negotiated_http_version),
                ),
            ));
        let body_source_id = super::super::new_network_body_source_id();
        let _ = self.completion.send(WorkerFetchEvent::StreamingStarted(
            WorkerFetchStreamingStarted {
                fetch_id: self.fetch_id,
                body_source_id,
                head: head.clone(),
            },
        ));
        self.started = Some(Box::new(StartedFetchStream {
            info: info.clone(),
            head,
            delivery: WorkerFetchStreamingChunk::new(body_source_id, body),
            delivered: 0,
        }));
        Ok(())
    }

    pub(super) fn chunk(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let started = self.started.as_mut().expect("response precedes body");
        let handle = started
            .info
            .network_request_handle
            .expect("native Fetch identity");
        started.delivered += bytes.len();
        self.interception
            .publish_network(ScriptNetworkOutputItem::SubresourceDataReceived(
                SubresourceDataReceived::new(handle, bytes.len(), bytes.len()),
            ));
        // Only publish a coalesced range. The native capture already owns
        // these bytes, and transport completion never waits for Worker JS.
        started
            .delivery
            .publish(started.delivered, &self.completion);
        Ok(())
    }

    pub(super) fn finish(&mut self, result: Result<(), String>) {
        let started = *self
            .started
            .take()
            .expect("streamed response must have headers");
        let handle = started
            .info
            .network_request_handle
            .expect("native Fetch identity");
        let body = started.delivery.body.finish_capture(started.delivered);
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
                body_source_id: started.delivery.body_source_id,
                head: started.head,
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
        let mut stream = super::ControlledResourceStream::WorkerFetch(ControlledFetchStream::new(
            completion.clone(),
            fetch_id,
            self.interception.clone(),
            self.request.request_mode,
            self.request.credentials_mode,
            policy,
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
                &self.info.document_url,
                &delivered.response.head(),
                delivered.response.body_bytes(),
                self.request.request_mode,
                self.request.credentials_mode,
                policy,
            ) {
                Ok(()) => {}
                Err(super::super::FetchResponseSecurityViolation::OpaqueResponseBlocked(_)) => {
                    blocked_opaque = true
                }
                Err(error) => result = Err(error.into_message()),
            }
        }
        let mut record = super::result::ResourceLoadResult::new(self.info.clone(), result, headers);
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

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::network::interception_output::ResourceInterceptionEvent;
    use crate::types::SubresourceResourceType;
    use moli_fetch::{FetchCancelHandle, StreamingRawResponse};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn blocked_worker_coalesces_large_body_without_delaying_network_terminal() {
        for worker_exited in [false, true] {
            let (sender, receiver) = mpsc::unbounded_channel();
            let mut receiver = (!worker_exited).then_some(receiver);
            let (chunks, body_rx) = mpsc::unbounded_channel();
            let (complete, completion_rx) = tokio::sync::oneshot::channel();
            let cancel = FetchCancelHandle::new();
            let url = "https://worker.test/body".parse::<url::Url>().unwrap();
            let response = StreamingRawResponse::new(
                url.clone(),
                200,
                Vec::new(),
                None,
                Vec::new(),
                false,
                Vec::new(),
                body_rx,
                cancel.clone(),
                completion_rx,
            );
            let pool = moli_disk_pool::DiskPool::new(None).unwrap();
            let loaded = super::super::LoadedResponse::from_stream(response, Some(pool.clone()));
            let info = PendingSubresourceFetchInfo {
                internal_id: 1,
                network_request_handle: Some(
                    crate::network::interception_control::next_subresource_network_handle(),
                ),
                frame_id: None,
                document_url: url.clone(),
                url,
                websocket_socket_id: None,
                method: "GET".into(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Fetch,
                request_cookie_report: None,
            };
            let terminals = Arc::new(AtomicUsize::new(0));
            let observed = terminals.clone();
            let interception = ResourceInterceptionContext::worker(None, move |event| {
                if let ResourceInterceptionEvent::Network(item) = event
                    && matches!(*item, ScriptNetworkOutputItem::SubresourceBodyFinished(_))
                {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                true
            });
            let mut stream =
                super::super::ControlledResourceStream::WorkerFetch(ControlledFetchStream::new(
                    sender,
                    1,
                    interception,
                    RequestMode::Cors,
                    RequestCredentialsMode::SameOrigin,
                    SubresourcePolicyContext::default(),
                ));
            const LENGTH: usize = 4 * 1024 * 1024;
            for value in 0..4 {
                chunks.send(vec![value; LENGTH / 4]).unwrap();
            }
            drop(chunks);
            complete.send(Ok(())).unwrap();
            assert!(
                loaded
                    .deliver(&info, Some(&mut stream), None)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(!cancel.is_cancelled());
            let super::super::ControlledResourceStream::WorkerFetch(stream) = &mut stream else {
                unreachable!()
            };
            stream.finish(Ok(()));
            assert_eq!(terminals.load(Ordering::SeqCst), 1);
            // No Worker task has run. The response uses the existing disk
            // capture, not a second queued allocation for every body chunk.
            assert!(pool.diagnostics().disk_footprint_bytes >= LENGTH as u64);
            if let Some(receiver) = &mut receiver {
                let WorkerFetchEvent::StreamingStarted(_) = receiver.try_recv().unwrap() else {
                    panic!("response headers")
                };
                let WorkerFetchEvent::StreamingChunk(chunk) = receiver.try_recv().unwrap() else {
                    panic!("one coalesced body wake")
                };
                assert_eq!(chunk.take_range(), 0..LENGTH);
                for value in 0..4 {
                    let bytes = chunk
                        .body
                        .read_captured_chunk(value * LENGTH / 4, LENGTH / 4)
                        .unwrap();
                    assert_eq!(bytes, vec![value as u8; LENGTH / 4]);
                }
                let WorkerFetchEvent::StreamingFinished(finished) = receiver.try_recv().unwrap()
                else {
                    panic!("network terminal does not wait for JS")
                };
                assert_eq!(finished.result.unwrap().len(), LENGTH);
                assert!(receiver.try_recv().is_err());
            }
        }
    }
}
