mod interfaces;
use super::objects::{build_webgl_debug_renderer_info_object, build_webgl_lose_context_object};
use super::*;
use crate::web_api_interfaces;
use crate::{
    util::{callback_data_item, get_private_value, set_private_value, v8_string, v8str},
    webidl,
};
pub(super) use interfaces::install_value_template_bindings;
use moli_webapi_declare::{WebApiObject, WebApiValue};
use strum::IntoEnumIterator;

const WEBGL_VIEWPORT_SLOT: &str = "__moliWebGlViewport";
const WEBGL_ERROR_SLOT: &str = "__moliWebGlError";
const WEBGL_EXTENSIONS_SLOT: &str = "__moliWebGlExtensions";
const WEBGL_VIEWPORT: u32 = 0x0BA2;
const WEBGL_INVALID_ENUM: u32 = 0x0500;
const WEBGL_INVALID_VALUE: u32 = 0x0501;
const WEBGL_MAX_VIEWPORT_DIMS: [i32; 2] = [8192, 8192];
// This query-only compatibility profile has no physical GPU identity provider.
// Keep enabled debug queries usable without inventing a device or driver name.
const WEBGL_COMPAT_VENDOR: &str = "WebKit";
const WEBGL_COMPAT_RENDERER: &str = "WebKit WebGL";
const WEBGL2_DRAWING_BUFFER_COLOR_SPACE_SLOT: &str = "__moliWebGl2DrawingBufferColorSpace";
const WEBGL2_UNPACK_COLOR_SPACE_SLOT: &str = "__moliWebGl2UnpackColorSpace";
const WEBGL2_COLOR_SPACE_SLOTS: &[&str] = &[
    WEBGL2_DRAWING_BUFFER_COLOR_SPACE_SLOT,
    WEBGL2_UNPACK_COLOR_SPACE_SLOT,
];

// Listing and enabling extensions must use the same registry. Do not advertise
// GPU extensions just because their names are known: their API must exist too.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::EnumIter, strum::IntoStaticStr,
)]
#[strum(ascii_case_insensitive)]
enum WebGlExtension {
    #[strum(serialize = "WEBGL_debug_renderer_info")]
    DebugRendererInfo,
    #[strum(serialize = "WEBGL_lose_context")]
    LoseContext,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGLRenderingContext.getExtension")]
struct WebGlGetExtensionArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGLRenderingContext.getParameter")]
struct WebGlGetParameterArgs {
    #[webidl(required)]
    pname: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGLRenderingContext.getShaderPrecisionFormat")]
struct WebGlGetShaderPrecisionFormatArgs {
    #[webidl(required)]
    shader_type: u32,
    #[webidl(required)]
    precision_type: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGLRenderingContext.viewport")]
struct WebGlViewportArgs {
    #[webidl(required)]
    x: i32,
    #[webidl(required)]
    y: i32,
    #[webidl(required)]
    width: i32,
    #[webidl(required)]
    height: i32,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WebGlContextStateDeclaration<'s> {
    #[webapi(slot = WEBGL_VIEWPORT_SLOT)]
    viewport: v8::Local<'s, v8::Array>,
    #[webapi(slot = WEBGL_ERROR_SLOT)]
    errors: v8::Local<'s, v8::Set>,
    #[webapi(slot = WEBGL_EXTENSIONS_SLOT)]
    extensions: v8::Local<'s, v8::Map>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGL2RenderingContext.getExtension")]
struct WebGl2GetExtensionArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGL2RenderingContext.getParameter")]
struct WebGl2GetParameterArgs {
    #[webidl(required)]
    pname: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGL2RenderingContext.getInternalformatParameter")]
struct WebGl2GetInternalformatParameterArgs {
    #[webidl(required)]
    target: u32,
    #[webidl(required)]
    internalformat: u32,
    #[webidl(required)]
    pname: u32,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::WebGLBuffer,
    fallback_to_string_tag = "WebGLBuffer"
)]
struct WebGlBufferHandleDeclaration {}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::WebGLProgram,
    fallback_to_string_tag = "WebGLProgram"
)]
struct WebGlProgramHandleDeclaration {}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::WebGLShader,
    fallback_to_string_tag = "WebGLShader"
)]
struct WebGlShaderHandleDeclaration {}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::WebGLUniformLocation,
    fallback_to_string_tag = "WebGLUniformLocation"
)]
struct WebGlUniformLocationHandleDeclaration {}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::WebGLFramebuffer,
    fallback_to_string_tag = "WebGLFramebuffer"
)]
struct WebGlFramebufferHandleDeclaration {}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::WebGLRenderbuffer,
    fallback_to_string_tag = "WebGLRenderbuffer"
)]
struct WebGlRenderbufferHandleDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebGL2RenderingContext)]
struct WebGl2ContextObjectDeclaration {
    #[webapi(slot = WEBGL2_DRAWING_BUFFER_COLOR_SPACE_SLOT)]
    drawing_buffer_color_space: String,
    #[webapi(slot = WEBGL2_UNPACK_COLOR_SPACE_SLOT)]
    unpack_color_space: String,
}

