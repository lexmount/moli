use super::*;
use crate::native_bridge::{JsContextHost, throw_dom_exception};
use crate::network_host::{
    BLOCKED_BY_CLIENT_ERROR_TEXT, active_subresource_network_partition_key,
    effective_subresource_policy_context, local_url_response, merge_subresource_request_headers,
    observe_subresource_request_cookie_report, resolve_context_url, spawn_async_subresource_fetch,
    subresource_request_scope_for_owner,
};
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerFetchRequestMetadata, ServiceWorkerRequestDestination,
};
use crate::types::{
    PendingSubresourceFetchInfo, SubresourceNetworkRecord, SubresourceResourceType,
};
use crate::webidl;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, Request, RequestCacheMode, RequestCredentialsMode,
    RequestMode, RequestRedirectMode, should_request_be_blocked_due_to_bad_port,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventSource")]
struct EventSourceConstructorArgs<'s> {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to construct 'EventSource': 1 argument required, but only 0 present."
    )]
    url: String,
    #[webidl(index = 1, converter = "raw")]
    options: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "EventSourceInit")]
struct EventSourceInit {
    #[webidl(default = false)]
    with_credentials: bool,
}

struct PreparedEventSourceRequest {
    frame_id: Option<String>,
    execution_context: crate::native_bridge::WindowExecutionContextBinding,
    owner: crate::native_bridge::OwnerDispatchScope,
    resource_loader: crate::network::context::DocumentResourceLoader,
    document_url: url::Url,
    resolved_url: url::Url,
    request_headers: Vec<(String, String)>,
    cors_preflight_request_headers: Vec<(String, String)>,
    credentials_mode: RequestCredentialsMode,
    request_cookie_report: Option<moli_cookie_jar::StoredCookieQueryReport>,
    network_partition_key: Option<String>,
    policy_context: crate::types::SubresourcePolicyContext,
}

pub(crate) fn event_source_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'EventSource': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<EventSourceConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let options = match event_source_options(scope, parsed.options) {
        Some(options) => options,
        None => return,
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(
            scope,
            "Failed to construct 'EventSource': runtime is unavailable.",
        );
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(execution_context) = host.current_runtime_window_execution_context_binding(scope)
    else {
        throw_type_error(
            scope,
            "Failed to construct 'EventSource': Window execution context is unavailable.",
        );
        return;
    };
    let owner = execution_context.dispatch_scope();
    let Some((_, document_url)) = subresource_request_scope_for_owner(scope, host, owner) else {
        throw_type_error(
            scope,
            "Failed to construct 'EventSource': Window execution context owner is retired.",
        );
        return;
    };
    let resolved_url = match resolve_context_url(&document_url, &parsed.url, None) {
        Ok(url) => url,
        Err(_) => {
            throw_dom_exception(
                scope,
                "SyntaxError",
                12,
                "Failed to construct 'EventSource': The URL is invalid.",
            );
            return;
        }
    };

    let event_source = args.this();
    initialize_event_source_object(
        scope,
        event_source,
        resolved_url.as_str(),
        options.with_credentials,
    );
    if !schedule_event_source_connect(scope, event_source, 0) {
        fail_event_source_connection(scope, event_source, EventSourceTerminalMode::Close);
    }
    rv.set(event_source.into());
}

