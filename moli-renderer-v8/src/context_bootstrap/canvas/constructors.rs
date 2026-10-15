pub(crate) use moli_webapi::canvas_context_values::{
    canvas_rendering_context_2d_constructor_callback,
    offscreen_canvas_rendering_context_2d_constructor_callback,
};

pub(crate) fn offscreen_canvas_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        crate::util::throw_type_error(
            scope,
            "Failed to construct 'OffscreenCanvas': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = crate::webidl::parse_args::<
        moli_webapi::canvas_context_values::OffscreenCanvasConstructorArgs,
    >(scope, &args) else {
        return;
    };
    super::offscreen::init_offscreen_canvas_object(scope, args.this(), parsed.width, parsed.height);
    rv.set(args.this().into());
}
