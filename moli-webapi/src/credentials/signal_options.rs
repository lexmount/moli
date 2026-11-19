use super::value::Text;
use moli_webidl as webidl;

// Convert members in WebIDL lexicographic order before API-specific validation.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "UnknownCredentialOptions")]
pub struct UnknownCredential {
    #[webidl(required, converter = "raw")]
    pub credential_id: Text,
    #[webidl(required, converter = "raw")]
    pub rp_id: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AllAcceptedCredentialsOptions")]
pub struct AllAcceptedCredentials {
    #[webidl(required, converter = "raw")]
    pub all_accepted_credential_ids: webidl::Sequence<Text>,
    #[webidl(required, converter = "raw")]
    pub rp_id: Text,
    #[webidl(required, converter = "raw")]
    pub user_id: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CurrentUserDetailsOptions")]
pub struct CurrentUserDetails {
    #[webidl(required, converter = "raw")]
    pub display_name: Text,
    #[webidl(required, converter = "raw")]
    pub name: Text,
    #[webidl(required, converter = "raw")]
    pub rp_id: Text,
    #[webidl(required, converter = "raw")]
    pub user_id: Text,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.signalUnknownCredential")]
pub struct UnknownCredentialArgs {
    #[webidl(required, dictionary)]
    pub options: UnknownCredential,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.signalAllAcceptedCredentials")]
pub struct AllAcceptedCredentialsArgs {
    #[webidl(required, dictionary)]
    pub options: AllAcceptedCredentials,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.signalCurrentUserDetails")]
pub struct CurrentUserDetailsArgs {
    #[webidl(required, dictionary)]
    pub options: CurrentUserDetails,
}