pub(crate) const WEBGL_CONSTANTS: &[(&str, u32)] = &[
    ("VIEWPORT", WEBGL_VIEWPORT),
    ("NO_ERROR", 0),
    ("INVALID_ENUM", WEBGL_INVALID_ENUM),
    ("INVALID_VALUE", WEBGL_INVALID_VALUE),
    ("DEPTH_TEST", 0x0B71),
    ("LEQUAL", 0x0203),
    ("COLOR_BUFFER_BIT", 0x4000),
    ("DEPTH_BUFFER_BIT", 0x0100),
    ("ARRAY_BUFFER", 0x8892),
    ("STATIC_DRAW", 0x88E4),
    ("VERTEX_SHADER", 0x8B31),
    ("FRAGMENT_SHADER", 0x8B30),
    ("COMPILE_STATUS", 0x8B81),
    ("LINK_STATUS", 0x8B82),
    ("FLOAT", 0x1406),
    ("TRIANGLE_STRIP", 0x0005),
    ("ALIASED_POINT_SIZE_RANGE", 0x846D),
    ("ALIASED_LINE_WIDTH_RANGE", 0x846E),
    ("RED_BITS", 0x0D52),
    ("GREEN_BITS", 0x0D53),
    ("BLUE_BITS", 0x0D54),
    ("ALPHA_BITS", 0x0D55),
    ("DEPTH_BITS", 0x0D56),
    ("STENCIL_BITS", 0x0D57),
    ("MAX_TEXTURE_SIZE", 0x0D33),
    ("MAX_VIEWPORT_DIMS", 0x0D3A),
    ("VENDOR", 0x1F00),
    ("RENDERER", 0x1F01),
    ("VERSION", 0x1F02),
    ("MAX_VERTEX_ATTRIBS", 0x8869),
    ("MAX_TEXTURE_IMAGE_UNITS", 0x8872),
    ("MAX_CUBE_MAP_TEXTURE_SIZE", 0x851C),
    ("MAX_RENDERBUFFER_SIZE", 0x84E8),
    ("MAX_VERTEX_TEXTURE_IMAGE_UNITS", 0x8B4C),
    ("MAX_COMBINED_TEXTURE_IMAGE_UNITS", 0x8B4D),
    ("SHADING_LANGUAGE_VERSION", 0x8B8C),
    ("MAX_VERTEX_UNIFORM_VECTORS", 0x8DFB),
    ("MAX_VARYING_VECTORS", 0x8DFC),
    ("MAX_FRAGMENT_UNIFORM_VECTORS", 0x8DFD),
    ("LOW_FLOAT", 0x8DF0),
    ("MEDIUM_FLOAT", 0x8DF1),
    ("HIGH_FLOAT", 0x8DF2),
    ("LOW_INT", 0x8DF3),
    ("MEDIUM_INT", 0x8DF4),
    ("HIGH_INT", 0x8DF5),
    ("COMPRESSED_TEXTURE_FORMATS", 0x86A3),
];

