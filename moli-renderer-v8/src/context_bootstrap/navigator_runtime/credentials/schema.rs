//! WebAuthn dictionaries whose member types are identical for binary and JSON inputs.

use crate::webidl;

use super::value::Text;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialRpEntity")]
pub(super) struct RelyingParty {
    // Inherited members precede the derived dictionary's lexical order.
    #[webidl(required, converter = "raw")]
    pub(super) name: Text,
    #[webidl(converter = "raw")]
    pub(super) id: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PublicKeyCredentialParameters")]
pub(super) struct Parameters {
    #[webidl(required)]
    pub(super) alg: i32,
    #[webidl(required, name = "type", converter = "raw")]
    pub(super) credential_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AuthenticatorSelectionCriteria")]
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
