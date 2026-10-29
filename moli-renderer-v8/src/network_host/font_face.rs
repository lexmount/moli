use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::{
    native_bridge::{JsContextHost, WindowExecutionContextIdentity},
    types::{SubresourceRequestInitiatorType, SubresourceResourceType},
};
use anyhow::{Context, Result, anyhow, ensure};

pub(crate) enum FontFaceFetchStart {
    Local(Vec<u8>),
    Pending,
}

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

    let base = crate::util::context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        let host = unsafe { &*host_ptr };
        let identity = request_context
            .filter(|identity| host.window_execution_context_identity_is_current(*identity))?;
        subresource_api_base_url(scope, host, identity.dispatch_scope())
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
pub(crate) fn start_font_face_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    face: v8::Local<'_, v8::Object>,
    request_context: Option<WindowExecutionContextIdentity>,
    source: &str,
) -> Result<FontFaceFetchStart> {
    let host_ptr = crate::util::context_host_ptr_from_global_bridge(scope)
        .context("FontFace URL loader is unavailable in this realm")?;
    // SAFETY: the current live realm owns the bridge. No author callback runs
    // while the host is used except its existing CSP notification boundary.
    let host: &mut JsContextHost = unsafe { &mut *host_ptr };
    let identity = request_context
        .filter(|identity| host.window_execution_context_identity_is_current(*identity))
        .context("FontFace creation Window is no longer available")?;
    let owner = identity.dispatch_scope();
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
    ensure!(
        host.font_request_allowed_by_csp(
            scope,
            owner,
            &environment.document_url,
            &request_url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        ),
        "Font request blocked by Content Security Policy"
    );
    if let Some(response) = local_url_response(&request_url) {
        let response: crate::types::NavigationResponse = response.into();
        let result = if (200..300).contains(&response.status) {
            Ok(FontFaceFetchStart::Local(response.body_bytes().to_vec()))
        } else {
            Err(anyhow!("Font request failed"))
        };
        host.record_get_subresource_network_result_with_initiator(
            environment.frame_id,
            environment.document_url,
            request_url,
            SubresourceResourceType::Font,
            SubresourceRequestInitiatorType::Script,
            &Ok(response),
        );
        return result;
    }
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url,
        SubresourceFetchOptions::FONT,
    )?;
    let internal_id = host.record_async_font_face_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        v8::Global::new(scope, face),
        owner,
        prepared.cancel_handle.clone(),
        prepared.network_partition_key.clone(),
        prepared.network_context.policy_context,
        prepared.pending_info.clone(),
    );
    prepared.dispatch(
        host,
        internal_id,
        "service worker font request dispatch failed",
    );
    Ok(FontFaceFetchStart::Pending)
}
