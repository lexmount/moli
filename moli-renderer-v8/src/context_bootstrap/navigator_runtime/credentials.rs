use crate::web_api_interfaces;
use moli_webapi::credentials::{base64url, value};
use moli_webapi_declare::WebApiFunctionTemplate;
mod container;
mod signals;
pub(super) use container::build as build_container;
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PublicKeyCredential, enumerable)]
struct PublicKeyCredentialSignalsDeclaration {
    #[webapi(static_method, returns_promise, length = 1, callback = signals::unknown_credential)]
    signal_unknown_credential: (),
    #[webapi(static_method, returns_promise, length = 1, callback = signals::all_accepted_credentials)]
    signal_all_accepted_credentials: (),
    #[webapi(static_method, returns_promise, length = 1, callback = signals::current_user_details)]
    signal_current_user_details: (),
}
pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    moli_webapi::credentials::install(scope, template, name);
    match name {
        "CredentialsContainer" => {
            let prototype = template.prototype_template(scope);
            container::install(scope, prototype);
        }
        "PublicKeyCredential" => {
            PublicKeyCredentialSignalsDeclaration::initialize_template(scope, template)
        }
        _ => {}
    }
}
