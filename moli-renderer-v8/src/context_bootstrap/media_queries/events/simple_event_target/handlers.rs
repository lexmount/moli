use super::*;
use crate::definitions::define_function_accessor_property;
use crate::util::{get_private_value, throw_type_error, v8str};

// These callbacks are shared by declared EventHandler accessors. Their callback
// data is the property name, which also identifies the private handler slot.
pub(crate) fn simple_event_target_handler_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((target, property)) = handler_callback_data(scope, &args) else {
        return;
    };
    rv.set(
        get_private_value(scope, target.identity.handler_owner, &property)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn simple_event_target_handler_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((target, property)) = handler_callback_data(scope, &args) else {
        return;
    };
    simple_object_event_set_handler_in_registry(
        scope,
        target.identity.handler_owner,
        property.strip_prefix("on").expect("event handler property"),
        &property,
        args.get(0),
        target.registry,
    );
}

fn handler_callback_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(ResolvedSimpleEventTarget<'s>, String)> {
    let Some(target) = resolve_simple_event_target(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return None;
    };
    v8::Local::<v8::String>::try_from(args.data())
        .ok()
        .map(|property| (target, property.to_rust_string_lossy(scope)))
}

/// Install explicit EventHandler accessors on object-backed Window surfaces.
/// Ordinary properties with an `on` prefix never become listeners implicitly.
pub(crate) fn install_simple_event_target_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    properties: &[&'static str],
) {
    for property in properties {
        let data = v8str(scope, property).into();
        define_function_accessor_property(
            scope,
            target,
            property,
            simple_event_target_handler_getter_callback,
            Some(data),
            simple_event_target_handler_setter_callback,
            Some(data),
            v8::PropertyAttribute::NONE,
        )
        .expect("simple event handler accessor should initialize");
    }
}
