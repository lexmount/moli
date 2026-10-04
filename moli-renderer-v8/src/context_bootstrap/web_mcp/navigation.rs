//! Carry invocation identity and origin across native navigation. V8 callbacks remain
//! in the source Document; the destination reports JSON-LD after parsing.
use super::devtools;
use super::state::FormInvocationState;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, WindowDocumentOwner};
use moli_page_types::{RendererWebMcpEvent, RendererWebMcpNavigation, RendererWebMcpResult};

use crate::frame_owner_model::FrameDocumentNavigationLoadBinding;

pub(crate) fn form_invocation(host: &JsContextHost, form: DomHandle) -> Option<u64> {
    host.native_bridge()
        .web_mcp
        .pending
        .iter()
        .find_map(|(id, pending)| {
            pending
                .form
                .as_ref()
                .is_some_and(|active| {
                    active.handle == form
                        && matches!(
                            active.state,
                            FormInvocationState::Ready
                                | FormInvocationState::Submitting
                                | FormInvocationState::Navigating
                        )
                })
                .then_some(*id)
        })
}

fn mark_navigating(host: &mut JsContextHost, id: u64) {
    host.native_bridge_mut()
        .web_mcp
        .pending
        .get_mut(&id)
        .expect("active form invocation")
        .form
        .as_mut()
        .expect("form invocation")
        .state = FormInvocationState::Navigating;
}

fn navigation_invocation(
    host: &JsContextHost,
    form: DomHandle,
) -> Option<RendererWebMcpNavigation> {
    let id = form_invocation(host, form)?;
    let pending = &host.native_bridge().web_mcp.pending[&id];
    if pending.frame_tree.is_some() {
        return None;
    }
    Some(RendererWebMcpNavigation {
        invocation_id: id,
        origin: host.native_bridge().web_mcp.documents[&pending.document]
            .origin
            .clone(),
    })
}

pub(crate) fn bind_root_navigation(host: &mut JsContextHost, form: DomHandle) {
    if let Some(id) = navigation_invocation(host, form)
        && host.bind_pending_web_mcp_navigation(id.clone())
    {
        mark_navigating(host, id.invocation_id);
    }
}

pub(crate) fn bind_child_navigation(host: &mut JsContextHost, form: DomHandle, child: DomHandle) {
    let Some(id) = navigation_invocation(host, form) else {
        return;
    };
    let Some(binding) = host.current_child_navigation_load(child) else {
        return;
    };
    host.native_bridge_mut()
        .web_mcp
        .child_navigations
        .insert(child, (binding, id.clone()));
    mark_navigating(host, id.invocation_id);
}

pub(crate) fn cancel_child_navigation(host: &mut JsContextHost, child: DomHandle) {
    if let Some((_, id)) = host
        .native_bridge_mut()
        .web_mcp
        .child_navigations
        .remove(&child)
    {
        fail_navigation(host, id);
    }
}

pub(crate) fn commit_child_navigation(
    host: &mut JsContextHost,
    child: DomHandle,
    binding: Option<FrameDocumentNavigationLoadBinding>,
    owner: WindowDocumentOwner,
) {
    if let Some((expected, id)) = host
        .native_bridge_mut()
        .web_mcp
        .child_navigations
        .remove(&child)
    {
        if binding == Some(expected) {
            // The owner store has committed, but the adapter's Document-handle
            // map still points at the retired Document at this boundary.
            let origin = host
                .child_document_security_origin(child)
                .unwrap_or_else(url::Origin::new_opaque);
            receive_navigation_from_origin(host, owner, id, origin);
        } else {
            fail_navigation(host, id);
        }
    }
}

pub(crate) fn receive_navigation(
    host: &mut JsContextHost,
    owner: WindowDocumentOwner,
    id: RendererWebMcpNavigation,
) {
    let origin = host.document_security_origin(host.document_handle());
    receive_navigation_from_origin(host, owner, id, origin);
}

fn receive_navigation_from_origin(
    host: &mut JsContextHost,
    owner: WindowDocumentOwner,
    id: RendererWebMcpNavigation,
    origin: url::Origin,
) {
    // Chromium reports a cross-origin commit immediately. Waiting for DCL
    // would leave the invocation hanging on a blocked destination resource.
    if origin != id.origin {
        devtools::emit(host, cross_origin_failure_event(id));
        return;
    }
    host.native_bridge_mut()
        .web_mcp
        .navigation_results
        .insert(owner, id);
}

fn cross_origin_failure_event(id: RendererWebMcpNavigation) -> RendererWebMcpEvent {
    RendererWebMcpEvent::ToolResponded {
        invocation_id: id.invocation_id,
        result: RendererWebMcpResult::Error {
            message: "Cannot return tool results after a cross-origin navigation".into(),
            exception: None,
        },
    }
}

pub(crate) fn failure_event(id: RendererWebMcpNavigation) -> RendererWebMcpEvent {
    RendererWebMcpEvent::ToolResponded {
        invocation_id: id.invocation_id,
        result: RendererWebMcpResult::navigation_failed(),
    }
}

pub(crate) fn fail_navigation(host: &JsContextHost, id: RendererWebMcpNavigation) {
    devtools::emit(host, failure_event(id));
}

pub(crate) fn complete_navigation(
    host: &mut JsContextHost,
    document: DomHandle,
    owner: WindowDocumentOwner,
) {
    let Some(id) = host
        .native_bridge_mut()
        .web_mcp
        .navigation_results
        .remove(&owner)
    else {
        return;
    };
    let origin = host
        .native_bridge()
        .web_mcp
        .documents
        .get(&document)
        .map(|entry| entry.origin.clone())
        .unwrap_or_else(|| host.document_security_origin(document));
    if origin != id.origin {
        devtools::emit(host, cross_origin_failure_event(id));
        return;
    }
    devtools::emit_in_tree(host, None, || RendererWebMcpEvent::ToolResponded {
        invocation_id: id.invocation_id,
        result: RendererWebMcpResult::Completed(moli_webmcp::navigation_result(
            host.dom_host(),
            document,
        )),
    });
}