fn event_source_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Option<EventSourceInit> {
    let Some(value) = value else {
        return Some(EventSourceInit::default());
    };
    match webidl::parse_dictionary::<EventSourceInit>(
        scope,
        value,
        webidl::Context::argument("EventSource", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(crate) fn start_event_source_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
) {
    if event_source_ready_state(scope, event_source) == EVENT_SOURCE_CLOSED
        || event_source_active_request_id(scope, event_source).is_some()
    {
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        fail_event_source_connection(scope, event_source, EventSourceTerminalMode::Close);
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let prepared = match prepare_event_source_request(scope, host, event_source) {
        Ok(prepared) => prepared,
        Err(error) => {
            record_event_source_preflight_failure(host, event_source, scope, error);
            return;
        }
    };

    if let Some(violation) = host
        .check_document_connect_csp_for_owner(
            scope,
            prepared.owner,
            &prepared.document_url,
            &prepared.resolved_url,
        )
        .into_blocking_violation()
    {
        let error = crate::document_runtime::document_content_security_policy_error_message(
            &violation,
            "EventSource",
        );
        record_prepared_event_source_failure(
            host,
            &prepared,
            error,
            scope,
            event_source,
            EventSourceTerminalMode::Close,
        );
        return;
    }
    if should_request_be_blocked_due_to_bad_port(&prepared.resolved_url) {
        let error = format!(
            "EventSource: blocked bad port for `{}`",
            prepared.resolved_url
        );
        record_prepared_event_source_failure(
            host,
            &prepared,
            error,
            scope,
            event_source,
            EventSourceTerminalMode::Close,
        );
        return;
    }
    if host.is_url_blocked(&prepared.resolved_url) {
        record_prepared_event_source_failure(
            host,
            &prepared,
            BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
            scope,
            event_source,
            EventSourceTerminalMode::Close,
        );
        return;
    }
    if host.network_offline() {
        record_prepared_event_source_failure(
            host,
            &prepared,
            "Network emulation offline".to_owned(),
            scope,
            event_source,
            EventSourceTerminalMode::Reconnect,
        );
        return;
    }

    if host.should_intercept_subresource(SubresourceResourceType::EventSource) {
        let registered =
            register_event_source_request(scope, host, event_source, &prepared, None, false);
        set_event_source_active_request_id(scope, event_source, Some(registered.internal_id));
        return;
    }

    if let Some(internal_id) =
        dispatch_service_worker_event_source(scope, host, event_source, &prepared)
    {
        set_event_source_active_request_id(scope, event_source, Some(internal_id));
        return;
    }

    if let Some(response) = local_url_response(&prepared.resolved_url) {
        let registered =
            register_event_source_request(scope, host, event_source, &prepared, None, true);
        set_event_source_active_request_id(scope, event_source, Some(registered.internal_id));
        let _ = host.resource_completion_sender().send_async_subresource(
            crate::types::AsyncSubresourceFetchCompletion {
                internal_id: registered.internal_id,
                request_url: prepared.resolved_url,
                request_method: "GET".to_owned(),
                request_headers: prepared.request_headers,
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                parkable_image: None,
                result: Ok(response.into()),
            },
        );
        return;
    }

    let request = match build_event_source_request(&prepared) {
        Ok(request) => request,
        Err(error) => {
            record_prepared_event_source_failure(
                host,
                &prepared,
                error,
                scope,
                event_source,
                EventSourceTerminalMode::Close,
            );
            return;
        }
    };
    let cancel_handle = FetchCancelHandle::new();
    let registered = register_event_source_request(
        scope,
        host,
        event_source,
        &prepared,
        Some(cancel_handle.clone()),
        true,
    );
    set_event_source_active_request_id(scope, event_source, Some(registered.internal_id));
    spawn_async_subresource_fetch(
        registered.load.task_runner(),
        host.resource_completion_sender(),
        registered.load.request_client(),
        request,
        Some(cancel_handle),
        prepared.cors_preflight_request_headers,
        registered.internal_id,
        crate::types::AsyncSubresourceNetworkContext {
            frame_id: prepared.frame_id,
            document_url: prepared.document_url,
            resource_type: SubresourceResourceType::EventSource,
            policy_context: prepared.policy_context,
        },
        prepared.resolved_url,
        "GET".to_owned(),
        prepared.request_headers,
        None,
    );
}

fn prepare_event_source_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    event_source: v8::Local<'s, v8::Object>,
) -> Result<PreparedEventSourceRequest, String> {
    let execution_context = host
        .current_runtime_window_execution_context_binding(scope)
        .ok_or_else(|| "EventSource: Window execution context is unavailable".to_owned())?;
    let owner = execution_context.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_dispatch_scope(owner)
        .ok_or_else(|| "EventSource: Document resource authority is unavailable".to_owned())?;
    let (frame_id, document_url) = subresource_request_scope_for_owner(scope, host, owner)
        .ok_or_else(|| "EventSource: Window execution context owner is retired".to_owned())?;
    let resolved_url = event_source_connection_url(scope, event_source)
        .ok_or_else(|| "EventSource: URL state is unavailable".to_owned())
        .and_then(|value| url::Url::parse(&value).map_err(|error| error.to_string()))?;
    let mut request_headers = vec![
        ("Accept".to_owned(), "text/event-stream".to_owned()),
        ("Cache-Control".to_owned(), "no-cache".to_owned()),
    ];
    let last_event_id = event_source_last_event_id(scope, event_source);
    if !last_event_id.is_empty() {
        request_headers.push(("Last-Event-ID".to_owned(), last_event_id));
    }
    let request_headers =
        merge_subresource_request_headers(host.extra_http_headers(), &request_headers);
    let credentials_mode = if event_source_with_credentials(scope, event_source) {
        RequestCredentialsMode::Include
    } else {
        RequestCredentialsMode::SameOrigin
    };
    let request_cookie_report = observe_subresource_request_cookie_report(
        resource_loader.request_client(),
        &document_url,
        &resolved_url,
        "GET",
        credentials_mode,
    );
    Ok(PreparedEventSourceRequest {
        frame_id,
        execution_context,
        owner,
        resource_loader,
        document_url,
        resolved_url,
        request_headers,
        cors_preflight_request_headers: host.extra_http_headers().to_vec(),
        credentials_mode,
        request_cookie_report,
        network_partition_key: active_subresource_network_partition_key(host, owner),
        policy_context: effective_subresource_policy_context(scope, host, owner),
    })
}

fn dispatch_service_worker_event_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    event_source: v8::Local<'s, v8::Object>,
    prepared: &PreparedEventSourceRequest,
) -> Option<u64> {
    if !matches!(prepared.resolved_url.scheme(), "http" | "https") {
        return None;
    }
    let client_id = host.service_worker_client_id_for_subresource_owner(prepared.owner);
    host.service_worker_controller_for_fetch(
        client_id,
        &prepared.document_url,
        &prepared.resolved_url,
    )?;

    let cancel_handle = FetchCancelHandle::new();
    let registered = register_event_source_request(
        scope,
        host,
        event_source,
        prepared,
        Some(cancel_handle.clone()),
        true,
    );
    let request = host.service_worker_fetch_request(
        client_id,
        prepared.resolved_url.clone(),
        "GET".to_owned(),
        prepared.request_headers.clone(),
        None,
        ServiceWorkerRequestDestination::Empty,
        RequestMode::Cors,
        prepared.credentials_mode,
        RequestRedirectMode::Follow,
        None,
        ServiceWorkerFetchRequestMetadata {
            cache: "no-store".to_owned(),
            ..ServiceWorkerFetchRequestMetadata::default()
        },
    );
    let dispatch = ServiceWorkerFetchDispatch {
        internal_id: registered.internal_id,
        request,
        request_body_text: None,
        cors_preflight_request_headers: prepared.cors_preflight_request_headers.clone(),
        request_cookie_report: prepared.request_cookie_report.clone(),
        network_context: crate::types::AsyncSubresourceNetworkContext {
            frame_id: prepared.frame_id.clone(),
            document_url: prepared.document_url.clone(),
            resource_type: SubresourceResourceType::EventSource,
            policy_context: prepared.policy_context,
        },
        completion_tx: host.resource_completion_sender(),
        request_client: registered.load.request_client(),
        resource_task_runner: registered.load.task_runner(),
        cancel_handle,
        direct_completion_tx: None,
    };
    if !host.dispatch_service_worker_fetch(dispatch) {
        let _ = host.resource_completion_sender().send_async_subresource(
            crate::types::AsyncSubresourceFetchCompletion {
                internal_id: registered.internal_id,
                request_url: prepared.resolved_url.clone(),
                request_method: "GET".to_owned(),
                request_headers: prepared.request_headers.clone(),
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                parkable_image: None,
                result: Err("service worker EventSource dispatch failed".to_owned()),
            },
        );
    }
    Some(registered.internal_id)
}

fn build_event_source_request(prepared: &PreparedEventSourceRequest) -> Result<Request, String> {
    Request::new(
        "GET",
        prepared.resolved_url.as_str(),
        None,
        prepared.request_headers.clone(),
    )
    .map_err(|error| error.to_string())
    .map(|request| {
        request
            .with_initiator_url(&prepared.document_url)
            .with_request_mode(RequestMode::Cors)
            .with_credentials_mode(prepared.credentials_mode)
            .with_network_partition_key(prepared.network_partition_key.clone())
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_cache_mode(RequestCacheMode::NoStore)
            .with_browser_request_metadata(BrowserRequestMetadata::EventSource)
            .with_subframe_context(prepared.frame_id.is_some())
            .without_request_timeout()
    })
}

struct RegisteredEventSourceRequest {
    internal_id: u64,
    load: crate::network::loads::ResourceLoadLease,
}

fn register_event_source_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut JsContextHost,
    event_source: v8::Local<'s, v8::Object>,
    prepared: &PreparedEventSourceRequest,
    cancel_handle: Option<FetchCancelHandle>,
    request_started: bool,
) -> RegisteredEventSourceRequest {
    let (internal_id, load) = host.record_async_subresource_event_source(
        prepared.execution_context.duplicate(scope),
        &prepared.resource_loader,
        v8::Global::new(scope, event_source),
        cancel_handle,
        prepared.credentials_mode,
        prepared.network_partition_key.clone(),
        prepared.policy_context,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: prepared.frame_id.clone(),
            document_url: prepared.document_url.clone(),
            url: prepared.resolved_url.clone(),
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: prepared.request_headers.clone(),
            request_body: None,
            request_body_bytes: None,
            resource_type: SubresourceResourceType::EventSource,
            request_cookie_report: prepared.request_cookie_report.clone(),
        },
        request_started,
    );
    RegisteredEventSourceRequest { internal_id, load }
}

fn record_event_source_preflight_failure<'s>(
    _host: &mut JsContextHost,
    event_source: v8::Local<'s, v8::Object>,
    scope: &mut v8::PinScope<'s, '_>,
    _error: String,
) {
    fail_event_source_connection(scope, event_source, EventSourceTerminalMode::Close);
}

fn record_prepared_event_source_failure<'s>(
    host: &mut JsContextHost,
    prepared: &PreparedEventSourceRequest,
    error: String,
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    mode: EventSourceTerminalMode,
) {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id.clone(),
        prepared.document_url.clone(),
        prepared.resolved_url.clone(),
        "GET".to_owned(),
        prepared.request_headers.clone(),
        None,
        SubresourceResourceType::EventSource,
        error,
    ));
    fail_event_source_connection(scope, event_source, mode);
}
