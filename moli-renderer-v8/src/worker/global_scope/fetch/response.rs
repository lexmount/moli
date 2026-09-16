use super::*;
use crate::network::{PausedResourceResponse, ResourceResponseBody, ResourceResponseConsumer};
#[cfg(test)]
use moli_fetch::StreamingRawResponse;

/// A resource result returns to the Worker that admitted it. The response and
/// load outlive the VM when its completion queue closes during transport.
pub(crate) struct WorkerResponseSender {
    pub(crate) response: Arc<ResourceResponseStream>,
    load: ResourceLoadLease,
    fetch_id: u32,
    sender: Option<WorkerResponseDestination>,
    body_source_id: Option<NetworkBodySourceId>,
    stream_to_script: bool,
    preflight: crate::network_host::CorsPreflightNetworkObserver,
}

#[derive(Clone)]
enum WorkerResponseDestination {
    Fetch(mpsc::UnboundedSender<WorkerFetchEvent>),
    Xhr(Arc<dyn Fn(WorkerXhrCompletion) + Send + Sync>),
}

impl WorkerResponseSender {
    pub(super) fn new(
        state: &WorkerGlobalState,
        pending: &PendingWorkerFetch,
        fetch_id: u32,
    ) -> Self {
        let observer = state.parent_tx.network_observer();
        Self {
            response: pending.response.clone(),
            load: pending.load.clone(),
            fetch_id,
            sender: Some(WorkerResponseDestination::Fetch(
                state.fetch_completion_tx.clone(),
            )),
            body_source_id: None,
            stream_to_script: false,
            preflight: crate::network_host::CorsPreflightNetworkObserver {
                request: pending
                    .response
                    .network
                    .request()
                    .expect("admitted Worker fetch"),
                observer: Arc::new(move |event| observer.publish(event)),
                frame_id: None,
                resource_type: SubresourceResourceType::Fetch,
                keepalive: pending.request_metadata.keepalive,
            },
        }
    }

    pub(in crate::worker) fn xhr(
        load: ResourceLoadLease,
        response: Arc<ResourceResponseStream>,
        observer: crate::worker::WorkerNetworkObserver,
        fetch_id: u32,
        deliver: impl Fn(WorkerXhrCompletion) + Send + Sync + 'static,
    ) -> Self {
        let preflight = crate::network_host::CorsPreflightNetworkObserver {
            request: response.network.request().expect("admitted Worker XHR"),
            observer: Arc::new(move |event| observer.publish(event)),
            frame_id: None,
            resource_type: SubresourceResourceType::Xhr,
            keepalive: false,
        };
        Self {
            load,
            response,
            fetch_id,
            sender: Some(WorkerResponseDestination::Xhr(Arc::new(deliver))),
            body_source_id: None,
            stream_to_script: false,
            preflight,
        }
    }

    pub(crate) fn complete(
        mut self,
        result: Result<ResourceBodyResponse, ResourceResponseFailure>,
        network_request_headers: Option<Vec<(String, String)>>,
    ) {
        self.send_completion(result, network_request_headers);
    }

    fn send_completion(
        &mut self,
        result: Result<ResourceBodyResponse, ResourceResponseFailure>,
        network_request_headers: Option<Vec<(String, String)>>,
    ) {
        let Some(sender) = self.sender.take() else {
            return;
        };
        if !result
            .as_ref()
            .is_ok_and(|response| self.response.intercepts_response(&response.head))
        {
            self.load.finish();
        }
        let delivery = WorkerRequestDelivery::new(
            self.response.clone(),
            WorkerRequestCompletion {
                id: self.fetch_id,
                network_request_headers,
                result,
            },
        );
        let WorkerResponseDestination::Fetch(sender) = sender else {
            let WorkerResponseDestination::Xhr(deliver) = sender else {
                unreachable!()
            };
            deliver(WorkerXhrCompletion::TransportCompletion(delivery));
            return;
        };
        let event = match self.body_source_id {
            Some(body_source_id) => {
                WorkerFetchEvent::StreamingFinished(WorkerFetchStreamingFinished {
                    body_source_id,
                    delivery,
                })
            }
            None => WorkerFetchEvent::TransportCompletion(delivery),
        };
        let _ = sender.send(event);
    }

