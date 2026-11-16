use super::*;

pub(super) fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: Request,
    document_url: Url,
    signal_id: Option<u32>,
    metadata: ServiceWorkerFetchRequestMetadata,
    policy_context: SubresourcePolicyContext,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let cancel = FetchCancelHandle::new();
    let loader = state.borrow().loader.clone();
    let Some(load) = loader.register_load(
        ResourceLoadKind::Fetch,
        if metadata.keepalive {
            ResourceLoadDisposition::Keepalive
        } else {
            ResourceLoadDisposition::Ordinary
        },
        Some(cancel.clone()),
    ) else {
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
        document_url: document_url.clone(),
        url: request.url.clone(),
        websocket_socket_id: None,
        method: request.method.clone(),

        request_body_bytes: request.body.clone(),
        resource_type: SubresourceResourceType::Fetch,
        request_cookie_report: None,
    };
    let (fetch_id, completion, control) = {
        let mut state = state.borrow_mut();
        let fetch_id = next_fetch_id(&mut state);
        let resource_timing =
            crate::context_bootstrap::FetchResourceTiming::capture(scope, &state.request_origin());
        state.pending_fetches.insert(
            fetch_id,
            PendingWorkerFetch {
                resolver: v8::Global::new(scope, resolver),
                resource_timing,
                credentials_mode: request.credentials_mode,
                request_mode: request.request_mode,
                redirect_mode: request.redirect_mode(),

                policy_context,
                signal_id,
                load: load.clone(),
                request_url: request.url.clone(),
                request_method: request.method.clone(),

                streaming_body_source_id: None,
            },
        );
        let control = crate::network_host::controlled_xhr::ControlledXhrContext {
            interception: state
                .interception_context
                .clone()
                .expect("native Worker fetch has an output owner"),
            connect_policy:
                crate::document_runtime::DocumentConnectPolicySnapshot::from_worker_policies(
                    state.content_security_policies.clone(),
                    state.content_security_report_only_policies.clone(),
                    state.content_security_reporting_endpoints.clone(),
                ),
        };
        (fetch_id, state.fetch_completion_tx.clone(), control)
    };
    let service_worker = state.borrow().service_worker_runtime.clone();
    let service_worker_client = state.borrow().service_worker_client_id;
    let task_runner = load.task_runner();
    task_runner.spawn(async move {
        control
            .fetch_load(request, info, load, cancel, policy_context)
            .with_service_worker(service_worker, service_worker_client, metadata)
            .run_fetch(completion, fetch_id)
            .await;
    });
    Some(promise)
}
