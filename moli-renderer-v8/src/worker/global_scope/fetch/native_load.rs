use super::*;

pub(super) struct NativeWorkerFetchRequest<'s> {
    pub(super) request: Request,
    pub(super) blob_url_entry: Option<CapturedBlobUrl>,
    pub(super) kind: WorkerFetchKind,
    pub(super) font_face: Option<v8::Local<'s, v8::Object>>,
}

pub(super) fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: NativeWorkerFetchRequest<'s>,
    document_url: Url,
    signal_id: Option<u32>,
    metadata: ServiceWorkerFetchRequestMetadata,
    policy_context: SubresourcePolicyContext,
) -> Option<v8::Local<'s, v8::Promise>> {
    let NativeWorkerFetchRequest { request, blob_url_entry, kind, font_face } = request;
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let cancel = FetchCancelHandle::new();
    let load = state.borrow().loader.register_load(
        kind.resource_type().into(),
        if metadata.keepalive {
            ResourceLoadDisposition::Keepalive
        } else {
            ResourceLoadDisposition::Ordinary
        },
        Some(cancel.clone()),
    );
    let Some(load) = load else {
        return Some(make_rejected_promise(
            scope,
            "fetch: worker global is shutting down",
        ));
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
    let (fetch_id, completion, control, resource_timing) = {
        let mut state = state.borrow_mut();
        let fetch_id = next_fetch_id(&mut state);
        let resource_timing =
            crate::context_bootstrap::FetchResourceTiming::capture(scope, &state.request_origin());
        state.pending_fetches.insert(
            fetch_id,
            PendingWorkerFetch {
                kind,
                font_face: font_face.map(|face| v8::Global::new(scope, face)),
                resolver: v8::Global::new(scope, resolver),
                resource_timing: resource_timing.clone(),
                signal_id,
                load: load.clone(),
                request_url: request.url.clone(),
                request_method: request.method.clone(),

                streaming_body_source_id: None,
                streaming_needs_orb_body_validation: false,
            },
        );
        let mut control = state.resource_load_control();
        control.connect_policy = control.connect_policy.for_resource_kind(kind.csp_resource_kind());
        (fetch_id, state.fetch_completion_tx.clone(), control, resource_timing)
    };
    let service_worker = state.borrow().service_worker_runtime.clone();
    let service_worker_client = state.borrow().service_worker_client_id;
    let task_runner = load.task_runner();
    task_runner.spawn(async move {
        let integrity = metadata.integrity.clone();
        control
            .fetch_load(request, info, load, cancel, policy_context)
            .with_blob_url_entry(blob_url_entry)
            .with_service_worker(service_worker, service_worker_client, metadata)
            .run_fetch(completion, fetch_id, resource_timing, integrity)
            .await;
    });
    Some(promise)
}
