use super::*;
use crate::{
    native_bridge::{JsContextHost, WindowDocumentTaskTarget},
    response_links::{DocumentResponseLinks, ResponsePreloadOptions},
    service_worker_runtime::{
        ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext, PendingResponsePreload,
        PendingSubresourceFetchInfo, SubresourceResourceType,
    },
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};
use url::Url;

pub(crate) fn start_document_response_links(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    target: WindowDocumentTaskTarget,
    links: &DocumentResponseLinks,
) {
    start_response_dictionary_loads(scope, host, target, &links.dictionaries);
    let scripting_enabled = host.owner_scripting_enabled(target.dispatch_scope());
    for (url, options) in links
        .scripts
        .iter()
        .filter(|_| scripting_enabled)
        .map(|preload| {
            (
                &preload.url,
                ResponsePreloadOptions::Script(preload.metadata.clone()),
            )
        })
        .chain(links.styles.iter().map(|preload| {
            (
                &preload.url,
                ResponsePreloadOptions::Style(preload.options.clone()),
            )
        }))
    {
        if !host.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) {
            break;
        }
        if let Err(error) = start_resource_preload(scope, host, target, url, options) {
            tracing::debug!(%url, error, "response preload was not loaded");
        }
    }
    if !scripting_enabled {
        return;
    }
    if let (
        crate::native_bridge::OwnerDispatchScope::Child(handle),
        crate::native_bridge::WindowDocumentOwner::Frame(owner),
    ) = (target.dispatch_scope(), target.owner())
    {
        for preload in &links.modules {
            if !host.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            ) {
                break;
            }
            let Some(loader) = host.document_resource_loader_for_window_owner(target.owner())
            else {
                break;
            };
            let Some(environment) =
                host.subresource_request_environment(&loader, target.dispatch_scope())
            else {
                break;
            };
            if let Some(candidate) = preload.fetch_candidate(
                &environment.document_url,
                host.document_response_preload_referrer_policy(target.dispatch_scope()),
            ) {
                host.queue_child_response_modulepreload(handle, owner, candidate.request);
            }
        }
    }
}

pub(crate) fn apply_response_preload_request_metadata(
    request: Request,
    preload: &PendingResponsePreload,
) -> Request {
    let request = match &preload.options {
        ResponsePreloadOptions::Script(metadata) => request
            .with_browser_request_metadata(BrowserRequestMetadata::Script)
            .with_resource_type(RequestResourceType::Script)
            .with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
                cross_origin: metadata.cross_origin.clone(),
                referrer_policy: metadata.referrer_policy.clone(),
                document_referrer_policy: preload.document_referrer_policy.clone(),
                charset: None,
                integrity: metadata.integrity.clone(),
                nonce: metadata.nonce.clone(),
                fetch_priority: metadata.fetch_priority,
                scheduler_priority: None,
            }),
        ResponsePreloadOptions::Style(options) => {
            let options = options
                .clone()
                .with_document_referrer_policy(preload.document_referrer_policy.clone());
            crate::stylesheet_blocking::apply_stylesheet_request_parameters(
                request.with_resource_type(RequestResourceType::CssStyleSheet),
                &options,
            )
            .with_fetch_priority_hint(moli_fetch::FetchPriorityHint::from_attribute(
                options.fetch_priority(),
            ))
        }
    };
    request.with_link_preload()
}

