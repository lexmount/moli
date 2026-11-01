//! Cache reads apply the policies of the receiver's relevant settings object.
use super::*;
use crate::cross_origin_isolation::DocumentIsolationPolicy;
use crate::network_host::check_opaque_response_policy;
use crate::types::SubresourcePolicyContext;
use moli_url::WebOrigin;
use url::Url;

struct Environment {
    origin: WebOrigin,
    url: Url,
    policy: SubresourcePolicyContext,
}

fn environment(scope: &mut v8::PinScope<'_, '_>) -> Option<Environment> {
    if let Some(policy) = crate::worker::worker_subresource_policy_context(scope) {
        return Some(Environment {
            origin: crate::worker::worker_global_origin(scope)?,
            url: crate::worker::worker_current_script_url(scope)?,
            policy,
        });
    }
    let context = scope.get_current_context();
    let host_ptr = crate::util::context_host_ptr_from_context_slot(context)?;
    // The current callback owns this native host. Policy lookup does not
    // execute JS or grant the receiver a replacement Window's task authority.
    let host = unsafe { &*host_ptr };
    if let Some(identity) = host.window_execution_context_identity_for_v8_context(scope, context)
        && host.window_execution_context_identity_is_current(identity)
    {
        let owner = identity.dispatch_scope();
        let (document, policy) = match owner {
            crate::native_bridge::OwnerDispatchScope::Top => (
                host.document_handle(),
                SubresourcePolicyContext::from_document_policy(host.document_policy_container()),
            ),
            crate::native_bridge::OwnerDispatchScope::Child(handle) => (
                host.child_browsing_context_document_handle(handle)?,
                host.frame_owner_current_child_snapshot(handle)?
                    .settings
                    .subresource_policy_context,
            ),
            crate::native_bridge::OwnerDispatchScope::LightweightPopup(id) => (
                host.lightweight_popup_document_handle(id)?,
                SubresourcePolicyContext::from_document_policy(
                    host.lightweight_popup_policy_container(id)?,
                ),
            ),
        };
        return Some(Environment {
            origin: WebOrigin::from_serialized(&host.window_document_origin(owner)?),
            url: host.dom_host().document_url_for_handle(document)?.clone(),
            policy,
        });
    }
    // Cache's existing lifecycle guard rejects retired Window receivers before
    // policy evaluation; never borrow a replacement document's settings.
    None
}

fn report(
    scope: &mut v8::PinScope<'_, '_>,
    report_type: &str,
    owner: &Environment,
    blocked_url: &Url,
    disposition: &str,
) {
    let mut blocked_url = blocked_url.clone();
    let _ = blocked_url.set_username("");
    let _ = blocked_url.set_password(None);
    blocked_url.set_fragment(None);
    crate::context_bootstrap::notify_reporting_observers(
        scope,
        report_type,
        owner.url.as_str(),
        serde_json::json!({"blockedURL":blocked_url.as_str(),"type":"corp","disposition":disposition,"destination":""}),
    );
}

pub(super) fn validate(
    scope: &mut v8::PinScope<'_, '_>,
    response: &StorageBucketCachedResponse,
) -> Result<(), &'static str> {
    // Opaque-redirect and other filtered response types are not subjected to
    // the Cache match algorithm's opaque-response CORP check.
    if response.response_type != "opaque" {
        return Ok(());
    }
    let owner = environment(scope).ok_or("Cache response settings are unavailable.")?;
    let url = Url::parse(&response.url).map_err(|_| "Cache entry has no internal response URL.")?;
    let report_only = check_opaque_response_policy(
        &owner.origin,
        &url,
        &response.headers,
        response.request_include_credentials,
        owner.policy.cross_origin_embedder_policy_report_only,
        DocumentIsolationPolicy::None,
    );
    if report_only.is_err_and(|violation| violation.is_due_to_coep()) {
        report(scope, "coep", &owner, &url, "reporting");
    }
    match check_opaque_response_policy(
        &owner.origin,
        &url,
        &response.headers,
        response.request_include_credentials,
        owner.policy.cross_origin_embedder_policy,
        owner.policy.document_isolation_policy,
    ) {
        Ok(()) => Ok(()),
        Err(violation) => {
            if violation.is_due_to_coep() {
                report(scope, "coep", &owner, &url, "enforce");
            }
            if violation.is_due_to_dip() {
                report(scope, "dip", &owner, &url, "enforce");
            }
            Err("Cache response was blocked by Cross-Origin-Resource-Policy.")
        }
    }
}