pub(crate) const WEBGL2_CONSTANTS: &[(&str, u32)] = &[
    ("IMPLEMENTATION_COLOR_READ_TYPE", 0x8B9A),
    ("IMPLEMENTATION_COLOR_READ_FORMAT", 0x8B9B),
    ("MAX_3D_TEXTURE_SIZE", 0x8073),
    ("MAX_ELEMENTS_VERTICES", 0x80E8),
    ("MAX_ELEMENTS_INDICES", 0x80E9),
    ("MAX_TEXTURE_LOD_BIAS", 0x84FD),
    ("MAX_DRAW_BUFFERS", 0x8824),
    ("MAX_FRAGMENT_UNIFORM_COMPONENTS", 0x8B49),
    ("MAX_VERTEX_UNIFORM_COMPONENTS", 0x8B4A),
    ("MAX_ARRAY_TEXTURE_LAYERS", 0x88FF),
    ("MAX_PROGRAM_TEXEL_OFFSET", 0x8905),
    ("MIN_PROGRAM_TEXEL_OFFSET", 0x8904),
    ("MAX_VARYING_COMPONENTS", 0x8B4B),
    ("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", 0x8C80),
    ("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", 0x8C8A),
    ("MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", 0x8C8B),
    ("MAX_COLOR_ATTACHMENTS", 0x8CDF),
    ("MAX_SAMPLES", 0x8D57),
    ("MAX_VERTEX_UNIFORM_BLOCKS", 0x8A2B),
    ("MAX_FRAGMENT_UNIFORM_BLOCKS", 0x8A2D),
    ("MAX_COMBINED_UNIFORM_BLOCKS", 0x8A2E),
    ("MAX_UNIFORM_BUFFER_BINDINGS", 0x8A2F),
    ("MAX_UNIFORM_BLOCK_SIZE", 0x8A30),
    ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", 0x8A31),
    ("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", 0x8A33),
    ("UNIFORM_BUFFER_OFFSET_ALIGNMENT", 0x8A34),
    ("MAX_VERTEX_OUTPUT_COMPONENTS", 0x9122),
    ("MAX_FRAGMENT_INPUT_COMPONENTS", 0x9125),
    ("MAX_SERVER_WAIT_TIMEOUT", 0x9111),
    ("MAX_ELEMENT_INDEX", 0x8D6B),
    ("MAX_CLIENT_WAIT_TIMEOUT_WEBGL", 0x9247),
    ("RENDERBUFFER", 0x8D41),
    ("SAMPLES", 0x80A9),
    ("FRAMEBUFFER", 0x8D40),
    ("COLOR_ATTACHMENT0", 0x8CE0),
    ("FRAMEBUFFER_COMPLETE", 0x8CD5),
    ("R8", 0x8229),
    ("RG8", 0x822B),
    ("RGB8", 0x8051),
    ("RGBA8", 0x8058),
    ("RGB10_A2", 0x8059),
    ("RGBA4", 0x8056),
    ("RGB5_A1", 0x8057),
    ("RGB565", 0x8D62),
    ("SRGB8_ALPHA8", 0x8C43),
    ("DEPTH_COMPONENT16", 0x81A5),
    ("DEPTH_COMPONENT24", 0x81A6),
    ("DEPTH_COMPONENT32F", 0x8CAC),
    ("STENCIL_INDEX8", 0x8D48),
    ("DEPTH24_STENCIL8", 0x88F0),
    ("DEPTH32F_STENCIL8", 0x8CAD),
    ("R16F", 0x822D),
    ("R32F", 0x822E),
    ("RG16F", 0x822F),
    ("RG32F", 0x8230),
    ("RGBA16F", 0x881A),
    ("RGBA32F", 0x8814),
    ("R11F_G11F_B10F", 0x8C3A),
];

fn webgl_array_value<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[T],
) -> Option<v8::Local<'s, v8::Value>>
where
    T: WebApiValue<'s>,
{
    values.to_v8_value(scope)
}

