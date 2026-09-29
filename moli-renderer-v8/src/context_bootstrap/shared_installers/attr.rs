pub(in crate::context_bootstrap) fn install_attr_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "Attr" {
        return;
    }
    crate::native_bridge::document::install_attr_prototype(
        scope,
        template.prototype_template(scope),
    );
}
