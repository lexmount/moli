use super::super::input::ParsedWindowFetchInput;
use super::super::*;
use anyhow::Context as _;

pub(super) struct PreparedWindowFetchRequest {
    pub(super) frame_id: Option<String>,
    pub(super) fetch_context: crate::native_bridge::WindowFetchContext,
    pub(super) resource_loader: crate::network::context::DocumentResourceLoader,
    pub(super) connect_policy: crate::document_runtime::DocumentConnectPolicySnapshot,
    pub(super) csp_report_context: crate::network_host::WindowCspReportRequestContext,
    pub(super) document_url: url::Url,
    pub(super) request_origin: moli_url::WebOrigin,
    pub(super) resource_timing: crate::context_bootstrap::FetchResourceTiming,
    pub(super) network_partition_key: Option<String>,
    pub(super) document_referrer_policy: Option<String>,
    pub(super) policy_context: crate::types::SubresourcePolicyContext,
    pub(super) resolved_url: url::Url,
    pub(super) method: String,
    pub(super) request_headers: moli_fetch::RequestHeaders,
    pub(super) cors_preflight_request_headers: Vec<(String, String)>,
    pub(super) body: Option<bytes::Bytes>,
    pub(super) request_mode: moli_fetch::RequestMode,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(super) redirect_mode: moli_fetch::RequestRedirectMode,
    pub(super) priority: Option<moli_fetch::FetchPriorityHint>,
    pub(super) cache: String,
    pub(super) referrer: String,
    pub(super) referrer_policy: String,
    pub(super) integrity: String,
    pub(super) keepalive: bool,
}

impl PreparedWindowFetchRequest {
    pub(super) fn cache_mode(&self) -> moli_fetch::RequestCacheMode {
        self.cache.parse().unwrap_or_default()
    }

    pub(super) fn request_scope(&self) -> crate::native_bridge::OwnerDispatchScope {
        self.fetch_context.request_target().dispatch_scope()
    }
}

pub(super) fn prepare_window_fetch_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: ParsedWindowFetchInput,
    fetch_context: crate::native_bridge::WindowFetchContext,
    host: &PageScriptHost,
) -> anyhow::Result<PreparedWindowFetchRequest> {
    // Combine duplicate values before preflight and context overrides while
    // preserving the first field spelling for native network serialization.
    let request_headers = combined_headers_entries(&parsed.headers);
    // Receiver capture and WebIDL conversion are complete before this pure
    // preparation stage. Never inspect `args.this()` here: doing so could bind
    // the operation to a replacement LocalWindow after an author getter
    // navigated the iframe.
    let request_scope = fetch_context.request_target().dispatch_scope();
    let document_target = host
        .current_window_document_task_target_for_dispatch_scope(request_scope)
        .context("fetch: Document execution context is unavailable")?;
    let resource_loader = host
        .document_resource_loader_for_window_owner(document_target.owner())
        .context("fetch: Document resource loader is unavailable")?;
    let environment = host
        .subresource_request_environment(&resource_loader, request_scope)
        .context("fetch: Window execution context owner is retired")?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        base_url,
        request_origin,
        frame_id,
    } = environment;
    let connect_policy = host
        .document_connect_policy_snapshot_for_owner(request_scope)
        .context("fetch: document policy context is unavailable")?;
    let csp_report_context =
        crate::network_host::capture_window_csp_report_request_context(scope, host, request_scope)
            .context("fetch: document report context is unavailable")?;
    let document_referrer_policy = effective_subresource_referrer_policy(host, request_scope);
    let policy_context = effective_subresource_policy_context(scope, host, request_scope);
    let network_partition_key = active_subresource_network_partition_key(host, request_scope);
    let cors_preflight_request_headers = request_headers.clone();
    let request_headers =
        merge_byte_string_request_headers(host.extra_http_headers(), &request_headers);
    let resolved_url = resolve_context_url(&base_url, parsed.url.as_str(), None)?;
    parsed.url.validate_credentials(&resolved_url)?;

    Ok(PreparedWindowFetchRequest {
        resource_timing: crate::context_bootstrap::FetchResourceTiming::capture(
            scope,
            &request_origin,
        ),
        frame_id,
        fetch_context,
        resource_loader,
        connect_policy,
        csp_report_context,
        document_url,
        request_origin,
        network_partition_key,
        document_referrer_policy,
        policy_context,
        resolved_url,
        method: parsed.method,
        request_headers,
        cors_preflight_request_headers,
        body: parsed.body,
        request_mode: parsed.request_mode,
        credentials_mode: parsed.credentials_mode,
        redirect_mode: parsed.redirect_mode,
        priority: parsed.priority,
        cache: parsed.cache,
        referrer: parsed.referrer,
        referrer_policy: parsed.referrer_policy,
        integrity: parsed.integrity,
        keepalive: parsed.keepalive,
    })
}
