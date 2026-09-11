use super::escape::css_escape_callback;
use super::lazy_state::install_css_lazy_state;
use super::registered_properties::css_register_property_callback;
use super::supports::css_supports_callback;
use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::document::install_adopted_style_sheets_array_primordials;
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(plain, own_to_string_tag = "CSS", readonly_to_string_tag)]
struct CssNamespaceObjectDeclaration {
    #[webapi(
        method,
        enumerable,
        name = "escape",
        callback = css_escape_callback,
        length = 1
    )]
    _escape: (),
    #[webapi(
        method,
        enumerable,
        name = "registerProperty",
        callback = css_register_property_callback,
        length = 1
    )]
    _register_property: (),
    #[webapi(
        method,
        enumerable,
        name = "supports",
        callback = css_supports_callback,
        length = 1
    )]
    _supports: (),
}

pub(in crate::context_bootstrap) fn install_css_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    install_css_runtime_state_for_document(scope, global, None)
}

pub(crate) fn install_css_runtime_state_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    owner_document: Option<DomHandle>,
) -> Result<()> {
    if !install_adopted_style_sheets_array_primordials(scope, global) {
        return Err(anyhow!(
            "failed to capture adoptedStyleSheets Array primordials"
        ));
    }
    install_css_lazy_state(scope, global, owner_document)
}

pub(super) fn build_css_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    CssNamespaceObjectDeclaration::default()
        .bind(scope)
        .expect("CSS namespace declaration should bind")
}
