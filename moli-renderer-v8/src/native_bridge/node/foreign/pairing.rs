use super::super::*;
use super::FOREIGN_IDENTITY_LIVE_HANDLE_SLOT;
use super::js_values::{js_child_node_objects, js_node_type};
use crate::{
    dom::native::NodeType,
    dom_parser::{DOM_PARSER_FOREIGN_NODE_SLOT, map_live_value_to_foreign},
    native_bridge::document::{DETACHED_LIVE_DELEGATE_SLOT, canonical_detached_node_object},
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
    let foreign = v8::Local::new(scope, foreign);
    let foreign = canonical_detached_node_object(scope, foreign)?;
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
    let foreign = v8::Local::new(scope, foreign);
    let foreign = node_backing_object(scope, foreign)?;
    let live_wrapper = v8::Local::new(scope, live_wrapper);
    // Rebinding to the backing itself restores its own runtime/handle. Clear
    // any previous delegate rather than leaving it stale or creating a cycle.
    let delegate = if foreign == live_wrapper {
        v8::undefined(scope).into()
    } else {
        live_wrapper.into()
    };
    set_private_value(scope, foreign, DETACHED_LIVE_DELEGATE_SLOT, delegate);
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
    let foreign = v8::Local::new(scope, foreign);
    let Some(foreign) = node_backing_object(scope, foreign) else {
        return;
    };
    let value = v8::BigInt::new_from_u64(scope, live_handle.index() as u64);
    set_private_value(
        scope,
        foreign,
        FOREIGN_IDENTITY_LIVE_HANDLE_SLOT,
        value.into(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::pin;

    #[test]
    fn native_alias_delegate_updates_only_backing_and_clears_self_rebinding() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let backing = v8::Object::new(scope);
        crate::web_api_interfaces::Node::DESCRIPTOR
            .initialize(scope, backing)
            .unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let alias = v8::Proxy::new(scope, backing, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, alias).unwrap();
        let delegate = v8::Object::new(scope);

        set_detached_live_delegate(scope, alias.into(), delegate).unwrap();
        assert_eq!(
            crate::util::get_private_object(scope, backing, DETACHED_LIVE_DELEGATE_SLOT),
            Some(delegate)
        );
        assert!(
            crate::util::get_private_value(scope, alias.into(), DETACHED_LIVE_DELEGATE_SLOT)
                .is_none()
        );

        set_detached_live_delegate(scope, alias.into(), backing).unwrap();
        assert!(
            crate::util::get_private_object(scope, backing, DETACHED_LIVE_DELEGATE_SLOT).is_none(),
            "returning to the original backing must neither keep the old delegate nor delegate to itself"
        );
        assert!(
            crate::util::get_private_value(scope, alias.into(), DETACHED_LIVE_DELEGATE_SLOT)
                .is_none()
        );
    }
}