pub(crate) fn webgl_get_supported_extensions_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if webgl_extensions(scope, args.this()).is_none() {
        return;
    }
    let names: Vec<&str> = WebGlExtension::iter().map(Into::into).collect();
    let value = webgl_array_value(scope, &names).unwrap_or_else(|| v8::Array::new(scope, 0).into());
    rv.set(value);
}

pub(crate) fn webgl2_get_supported_extensions_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    webgl_get_supported_extensions_callback(scope, args, rv);
}

pub(crate) fn webgl_get_extension_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if is_webgl2_context(scope, args.this()) {
        webgl2_get_extension_callback(scope, args, rv);
        return;
    }
    let Some(extensions) = webgl_extensions(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<WebGlGetExtensionArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    rv.set(get_webgl_extension(scope, extensions, &parsed.name));
}

pub(crate) fn webgl2_get_extension_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(extensions) = webgl_extensions(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<WebGl2GetExtensionArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    rv.set(get_webgl_extension(scope, extensions, &parsed.name));
}

fn webgl_extensions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    let extensions = get_private_value(scope, context, WEBGL_EXTENSIONS_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok());
    if extensions.is_none() {
        throw_type_error(scope, "Illegal invocation");
    }
    extensions
}

fn get_webgl_extension<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    extensions: v8::Local<'s, v8::Map>,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    let Ok(extension) = name.parse::<WebGlExtension>() else {
        return v8::null(scope).into();
    };
    let key = v8str(scope, extension.into()).into();
    if let Some(value) = extensions.get(scope, key).filter(|value| value.is_object()) {
        return value;
    }
    let object = match extension {
        WebGlExtension::DebugRendererInfo => build_webgl_debug_renderer_info_object(scope),
        WebGlExtension::LoseContext => build_webgl_lose_context_object(scope),
    }
    .expect("registered WebGL extension should construct in its realm");
    extensions.set(scope, key, object.into());
    object.into()
}

pub(crate) fn webgl_get_parameter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if is_webgl2_context(scope, args.this()) {
        webgl2_get_parameter_callback(scope, args, rv);
        return;
    }
    let Some(extensions) = webgl_extensions(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<WebGlGetParameterArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    match parsed.pname {
        WEBGL_VIEWPORT => return_webgl_viewport(scope, args.this(), &mut rv),
        0x846D | 0x846E => rv.set(webgl_float32_array(scope, &[1.0, 1.0])),
        0x86A3 => rv.set(webgl_uint32_array(scope, &[])),
        0x0D3A => rv.set(webgl_int32_array(scope, &WEBGL_MAX_VIEWPORT_DIMS)),
        0x0D52..=0x0D55 => rv.set(v8::Integer::new(scope, 8).into()),
        0x0D56 => rv.set(v8::Integer::new(scope, 24).into()),
        0x0D57 => rv.set(v8::Integer::new(scope, 0).into()),
        0x0D33 | 0x84E8 | 0x851C => rv.set(v8::Integer::new(scope, 4096).into()),
        0x8869 => rv.set(v8::Integer::new(scope, 16).into()),
        0x8872 | 0x8B4C => rv.set(v8::Integer::new(scope, 8).into()),
        0x8B4D => rv.set(v8::Integer::new(scope, 16).into()),
        0x8DFB..=0x8DFD => rv.set(v8::Integer::new(scope, 128).into()),
        0x1F00 => rv.set(v8str(scope, WEBGL_COMPAT_VENDOR).into()),
        0x1F01 => rv.set(v8str(scope, WEBGL_COMPAT_RENDERER).into()),
        0x9245 if webgl_debug_renderer_info_enabled(scope, extensions) => {
            rv.set(v8str(scope, WEBGL_COMPAT_VENDOR).into())
        }
        0x9246 if webgl_debug_renderer_info_enabled(scope, extensions) => {
            rv.set(v8str(scope, WEBGL_COMPAT_RENDERER).into())
        }
        0x1F02 => rv.set(v8str(scope, "WebGL 1.0 (OpenGL ES 2.0 Chromium)").into()),
        0x8B8C => rv.set(v8str(scope, "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)").into()),
        _ => {
            record_webgl_error(scope, args.this(), WEBGL_INVALID_ENUM);
            rv.set_null();
        }
    }
}

