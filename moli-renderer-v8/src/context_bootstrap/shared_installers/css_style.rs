use super::*;
use crate::native_bridge::element;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSStyleDeclaration)]
struct CssStyleDeclarationPrototypeDeclaration {
    #[webapi(accessor_property, enumerable, getter = element::style_length_getter_callback)]
    length: (),

    #[webapi(
        accessor_property = "cssText",
        enumerable,
        getter = element::style_css_text_getter_callback,
        setter = element::style_css_text_setter_callback
    )]
    css_text: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

pub(in crate::context_bootstrap) fn install_css_style_declaration_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "CSSStyleDeclaration" {
        return;
    }
    CssStyleDeclarationPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
    let prototype = template.prototype_template(scope);
    // The WebKit-cased compatibility aliases are installed as real
    // accessors by `install_css_style_declaration_template_bindings`.
    // Installing placeholder values for those names as well creates
    // duplicate ObjectTemplate descriptors, which V8 rejects when the
    // prototype is instantiated.
    for name in ["color", "cssFloat", "transform"] {
        prototype.set(v8str(scope, name).into(), v8::undefined(scope).into());
    }
}
