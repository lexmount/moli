use super::helpers::init_canvas_like_context_object;
use super::offscreen::init_offscreen_canvas_object;
use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WEBGLDebugRendererInfo)]
struct WebGlDebugRendererInfoObjectDeclaration {
    #[webapi(data_property = "UNMASKED_VENDOR_WEBGL")]
    unmasked_vendor_webgl: f64,
    #[webapi(data_property = "UNMASKED_RENDERER_WEBGL")]
    unmasked_renderer_webgl: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OffscreenCanvas")]
struct OffscreenCanvasConstructorArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    width: u32,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    height: u32,
}

pub(crate) fn offscreen_canvas_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'OffscreenCanvas': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<OffscreenCanvasConstructorArgs>(scope, &args) else {
        return;
    };
    init_offscreen_canvas_object(scope, args.this(), parsed.width, parsed.height);
    rv.set(args.this().into());
}

pub(crate) fn canvas_rendering_context_2d_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    init_canvas_like_context_object(scope, args.this());
    rv.set(args.this().into());
}

pub(crate) fn offscreen_canvas_rendering_context_2d_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    init_canvas_like_context_object(scope, args.this());
    rv.set(args.this().into());
}

pub(crate) fn webgl_rendering_context_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::webgl::init_webgl_context_object(scope, args.this());
    rv.set(args.this().into());
}

pub(crate) fn webgl_debug_renderer_info_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    WebGlDebugRendererInfoObjectDeclaration::new(0x9245 as f64, 0x9246 as f64)
        .initialize(scope, args.this())
        .expect("WEBGL_debug_renderer_info declaration should initialize object");
    rv.set(args.this().into());
}

pub(crate) fn webgl_lose_context_constructor_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.this().into());
}