pub(crate) fn webgl2_get_parameter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(extensions) = webgl_extensions(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<WebGl2GetParameterArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    match parsed.pname {
        WEBGL_VIEWPORT => return_webgl_viewport(scope, args.this(), &mut rv),
        0x8B9B => rv.set(v8::Integer::new(scope, 0x1908).into()),
        0x8B9A => rv.set(v8::Integer::new(scope, 0x1401).into()),
        0x8073 => rv.set(v8::Integer::new(scope, 2048).into()),
        0x80E8 | 0x80E9 => rv.set(v8::Integer::new(scope, i32::MAX).into()),
        0x84FD => rv.set(v8::Integer::new(scope, 15).into()),
        0x8824 | 0x8CDF => rv.set(v8::Integer::new(scope, 6).into()),
        0x8B49 | 0x8B4A => rv.set(v8::Integer::new(scope, 16_384).into()),
        0x88FF => rv.set(v8::Integer::new(scope, 2048).into()),
        0x8905 => rv.set(v8::Integer::new(scope, 7).into()),
        0x8904 => rv.set(v8::Integer::new(scope, -8).into()),
        0x8B4B => rv.set(v8::Integer::new(scope, 124).into()),
        0x8C80 | 0x8C8B => rv.set(v8::Integer::new(scope, 4).into()),
        0x8C8A | 0x9122 | 0x9125 => rv.set(v8::Integer::new(scope, 128).into()),
        0x8D57 => rv.set(v8::Integer::new(scope, 4).into()),
        0x8A2B | 0x8A2D => rv.set(v8::Integer::new(scope, 14).into()),
        0x8A2E => rv.set(v8::Integer::new(scope, 60).into()),
        0x8A2F => rv.set(v8::Integer::new(scope, 72).into()),
        0x8A30 => rv.set(v8::Integer::new(scope, 65_536).into()),
        0x8A31 | 0x8A33 => rv.set(v8::Integer::new(scope, 245_760).into()),
        0x9111 | 0x9247 => rv.set(v8::Integer::new(scope, 0).into()),
        0x8D6B => rv.set(v8::Integer::new(scope, 1_073_741_823).into()),
        0x8A34 => rv.set(v8::Integer::new(scope, 256).into()),
        0x1F00 => rv.set(v8str(scope, WEBGL_COMPAT_VENDOR).into()),
        0x1F01 => rv.set(v8str(scope, WEBGL_COMPAT_RENDERER).into()),
        0x9245 if webgl_debug_renderer_info_enabled(scope, extensions) => {
            rv.set(v8str(scope, WEBGL_COMPAT_VENDOR).into())
        }
        0x9246 if webgl_debug_renderer_info_enabled(scope, extensions) => {
            rv.set(v8str(scope, WEBGL_COMPAT_RENDERER).into())
        }
        0x1F02 => rv.set(v8str(scope, "WebGL 2.0 (OpenGL ES 3.0 Chromium)").into()),
        0x8B8C => {
            rv.set(v8str(scope, "WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)").into())
        }
        0x86A3 => rv.set(webgl_uint32_array(scope, &[])),
        0x0D33 | 0x84E8 | 0x851C => rv.set(v8::Integer::new(scope, 8192).into()),
        0x0D3A => rv.set(webgl_int32_array(scope, &WEBGL_MAX_VIEWPORT_DIMS)),
        0x846D | 0x846E => rv.set(webgl_float32_array(scope, &[1.0, 1.0])),
        0x0D52..=0x0D55 => rv.set(v8::Integer::new(scope, 8).into()),
        0x0D56 => rv.set(v8::Integer::new(scope, 24).into()),
        0x0D57 => rv.set(v8::Integer::new(scope, 0).into()),
        0x8869 => rv.set(v8::Integer::new(scope, 16).into()),
        0x8872 | 0x8B4C => rv.set(v8::Integer::new(scope, 16).into()),
        0x8B4D => rv.set(v8::Integer::new(scope, 64).into()),
        _ => {
            record_webgl_error(scope, args.this(), WEBGL_INVALID_ENUM);
            rv.set_null();
        }
    }
}