    pub(crate) fn fetch_network(
        self,
        request: Request,
        cancel: FetchCancelHandle,
        preflight_headers: Vec<(String, String)>,
    ) {
        let kind = if matches!(self.sender, Some(WorkerResponseDestination::Xhr(_))) {
            "xhr"
        } else {
            "fetch"
        };
        self.load.task_runner().spawn(async move {
            let result = if let Err(message) = moli_fetch::FetchUrlList::new(&request.url, &[])
                .validate_request_mode(
                    request.request_mode,
                    request.request_origin().expect("browser request origin"),
                ) {
                Err(ResourceResponseFailure::Request(message))
            } else if let Some(result) = local_url_response_result(&request.url, &request.method) {
                result.map(ResourceBodyResponse::from).map_err(|message| {
                    ResourceResponseFailure::Request(format!("{kind}: {message}"))
                })
            } else {
                let stream_to_script =
                    matches!(self.sender, Some(WorkerResponseDestination::Fetch(_)))
                        && request.request_mode != RequestMode::NoCors
                        && request.follow_redirects;
                let loader = self.load.request_client();
                match fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
                    &loader,
                    request,
                    Some(cancel),
                    preflight_headers,
                    Some(&self.preflight),
                )
                .await
                {
                    Ok(observed) => {
                        let (response, headers) = worker_network_result_parts(observed);
                        self.response.record_request_headers(headers);
                        let body =
                            ResourceResponseBody::streaming(self.response.clone(), response, None);
                        self.receive_or_pause(body, stream_to_script);
                        return;
                    }
                    Err(error) => {
                        let message = format!("{kind}: {error}");
                        Err(error.with_message(message))
                    }
                }
            };
            self.complete(result, None);
        });
    }

    pub(crate) fn receive_or_pause(
        mut self,
        body: Arc<ResourceResponseBody>,
        stream_to_script: bool,
    ) {
        self.stream_to_script = stream_to_script;
        if self.response.intercepts_response(&body.head()) {
            let fetch_id = self.fetch_id;
            let sender = self.sender.as_ref().expect("active producer").clone();
            let response = self.pause(body, stream_to_script);
            match sender {
                WorkerResponseDestination::Fetch(sender) => {
                    let _ = sender.send(WorkerFetchEvent::ResponsePaused {
                        fetch_id,
                        response: Box::new(response),
                    });
                }
                WorkerResponseDestination::Xhr(deliver) => {
                    deliver(WorkerXhrCompletion::ResponsePaused {
                        xhr_id: fetch_id,
                        response: Box::new(response),
                    })
                }
            }
        } else {
            let runner = self.load.task_runner();
            body.resume(Box::new(self));
            body.start(&runner);
        }
    }

    pub(in crate::worker) fn pause(
        mut self,
        body: Arc<ResourceResponseBody>,
        stream_to_script: bool,
    ) -> PausedResourceResponse {
        self.stream_to_script = stream_to_script;
        PausedResourceResponse::new(body, Box::new(self))
    }
}

