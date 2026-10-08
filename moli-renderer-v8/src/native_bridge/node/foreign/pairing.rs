use super::super::*;
use super::FOREIGN_IDENTITY_LIVE_HANDLE_SLOT;
use super::js_values::{js_child_node_objects, js_node_type};
use crate::{
    dom::native::NodeType,
    dom_parser::{DOM_PARSER_FOREIGN_NODE_SLOT, map_live_value_to_foreign},
    native_bridge::document::DETACHED_LIVE_DELEGATE_SLOT,
    util::set_private_value,
};

pub(super) fn pair_foreign_node_with_live_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    foreign: v8::Local<'_, v8::Object>,
    live_handle: DomHandle,
) -> Option<()> {
    pair_foreign_node_with_live_handle_inner(scope, runtime_ptr, foreign, live_handle, true)
}

pub(super) fn pair_foreign_node_with_live_handle_for_identity(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    foreign: v8::Local<'_, v8::Object>,
    live_handle: DomHandle,
) -> Option<()> {
    pair_foreign_node_with_live_handle_inner(scope, runtime_ptr, foreign, live_handle, false)
}

fn pair_foreign_node_with_live_handle_inner(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    foreign: v8::Local<'_, v8::Object>,
    live_handle: DomHandle,
    attach_live_delegate: bool,
) -> Option<()> {
    if attach_live_delegate
        && let Ok((source_ptr, source_handle)) =
            super::super::node_runtime_and_handle_from_object(scope, foreign)
        && source_ptr != runtime_ptr
        && let Some(environment) = unsafe { &*runtime_ptr }.page_script_environment()
        && let Some(source) = environment
            .related_document_hosts()
            .into_iter()
            .find(|source| source.as_ptr() == source_ptr)
    {
        unsafe { &mut *runtime_ptr }.adopt_node_event_target_from(
            scope,
            &source,
            source_handle,
            live_handle,
        );
    }
    let live_wrapper = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, live_handle)?
    };
    let foreign_children = js_child_node_objects(scope, foreign);
    if attach_live_delegate {
        set_detached_live_delegate(scope, foreign, live_wrapper)?;
    }
    if foreign_node_type_matches_live_node(scope, runtime_ptr, foreign, live_handle) {
        set_live_wrapper_foreign_wrapper(scope, live_wrapper, foreign)?;
        if !attach_live_delegate {
            set_foreign_identity_live_handle(scope, foreign, live_handle);
        }
    }

    let live_children = {
        let runtime = unsafe { &*runtime_ptr };
        runtime
            .dom_host()
            .node(live_handle)
            .map(|node| node.child_ids(runtime.dom_host().dom()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for (foreign_child, live_child) in foreign_children.into_iter().zip(live_children) {
        pair_foreign_node_with_live_handle_inner(
            scope,
            runtime_ptr,
            foreign_child,
            live_child,
            attach_live_delegate,
        )?;
    }
    if attach_live_delegate {
        sync_foreign_node_core_from_live_delegate(scope, foreign, live_wrapper);
    }
    Some(())
}

fn foreign_node_type_matches_live_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    foreign: v8::Local<'_, v8::Object>,
    live_handle: DomHandle,
) -> bool {
    let Some(foreign_type) = js_node_type(scope, foreign) else {
        return false;
    };
    let Some(live_type) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(live_handle)
        .map(|node| node.node_type())
    else {
        return false;
    };
    matches!(
        (foreign_type, live_type),
        (1, NodeType::Element)
            | (3, NodeType::Text)
            | (4, NodeType::CDataSection)
            | (7, NodeType::ProcessingInstruction)
            | (8, NodeType::Comment)
            | (9, NodeType::Document)
            | (10, NodeType::DocumentType)
            | (11, NodeType::DocumentFragment)
    )
}

fn sync_foreign_node_core_from_live_delegate(
    scope: &mut v8::PinScope<'_, '_>,
    foreign: v8::Local<'_, v8::Object>,
    live_wrapper: v8::Local<'_, v8::Object>,
) {
    for key in [
        "ownerDocument",
        "parentNode",
        "parentElement",
        "firstChild",
        "lastChild",
        "previousSibling",
        "nextSibling",
        "isConnected",
    ] {
        if let Some(value) = live_wrapper.get(scope, v8str(scope, key).into()) {
            let value = map_live_value_to_foreign(scope, value);
            let _ = foreign.set(scope, v8str(scope, key).into(), value);
        }
    }
}

fn set_detached_live_delegate(
    scope: &mut v8::PinScope<'_, '_>,
    foreign: v8::Local<'_, v8::Object>,
    live_wrapper: v8::Local<'_, v8::Object>,
) -> Option<()> {
    set_private_value(
        scope,
        foreign,
        DETACHED_LIVE_DELEGATE_SLOT,
        live_wrapper.into(),
    );
    Some(())
}

fn set_live_wrapper_foreign_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    live_wrapper: v8::Local<'_, v8::Object>,
    foreign: v8::Local<'_, v8::Object>,
) -> Option<()> {
    set_private_value(
        scope,
        live_wrapper,
        DOM_PARSER_FOREIGN_NODE_SLOT,
        foreign.into(),
    );
    Some(())
}

fn set_foreign_identity_live_handle(
    scope: &mut v8::PinScope<'_, '_>,
    foreign: v8::Local<'_, v8::Object>,
    live_handle: DomHandle,
) {
    let value = v8::BigInt::new_from_u64(scope, live_handle.index() as u64);
    set_private_value(
        scope,
        foreign,
        FOREIGN_IDENTITY_LIVE_HANDLE_SLOT,
        value.into(),
    );
}
