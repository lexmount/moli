use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Attr)]
struct AttrPrototypeDeclaration {
    #[webapi(method = "toString", length = 0, callback = attr_to_string_callback)]
    to_string: (),
}

fn attr_to_string_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = v8_string(scope, "[object Attr]") {
        rv.set(value.into());
    }
}

pub(in crate::context_bootstrap) fn install_attr_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "Attr" {
        return;
    }
    AttrPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}
