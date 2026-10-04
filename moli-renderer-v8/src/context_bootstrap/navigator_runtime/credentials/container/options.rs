//! Convert credential dictionaries before entering the backend algorithm.
//!
//! All dictionaries preserve WebIDL's inherited-then-lexical member order.
//! Underscored fields are converted even though the unavailable backend cannot
//! consume them. BufferSource validation uses native identity and never copies
//! authenticator input or reads an author-visible `.buffer` property.

use crate::{abort_signal_route::ResolvedAbortSignal, web_api_interfaces, webidl};

use super::super::{
    schema::{Parameters, RelyingParty, Selection},
    value::{Dictionary, Text},
};

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "CredentialMediationRequirement")]
enum Mediation {
    Silent,
    Optional,
    Conditional,
    Required,
}

pub(super) struct Signal<'s>(pub(super) ResolvedAbortSignal<'s>);

impl<'s> webidl::WebIdlConverter<'s> for Signal<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let signal = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .filter(|object| web_api_interfaces::AbortSignal::is_instance(scope, *object))
            .and_then(|object| moli_webapi_declare::web_api_object_target(scope, object))
            .and_then(|object| ResolvedAbortSignal::resolve(scope, object));
        signal
            .map(Self)
            .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "AbortSignal"))
    }
}

struct Credential;

impl<'s> webidl::WebIdlConverter<'s> for Credential {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::Credential::is_instance(scope, object)
        {
            Ok(Self)
        } else {
            Err(webidl::WebIdlError::cannot_convert(context, "Credential"))
        }
    }
}

// This is validation for WebAuthn's BufferSource without AllowShared or
// AllowResizable. The existing byte-copy converter also accepts shared input,
// so it cannot be used for these dictionaries.
struct Buffer;

impl<'s> webidl::WebIdlConverter<'s> for Buffer {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let binary = value.is_array_buffer() || value.is_array_buffer_view();
        if binary && !crate::blob::buffer_source_has_shared_or_resizable_backing_store(value) {
            Ok(Self)
        } else {
            Err(webidl::WebIdlError::cannot_convert(context, "BufferSource"))
        }
    }
}

pub(super) struct PasswordInit;

