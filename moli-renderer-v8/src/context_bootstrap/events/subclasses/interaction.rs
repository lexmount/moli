use super::super::event_attribute;
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::web_api_interfaces;
use moli_webapi_declare::web_api_object_target;

fn native_node_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if !web_api_interfaces::Node::is_instance(scope, object) {
        return None;
    }
    let target = web_api_object_target(scope, object)?;
    let context = target.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, target).ok()
}

fn retargeted_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let source = event_attribute(scope, event, "source")?;
    let Some((source_host, source_node)) = native_node_identity(scope, source) else {
        return Some(source);
    };
    let current_target = event_attribute(scope, event, "currentTarget")?;
    let against = native_node_identity(scope, current_target).and_then(|(host, node)| {
        // Child realms can have different hosts over the same DOM arena.
        std::ptr::eq(
            unsafe { &*host }.dom_host(),
            unsafe { &*source_host }.dom_host(),
        )
        .then_some(node)
    });
    let retargeted = unsafe { &*source_host }
        .dom_host()
        .retarget(source_node, against);
    if retargeted == source_node {
        return Some(source);
    }
    let source_object = v8::Local::<v8::Object>::try_from(source).ok()?;
    unsafe { &mut *source_host }
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, source_host, source_object, retargeted)
        .map(Into::into)
}

pub(in crate::context_bootstrap) fn interaction_event_source_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Generated WebIDL bindings validate the receiver. Always retarget the
    // retained original source against native currentTarget at access time.
    let Some(value) = retargeted_source(scope, args.this()) else {
        return;
    };
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    if let Some(value) =
        crate::context_bootstrap::platform_object_worlds::in_realm(scope, value, context)
    {
        rv.set(value);
    }
}
