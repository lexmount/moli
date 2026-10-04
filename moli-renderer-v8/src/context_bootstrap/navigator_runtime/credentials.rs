//! Secure Window credential surfaces without an authenticator backend.
//!
//! Availability queries describe the capabilities actually supported by Moli.
//! Response objects cannot be produced yet; keep their native members and brand
//! checks without manufacturing credential identifiers, signatures, or keys.

use moli_webapi_declare::WebApiFunctionTemplate;

use crate::{native_bridge::throw_dom_exception, util::v8str, web_api_interfaces};

mod json_options;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Credential, enumerable, receiver)]
struct CredentialDeclaration {
    #[webapi(accessor_property, getter = credential_unavailable)]
    id: (),
    #[webapi(accessor_property = "type", getter = credential_unavailable)]
    credential_type: (),
    #[webapi(static_method, returns_promise, length = 0, callback = unavailable_capability)]
    is_conditional_mediation_available: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PublicKeyCredential, enumerable, receiver)]
struct PublicKeyCredentialDeclaration {
    #[webapi(accessor_property, getter = credential_unavailable)]
    raw_id: (),
    #[webapi(accessor_property, getter = credential_unavailable)]
    response: (),
    #[webapi(accessor_property, getter = credential_unavailable)]
    authenticator_attachment: (),
    #[webapi(method, length = 0, callback = credential_unavailable)]
    get_client_extension_results: (),
    #[webapi(method = "toJSON", length = 0, callback = credential_unavailable)]
    to_json: (),
    #[webapi(static_method, returns_promise, length = 0, callback = unavailable_capability)]
    is_conditional_mediation_available: (),
    #[webapi(static_method, returns_promise, length = 0, callback = unavailable_capability)]
    is_user_verifying_platform_authenticator_available: (),
    #[webapi(static_method, returns_promise, length = 0, callback = client_capabilities)]
    get_client_capabilities: (),
    #[webapi(static_method = "parseCreationOptionsFromJSON", length = 1, callback = json_options::parse_creation)]
    parse_creation_options_from_json: (),
    #[webapi(static_method = "parseRequestOptionsFromJSON", length = 1, callback = json_options::parse_request)]
    parse_request_options_from_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AuthenticatorResponse, enumerable, receiver)]
struct AuthenticatorResponseDeclaration {
    #[webapi(accessor_property = "clientDataJSON", getter = credential_unavailable)]
    client_data_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AuthenticatorAttestationResponse, enumerable, receiver)]
struct AuthenticatorAttestationResponseDeclaration {
    #[webapi(accessor_property, getter = credential_unavailable)]
    attestation_object: (),
    #[webapi(method, length = 0, callback = credential_unavailable)]
    get_transports: (),
    #[webapi(method, length = 0, callback = credential_unavailable)]
    get_authenticator_data: (),
    #[webapi(method, length = 0, callback = credential_unavailable)]
    get_public_key: (),
    #[webapi(method, length = 0, callback = credential_unavailable)]
    get_public_key_algorithm: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AuthenticatorAssertionResponse, enumerable, receiver)]
struct AuthenticatorAssertionResponseDeclaration {
    #[webapi(accessor_property, getter = credential_unavailable)]
    authenticator_data: (),
    #[webapi(accessor_property, getter = credential_unavailable)]
    signature: (),
    #[webapi(accessor_property, getter = credential_unavailable)]
    user_handle: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    // Static members are installed on the constructor template by the derive.
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Credential" => {
            CredentialDeclaration::initialize_template(scope, template);
            CredentialDeclaration::initialize_prototype_template(scope, prototype);
        }
        "PublicKeyCredential" => {
            PublicKeyCredentialDeclaration::initialize_template(scope, template);
            PublicKeyCredentialDeclaration::initialize_prototype_template(scope, prototype);
        }
        "AuthenticatorResponse" => {
            AuthenticatorResponseDeclaration::initialize_prototype_template(scope, prototype);
        }
        "AuthenticatorAttestationResponse" => {
            AuthenticatorAttestationResponseDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "AuthenticatorAssertionResponse" => {
            AuthenticatorAssertionResponseDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn credential_unavailable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Credential responses are unavailable without an authenticator backend.",
    );
}

fn unavailable_capability<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let value = v8::Boolean::new(scope, false).into();
    super::super::stream_adapter::set_resolved_promise(scope, &mut rv, value);
}

fn client_capabilities<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let result = v8::Object::new(scope);
    // WebAuthn's ClientCapability values in lexicographical order. No platform
    // backend, ceremony UI, signal operations, or extensions are supported yet.
    // Each call returns a fresh record in the function's realm.
    let unavailable = v8::Boolean::new(scope, false);
    for name in [
        "conditionalCreate",
        "conditionalGet",
        "hybridTransport",
        "passkeyPlatformAuthenticator",
        "relatedOrigins",
        "signalAllAcceptedCredentials",
        "signalCurrentUserDetails",
        "signalUnknownCredential",
        "userVerifyingPlatformAuthenticator",
    ] {
        let key = v8str(scope, name);
        if result.create_data_property(scope, key.into(), unavailable.into()) != Some(true) {
            return;
        }
    }
    super::super::stream_adapter::set_resolved_promise(scope, &mut rv, result.into());
}
