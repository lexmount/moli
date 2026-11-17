use moli_webidl as webidl;

pub use super::super::schema::{Parameters, RelyingParty, Selection};
use super::super::value::Text;
use webidl::Dictionary;

// The derive orders each dictionary's final member names lexicographically;
// inherited dictionaries such as RelyingParty declare their base explicitly.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialCreationOptionsJSON", snapshot)]
pub struct CreationJson {
    #[webidl(converter = "raw", default = Text::from("none"))]
    pub attestation: Text,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub attestation_formats: webidl::Sequence<Text>,
    #[webidl(converter = "raw")]
    pub authenticator_selection: Option<Dictionary<Selection>>,
    #[webidl(required, converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub challenge: Text,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub exclude_credentials: webidl::Sequence<Dictionary<DescriptorJson>>,
    #[webidl(converter = "raw")]
    pub extensions: Option<Dictionary<ExtensionsJson>>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub hints: webidl::Sequence<Text>,
    #[webidl(required, converter = "raw")]
    pub pub_key_cred_params: webidl::Sequence<Dictionary<Parameters>>,
    #[webidl(required, converter = "raw")]
    pub rp: Dictionary<RelyingParty>,
    pub timeout: Option<u32>,
    #[webidl(required, converter = "raw")]
    pub user: Dictionary<UserJson>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialRequestOptionsJSON", snapshot)]
pub struct RequestJson {
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub allow_credentials: webidl::Sequence<Dictionary<DescriptorJson>>,
    #[webidl(required, converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub challenge: Text,
    #[webidl(converter = "raw")]
    pub extensions: Option<Dictionary<ExtensionsJson>>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub hints: webidl::Sequence<Text>,
    #[webidl(converter = "raw")]
    pub rp_id: Option<Text>,
    pub timeout: Option<u32>,
    #[webidl(converter = "raw", default = Text::from("preferred"))]
    pub user_verification: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialUserEntityJSON", snapshot)]
pub struct UserJson {
    #[webidl(required, converter = "raw")]
    pub display_name: Text,
    #[webidl(required, converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub id: Text,
    #[webidl(required, converter = "raw", snapshot_order = 0)]
    pub name: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialDescriptorJSON", snapshot)]
pub struct DescriptorJson {
    #[webidl(required, converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub id: Text,
    #[webidl(converter = "raw")]
    pub transports: Option<webidl::Sequence<Text>>,
    #[webidl(required, name = "type", converter = "raw")]
    pub credential_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsClientInputsJSON", snapshot)]
pub struct ExtensionsJson {
    #[webidl(converter = "raw")]
    pub appid: Option<Text>,
    #[webidl(converter = "raw")]
    pub appid_exclude: Option<Text>,
    #[webidl(converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub cred_blob: Option<Text>,
    pub cred_props: Option<bool>,
    #[webidl(converter = "usv_string")]
    pub credential_protection_policy: Option<String>,
    #[webidl(default = false)]
    pub enforce_credential_protection_policy: bool,
    pub get_cred_blob: Option<bool>,
    pub hmac_create_secret: Option<bool>,
    #[webidl(converter = "raw")]
    pub large_blob: Option<Dictionary<LargeBlobJson>>,
    pub min_pin_length: Option<bool>,
    #[webidl(converter = "raw")]
    pub prf: Option<Dictionary<PrfJson>>,
    #[webidl(name = "remoteClientDataJSON", converter = "raw")]
    pub remote_client_data_json: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsLargeBlobInputsJSON", snapshot)]
pub struct LargeBlobJson {
    pub read: Option<bool>,
    #[webidl(converter = "raw")]
    pub support: Option<Text>,
    #[webidl(converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub write: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsPRFInputsJSON", snapshot)]
pub struct PrfJson {
    #[webidl(converter = "raw")]
    pub eval: Option<Dictionary<PrfValuesJson>>,
    #[webidl(converter = "raw")]
    pub eval_by_credential: Option<webidl::Record<Text, Dictionary<PrfValuesJson>>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticationExtensionsPRFValuesJSON", snapshot)]
pub struct PrfValuesJson {
    #[webidl(required, converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub first: Text,
    #[webidl(converter = "raw", snapshot_with = super::encode::decode_buffer, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    pub second: Option<Text>,
}
