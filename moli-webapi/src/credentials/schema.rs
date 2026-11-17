//! WebAuthn dictionaries whose member types are identical for binary and JSON inputs.

use moli_webidl as webidl;

use super::value::Text;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialEntity", snapshot)]
pub struct Entity {
    #[webidl(required, converter = "raw")]
    pub name: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialRpEntity", snapshot)]
pub struct RelyingParty {
    #[webidl(inherit)]
    pub base: Entity,
    #[webidl(converter = "raw")]
    pub id: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialParameters", snapshot)]
pub struct Parameters {
    #[webidl(required)]
    pub alg: i32,
    #[webidl(required, name = "type", converter = "raw")]
    pub credential_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticatorSelectionCriteria", snapshot)]
pub struct Selection {
    #[webidl(converter = "raw")]
    pub authenticator_attachment: Option<Text>,
    #[webidl(default = false)]
    pub require_resident_key: bool,
    #[webidl(converter = "raw")]
    pub resident_key: Option<Text>,
    #[webidl(converter = "raw", default = Text::from("preferred"))]
    pub user_verification: Text,
}