fn webgl_debug_renderer_info_enabled(
    scope: &mut v8::PinScope<'_, '_>,
    extensions: v8::Local<'_, v8::Map>,
) -> bool {
    let key = v8str(scope, WebGlExtension::DebugRendererInfo.into()).into();
    extensions.has(scope, key) == Some(true)
}

pub(crate) fn webgl2_get_internalformat_parameter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WebGl2GetInternalformatParameterArgs>(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    const RENDERBUFFER: u32 = 0x8D41;
    const SAMPLES: u32 = 0x80A9;
    const MULTISAMPLED_FORMATS: &[u32] = &[
        0x8056, 0x8057, 0x8D62, 0x81A5, 0x8D48, 0x8051, 0x8058, 0x8059, 0x81A6, 0x8C43, 0x8CAC,
        0x8CAD, 0x88F0, 0x8229, 0x822B,
    ];
    let samples = if parsed.target == RENDERBUFFER
        && parsed.pname == SAMPLES
        && MULTISAMPLED_FORMATS.contains(&parsed.internalformat)
    {
        &[4][..]
    } else {
        &[][..]
    };
    rv.set(webgl_int32_array(scope, samples));
}

fn webgl_int32_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[i32],
) -> v8::Local<'s, v8::Value> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Int32Array::new(scope, buffer, 0, values.len())
        .expect("WebGL Int32Array construction should succeed")
        .into()
}

fn webgl_float32_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[f32],
) -> v8::Local<'s, v8::Value> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Float32Array::new(scope, buffer, 0, values.len())
        .expect("WebGL Float32Array construction should succeed")
        .into()
}

fn webgl_uint32_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[u32],
) -> v8::Local<'s, v8::Value> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Uint32Array::new(scope, buffer, 0, values.len())
        .expect("WebGL Uint32Array construction should succeed")
        .into()
}

pub(crate) fn webgl_create_buffer_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlBufferHandleDeclaration::new()
        .bind(scope)
        .expect("WebGLBuffer handle declaration should bind");
    rv.set(value.into());
}

pub(crate) fn webgl_create_framebuffer_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlFramebufferHandleDeclaration::new()
        .bind(scope)
        .expect("WebGLFramebuffer handle declaration should bind");
    rv.set(value.into());
}

pub(crate) fn webgl_create_renderbuffer_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlRenderbufferHandleDeclaration::new()
        .bind(scope)
        .expect("WebGLRenderbuffer handle declaration should bind");
    rv.set(value.into());
}

pub(crate) fn webgl_check_framebuffer_status_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, 0x8CD5).into());
}

pub(crate) fn webgl_create_program_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlProgramHandleDeclaration::new()
        .bind(scope)
        .expect("WebGLProgram handle declaration should bind");
    rv.set(value.into());
}

pub(crate) fn webgl_create_shader_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlShaderHandleDeclaration::new()
        .bind(scope)
        .expect("WebGLShader handle declaration should bind");
    rv.set(value.into());
}

pub(crate) fn webgl_uniform_location_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlUniformLocationHandleDeclaration::new()
        .bind(scope)
        .expect("WebGLUniformLocation handle declaration should bind");
    rv.set(value.into());
}

pub(crate) fn webgl_get_attrib_location_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, 0).into());
}

pub(crate) fn webgl_get_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(errors) = get_private_value(scope, args.this(), WEBGL_ERROR_SLOT)
        .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if errors.size() == 0 {
        rv.set_uint32(0);
        return;
    }
    let error = errors
        .as_array(scope)
        .get_index(scope, 0)
        .expect("pending WebGL errors have an own first entry");
    errors.delete(scope, error);
    rv.set(error);
}

