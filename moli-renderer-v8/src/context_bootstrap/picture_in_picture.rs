//! Picture-in-Picture interface members. Opening a native window still requires
//! a presentation backend; these bindings do not manufacture a successful request.

use super::media_queries;
use crate::{
    util::{get_private_value, set_private_value, v8str},
    web_api_interfaces,
};
use moli_webapi_declare::WebApiFunctionTemplate;

const WIDTH: &str = "__moliPictureInPictureWindowWidth";
const HEIGHT: &str = "__moliPictureInPictureWindowHeight";
const ON_RESIZE: &str = "__moliPictureInPictureWindowOnresize";
const LISTENERS: &str = "__moliPictureInPictureWindowListeners";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PictureInPictureWindow, enumerable, receiver)]
struct PictureInPictureWindowPrototype {
    #[webapi(accessor_property, getter = dimension_getter, data = v8str(scope, WIDTH))]
    width: (),
    #[webapi(accessor_property, getter = dimension_getter, data = v8str(scope, HEIGHT))]
    height: (),
    #[webapi(accessor_property, getter = resize_handler_getter, setter = resize_handler_setter)]
    onresize: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "PictureInPictureWindow" {
        PictureInPictureWindowPrototype::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("PictureInPictureWindow receiver was validated")
}

fn dimension_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let window = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    // Closed windows have zero dimensions. No presentation backend currently
    // produces an opened PictureInPictureWindow.
    rv.set(
        get_private_value(scope, window, &slot)
            .unwrap_or_else(|| v8::Integer::new(scope, 0).into()),
    );
}

fn resize_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let window = target(scope, args.this());
    rv.set(get_private_value(scope, window, ON_RESIZE).unwrap_or_else(|| v8::null(scope).into()));
}

fn resize_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let window = target(scope, args.this());
    let active = args.get(0).is_object();
    let handler = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, window, ON_RESIZE, handler);
    media_queries::simple_object_event_set_ordered_handler(
        scope, window, LISTENERS, "resize", ON_RESIZE, active,
    );
}

/// A closed native window for binding tests, never a production PiP request.
#[cfg(test)]
pub(crate) fn closed_window_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    #[derive(Default, moli_webapi_declare::WebApiObject)]
    #[webapi(interface = web_api_interfaces::PictureInPictureWindow)]
    struct ClosedWindow {
        #[webapi(slot = super::shared::SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
        event_target: (),
        #[webapi(slot = super::shared::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
        ordered_handlers: (),
    }
    ClosedWindow::default()
        .bind(scope)
        .expect("closed native PiP window")
}
