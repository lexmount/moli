use super::util::{call_script_visible_function, v8_string, v8str};
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
    let global = scope.get_current_context().global(scope);
    let constructor = global.get(scope, v8str(scope, "FocusEvent").into())?;
    let constructor = v8::Local::<v8::Function>::try_from(constructor).ok()?;
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
    let global = scope.get_current_context().global(scope);
    let constructor = global.get(scope, v8str(scope, "Event").into())?;
    let constructor = v8::Local::<v8::Function>::try_from(constructor).ok()?;
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
    let Some(dispatch_event) = target
        .get(scope, v8str(scope, "dispatchEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return true;
    };
    call_script_visible_function(
        scope,
        dispatch_event,
        target.into(),
        &[event.into()],
        "detached dispatchEvent",
    )
    .is_none_or(|value| value.boolean_value(scope))
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
    let Some(dispatch_event) = target
        .get(scope, v8str(scope, "dispatchEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return true;
    };
    call_script_visible_function(
        scope,
        dispatch_event,
        target.into(),
        &[event.into()],
        "detached dispatchEvent",
    )
    .is_none_or(|value| value.boolean_value(scope))
}
