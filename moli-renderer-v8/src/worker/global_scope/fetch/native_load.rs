use super::*;

pub(in crate::worker::global_scope) struct NativeWorkerFetchRequest<'s> {
    pub(in crate::worker::global_scope) request: Request,
    pub(in crate::worker::global_scope) race_response:
        Option<crate::service_worker_runtime::static_router::race::RouterRaceResponseReader>,
    pub(in crate::worker::global_scope) event_source: Option<v8::Local<'s, v8::Object>>,
    pub(in crate::worker::global_scope) admission_error: Option<String>,
    pub(in crate::worker::global_scope) body_stream: Option<v8::Global<v8::Object>>,
    pub(in crate::worker::global_scope) body_stream_locked: bool,
    pub(in crate::worker::global_scope) signal: Option<v8::Local<'s, v8::Object>>,
    pub(in crate::worker::global_scope) blob_url_entry: Option<CapturedBlobUrl>,
    pub(in crate::worker::global_scope) kind: WorkerFetchKind,
    pub(in crate::worker::global_scope) timing: WorkerResourceTiming,
    pub(in crate::worker::global_scope) font_face: Option<v8::Local<'s, v8::Object>>,
}

pub(in crate::worker::global_scope) fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: NativeWorkerFetchRequest<'s>,
    document_url: Url,
    signal_id: Option<u32>,
    metadata: ServiceWorkerFetchRequestMetadata,
    policy_context: SubresourcePolicyContext,
) -> Option<v8::Local<'s, v8::Promise>> {
    let NativeWorkerFetchRequest {
        mut request,
        blob_url_entry,
        kind,
        font_face,
        timing,
        body_stream,
        body_stream_locked,
        signal,
        race_response,
        event_source,
        admission_error,
    } = request;
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let cancel = FetchCancelHandle::new();
    timing.begin_transport(cancel.clone());
    let Some(load) = state.borrow().loader.register_load(
        kind.resource_type().into(),
        if metadata.keepalive {
            ResourceLoadDisposition::Keepalive {
                body_bytes: if matches!(request.url.scheme(), "http" | "https") {
                    request.body.as_ref().map_or(0, bytes::Bytes::len)
                } else {
                    0
                },
            }
        } else {
            ResourceLoadDisposition::Ordinary
        },
        Some(cancel.clone()),
    ) else {
        return Some(make_rejected_promise(scope, "Failed to fetch"));
    };
    let handle = crate::network::interception_control::next_subresource_network_handle();
    let info = PendingSubresourceFetchInfo {
        internal_id: crate::network::interception_control::next_subresource_request_id(),
        network_request_handle: Some(handle),
        request_headers: request.interception_request_headers_with_web_security(
            load.request_client().web_security_disabled(),
        ),
        request_body: request_body_text(&request.body),

        frame_id: None,
        document_url,
        url: request.url.clone(),
        websocket_socket_id: None,
        method: request.method.clone(),

        request_body_bytes: request.body.clone(),
        resource_type: kind.resource_type(),
        request_cookie_report: None,
    };
    let (fetch_id, completion, control) = {
        let mut state = state.borrow_mut();
        let fetch_id = next_fetch_id(&mut state);
        state.pending_fetches.insert(
            fetch_id,
            PendingWorkerFetch {
                upload: None,
                kind,
                client: if let Some(source) = event_source {
                    WorkerFetchClient::EventSource(v8::Global::new(scope, source))
                } else {
                    WorkerFetchClient::Promise {
                        resolver: v8::Global::new(scope, resolver),
                        font_face: font_face.map(|face| v8::Global::new(scope, face)),
                    }
                },
                request_headers: request.request_headers.clone(),
                timing: timing.clone(),
                signal_id,
                load: load.clone(),
                request_method: request.method.clone(),

                streaming_body_source_id: None,
            },
        );
        let mut control = state.resource_load_control();
        control.connect_policy = control
            .connect_policy
            .for_resource_kind(kind.csp_resource_kind());
        (fetch_id, state.fetch_completion_tx.clone(), control)
    };
    if let Some(source) = event_source {
        crate::network_host::set_event_source_active_request_id(
            scope,
            source,
            Some(u64::from(fetch_id)),
        );
    }
    if let Some(error) = admission_error {
        record_worker_subresource_failure_with_handle(
            &state.borrow(),
            info.network_request_handle,
            info.document_url.clone(),
            info.url.clone(),
            info.method.clone(),
            info.request_headers.clone(),
            info.request_body_bytes.clone(),
            info.resource_type,
            error.clone(),
        );
        let _ = completion.send(WorkerFetchEvent::EventSourceFailure {
            fetch_id,
            error,
            mode: crate::network_host::EventSourceTerminalMode::Close,
        });
        return Some(promise);
    }
    let service_worker = state.borrow().service_worker_runtime.clone();
    let service_worker_client = state.borrow().service_worker_client_id;
    let controlled = service_worker
        .as_ref()
        .zip(service_worker_client)
        .is_some_and(|(runtime, client)| {
            runtime
                .matching_controller_for_client_fetch(client, &request.url)
                .is_some()
        });
    let mut transferred = None;
    let mut request_body_owner = None;
    if controlled {
        if metadata.body_source_is_null
            && let Some(stream) = body_stream.as_ref()
        {
            let stream = v8::Local::new(scope, stream);
            transferred =
                crate::context_bootstrap::transfer_request_body(scope, stream, body_stream_locked);
            if transferred.is_none() {
                state.borrow_mut().pending_fetches.remove(&fetch_id);
                let _ = resolver.reject(
                    scope,
                    v8::Exception::type_error(
                        scope,
                        v8str(scope, "Failed to transfer request body"),
                    ),
                );
                return Some(promise);
            }
            if let Some(signal) = signal {
                crate::context_bootstrap::install_request_body_abort(scope, stream, signal);
            }
        }
        let events = completion.clone();
        request_body_owner = Some(crate::context_bootstrap::request_body_owner(
            scope,
            move |producer| {
                let _ = events.send(WorkerFetchEvent::RequestBodyProducer { fetch_id, producer });
            },
        ));
    }
    if !controlled
        && metadata.body_source_is_null
        && matches!(request.url.scheme(), "http" | "https")
        && let Some(stream) = body_stream.as_ref()
    {
        let stream = v8::Local::new(scope, stream);
        let completion = completion.clone();
        match crate::network_host::RequestUpload::new(
            scope,
            stream,
            resolver,
            body_stream_locked,
            load.task_runner(),
            cancel.clone(),
            move |result| {
                let _ = completion.send(WorkerFetchEvent::RequestUploadReady { fetch_id, result });
            },
        ) {
            Ok((upload, body)) => {
                state
                    .borrow_mut()
                    .pending_fetches
                    .get_mut(&fetch_id)
                    .expect("new worker fetch")
                    .upload = Some(upload.clone());
                upload.start(scope);
                request = request.with_upload_body(body);
            }
            Err(reason) => {
                state.borrow_mut().pending_fetches.remove(&fetch_id);
                let _ = resolver.reject(scope, reason);
                return Some(promise);
            }
        }
    }
    let task_runner = load.task_runner();
    let event_source_parser = crate::worker::global_scope::event_source::parser_for_request(
        kind,
        &request.request_headers,
    );
    let redirect_owner = (kind != WorkerFetchKind::Font).then(|| {
        crate::network_host::RedirectOwner::Worker(WorkerRedirectOwner::fetch(
            completion.clone(),
            fetch_id,
            load.clone(),
        ))
    });
    task_runner.spawn(async move {
        let integrity = metadata.integrity.clone();
        control
            .fetch_load(request, info, load, cancel, policy_context)
            .with_redirect_owner(redirect_owner)
            .with_race_response(race_response)
            .with_blob_url_entry(blob_url_entry)
            .with_worker_service_worker(service_worker, service_worker_client, metadata)
            .with_service_worker_request_body(transferred, request_body_owner)
            .run_fetch(completion, fetch_id, integrity, timing, event_source_parser)
            .await;
    });
    Some(promise)
}
