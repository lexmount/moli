use crate::context_bootstrap::dom_rect::build_dom_rect_readonly_object;
use crate::util::{callback_data_index_value, callback_data_item, get_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const TARGET_SLOT: &str = "__moliResizeObserverEntryTarget";
const CONTENT_RECT_SLOT: &str = "__moliResizeObserverEntryContentRect";
const CONTENT_BOX_SIZE_SLOT: &str = "__moliResizeObserverEntryContentBoxSize";
const BORDER_BOX_SIZE_SLOT: &str = "__moliResizeObserverEntryBorderBoxSize";
const DEVICE_PIXEL_CONTENT_BOX_SIZE_SLOT: &str =
    "__moliResizeObserverEntryDevicePixelContentBoxSize";
const INLINE_SIZE_SLOT: &str = "__moliResizeObserverSizeInlineSize";
const BLOCK_SIZE_SLOT: &str = "__moliResizeObserverSizeBlockSize";

const ATTRIBUTE_SLOTS: &[&str] = &[
    TARGET_SLOT,
    CONTENT_RECT_SLOT,
    CONTENT_BOX_SIZE_SLOT,
    BORDER_BOX_SIZE_SLOT,
    DEVICE_PIXEL_CONTENT_BOX_SIZE_SLOT,
    INLINE_SIZE_SLOT,
    BLOCK_SIZE_SLOT,
];

// Sampling retains native dimensions. JS wrappers are created only for active
// observations, in the callback's relevant realm when they are delivered.
pub(super) struct ResizeObserverEntryData<'s> {
    pub(super) target: v8::Local<'s, v8::Value>,
    pub(super) content_rect: (f64, f64, f64, f64),
    pub(super) content_box_size: (f64, f64),
    pub(super) border_box_size: (f64, f64),
    pub(super) device_pixel_content_box_size: (f64, f64),
}

impl<'s> ResizeObserverEntryData<'s> {
    pub(super) fn into_object(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        let (x, y, width, height) = self.content_rect;
        let content_rect = build_dom_rect_readonly_object(scope, x, y, width, height);
        let content_box_size = frozen_box_size(scope, self.content_box_size);
        let border_box_size = frozen_box_size(scope, self.border_box_size);
        let device_pixel_content_box_size =
            frozen_box_size(scope, self.device_pixel_content_box_size);
        ResizeObserverEntryDeclaration {
            target: self.target,
            content_rect,
            content_box_size,
            border_box_size,
            device_pixel_content_box_size,
        }
        .bind(scope)
        .expect("ResizeObserverEntry declaration should bind")
    }
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ResizeObserverEntry)]
struct ResizeObserverEntryDeclaration<'s> {
    #[webapi(slot = TARGET_SLOT)]
    target: v8::Local<'s, v8::Value>,
    #[webapi(slot = CONTENT_RECT_SLOT)]
    content_rect: v8::Local<'s, v8::Object>,
    #[webapi(slot = CONTENT_BOX_SIZE_SLOT)]
    content_box_size: v8::Local<'s, v8::Array>,
    #[webapi(slot = BORDER_BOX_SIZE_SLOT)]
    border_box_size: v8::Local<'s, v8::Array>,
    #[webapi(slot = DEVICE_PIXEL_CONTENT_BOX_SIZE_SLOT)]
    device_pixel_content_box_size: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ResizeObserverSize)]
struct ResizeObserverSizeDeclaration {
    #[webapi(slot = INLINE_SIZE_SLOT)]
    inline_size: f64,
    #[webapi(slot = BLOCK_SIZE_SLOT)]
    block_size: f64,
}

fn frozen_box_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    (inline_size, block_size): (f64, f64),
) -> v8::Local<'s, v8::Array> {
    let size = ResizeObserverSizeDeclaration::new(inline_size, block_size)
        .bind(scope)
        .expect("ResizeObserverSize declaration should bind");
    let array = v8::Array::new_with_elements(scope, &[size.into()]);
    assert_eq!(
        array.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true),
        "new ResizeObserver box-size array should freeze"
    );
    array
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ResizeObserverEntry, enumerable, receiver)]
struct ResizeObserverEntryPrototypeDeclaration {
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 0))]
    target: (),
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 1))]
    content_rect: (),
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 2))]
    content_box_size: (),
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 3))]
    border_box_size: (),
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 4))]
    device_pixel_content_box_size: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ResizeObserverSize, enumerable, receiver)]
struct ResizeObserverSizePrototypeDeclaration {
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 5))]
    inline_size: (),
    #[webapi(accessor_property, getter = result_attribute_getter, data = callback_data_index_value(scope, 6))]
    block_size: (),
}

pub(in crate::context_bootstrap) fn install_resize_observer_entry_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "ResizeObserverEntry" => {
            ResizeObserverEntryPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "ResizeObserverSize" => {
            ResizeObserverSizePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

fn result_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) =
        callback_data_item(scope, &args, ATTRIBUTE_SLOTS, "ResizeObserver result slots")
    else {
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot)
            .expect("branded ResizeObserver result should retain its attributes"),
    );
}
