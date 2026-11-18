use super::fetch::window_fetch_callback;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
struct WindowNetworkTemplateMethodsDeclaration {
    // The callback authorizes WindowProxy receivers and selects the exception realm.
    #[webapi(method = "fetch", length = 1, callback = window_fetch_callback, receiver = false)]
    fetch: (),
}

pub(crate) fn install_window_network_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    WindowNetworkTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
}
