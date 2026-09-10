use super::*;
use crate::util::set_private_value;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "EventTarget", prototype = "Object", enumerable)]
struct SimpleEventTargetMethodsDeclaration<'scope> {
    #[webapi(method, length = 2, callback = simple_event_target_add_event_listener_callback)]
    add_event_listener: (),
    #[webapi(
        method,
        length = 2,
        callback = simple_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),
    #[webapi(method, length = 1, callback = simple_event_target_dispatch_event_callback)]
    dispatch_event: (),
    #[webapi(data_property = "onchange")]
    onchange: Option<v8::Local<'scope, v8::Value>>,
}

pub(crate) fn install_simple_event_target_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    with_onchange: bool,
) {
    mark_simple_event_target_slot(scope, target, slot_name);
    install_simple_event_target_methods_without_slot(scope, target, with_onchange);
}

pub(crate) fn mark_simple_event_target_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
) {
    set_private_value(
        scope,
        target,
        SIMPLE_EVENT_TARGET_SLOT,
        v8str(scope, slot_name).into(),
    );
}

fn install_simple_event_target_methods_without_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    with_onchange: bool,
) {
    SimpleEventTargetMethodsDeclaration::new(with_onchange.then(|| v8::null(scope).into()))
        .initialize(scope, target)
        .expect("simple event target declaration should initialize");
}

pub(crate) fn install_simple_event_target_ordered_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        target,
        SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}
