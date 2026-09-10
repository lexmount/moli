use super::helpers::init_canvas_like_context_object;
use super::offscreen::init_offscreen_canvas_object;
use super::*;
use crate::webidl;

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
