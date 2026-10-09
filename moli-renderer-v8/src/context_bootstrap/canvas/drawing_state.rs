//! V8-traced snapshots of native drawing attributes, excluding paths and pixels.

use super::{
    backing_store::{canvas_owner_from_context, reset_canvas_like_backing_store},
    helpers::CANVAS_DRAWING_SLOTS,
    state::canvas_path_state,
};
use crate::{
    util::{get_private_object, get_private_value, new_null_prototype_object, set_private_value},
    web_api_interfaces,
};
use moli_layout::LayoutTransform2D;
use moli_webapi_declare::{WebApiFunctionTemplate, web_api_object_target};

const SAVED_STATE_SLOT: &str = "__moliCanvasSavedDrawingState";
const SAVED_TRANSFORM_SLOT: &str = "__moliCanvasSavedTransform";

macro_rules! canvas_drawing_state_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method, length = 0, callback = save_callback)]
            save: (),
            #[webapi(method, length = 0, callback = restore_callback)]
            restore: (),
            #[webapi(method, length = 0, callback = reset_callback)]
            reset: (),
        }
    };
}

canvas_drawing_state_declaration!(
    CanvasDrawingStateDeclaration,
    web_api_interfaces::CanvasRenderingContext2D
);
canvas_drawing_state_declaration!(
    OffscreenCanvasDrawingStateDeclaration,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_drawing_state_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "CanvasRenderingContext2D" => {
            CanvasDrawingStateDeclaration::initialize_prototype_template(scope, prototype);
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenCanvasDrawingStateDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => unreachable!("only 2D contexts have drawing state bindings"),
    }
}

pub(super) fn clear_canvas_drawing_stack<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, context, SAVED_STATE_SLOT, v8::null(scope).into());
}

fn copy_drawing_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    destination: v8::Local<'s, v8::Object>,
) {
    for slot in CANVAS_DRAWING_SLOTS {
        let value = get_private_value(scope, source, slot)
            .expect("native drawing attributes are initialized together");
        // The private dash array is immutable: setLineDash replaces it and
        // getLineDash returns a copy. Retaining it cannot alias author data.
        set_private_value(scope, destination, slot, value);
    }
}

fn save_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated 2D context receiver");
    let saved = new_null_prototype_object(scope);
    copy_drawing_slots(scope, context, saved);
    let transform = canvas_path_state(scope, context).borrow().transform();
    let values = transform
        .coefficients
        .map(|value| v8::Number::new(scope, value).into());
    let transform = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, saved, SAVED_TRANSFORM_SLOT, transform.into());
    let previous = get_private_value(scope, context, SAVED_STATE_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, saved, SAVED_STATE_SLOT, previous);
    // Publish only after the snapshot is complete. V8 traces the linked stack;
    // no Rust Global handle can keep an otherwise unreachable context alive.
    set_private_value(scope, context, SAVED_STATE_SLOT, saved.into());
}

fn restore_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated 2D context receiver");
    let Some(saved) = get_private_object(scope, context, SAVED_STATE_SLOT) else {
        return;
    };
    let transform = get_private_value(scope, saved, SAVED_TRANSFORM_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("saved drawing state contains its transform");
    let coefficients = std::array::from_fn(|index| {
        transform
            .get_index(scope, index as u32)
            .and_then(|value| value.number_value(scope))
            .expect("saved transform contains six own numeric values")
    });
    copy_drawing_slots(scope, saved, context);
    canvas_path_state(scope, context)
        .borrow_mut()
        .restore_transform(LayoutTransform2D::new(coefficients));
    let previous = get_private_value(scope, saved, SAVED_STATE_SLOT)
        .expect("saved drawing state contains its predecessor");
    set_private_value(scope, context, SAVED_STATE_SLOT, previous);
}

fn reset_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated 2D context receiver");
    if let Some(canvas) = canvas_owner_from_context(scope, context) {
        reset_canvas_like_backing_store(scope, canvas);
    } else {
        // Existing native construction can precede attachment to a canvas.
        // Such a context still owns drawing state, but has no bitmap to clear.
        super::context2d::reset_canvas_context_state(scope, context);
    }
}
