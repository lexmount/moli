use crate::fetch_request::RequestCredentialsModeWebIdl;
use moli_fetch_types::RequestCredentialsMode;
use moli_webidl as webidl;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "WorkerType", parse_with = Self::parse_webidl_token)]
#[strum(serialize_all = "lowercase")]
pub enum WorkerScriptKind {
    Classic,
    Module,
}

impl WorkerScriptKind {
    fn parse_webidl_token(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WorkerOptions")]
pub struct WorkerOptionsMembers {
    #[webidl(name = "type", default = WorkerScriptKind::Classic, converter = "enum")]
    pub worker_type: WorkerScriptKind,
    #[webidl(default = "")]
    pub name: String,
    #[webidl(name = "credentials", default = RequestCredentialsModeWebIdl(RequestCredentialsMode::SameOrigin), converter = "enum")]
    pub credentials_mode: RequestCredentialsModeWebIdl,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SharedWorkerOptions")]
pub struct SharedWorkerOptionsMembers {
    #[webidl(inherit)]
    pub worker: WorkerOptionsMembers,
    #[webidl(default = false)]
    pub extended_lifetime: bool,
    #[webidl(converter = "enum")]
    pub same_site_cookies: Option<SameSiteCookies>,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "SharedWorkerSameSiteCookies")]
pub enum SameSiteCookies {
    #[webidl(token = "all")]
    All,
    #[webidl(token = "none")]
    None,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "WorkletOptions")]
pub struct WorkletOptions {
    #[webidl(converter = "enum", default = RequestCredentialsModeWebIdl(RequestCredentialsMode::SameOrigin))]
    pub credentials: RequestCredentialsModeWebIdl,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Worklet.addModule")]
pub struct WorkletAddModuleArgs {
    #[webidl(required, converter = "usv_string")]
    pub module_url: String,
    #[webidl(dictionary)]
    pub options: WorkletOptions,
}
