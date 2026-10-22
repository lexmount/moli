use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::{
    native_bridge::{JsContextHost, WindowExecutionContextIdentity},
    types::{AsyncSubresourceFetchCompletion, PendingFontFaceFetch},
};
use anyhow::{Context, Result, anyhow};

/// Freeze the constructor's relevant Window, independently of an enclosing
/// child request callback. The identity contains no strong V8 handles.
pub(crate) fn capture_font_face_request_context(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<WindowExecutionContextIdentity> {
    let host_ptr = crate::util::context_host_ptr_from_global_bridge(scope)?;
    let context = scope.get_current_context();
    unsafe { &*host_ptr }.window_execution_context_identity_for_v8_context(scope, context)
}

/// Resolve URL sources at construction, before a later base mutation or source
/// fallback. Invalid URLs remain unavailable sources, not CSS syntax errors;
/// source exhaustion is reported only when loading is requested.
pub(crate) fn resolve_font_face_sources(
    scope: &mut v8::PinScope<'_, '_>,
    request_context: Option<WindowExecutionContextIdentity>,
    sources: Vec<moli_css_parse::CssFontSource>,
) -> Vec<moli_css_parse::CssFontSource> {
    use moli_css_parse::CssFontSource;

    let base = crate::worker::worker_current_script_url(scope).or_else(|| {
        crate::util::context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
            let host = unsafe { &*host_ptr };
            let identity = request_context
                .filter(|identity| host.window_execution_context_identity_is_current(*identity))?;
            subresource_api_base_url(scope, host, identity.dispatch_scope())
        })
    });
    sources
        .into_iter()
        .filter_map(|source| match source {
            CssFontSource::Local(_) => Some(source),
            CssFontSource::Url(url) => base
                .as_ref()
                .map_or_else(|| url::Url::parse(&url), |base| base.join(&url))
                .ok()
                .map(|url| CssFontSource::Url(url.into())),
        })
        .collect()
}

/// Explicit CSS Font Loading requests share document ownership, CORS and the
/// renderer network queue with other resources. Never call page-defined fetch.
pub(crate) fn start_font_face_fetch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    request_context: Option<WindowExecutionContextIdentity>,
    source: &str,
) -> Result<()> {
    if crate::worker::get_worker_state(scope).is_some() {
        let url =
            url::Url::parse(source).context("failed to resolve worker FontFace source URL")?;
        return crate::worker::start_worker_font_face_fetch(scope, face, url)
            .map_err(anyhow::Error::msg);
    }
    let host_ptr = crate::util::context_host_ptr_from_global_bridge(scope)
        .context("FontFace URL loader is unavailable in this realm")?;
    // SAFETY: the current live realm owns the bridge. No author callback runs
    // while the host is used except its existing CSP notification boundary.
    let host: &mut JsContextHost = unsafe { &mut *host_ptr };
    let identity = request_context
        .filter(|identity| host.window_execution_context_identity_is_current(*identity))
        .context("FontFace creation Window is no longer available")?;
    let owner = identity.dispatch_scope();
    let context = face
        .get_creation_context(scope)
        .context("FontFace creation realm is unavailable")?;
    let binding = crate::native_bridge::WindowExecutionContextBinding::new(
        identity.owner(),
        owner,
        identity.realm_token(),
        v8::Global::new(scope, context),
    );
    let resource_loader = host
        .document_resource_loader_for_dispatch_scope(owner)
        .context("Font request has no live document resource loader")?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .context("Font request has no live document request environment")?;
    let request_url = environment
        .base_url
        .join(source)
        .context("failed to resolve FontFace source URL")?;
    let policy = host
        .document_connect_policy_snapshot_for_owner(owner)
        .context("FontFace policy owner is unavailable")?
        .for_resource_kind(
            crate::content_security_policy::ContentSecurityPolicyResourceKind::DocumentFont,
        );
    let report_context = capture_window_csp_report_request_context(scope, host, owner)
        .context("FontFace reporting owner is unavailable")?;
    let (reports, enforced) = policy
        .check_url(
            &environment.document_url,
            &request_url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .into_violations();
    let blocked = !enforced.is_empty();
    for violation in reports.into_iter().chain(enforced) {
        send_content_security_policy_violation_report_from_window_context(
            host,
            &report_context,
            &violation,
        );
        host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
            scope, host_ptr, report_context.identity(), &violation,
        );
    }
    if blocked {
        return Err(anyhow!(
            "FontFace request blocked by Content Security Policy"
        ));
    }
    moli_fetch::FetchUrlList::new(&request_url, &[])
        .validate_request_mode(moli_fetch::RequestMode::Cors, &environment.request_origin)
        .map_err(anyhow::Error::msg)?;
    let font = PendingFontFaceFetch {
        face: v8::Global::new(scope, face),
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
    };
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url.clone(),
        SubresourceFetchOptions::FONT,
    )?;
    let internal_id = host
        .record_async_font_face_fetch(
            binding,
            &resource_loader,
            font,
            prepared.cancel_handle.clone(),
            prepared.network_partition_key.clone(),
            prepared.network_context.policy_context,
            prepared.pending_info.clone(),
        )
        .context("FontFace Document was retired")?;
    // Local URL responses use the same resource completion task as network
    // responses, preserving the observable loading state before settlement.
    let blob = CapturedBlobUrl::capture(&request_url);
    if let Some(response) =
        local_url_response_with_blob_entry(&request_url, "GET", &[], blob.as_ref())
    {
        let result = response
            .map(Into::into)
            .map_err(|error| error.into_message());
        let _ = host.resource_completion_sender().send_async_subresource(
            AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: result.into(),
            },
        );
        return Ok(());
    }
    prepared.dispatch(
        host,
        internal_id,
        "service worker font request dispatch failed",
    );
    Ok(())
}
