use super::*;
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

#[derive(moli_webapi_declare::WebApiFunctionTemplate)]
#[webapi(interface = crate::web_api_interfaces::Window)]
struct SimpleWindowEventHandlerDeclaration {
    #[webapi(accessor_property = "handler", data = true,
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver)]
    handler: (),
}

/// Install explicit EventHandler accessors on object-backed Window surfaces.
/// Ordinary properties with an `on` prefix never become listeners implicitly.
pub(crate) fn install_simple_event_target_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    properties: &[&'static str],
) {
    use moli_webapi_declare::WebApiFunctionTemplateDeclaration;
    let (getter, setter) = if crate::context_bootstrap::is_window_receiver(scope, target) {
        (
            SimpleWindowEventHandlerDeclaration::accessor_getter_callback("handler")
                .expect("declared Window handler getter"),
            SimpleWindowEventHandlerDeclaration::accessor_setter_callback("handler")
                .expect("declared Window handler setter"),
        )
    } else {
        (
            v8::MapFnTo::<v8::FunctionCallback>::map_fn_to(
                simple_event_target_handler_getter_callback,
            ),
            v8::MapFnTo::<v8::FunctionCallback>::map_fn_to(
                simple_event_target_handler_setter_callback,
            ),
        )
    };
    for property in properties {
        let data = v8str(scope, property).into();
        let getter = v8::Function::builder_raw(getter)
            .data(data)
            .build(scope)
            .expect("event handler getter");
        let setter = v8::Function::builder_raw(setter)
            .data(data)
            .length(1)
            .build(scope)
            .expect("event handler setter");
        getter.set_name(
            crate::util::v8_string(scope, &format!("get {property}")).expect("handler getter name"),
        );
        setter.set_name(
            crate::util::v8_string(scope, &format!("set {property}")).expect("handler setter name"),
        );
        crate::definitions::define_get_set_property(
            scope,
            target,
            v8str(scope, property).into(),
            getter.into(),
            setter.into(),
            v8::PropertyAttribute::NONE,
            property,
        )
        .expect("event handler accessor");
    }
}
