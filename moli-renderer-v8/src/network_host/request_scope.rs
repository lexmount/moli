use super::*;

pub(in crate::network_host) use crate::context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT as XHR_CHILD_CONTEXT_HANDLE_SLOT;

pub(in crate::network_host) fn observe_subresource_request_cookie_report(
    loader: &crate::network::ResourceRequestClient,
    document_url: &url::Url,
    request_origin: &moli_url::WebOrigin,
    request_url: &url::Url,
    method: &str,
    credentials_mode: moli_fetch::RequestCredentialsMode,
) -> Option<moli_cookie_jar::StoredCookieQueryReport> {
    let mut request = Request::new_browser(
        method,
        request_url.clone(),
        None,
        Vec::new(),
        request_origin.clone(),
    )
    .with_initiator_url(document_url)
    .with_credentials_mode(credentials_mode);
    if let Some(context) = loader.browser_site_context() {
        request = request.with_browser_site_context(context.clone());
    }
    if !request.allows_credentials_for_url(request_url) {
        return None;
    }
    let request_url = request.url.clone();
    let request_context = request.network_cookie_context();
    observe_cookie_access_report_for_request(&loader.cookie_store(), &request_url, request_context)
        .ok()
        .flatten()
}

pub(in crate::network_host) fn active_subresource_network_partition_key(
    host: &JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<String> {
    owner
        .child_window()
        .and_then(|handle| host.child_browsing_context_network_partition_key(handle))
}

pub(crate) fn effective_subresource_policy_context(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> crate::types::SubresourcePolicyContext {
    match owner {
        crate::native_bridge::OwnerDispatchScope::Top => crate::types::SubresourcePolicyContext {
            cross_origin_embedder_policy: host.cross_origin_embedder_policy(),
            document_isolation_policy: host.document_isolation_policy(),
            cross_origin_isolated: host.cross_origin_isolated(),
        },
        crate::native_bridge::OwnerDispatchScope::Child(handle) => host
            .frame_owner_current_child_snapshot(handle)
            .map(|snapshot| snapshot.settings.subresource_policy_context)
            .unwrap_or_default(),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            let _ = scope;
            crate::types::SubresourcePolicyContext {
                cross_origin_embedder_policy: host
                    .lightweight_popup_cross_origin_embedder_policy(popup_id),
                document_isolation_policy: host
                    .lightweight_popup_document_isolation_policy(popup_id),
                cross_origin_isolated: host.lightweight_popup_cross_origin_isolated(popup_id),
            }
        }
    }
}

/// Select request attribution without using a document's mutable base URL.
pub(in crate::network_host) fn effective_subresource_request_owner(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
) -> crate::native_bridge::OwnerDispatchScope {
    let handle = host.active_child_subresource_request_handle().or_else(|| {
        crate::context_bootstrap::current_child_browsing_context_handle_for_runtime_scope(scope)
    });
    if let Some(handle) = handle
        && host.frame_owner_frame_id_for_child_handle(handle).is_some()
    {
        return crate::native_bridge::OwnerDispatchScope::Child(handle);
    }
    if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope)
        && host.lightweight_popup_document_url(popup_id).is_some()
    {
        return crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id);
    }
    crate::native_bridge::OwnerDispatchScope::Top
}

/// The API base URL resolves relative input; it is neither the document URL
/// nor the origin used for request security checks. Read it live for every call.
pub(crate) fn subresource_api_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<url::Url> {
    match owner {
        crate::native_bridge::OwnerDispatchScope::Top => {
            Some(host.document_base_url_for_handle(host.document_handle()))
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_base_url(handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.lightweight_popup_request_base_url(scope, popup_id)
        }
    }
}

pub(in crate::network_host) fn effective_subresource_referrer_policy(
    host: &JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<String> {
    let document = match owner {
        crate::native_bridge::OwnerDispatchScope::Top => Some(host.document_handle()),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_document_handle(handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.lightweight_popup_document_handle(popup_id)
        }
    }?;
    crate::context_bootstrap::document_referrer_policy_for_native_document(host, document)
}