impl ResourceResponseConsumer for WorkerResponseSender {
    fn task_runner(&self) -> crate::network::RendererResourceTaskRunner {
        self.load.task_runner()
    }
    fn is_cancelled(&self) -> bool {
        self.load.is_cancelled()
    }
    fn detach(&mut self) {
        self.stream_to_script = false;
    }
    fn response_started(&mut self, head: ResponseHead) {
        if self.stream_to_script
            && let Some(WorkerResponseDestination::Fetch(sender)) = &self.sender
        {
            let body_source_id = crate::network_host::new_network_body_source_id();
            self.body_source_id = Some(body_source_id);
            let _ = sender.send(WorkerFetchEvent::StreamingStarted(Box::new(
                WorkerFetchStreamingStarted {
                    fetch_id: self.fetch_id,
                    body_source_id,
                    head,
                },
            )));
        }
    }
    fn data_received(&mut self, bytes: Vec<u8>) {
        if let Some(body_source_id) = self.body_source_id
            && let Some(WorkerResponseDestination::Fetch(sender)) = &self.sender
        {
            let _ = sender.send(WorkerFetchEvent::StreamingChunk(
                WorkerFetchStreamingChunk {
                    body_source_id,
                    bytes,
                },
            ));
        }
    }
    fn complete(
        mut self: Box<Self>,
        result: Result<ResourceBodyResponse, ResourceResponseFailure>,
        _network_error_text: Option<String>,
    ) {
        self.send_completion(result, None);
    }
    fn discard(mut self: Box<Self>) {
        self.sender.take();
    }
}

pub(super) fn pause_worker_fetch_response(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    fetch_id: u32,
    response: PausedResourceResponse,
) {
    if !state
        .borrow()
        .pending_fetches
        .get(&fetch_id)
        .is_some_and(|pending| {
            Arc::ptr_eq(&pending.response, &response.body.resource) && !pending.load.is_cancelled()
        })
    {
        return;
    }
    let head = response.body.head();
    if let Some(message) =
        worker_response_csp_error(scope, state, WorkerFetchTarget::Fetch(fetch_id), &head)
    {
        let resource = response.body.resource.clone();
        response.discard();
        let completion = WorkerRequestCompletion {
            id: fetch_id,
            network_request_headers: None,
            result: Err(resource.failure(message)),
        };
        drain_worker_fetch_completion_result(scope, state, completion);
        return;
    }
    if response.body.resource.handle_auth_requests()
        && matches!(head.status, 401 | 407)
        && let Some(challenge) = extract_subresource_auth_challenge(&head.headers)
    {
        pause_worker_fetch_auth(state, fetch_id, &head, response, challenge);
        return;
    }
    let mut state = state.borrow_mut();
    let pending = state
        .pending_fetches
        .get_mut(&fetch_id)
        .expect("originating fetch");
    let (url, method, headers, body) = worker_fetch_request_metadata(pending);
    let info = PendingSubresourceResponseInfo {
        internal_id: pending.response.network.handle().get(),
        url: url.clone(),
        final_url: head.final_url,
        method: method.to_owned(),
        request_headers: headers.clone(),
        request_body: request_body_text(body),
        resource_type: SubresourceResourceType::Fetch,
        request_cookie_report: head.request_cookie_report,
        network_request_headers: pending.response.record_request_headers(None),
        response_status: head.status,
        response_headers: head.headers,
        response_body: response.body.body_source(),
        from_cache: head.from_cache,
    };
    pending.paused_response = Some(response);
    let handle = pending.response.network.handle();
    let load = pending.load.clone();
    publish_worker_fetch_pause(
        &state,
        crate::runtime::WorkerFetchTarget::Fetch(fetch_id),
        handle,
        load,
        crate::runtime::RendererWorkerFetchStage::Response(Box::new(info)),
    );
}