fn start_resource_preload(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    target: WindowDocumentTaskTarget,
    url: &Url,
    options: ResponsePreloadOptions,
) -> Result<(), String> {
    let timing = crate::context_bootstrap::FetchResourceTiming::for_initiator_type("link");
    let owner = target.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_window_owner(target.owner())
        .ok_or("preload Document resource authority is unavailable")?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or("preload request environment is unavailable")?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
    let policy = host
        .document_connect_policy_snapshot_for_owner(owner)
        .ok_or("preload policy owner is unavailable")?;
    let (policy, request_mode, credentials_mode, resource_type, destination) = match &options {
        ResponsePreloadOptions::Script(metadata) => {
            let mode = if metadata.cross_origin.is_some() {
                RequestMode::Cors
            } else {
                RequestMode::NoCors
            };
            let credentials = if metadata
                .cross_origin
                .as_deref()
                .is_some_and(|value| !value.eq_ignore_ascii_case("use-credentials"))
            {
                RequestCredentialsMode::SameOrigin
            } else {
                RequestCredentialsMode::Include
            };
            (
                policy.for_script_preload(metadata.clone()),
                mode,
                credentials,
                SubresourceResourceType::Script,
                ServiceWorkerRequestDestination::Script,
            )
        }
        ResponsePreloadOptions::Style(options) => {
            let (mode, credentials) = options.request_mode_and_credentials();
            (
                policy.for_style_preload(options.nonce().map(str::to_owned)),
                mode,
                credentials,
                SubresourceResourceType::Stylesheet,
                ServiceWorkerRequestDestination::Style,
            )
        }
    };
    let report_context = capture_window_csp_report_request_context(scope, host, owner)
        .ok_or("preload reporting owner is unavailable")?;
    let (reports, enforced) = policy
        .check_url(
            &document_url,
            url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .into_violations();
    let blocked = !enforced.is_empty();
    let host_ptr = host as *mut JsContextHost;
    for violation in reports.into_iter().chain(enforced) {
        send_content_security_policy_violation_report_from_window_context(
            host,
            &report_context,
            &violation,
        );
        host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(scope, host_ptr, report_context.identity(), &violation);
    }
    if blocked {
        host.record_document_resource_timing(scope, target, timing.failure(url));
        return Err("preload blocked by Content Security Policy".into());
    }
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    let mut pending = PendingResponsePreload {
        options,
        document_referrer_policy: host.document_response_preload_referrer_policy(owner),
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
        producer: None,
        timing,
    };
    let mut request = apply_response_preload_request_metadata(
        Request::get_with_url(url.clone())
            .with_page_network_policy()
            .with_initiator_url(&document_url)
            .with_request_origin(request_origin.clone()),
        &pending,
    )
    .with_request_mode(request_mode)
    .with_credentials_mode(credentials_mode)
    .with_network_partition_key(network_partition_key.clone())
    .with_subframe_context(frame_id.is_some());
    let Some(producer) = resource_loader
        .request_client()
        .register_document_response_preload(&request)
    else {
        return Ok(());
    };
    pending.producer = Some(producer);
    let cancel_handle = FetchCancelHandle::new();
    let (internal_id, load) = host
        .record_async_response_preload(
            target,
            pending,
            cancel_handle.clone(),
            network_partition_key,
            policy_context,
            request_mode,
            credentials_mode,
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: None,
                frame_id: frame_id.clone(),
                document_url: document_url.clone(),
                url: url.clone(),
                websocket_socket_id: None,
                method: "GET".into(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type,
                request_cookie_report: None,
            },
        )
        .ok_or("preload Document was retired before request admission")?;
    let redirect_check = host.window_fetch_redirect_check(internal_id);
    if let Some(check) = redirect_check.clone() {
        request = request.with_redirect_check(check);
    }
    let loader = load.request_client();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id,
        request_origin,
        document_url: document_url.clone(),
        resource_type,
        policy_context,
    };
    if host
        .service_worker_controller_for_fetch(client_id, &document_url, url)
        .is_some()
    {
        let dispatch = ServiceWorkerFetchDispatch {
            redirect_check,
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                url.clone(),
                "GET".into(),
                Vec::new(),
                None,
                destination,
                request_mode,
                credentials_mode,
                RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context,
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: load.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url: url.clone(),
                    request_method: "GET".into(),
                    request_headers: Default::default(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker preload dispatch failed".to_owned()).into(),
                },
            );
        }
    } else if host.should_intercept_subresource(resource_type) {
        host.pause_pending_subresource_fetch(internal_id);
    } else {
        spawn_async_subresource_fetch(
            load.task_runner(),
            host.resource_completion_sender(),
            loader,
            request,
            Some(cancel_handle),
            Vec::new(),
            internal_id,
            network_context,
            url.clone(),
            "GET".into(),
            Default::default(),
            None,
        );
    }
    Ok(())
}
