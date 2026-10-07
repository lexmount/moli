use crate::webidl;

pub(super) use super::super::schema::{Parameters, RelyingParty, Selection};
use super::super::value::{Dictionary, Text};

// The derive orders each dictionary's final member names lexicographically;
// inherited dictionaries such as RelyingParty declare their base explicitly.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialCreationOptionsJSON")]
pub(super) struct CreationJson {
    #[webidl(converter = "raw", default = Text::from("none"))]
    pub(super) attestation: Text,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) attestation_formats: webidl::Sequence<Text>,
    #[webidl(converter = "raw")]
    pub(super) authenticator_selection: Option<Dictionary<Selection>>,
    #[webidl(required, converter = "raw")]
    pub(super) challenge: Text,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) exclude_credentials: webidl::Sequence<Dictionary<DescriptorJson>>,
    #[webidl(converter = "raw")]
    pub(super) extensions: Option<Dictionary<ExtensionsJson>>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) hints: webidl::Sequence<Text>,
    #[webidl(required, converter = "raw")]
    pub(super) pub_key_cred_params: webidl::Sequence<Dictionary<Parameters>>,
    #[webidl(required, converter = "raw")]
    pub(super) rp: Dictionary<RelyingParty>,
    pub(super) timeout: Option<u32>,
    #[webidl(required, converter = "raw")]
    pub(super) user: Dictionary<UserJson>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialRequestOptionsJSON")]
pub(super) struct RequestJson {
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) allow_credentials: webidl::Sequence<Dictionary<DescriptorJson>>,
    #[webidl(required, converter = "raw")]
    pub(super) challenge: Text,
    #[webidl(converter = "raw")]
    pub(super) extensions: Option<Dictionary<ExtensionsJson>>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) hints: webidl::Sequence<Text>,
    #[webidl(converter = "raw")]
    pub(super) rp_id: Option<Text>,
    pub(super) timeout: Option<u32>,
    #[webidl(converter = "raw", default = Text::from("preferred"))]
    pub(super) user_verification: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialUserEntityJSON")]
pub(super) struct UserJson {
    #[webidl(required, converter = "raw")]
    pub(super) display_name: Text,
    #[webidl(required, converter = "raw")]
    pub(super) id: Text,
    #[webidl(required, converter = "raw")]
    pub(super) name: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialDescriptorJSON")]
pub(super) struct DescriptorJson {
    #[webidl(required, converter = "raw")]
    pub(super) id: Text,
    #[webidl(converter = "raw")]
    pub(super) transports: Option<webidl::Sequence<Text>>,
    #[webidl(required, name = "type", converter = "raw")]
    pub(super) credential_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsClientInputsJSON")]
pub(super) struct ExtensionsJson {
    #[webidl(converter = "raw")]
    pub(super) appid: Option<Text>,
    #[webidl(converter = "raw")]
    pub(super) appid_exclude: Option<Text>,
    #[webidl(converter = "raw")]
    pub(super) cred_blob: Option<Text>,
    pub(super) cred_props: Option<bool>,
    #[webidl(converter = "usv_string")]
    pub(super) credential_protection_policy: Option<String>,
    #[webidl(default = false)]
    pub(super) enforce_credential_protection_policy: bool,
    pub(super) get_cred_blob: Option<bool>,
    pub(super) hmac_create_secret: Option<bool>,
    #[webidl(converter = "raw")]
    pub(super) large_blob: Option<Dictionary<LargeBlobJson>>,
    pub(super) min_pin_length: Option<bool>,
    #[webidl(converter = "raw")]
    pub(super) prf: Option<Dictionary<PrfJson>>,
    #[webidl(name = "remoteClientDataJSON", converter = "raw")]
    pub(super) remote_client_data_json: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsLargeBlobInputsJSON")]
pub(super) struct LargeBlobJson {
    pub(super) read: Option<bool>,
    #[webidl(converter = "raw")]
    pub(super) support: Option<Text>,
    #[webidl(converter = "raw")]
    pub(super) write: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsPRFInputsJSON")]
pub(super) struct PrfJson {
    #[webidl(converter = "raw")]
    pub(super) eval: Option<Dictionary<PrfValuesJson>>,
    #[webidl(converter = "raw")]
    pub(super) eval_by_credential: Option<webidl::Record<Text, Dictionary<PrfValuesJson>>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsPRFValuesJSON")]
pub(super) struct PrfValuesJson {
    #[webidl(required, converter = "raw")]
    pub(super) first: Text,
    #[webidl(converter = "raw")]
    pub(super) second: Option<Text>,
}
