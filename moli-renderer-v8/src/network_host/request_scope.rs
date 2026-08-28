use super::*;
use crate::util::get_private_value;

pub(in crate::network_host) use crate::context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT as XHR_CHILD_CONTEXT_HANDLE_SLOT;

pub(in crate::network_host) fn observe_subresource_request_cookie_report(
    loader: &crate::network::ResourceRequestClient,
    document_url: &url::Url,
    request_url: &url::Url,
    method: &str,
    credentials_mode: moli_fetch::RequestCredentialsMode,
) -> Option<moli_cookie_jar::StoredCookieQueryReport> {
    let request = Request::new(method, request_url.as_str(), None, Vec::new())
        .ok()?
        .with_initiator_url(document_url)
        .with_credentials_mode(credentials_mode);
    if !request.allows_credentials_for_url(request_url) {
        return None;
    }
    let request_url = request.url.clone();
    let request_context = request.cookie_context;
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
    let _ = scope;
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
    }
}

fn child_browsing_context_handle_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    let object = local_object_in_scope(scope, object);
    get_private_value(scope, object, XHR_CHILD_CONTEXT_HANDLE_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| crate::document_runtime::DomHandle::new(value as usize))
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

pub(in crate::network_host) fn effective_subresource_request_scope(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    receiver: Option<v8::Local<'_, v8::Object>>,
) -> (
    Option<String>,
    url::Url,
    crate::native_bridge::OwnerDispatchScope,
) {
    let handle = receiver
        .and_then(|receiver| child_browsing_context_handle_from_object(scope, receiver))
        .or_else(|| {
            host.active_child_subresource_request_scope()
                .map(|(handle, _, _)| handle)
        })
        .or_else(|| {
            crate::context_bootstrap::current_child_browsing_context_handle_for_runtime_scope(scope)
        });
    if let Some(handle) = handle
        && let Some((frame_id, document_url)) = host.child_browsing_context_request_scope(handle)
    {
        return (
            Some(frame_id),
            document_url,
            crate::native_bridge::OwnerDispatchScope::Child(handle),
        );
    }
    (
        None,
        host.document_url().clone(),
        crate::native_bridge::OwnerDispatchScope::Top,
    )
}

pub(in crate::network_host) fn subresource_request_scope_for_owner(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<(Option<String>, url::Url)> {
    let _ = scope;
    match owner {
        crate::native_bridge::OwnerDispatchScope::Top => Some((None, host.document_url().clone())),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => host
            .child_browsing_context_request_scope(handle)
            .map(|(frame_id, document_url)| (Some(frame_id), document_url)),
    }
}

pub(in crate::network_host) fn effective_subresource_referrer_policy(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<String> {
    let _ = scope;
    match owner {
        crate::native_bridge::OwnerDispatchScope::Top => {
            host.response_referrer_policy().map(ToOwned::to_owned)
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_response_referrer_policy(handle)
        }
    }
}
