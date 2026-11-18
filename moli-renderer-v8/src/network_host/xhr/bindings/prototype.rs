use super::abort::xhr_abort_callback;
use super::open::xhr_open_callback;
use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XMLHttpRequest, enumerable)]
struct XmlHttpRequestTemplateMethodsDeclaration {
    #[webapi(method = "open", length = 2, callback = xhr_open_callback)]
    open: (),

    #[webapi(method = "send", length = 0, callback = xhr_send_callback)]
    send: (),

    #[webapi(method = "abort", length = 0, callback = xhr_abort_callback)]
    abort: (),

    #[webapi(
        method = "setRequestHeader",
        length = 2,
        callback = xhr_set_request_header_callback
    )]
    set_request_header: (),

    #[webapi(
        method = "getAllResponseHeaders",
        length = 0,
        callback = xhr_get_all_response_headers_callback
    )]
    get_all_response_headers: (),

    #[webapi(
        method = "getResponseHeader",
        length = 1,
        callback = xhr_get_response_header_callback
    )]
    get_response_header: (),

    #[webapi(
        method = "overrideMimeType",
        length = 1,
        callback = xhr_override_mime_type_callback
    )]
    override_mime_type: (),
}

pub(crate) fn install_xml_http_request_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    configure_xml_http_request_instance_template(scope, template);
    let prototype = template.prototype_template(scope);
    XmlHttpRequestTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    super::install_xml_http_request_template_surface(scope, template, "XMLHttpRequest");
}

pub(crate) fn install_xml_http_request_event_target_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    super::install_xml_http_request_template_surface(scope, template, "XMLHttpRequestEventTarget");
}
