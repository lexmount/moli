//! DedicatedWorker startup checks run in the exact creator's task and realm.

use crate::cross_origin_isolation::CrossOriginEmbedderPolicy;
use crate::types::SubresourcePolicyContext;
use url::Url;

pub(crate) fn worker_creator_document_url(
    host: &crate::native_bridge::JsContextHost,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<Url> {
    use crate::native_bridge::OwnerDispatchScope;
    let document = match owner {
        OwnerDispatchScope::Top => Some(host.document_handle()),
        OwnerDispatchScope::Child(handle) => host.child_browsing_context_document_handle(handle),
        OwnerDispatchScope::LightweightPopup(id) => host.lightweight_popup_document_handle(id),
    }?;
    Some(host.document_url_for_handle(document))
}

pub(crate) fn check_dedicated_worker_embedder_policy(
    scope: &mut v8::PinScope<'_, '_>,
    creator: SubresourcePolicyContext,
    worker: CrossOriginEmbedderPolicy,
    owner_url: &Url,
    script_url: &Url,
) -> bool {
    // Either strict COEP value is compatible; neither actual COI capability
    // nor equality of the creator and worker's policy values is required.
    if worker.enables_cross_origin_isolation() {
        return true;
    }
    let report_only = creator
        .cross_origin_embedder_policy_report_only
        .enables_cross_origin_isolation();
    let blocked = creator
        .cross_origin_embedder_policy
        .enables_cross_origin_isolation();
    if report_only || blocked {
        let mut blocked_url = script_url.clone();
        let _ = blocked_url.set_username("");
        let _ = blocked_url.set_password(None);
        blocked_url.set_fragment(None);
        for disposition in [
            report_only.then_some("reporting"),
            blocked.then_some("enforce"),
        ]
        .into_iter()
        .flatten()
        {
            super::reporting_observer::queue_report(
                scope,
                "coep",
                owner_url.as_str(),
                serde_json::json!({
                    "type": "worker initialization",
                    "blockedURL": blocked_url.as_str(),
                    "disposition": disposition,
                }),
            );
        }
    }
    !blocked
}