fn record_webgl_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    error: u32,
) {
    let errors = get_private_value(scope, context, WEBGL_ERROR_SLOT)
        .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
        .expect("validated WebGL receiver has error state");
    // Like Blink's synthetic error list, preserve distinct pending errors but
    // coalesce repeated occurrences until getError consumes them.
    errors.add(scope, v8::Integer::new_from_unsigned(scope, error).into());
}

pub(crate) fn webgl_viewport_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if get_private_value(scope, args.this(), WEBGL_VIEWPORT_SLOT).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<WebGlViewportArgs>(scope, &args) else {
        return;
    };
    if parsed.width < 0 || parsed.height < 0 {
        // GL errors preserve the previous viewport and remain pending until
        // getError consumes them. WebIDL conversion failures throw instead.
        record_webgl_error(scope, args.this(), WEBGL_INVALID_VALUE);
        return;
    }
    set_webgl_viewport(
        scope,
        args.this(),
        [
            parsed.x,
            parsed.y,
            parsed.width.min(WEBGL_MAX_VIEWPORT_DIMS[0]),
            parsed.height.min(WEBGL_MAX_VIEWPORT_DIMS[1]),
        ],
    );
}

fn set_webgl_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    values: [i32; 4],
) {
    let values = values.map(|value| v8::Integer::new(scope, value).into());
    let viewport = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, context, WEBGL_VIEWPORT_SLOT, viewport.into());
}

fn return_webgl_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(viewport) = get_private_value(scope, context, WEBGL_VIEWPORT_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let values = [0, 1, 2, 3].map(|index| {
        viewport
            .get_index(scope, index)
            .and_then(|value| value.int32_value(scope))
            .unwrap_or_default()
    });
    // Each query returns a copy; mutating or detaching the result cannot
    // change the context's GL state. This does not claim GPU raster support.
    rv.set(webgl_int32_array(scope, &values));
}

pub(super) fn init_webgl_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    let values = [0, 0, 300, 150].map(|value| v8::Integer::new(scope, value).into());
    let viewport = v8::Array::new_with_elements(scope, &values);
    let extensions = v8::Map::new(scope);
    let errors = v8::Set::new(scope);
    WebGlContextStateDeclaration::new(viewport, errors, extensions)
        .initialize(scope, context)
        .expect("WebGL context state should initialize");
}

pub(super) fn initialize_attached_webgl_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
) {
    if get_private_value(scope, context, WEBGL_VIEWPORT_SLOT).is_none() {
        return;
    }
    // Only context creation takes the canvas dimensions as the initial
    // viewport. Resizing the drawing buffer must not reset viewport state.
    if let Some((width, height)) = super::backing_store::canvas_like_dimensions(scope, canvas) {
        set_webgl_viewport(
            scope,
            context,
            [
                0,
                0,
                width.min(WEBGL_MAX_VIEWPORT_DIMS[0] as u32) as i32,
                height.min(WEBGL_MAX_VIEWPORT_DIMS[1] as u32) as i32,
            ],
        );
    }
}

pub(crate) fn webgl_boolean_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Boolean::new(scope, true).into());
}

pub(crate) fn webgl_get_shader_info_log_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::String::empty(scope).into());
}

pub(crate) fn webgl_noop_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

pub(crate) fn webgl_get_context_attributes_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = WebGlContextAttributes::default()
        .bind(scope)
        .expect("WebGL context attributes declaration should bind");
    rv.set(value.into());
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WebGlContextAttributes {
    // WebIDL dictionary members become own properties in lexicographic order.
    alpha: bool,
    antialias: bool,
    depth: bool,
    desynchronized: bool,
    fail_if_major_performance_caveat: bool,
    power_preference: &'static str,
    premultiplied_alpha: bool,
    preserve_drawing_buffer: bool,
    stencil: bool,
    xr_compatible: bool,
}

impl Default for WebGlContextAttributes {
    fn default() -> Self {
        Self {
            alpha: true,
            antialias: true,
            depth: true,
            desynchronized: false,
            fail_if_major_performance_caveat: false,
            power_preference: "default",
            premultiplied_alpha: true,
            preserve_drawing_buffer: false,
            stencil: false,
            xr_compatible: false,
        }
    }
}

