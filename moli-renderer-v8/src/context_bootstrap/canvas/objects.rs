use super::*;

pub(crate) fn build_offscreen_canvas_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let ctor =
        v8::Local::<v8::Function>::try_from(global_constructor_object(scope, "OffscreenCanvas")?)
            .ok()?;
    let object = ctor.new_instance(
        scope,
        &[
            v8::Integer::new(scope, width as i32).into(),
            v8::Integer::new(scope, height as i32).into(),
        ],
    )?;
    Some(object)
}

pub(crate) fn build_canvas_rendering_context_2d_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    build_constructed_object(scope, "CanvasRenderingContext2D")
}

pub(super) fn build_offscreen_2d_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    build_constructed_object(scope, "OffscreenCanvasRenderingContext2D")
}

pub(crate) fn build_webgl_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    build_constructed_object(scope, "WebGLRenderingContext")
}

pub(crate) fn build_webgl2_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "WebGL2RenderingContext")?;
    let object = v8::Object::new(scope);
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    super::webgl::init_webgl2_context_object(scope, object);
    Some(object)
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

pub(super) fn build_webgl_debug_renderer_info_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(object) = build_constructed_object(scope, "WEBGL_debug_renderer_info") {
        return Some(object);
    }

    // Extension interfaces are not exposed as Worker globals in Chromium.
    // Construct the extension object directly when WebGL is running in a
    // worker realm, where no public constructor exists to instantiate.
    let object = v8::Object::new(scope);
    for (name, value) in [
        ("UNMASKED_VENDOR_WEBGL", 0x9245),
        ("UNMASKED_RENDERER_WEBGL", 0x9246),
    ] {
        let name = v8::String::new(scope, name)?;
        let defined = object.define_own_property(
            scope,
            name.into(),
            v8::Integer::new(scope, value).into(),
            v8::PropertyAttribute::READ_ONLY
                | v8::PropertyAttribute::DONT_DELETE
                | v8::PropertyAttribute::DONT_ENUM,
        );
        if defined != Some(true) {
            return None;
        }
    }
    Some(object)
}

pub(super) fn build_webgl_lose_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    build_constructed_object(scope, "WEBGL_lose_context")
}
