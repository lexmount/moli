//! CaretPosition snapshots of an already-resolved node and geometry.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const CARET_POSITION_OFFSET_NODE_SLOT: &str = "__moliCaretPositionOffsetNode";
const CARET_POSITION_OFFSET_SLOT: &str = "__moliCaretPositionOffset";
const CARET_POSITION_RECT_X_SLOT: &str = "__moliCaretPositionRectX";
const CARET_POSITION_RECT_Y_SLOT: &str = "__moliCaretPositionRectY";
const CARET_POSITION_RECT_WIDTH_SLOT: &str = "__moliCaretPositionRectWidth";
const CARET_POSITION_RECT_HEIGHT_SLOT: &str = "__moliCaretPositionRectHeight";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CaretPosition)]
struct CaretPositionDeclaration<'scope> {
    #[webapi(slot = CARET_POSITION_OFFSET_NODE_SLOT)]
    offset_node: v8::Local<'scope, v8::Object>,
    #[webapi(slot = CARET_POSITION_OFFSET_SLOT)]
    offset: f64,
    #[webapi(slot = CARET_POSITION_RECT_X_SLOT)]
    rect_x: f64,
    #[webapi(slot = CARET_POSITION_RECT_Y_SLOT)]
    rect_y: f64,
    #[webapi(slot = CARET_POSITION_RECT_WIDTH_SLOT)]
    rect_width: f64,
    #[webapi(slot = CARET_POSITION_RECT_HEIGHT_SLOT)]
    rect_height: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CaretPosition, enumerable)]
struct CaretPositionPrototypeDeclaration {
    #[webapi(accessor_property, getter = caret_position_offset_node_getter)]
    offset_node: (),
    #[webapi(accessor_property, getter = caret_position_offset_getter)]
    offset: (),
    #[webapi(method, length = 0, callback = caret_position_get_client_rect_callback, receiver = false)]
    get_client_rect: (),
}

pub fn new_caret_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    offset_node: v8::Local<'s, v8::Object>,
    offset: u32,
    rect: [f64; 4],
) -> Option<v8::Local<'s, v8::Object>> {
    let [x, y, width, height] = rect;
    CaretPositionDeclaration::new(offset_node, f64::from(offset), x, y, width, height)
        .bind(scope)
        .ok()
}

fn number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> f64 {
    get_private_value(scope, object, slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::NAN)
}

fn caret_position_offset_node_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), CARET_POSITION_OFFSET_NODE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn caret_position_offset_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), CARET_POSITION_OFFSET_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn caret_position_get_client_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let x = number_slot(scope, this, CARET_POSITION_RECT_X_SLOT);
    let y = number_slot(scope, this, CARET_POSITION_RECT_Y_SLOT);
    let width = number_slot(scope, this, CARET_POSITION_RECT_WIDTH_SLOT);
    let height = number_slot(scope, this, CARET_POSITION_RECT_HEIGHT_SLOT);
    let global = scope.get_current_context().global(scope);
    let Some(constructor) = global
        .get(scope, v8str(scope, "DOMRect").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let x = v8::Number::new(scope, x);
    let y = v8::Number::new(scope, y);
    let width = v8::Number::new(scope, width);
    let height = v8::Number::new(scope, height);
    if let Some(rect) =
        constructor.new_instance(scope, &[x.into(), y.into(), width.into(), height.into()])
    {
        rv.set(rect.into());
    } else {
        rv.set_null();
    }
}

pub fn install_caret_position_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "CaretPosition" {
        CaretPositionPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}
