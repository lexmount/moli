//! WebAuthn dictionaries whose member types are identical for binary and JSON inputs.

use crate::webidl;

use super::value::Text;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialEntity", snapshot)]
pub(super) struct Entity {
    #[webidl(required, converter = "raw")]
    pub(super) name: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialRpEntity", snapshot)]
pub(super) struct RelyingParty {
    #[webidl(inherit)]
    pub(super) base: Entity,
    #[webidl(converter = "raw")]
    pub(super) id: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialParameters", snapshot)]
pub(super) struct Parameters {
    #[webidl(required)]
    pub(super) alg: i32,
    #[webidl(required, name = "type", converter = "raw")]
    pub(super) credential_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticatorSelectionCriteria", snapshot)]
pub(super) struct Selection {
    #[webidl(converter = "raw")]
    pub(super) authenticator_attachment: Option<Text>,
    #[webidl(default = false)]
    pub(super) require_resident_key: bool,
    #[webidl(converter = "raw")]
    pub(super) resident_key: Option<Text>,
    #[webidl(converter = "raw", default = Text::from("preferred"))]
    pub(super) user_verification: Text,
}