pub(crate) fn webgl_is_context_lost_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Boolean::new(scope, false).into());
}

pub(crate) fn webgl_get_shader_precision_format_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webgl_extensions(scope, args.this()).is_none() {
        return;
    }
    let Some(parsed) = webidl::parse_args::<WebGlGetShaderPrecisionFormatArgs>(scope, &args) else {
        return;
    };
    if !matches!(parsed.shader_type, 0x8B30 | 0x8B31) {
        record_webgl_error(scope, args.this(), WEBGL_INVALID_ENUM);
        rv.set_null();
        return;
    }
    // A stable software-GL precision profile. Integer precision is always 0;
    // the floating-point mantissa width is not meaningful for integer formats.
    let (range_min, range_max, precision) = match parsed.precision_type {
        0x8DF0 | 0x8DF1 => (15, 15, 10),
        0x8DF2 => (127, 127, 23),
        0x8DF3 | 0x8DF4 => (15, 14, 0),
        0x8DF5 => (31, 30, 0),
        _ => {
            record_webgl_error(scope, args.this(), WEBGL_INVALID_ENUM);
            rv.set_null();
            return;
        }
    };
    let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
        scope,
        "WebGLShaderPrecisionFormat",
    )
    .expect("WebGLShaderPrecisionFormat intrinsic prototype should be available");
    let value = WebGlShaderPrecisionFormat::new(prototype, precision, range_min, range_max)
        .bind(scope)
        .expect("WebGL shader precision format declaration should bind");
    rv.set(value.into());
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebGLShaderPrecisionFormat)]
struct WebGlShaderPrecisionFormat<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = interfaces::PRECISION)]
    precision: i32,
    #[webapi(slot = interfaces::RANGE_MIN)]
    range_min: i32,
    #[webapi(slot = interfaces::RANGE_MAX)]
    range_max: i32,
}

pub(crate) fn webgl_lose_context_noop_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

pub(super) fn init_webgl2_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    init_webgl_context_object(scope, object);
    WebGl2ContextObjectDeclaration::new("srgb".to_owned(), "srgb".to_owned())
        .initialize(scope, object)
        .expect("WebGL2RenderingContext declaration should initialize object");
}

fn is_webgl2_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::WebGL2RenderingContext::is_instance(scope, object)
}

pub(crate) fn webgl2_color_space_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::WebGL2RenderingContext::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        WEBGL2_COLOR_SPACE_SLOTS,
        "WebGL2RenderingContext color-space slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value = get_private_value(scope, args.this(), slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "srgb".to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8::String::empty(scope))
            .into(),
    );
}

pub(crate) fn webgl2_color_space_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::WebGL2RenderingContext::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        WEBGL2_COLOR_SPACE_SLOTS,
        "WebGL2RenderingContext color-space slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        args.get(0),
        webidl::Context::member("WebGL2RenderingContext", "colorSpace"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !matches!(value.as_str(), "srgb" | "display-p3") {
        throw_type_error(
            scope,
            &format!("'{value}' is not a valid enum value of type PredefinedColorSpace."),
        );
        return;
    }
    if let Some(value) = v8_string(scope, &value) {
        set_private_value(scope, args.this(), slot, value.into());
    }
    rv.set_undefined();
}

#[cfg(test)]
mod tests {
    use super::WebGlExtension;

    #[test]
    fn webgl_extension_names_are_ascii_case_insensitive() {
        assert_eq!(
            "WEBGL_debug_renderer_info".parse::<WebGlExtension>(),
            Ok(WebGlExtension::DebugRendererInfo)
        );
        assert_eq!(
            "WEBGL_lose_context".parse::<WebGlExtension>(),
            Ok(WebGlExtension::LoseContext)
        );
        assert_eq!(
            "webgl_lose_context".parse::<WebGlExtension>(),
            Ok(WebGlExtension::LoseContext)
        );
        assert!("WEBGL_debug_shaders".parse::<WebGlExtension>().is_err());
    }
}
