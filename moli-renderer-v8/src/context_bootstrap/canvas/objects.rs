use super::*;

pub(crate) fn build_canvas_rendering_context_2d_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    settings: Canvas2dSettings,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = build_constructed_object(scope, "CanvasRenderingContext2D")?;
    settings.initialize(scope, context)?;
    Some(context)
}

pub(super) fn build_offscreen_2d_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    settings: Canvas2dSettings,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = build_constructed_object(scope, "OffscreenCanvasRenderingContext2D")?;
    settings.initialize(scope, context)?;
    Some(context)
}

fn build_constructed_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let ctor =
        v8::Local::<v8::Function>::try_from(global_constructor_object(scope, constructor_name)?)
            .ok()?;
    let object = ctor.new_instance(scope, &[])?;
    Some(object)
}
