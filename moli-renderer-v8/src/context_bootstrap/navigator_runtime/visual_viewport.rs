use super::super::media_queries::initialize_simple_event_target;
use super::super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_type_error,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const VISUAL_VIEWPORT_OFFSET_LEFT_SLOT: &str = "__moliVisualViewportOffsetLeft";
const VISUAL_VIEWPORT_OFFSET_TOP_SLOT: &str = "__moliVisualViewportOffsetTop";
const VISUAL_VIEWPORT_PAGE_LEFT_SLOT: &str = "__moliVisualViewportPageLeft";
const VISUAL_VIEWPORT_PAGE_TOP_SLOT: &str = "__moliVisualViewportPageTop";
const VISUAL_VIEWPORT_WIDTH_SLOT: &str = "__moliVisualViewportWidth";
const VISUAL_VIEWPORT_HEIGHT_SLOT: &str = "__moliVisualViewportHeight";
const VISUAL_VIEWPORT_SCALE_SLOT: &str = "__moliVisualViewportScale";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VisualViewport)]
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
#[webapi(interface = web_api_interfaces::VisualViewport)]
struct VisualViewportPrototypeDeclaration {
    #[webapi(
        accessor_property = "onresize",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onresize"),
        setter_data = v8str(scope, "onresize"),
        enumerable,
        receiver = web_api_interfaces::VisualViewport::is_instance
    )]
    onresize: (),
    #[webapi(
        accessor_property = "onscroll",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onscroll"),
        setter_data = v8str(scope, "onscroll"),
        enumerable,
        receiver = web_api_interfaces::VisualViewport::is_instance
    )]
    onscroll: (),
    #[webapi(
        accessor_property = "onscrollend",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onscrollend"),
        setter_data = v8str(scope, "onscrollend"),
        enumerable,
        receiver = web_api_interfaces::VisualViewport::is_instance
    )]
    onscrollend: (),

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
    let viewport = VisualViewportObjectDeclaration {
        offset_left: 0.0,
        offset_top: 0.0,
        page_left: 0.0,
        page_top: 0.0,
        width: super::super::window_accessors::window_inner_surface_width(scope, window),
        height: super::super::window_accessors::window_inner_surface_height(scope, window),
        scale: profile.visual_viewport_scale,
    }
    .bind(scope)?;
    initialize_simple_event_target(scope, viewport);
    Ok(viewport)
}

pub(crate) fn dispatch_window_visual_viewport_resize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(viewport) = get_private_value(scope, window, WINDOW_VISUAL_VIEWPORT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Ok(event) = crate::host::create_host_event(
        scope,
        "resize",
        viewport.into(),
        viewport.into(),
        false,
        false,
    ) else {
        return false;
    };
    dispatch_simple_event_target_event(scope, viewport, "resize", event)
}

fn visual_viewport_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::VisualViewport::is_instance(scope, args.this()) {
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
    if matches!(
        slot,
        VISUAL_VIEWPORT_WIDTH_SLOT | VISUAL_VIEWPORT_HEIGHT_SLOT
    ) {
        // A borrowed getter belongs to its receiver's Window, even when the
        // function was obtained from another realm's VisualViewport prototype.
        let context = args
            .this()
            .get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let scope = &mut v8::ContextScope::new(scope, context);
        let window = context.global(scope);
        let current = super::super::window_accessors::window_host_ptr(scope, window).is_some_and(
            |host_ptr| {
                super::super::window_accessors::window_current_dispatch_scope(
                    scope,
                    window,
                    unsafe { &*host_ptr },
                )
                .is_some()
            },
        );
        if !current {
            rv.set_double(0.0);
            return;
        }
        let value = if slot == VISUAL_VIEWPORT_WIDTH_SLOT {
            super::super::window_accessors::window_inner_surface_width(scope, window)
        } else {
            super::super::window_accessors::window_inner_surface_height(scope, window)
        };
        rv.set_double(value);
        return;
    }
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