impl<'s> webidl::WebIdlConverter<'s> for PasswordInit {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::HTMLFormElement::is_instance(scope, object)
        {
            return Ok(Self);
        }
        webidl::convert::<Dictionary<PasswordData>>(scope, value, context)?;
        Ok(Self)
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CredentialsContainer.get")]
pub(super) struct GetArgs<'s> {
    #[webidl(converter = "raw", default = Dictionary::empty(scope)?)]
    pub(super) options: Dictionary<Request<'s>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CredentialsContainer.create")]
pub(super) struct CreateArgs<'s> {
    #[webidl(converter = "raw", default = Dictionary::empty(scope)?)]
    pub(super) options: Dictionary<Creation<'s>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CredentialsContainer.store")]
pub(super) struct StoreArgs {
    #[webidl(required, converter = "raw")]
    _credential: Credential,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CredentialRequestOptions")]
pub(super) struct Request<'s> {
    #[webidl(converter = "raw")]
    _federated: Option<Dictionary<FederatedRequest>>,
    #[webidl(converter = "enum", default = Mediation::Optional)]
    _mediation: Mediation,
    #[webidl(default = false)]
    _password: bool,
    #[webidl(converter = "raw")]
    _public_key: Option<Dictionary<PublicKeyRequest>>,
    #[webidl(converter = "raw")]
    pub(super) signal: Option<Signal<'s>>,
    #[webidl(converter = "raw")]
    _ui_mode: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CredentialCreationOptions")]
pub(super) struct Creation<'s> {
    #[webidl(converter = "raw")]
    pub(super) federated: Option<Dictionary<FederatedInit>>,
    #[webidl(converter = "enum", default = Mediation::Optional)]
    _mediation: Mediation,
    #[webidl(converter = "raw")]
    pub(super) password: Option<PasswordInit>,
    #[webidl(converter = "raw")]
    pub(super) public_key: Option<Dictionary<PublicKeyCreation>>,
    #[webidl(converter = "raw")]
    pub(super) signal: Option<Signal<'s>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FederatedCredentialRequestOptions")]
struct FederatedRequest {
    #[webidl(converter = "raw")]
    _protocols: Option<webidl::Sequence<Text>>,
    #[webidl(converter = "raw")]
    _providers: Option<webidl::Sequence<webidl::UsvString>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FederatedCredentialInit")]
pub(super) struct FederatedInit {
    // CredentialData.id is inherited.
    #[webidl(required, converter = "usv_string")]
    _id: String,
    #[webidl(name = "iconURL", converter = "usv_string")]
    _icon_url: Option<String>,
    #[webidl(converter = "usv_string")]
    _name: Option<String>,
    #[webidl(required, converter = "usv_string")]
    _origin: String,
    #[webidl(converter = "raw")]
    _protocol: Option<Text>,
    #[webidl(required, converter = "usv_string")]
    _provider: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PasswordCredentialData")]
struct PasswordData {
    #[webidl(required, converter = "usv_string")]
    _id: String,
    #[webidl(name = "iconURL", converter = "usv_string")]
    _icon_url: Option<String>,
    #[webidl(converter = "usv_string")]
    _name: Option<String>,
    #[webidl(required, converter = "usv_string")]
    _origin: String,
    #[webidl(required, converter = "usv_string")]
    _password: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialCreationOptions")]
pub(super) struct PublicKeyCreation {
    #[webidl(converter = "raw", default = Text::from("none"))]
    _attestation: Text,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    _attestation_formats: webidl::Sequence<Text>,
    #[webidl(converter = "raw")]
    _authenticator_selection: Option<Dictionary<Selection>>,
    #[webidl(required, converter = "raw")]
    _challenge: Buffer,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    _exclude_credentials: webidl::Sequence<Dictionary<Descriptor>>,
    #[webidl(converter = "raw")]
    _extensions: Option<Dictionary<Extensions>>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    _hints: webidl::Sequence<Text>,
    #[webidl(required, converter = "raw")]
    _pub_key_cred_params: webidl::Sequence<Dictionary<Parameters>>,
    #[webidl(required, converter = "raw")]
    _rp: Dictionary<RelyingParty>,
    _timeout: Option<u32>,
    #[webidl(required, converter = "raw")]
    _user: Dictionary<User>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialRequestOptions")]
struct PublicKeyRequest {
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    _allow_credentials: webidl::Sequence<Dictionary<Descriptor>>,
    #[webidl(required, converter = "raw")]
    _challenge: Buffer,
    #[webidl(converter = "raw")]
    _extensions: Option<Dictionary<Extensions>>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    _hints: webidl::Sequence<Text>,
    #[webidl(converter = "raw")]
    _rp_id: Option<Text>,
    _timeout: Option<u32>,
    #[webidl(converter = "raw", default = Text::from("preferred"))]
    _user_verification: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialUserEntity")]
struct User {
    // Unlike the JSON dictionary, this inherits name before displayName/id.
    #[webidl(required, converter = "raw")]
    _name: Text,
    #[webidl(required, converter = "raw")]
    _display_name: Text,
    #[webidl(required, converter = "raw")]
    _id: Buffer,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialDescriptor")]
struct Descriptor {
    #[webidl(required, converter = "raw")]
    _id: Buffer,
    #[webidl(converter = "raw")]
    _transports: Option<webidl::Sequence<Text>>,
    #[webidl(required, name = "type", converter = "raw")]
    _credential_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsClientInputs")]
struct Extensions {
    #[webidl(converter = "raw")]
    _appid: Option<Text>,
    #[webidl(converter = "raw")]
    _appid_exclude: Option<Text>,
    #[webidl(converter = "raw")]
    _cred_blob: Option<Buffer>,
    _cred_props: Option<bool>,
    #[webidl(converter = "usv_string")]
    _credential_protection_policy: Option<String>,
    #[webidl(default = false)]
    _enforce_credential_protection_policy: bool,
    _get_cred_blob: Option<bool>,
    _hmac_create_secret: Option<bool>,
    #[webidl(converter = "raw")]
    _large_blob: Option<Dictionary<LargeBlob>>,
    _min_pin_length: Option<bool>,
    #[webidl(converter = "raw")]
    _prf: Option<Dictionary<Prf>>,
    #[webidl(name = "remoteClientDataJSON", converter = "raw")]
    _remote_client_data_json: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsLargeBlobInputs")]
struct LargeBlob {
    _read: Option<bool>,
    #[webidl(converter = "raw")]
    _support: Option<Text>,
    #[webidl(converter = "raw")]
    _write: Option<Buffer>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsPRFInputs")]
struct Prf {
    #[webidl(converter = "raw")]
    _eval: Option<Dictionary<PrfValues>>,
    #[webidl(converter = "raw")]
    _eval_by_credential: Option<webidl::Record<Text, Dictionary<PrfValues>>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsPRFValues")]
struct PrfValues {
    #[webidl(required, converter = "raw")]
    _first: Buffer,
    #[webidl(converter = "raw")]
    _second: Option<Buffer>,
}
