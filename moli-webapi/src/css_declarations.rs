//! CSS declaration objects and their private declaration metadata.

pub mod options;

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value, v8_string};
use moli_webapi_declare::{WebApiObject, serialize_v8_array};

const STYLE_NAMES_SLOT: &str = "__moliStyleNames";
const STYLE_INTERFACE_SLOT: &str = "__moliStyleInterface";

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::CSSStyleProperties,
    own_to_string_tag = "CSSStyleProperties"
)]
struct LightweightCssStylePropertiesDeclaration<'scope> {
    #[webapi(slot = STYLE_INTERFACE_SLOT)]
    interface: &'static str,
    #[webapi(slot = STYLE_NAMES_SLOT)]
    names: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::CSSFontFaceDescriptors,
    own_to_string_tag = "CSSFontFaceDescriptors"
)]
struct LightweightCssFontFaceDescriptorsDeclaration<'scope> {
    #[webapi(slot = STYLE_INTERFACE_SLOT)]
    interface: &'static str,
    #[webapi(slot = STYLE_NAMES_SLOT)]
    names: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::CSSPageDescriptors,
    own_to_string_tag = "CSSPageDescriptors"
)]
struct LightweightCssPageDescriptorsDeclaration<'scope> {
    #[webapi(slot = STYLE_INTERFACE_SLOT)]
    interface: &'static str,
    #[webapi(slot = STYLE_NAMES_SLOT)]
    names: v8::Local<'scope, v8::Array>,
}

pub fn build_lightweight_css_style_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    LightweightCssStylePropertiesDeclaration {
        interface: "CSSStyleProperties",
        names: v8::Array::new(scope, 0),
    }
    .bind(scope)
    .expect("CSSStyleProperties declaration should bind")
}

pub fn build_lightweight_css_font_face_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    LightweightCssFontFaceDescriptorsDeclaration {
        interface: "CSSFontFaceDescriptors",
        names: v8::Array::new(scope, 0),
    }
    .bind(scope)
    .expect("CSSFontFaceDescriptors declaration should bind")
}

pub fn build_lightweight_css_page_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    LightweightCssPageDescriptorsDeclaration {
        interface: "CSSPageDescriptors",
        names: v8::Array::new(scope, 0),
    }
    .bind(scope)
    .expect("CSSPageDescriptors declaration should bind")
}

pub fn set_style_interface(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
    interface_name: &str,
) {
    let Some(interface_name) = v8_string(scope, interface_name) else {
        return;
    };
    set_private_value(scope, style, STYLE_INTERFACE_SLOT, interface_name.into());
}

pub fn is_lightweight_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, style, STYLE_NAMES_SLOT).is_some()
}

pub fn style_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    get_private_value(scope, style, STYLE_INTERFACE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub fn style_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let Some(value) = get_private_value(scope, style, STYLE_NAMES_SLOT) else {
        return Vec::new();
    };
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for index in 0..array.length() {
        let Some(value) = array.get_index(scope, index) else {
            continue;
        };
        let Some(value) = value.to_string(scope) else {
            continue;
        };
        names.push(value.to_rust_string_lossy(scope));
    }
    names
}

pub fn store_style_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    names: &[String],
) {
    let array = serialize_v8_array(scope, names).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(scope, style, STYLE_NAMES_SLOT, array.into());
}
