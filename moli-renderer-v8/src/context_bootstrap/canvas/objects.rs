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
    let object = build_webgl_extension_object(
        scope,
        web_api_interfaces::WEBGLDebugRendererInfo::DESCRIPTOR,
    )?;
    super::constructors::WebGlDebugRendererInfoObjectDeclaration::new(0x9245 as f64, 0x9246 as f64)
        .initialize(scope, object)
        .ok()?;
    Some(object)
}

pub(super) fn build_webgl_lose_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    build_webgl_extension_object(scope, web_api_interfaces::WEBGLLoseContext::DESCRIPTOR)
}

fn build_webgl_extension_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface: moli_webapi_declare::WebApiInterfaceDescriptor,
) -> Option<v8::Local<'s, v8::Object>> {
    // Workers do not expose these constructors. Use the realm's intrinsic
    // prototype, also avoiding calls to page-replaced global constructors.
    let prototype =
        crate::context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_prototype(
            scope,
            interface.name(),
        )
        .ok()?;
    let object = v8::Object::new(scope);
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    interface.initialize(scope, object).ok()?;
    Some(object)
}
