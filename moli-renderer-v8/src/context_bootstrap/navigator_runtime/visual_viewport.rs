use super::super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_type_error,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const VISUAL_VIEWPORT_OFFSET_LEFT_SLOT: &str = "__moliVisualViewportOffsetLeft";
const VISUAL_VIEWPORT_OFFSET_TOP_SLOT: &str = "__moliVisualViewportOffsetTop";
const VISUAL_VIEWPORT_PAGE_LEFT_SLOT: &str = "__moliVisualViewportPageLeft";
const VISUAL_VIEWPORT_PAGE_TOP_SLOT: &str = "__moliVisualViewportPageTop";
const VISUAL_VIEWPORT_WIDTH_SLOT: &str = "__moliVisualViewportWidth";
const VISUAL_VIEWPORT_HEIGHT_SLOT: &str = "__moliVisualViewportHeight";
const VISUAL_VIEWPORT_SCALE_SLOT: &str = "__moliVisualViewportScale";

#[derive(WebApiObject)]
#[webapi(interface = "VisualViewport")]
struct VisualViewportObjectDeclaration {
    #[webapi(slot = VISUAL_VIEWPORT_OFFSET_LEFT_SLOT)]
    offset_left: f64,
    #[webapi(slot = VISUAL_VIEWPORT_OFFSET_TOP_SLOT)]
    offset_top: f64,
    #[webapi(slot = VISUAL_VIEWPORT_PAGE_LEFT_SLOT)]
    page_left: f64,
    #[webapi(slot = VISUAL_VIEWPORT_PAGE_TOP_SLOT)]
    page_top: f64,
    #[webapi(slot = VISUAL_VIEWPORT_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = VISUAL_VIEWPORT_HEIGHT_SLOT)]
    height: f64,
    #[webapi(slot = VISUAL_VIEWPORT_SCALE_SLOT)]
    scale: f64,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "VisualViewport")]
struct VisualViewportPrototypeDeclaration {
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 0), enumerable)]
    offset_left: (),
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 1), enumerable)]
    offset_top: (),
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 2), enumerable)]
    page_left: (),
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 3), enumerable)]
    page_top: (),
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 4), enumerable)]
    width: (),
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 5), enumerable)]
    height: (),
    #[webapi(accessor_property, getter = visual_viewport_attribute_getter_callback, data = callback_data_index_value(scope, 6), enumerable)]
    scale: (),
}

pub(in crate::context_bootstrap) fn install_visual_viewport_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "VisualViewport" {
        VisualViewportPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(in crate::context_bootstrap) fn build_window_visual_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    Ok(VisualViewportObjectDeclaration {
        offset_left: 0.0,
        offset_top: 0.0,
        page_left: 0.0,
        page_top: 0.0,
        width: super::super::window_accessors::window_inner_surface_width(scope, window),
        height: super::super::window_accessors::window_inner_surface_height(scope, window),
        scale: profile.visual_viewport_scale,
    }
    .bind(scope)?)
}

pub(crate) fn update_cached_window_visual_viewport_dimensions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    width: f64,
    height: f64,
) {
    let Some(viewport) = get_private_value(scope, window, WINDOW_VISUAL_VIEWPORT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let width = v8::Number::new(scope, width);
    set_private_value(scope, viewport, VISUAL_VIEWPORT_WIDTH_SLOT, width.into());
    let height = v8::Number::new(scope, height);
    set_private_value(scope, viewport, VISUAL_VIEWPORT_HEIGHT_SLOT, height.into());
}

fn visual_viewport_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !moli_webapi_declare::implements_interface(scope, args.this(), "VisualViewport") {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        VISUAL_VIEWPORT_ATTRIBUTE_SLOTS,
        "VisualViewport attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const VISUAL_VIEWPORT_ATTRIBUTE_SLOTS: &[&str] = &[
    VISUAL_VIEWPORT_OFFSET_LEFT_SLOT,
    VISUAL_VIEWPORT_OFFSET_TOP_SLOT,
    VISUAL_VIEWPORT_PAGE_LEFT_SLOT,
    VISUAL_VIEWPORT_PAGE_TOP_SLOT,
    VISUAL_VIEWPORT_WIDTH_SLOT,
    VISUAL_VIEWPORT_HEIGHT_SLOT,
    VISUAL_VIEWPORT_SCALE_SLOT,
];
