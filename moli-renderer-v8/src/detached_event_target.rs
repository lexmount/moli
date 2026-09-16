use super::util::v8_string;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DetachedFocusEventInitDeclaration<'scope> {
    bubbles: bool,
    #[webapi(constructor_default = false)]
    cancelable: bool,
    #[webapi(constructor_default = true)]
    composed: bool,
    related_target: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DetachedSimpleEventInitDeclaration {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
}

fn build_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor =
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "FocusEvent")
            .ok()?;
    let init = DetachedFocusEventInitDeclaration::new(
        bubbles,
        related_target.unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .ok()?;
    let event_type = v8_string(scope, event_type)?;
    let value = constructor.new_instance(scope, &[event_type.into(), init.into()])?;
    Some(value)
}

fn build_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor =
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "Event").ok()?;
    let init = DetachedSimpleEventInitDeclaration::new(bubbles, cancelable, composed)
        .bind(scope)
        .ok()?;
    let event_type = v8_string(scope, event_type)?;
    let value = constructor.new_instance(scope, &[event_type.into(), init.into()])?;
    Some(value)
}

pub(super) fn dispatch_detached_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> bool {
    let Some(event) = build_simple_event(scope, event_type, bubbles, cancelable, composed) else {
        return true;
    };
    dispatch_node_event(scope, target, event)
}

pub(super) fn dispatch_detached_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
) -> bool {
    let Some(event) = build_focus_event(scope, event_type, related_target, bubbles) else {
        return true;
    };
    dispatch_node_event(scope, target, event)
}

fn dispatch_node_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, target)
    else {
        return true;
    };
    // Host dispatch must not enter JavaScript through the public dispatchEvent
    // method. Its extra execution boundary would defer listener cleanup until
    // after dispatch, and an author override could intercept a browser event.
    crate::native_bridge::element::dispatch_public_event(scope, runtime_ptr, handle, event)
        .allows_default()
}