impl Drop for WorkerResponseSender {
    fn drop(&mut self) {
        if self.sender.is_some() {
            self.load.cancel();
            let failure = self
                .response
                .failure("Worker response producer closed".into());
            self.send_completion(Err(failure), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropped_worker_response_producer_settles_its_original_route() {
        for streamed in [false, true] {
            let cancel = FetchCancelHandle::new();
            let (mut producer, mut receive) = producer_for_test(cancel.clone());
            let response = producer.response.clone();
            let body_source_id = streamed.then(crate::network_host::new_network_body_source_id);
            producer.body_source_id = body_source_id;
            let retained_sender = producer.sender.clone();
            drop(producer);
            assert!(
                cancel.is_cancelled(),
                "a vanished producer must cancel its transport"
            );
            let delivery = match receive.recv().await.unwrap() {
                WorkerFetchEvent::TransportCompletion(delivery) if !streamed => delivery,
                WorkerFetchEvent::StreamingFinished(finished) if streamed => {
                    assert_eq!(finished.body_source_id, body_source_id.unwrap());
                    finished.delivery
                }
                _ => panic!("the original response route must receive its failure"),
            };
            let completion = delivery
                .claim(&response)
                .expect("the originating response must own the completion");
            assert_eq!(completion.id, 42);
            assert!(
                matches!(completion.result, Err(ResourceResponseFailure::Request(message)) if message == "Worker response producer closed")
            );
            // Retaining a queue endpoint cannot retain the load or delay its
            // terminal delivery when the producer disappears.
            drop(retained_sender);
            assert!(
                receive.recv().await.is_none(),
                "only one completion is sent"
            );
        }
    }

    fn producer_for_test(
        cancel: FetchCancelHandle,
    ) -> (
        WorkerResponseSender,
        mpsc::UnboundedReceiver<WorkerFetchEvent>,
    ) {
        let client =
            crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
                .unwrap();
        let load = crate::network::loads::resource_load_lease_for_test(
            client.handle(),
            Some(cancel.clone()),
        );
        let response = ResourceResponseStream::unobserved_for_test();
        let (send, receive) = mpsc::unbounded_channel();
        let producer = WorkerResponseSender {
            response: response.clone(),
            load,
            fetch_id: 42,
            sender: Some(WorkerResponseDestination::Fetch(send)),
            body_source_id: None,
            stream_to_script: false,
            preflight: crate::network_host::CorsPreflightNetworkObserver {
                request: response.network.request().unwrap(),
                observer: Arc::new(|_| {}),
                frame_id: None,
                resource_type: SubresourceResourceType::Fetch,
                keepalive: false,
            },
        };
        (producer, receive)
    }

    #[tokio::test]
    async fn controlled_worker_response_pauses_at_head_and_completes_on_its_original_queue() {
        for authentication in [false, true] {
            for retire in [false, true] {
                let cancel = FetchCancelHandle::new();
                let (producer, mut receive) = producer_for_test(cancel.clone());
                let resource = producer.response.clone();
                let load = producer.load.clone();
                resource.configure_interception(!authentication, authentication);
                let mut head =
                    crate::network_host::local_url_response(&Url::parse("data:,held").unwrap())
                        .unwrap()
                        .head();
                if authentication {
                    head.status = 401;
                    head.headers = vec![("www-authenticate".into(), "Basic realm=held".into())];
                }
                let released = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let release = released.clone();
                let body = ResourceResponseBody::controlled(
                    resource.clone(),
                    crate::network::ResourceResponseHead {
                        head,
                        status_text: None,
                        network_request_headers: None,
                    },
                    cancel,
                    move || {
                        release.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    },
                );
                producer.receive_or_pause(body.clone(), true);
                let WorkerFetchEvent::ResponsePaused { fetch_id, response } =
                    receive.try_recv().unwrap()
                else {
                    panic!(
                        "the controlled head must reach the original decision queue before body input"
                    )
                };
                assert_eq!(fetch_id, 42);
                assert_eq!(
                    response.body.head().status,
                    if authentication { 401 } else { 200 }
                );
                body.data_received(vec![0, 128]);
                assert_eq!(
                    response.body.read(0, 4).await.unwrap(),
                    (vec![0, 128], false)
                );
                assert!(
                    receive.try_recv().is_err(),
                    "no JS stream before the decision"
                );
                let delivery = if retire {
                    load.cancel();
                    drop(response);
                    let WorkerFetchEvent::TransportCompletion(delivery) =
                        receive.try_recv().unwrap()
                    else {
                        panic!("retirement must synchronously queue the failed response")
                    };
                    delivery
                } else {
                    response.resume(Some(206), None);
                    let WorkerFetchEvent::StreamingStarted(started) = receive.try_recv().unwrap()
                    else {
                        panic!("accepted head first")
                    };
                    assert_eq!(started.head.status, 206);
                    let WorkerFetchEvent::StreamingChunk(prefix) = receive.try_recv().unwrap()
                    else {
                        panic!("replay the held prefix")
                    };
                    assert_eq!(prefix.body_source_id, started.body_source_id);
                    assert_eq!(prefix.bytes, [0, 128]);
                    body.data_received(vec![255, 65]);
                    let WorkerFetchEvent::StreamingChunk(tail) = receive.try_recv().unwrap() else {
                        panic!("actual tail before completion")
                    };
                    assert_eq!(tail.body_source_id, started.body_source_id);
                    assert_eq!(tail.bytes, [255, 65]);
                    body.complete(Ok(()), None);
                    let WorkerFetchEvent::StreamingFinished(finished) = receive.try_recv().unwrap()
                    else {
                        panic!("completion cannot be deferred past Worker close")
                    };
                    assert_eq!(finished.body_source_id, started.body_source_id);
                    finished.delivery
                };
                body.data_received(b"late".to_vec());
                body.complete(Err("late".into()), None);
                assert_eq!(
                    released.load(std::sync::atomic::Ordering::SeqCst),
                    usize::from(retire)
                );
                let completion = delivery
                    .claim(&resource)
                    .expect("original response owns completion");
                assert_eq!(completion.id, 42);
                match completion.result {
                    Ok(response) if !retire => {
                        assert_eq!(response.body.clone_body_bytes(), [0, 128, 255, 65])
                    }
                    Err(ResourceResponseFailure::PartialBody { message, body, .. }) if retire => {
                        assert!(message.contains("ERR_ABORTED"));
                        assert_eq!(body.clone_body_bytes(), [0, 128]);
                    }
                    result => panic!("retain the first terminal result: {result:?}"),
                }
                assert!(
                    receive.try_recv().is_err(),
                    "one terminal despite late callbacks"
                );
            }
        }
    }

    #[tokio::test]
    async fn discarded_auth_response_retains_queued_bytes_without_completing_the_request() {
        let cancel = FetchCancelHandle::new();
        let (producer, mut receive) = producer_for_test(cancel.clone());
        let resource = producer.response.clone();
        let load = producer.load.clone();
        let mut head = crate::network_host::local_url_response(&Url::parse("data:,auth").unwrap())
            .unwrap()
            .head();
        head.status = 401;
        head.headers = vec![("www-authenticate".into(), "Basic realm=held".into())];
        resource.configure_interception(false, true);
        let (chunks, receiver) = mpsc::unbounded_channel();
        chunks.send(vec![0, 128]).unwrap();
        chunks.send(vec![255, 65]).unwrap();
        let (_finished, completion) = tokio::sync::oneshot::channel();
        let response =
            StreamingRawResponse::new_with_head(head, receiver, cancel.clone(), completion);
        let body = ResourceResponseBody::streaming(resource.clone(), response, None);
        let paused = producer.pause(body, true);
        assert_eq!(paused.discard().status, 401);
        assert!(cancel.is_cancelled());
        assert!(
            !load.is_cancelled(),
            "the same admission must still allow the authentication retry"
        );
        assert!(
            receive.try_recv().is_err(),
            "discard does not invent a request completion"
        );
        let ResourceResponseFailure::PartialBody { response, body, .. } =
            resource.failure("stopped".into())
        else {
            panic!("received head and bytes must survive")
        };
        assert_eq!(response.head.status, 401);
        assert_eq!(body.clone_body_bytes(), [0, 128, 255, 65]);
    }
}
