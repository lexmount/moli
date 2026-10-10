//! Convert all recognized dictionary members even when no CDM supports the
//! request. Underscored owned fields are deliberately discarded after conversion.

use crate::{web_api_interfaces, webidl};

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "MediaKeysRequirement", rename_all = "kebab-case")]
pub(in crate::context_bootstrap::navigator_runtime) enum MediaKeysRequirement {
    Required,
    Optional,
    NotAllowed,
}

impl MediaKeysRequirement {
    pub(in crate::context_bootstrap::navigator_runtime) fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::NotAllowed => "not-allowed",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaKeySystemMediaCapability")]
struct Capability {
    #[webidl(name = "contentType", converter = "raw", default = webidl::DomString16(Vec::new()))]
    _content_type: webidl::DomString16,
    #[webidl(name = "encryptionScheme", nullable, converter = "raw")]
    _encryption_scheme: Option<webidl::DomString16>,
    #[webidl(name = "robustness", converter = "raw", default = webidl::DomString16(Vec::new()))]
    _robustness: webidl::DomString16,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaKeySystemConfiguration")]
pub(super) struct Configuration {
    #[webidl(name = "audioCapabilities", sequence, converter = "dictionary")]
    _audio_capabilities: Option<Vec<Capability>>,
    #[webidl(name = "distinctiveIdentifier", converter = "enum", default = MediaKeysRequirement::Optional)]
    _distinctive_identifier: MediaKeysRequirement,
    #[webidl(name = "initDataTypes", sequence, converter = "raw")]
    _init_data_types: Option<Vec<webidl::DomString16>>,
    #[webidl(name = "label", converter = "raw", default = webidl::DomString16(Vec::new()))]
    _label: webidl::DomString16,
    #[webidl(name = "persistentState", converter = "enum", default = MediaKeysRequirement::Optional)]
    _persistent_state: MediaKeysRequirement,
    #[webidl(name = "sessionTypes", sequence, converter = "raw")]
    _session_types: Option<Vec<webidl::DomString16>>,
    #[webidl(name = "videoCapabilities", sequence, converter = "dictionary")]
    _video_capabilities: Option<Vec<Capability>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigator.requestMediaKeySystemAccess")]
pub(super) struct RequestArgs {
    #[webidl(required, converter = "raw")]
    pub(super) key_system: webidl::DomString16,
    #[webidl(
        required,
        name = "supportedConfigurations",
        sequence,
        converter = "dictionary"
    )]
    pub(super) configurations: Vec<Configuration>,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "MediaKeySessionType", rename_all = "kebab-case")]
enum SessionType {
    Temporary,
    PersistentLicense,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeys.createSession")]
pub(super) struct CreateSessionArgs {
    #[webidl(name = "sessionType", converter = "enum", default = SessionType::Temporary)]
    _session_type: SessionType,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaKeysPolicy")]
struct Policy {
    #[webidl(name = "minHdcpVersion", converter = "raw")]
    _min_hdcp_version: Option<webidl::DomString16>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeys.getStatusForPolicy")]
pub(super) struct PolicyArgs {
    #[webidl(name = "policy", dictionary, default = Policy::default())]
    _policy: Policy,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeys.setServerCertificate")]
pub(super) struct CertificateArgs<'s> {
    #[webidl(name = "serverCertificate", required, converter = "raw")]
    _certificate: webidl::NonSharedBufferSource<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeySession.generateRequest")]
pub(super) struct GenerateArgs<'s> {
    #[webidl(name = "initDataType", required, converter = "raw")]
    _data_type: webidl::DomString16,
    #[webidl(name = "initData", required, converter = "raw")]
    _data: webidl::NonSharedBufferSource<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeySession.load")]
pub(super) struct LoadArgs {
    #[webidl(name = "sessionId", required, converter = "raw")]
    _session_id: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeySession.update")]
pub(super) struct UpdateArgs<'s> {
    #[webidl(name = "response", required, converter = "raw")]
    _response: webidl::NonSharedBufferSource<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLMediaElement.setMediaKeys")]
pub(super) struct SetKeysArgs<'s> {
    #[webidl(name = "mediaKeys", required, nullable, interface = web_api_interfaces::MediaKeys)]
    pub(super) keys: Option<v8::Local<'s, v8::Object>>,
}
