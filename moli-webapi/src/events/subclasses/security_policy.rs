use super::super::init::EventInit;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(
    name = "SecurityPolicyViolationEventDisposition",
    rename_all = "kebab-case"
)]
pub enum SecurityPolicyViolationEventDisposition {
    Enforce,
    Report,
}

// Convert EventInit first, then the derived members in lexical order. Keep
// DOMString code units distinct from the four USVString URL fields.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SecurityPolicyViolationEventInit", snapshot)]
pub struct SecurityPolicyViolationEventInit {
    #[webidl(inherit)]
    pub base: EventInit,
    #[webidl(name = "blockedURI", default = "", converter = "usv_string")]
    pub blocked_uri: String,
    #[webidl(name = "columnNumber", default = 0)]
    pub column_number: u32,
    #[webidl(converter = "enum", default = SecurityPolicyViolationEventDisposition::Enforce)]
    pub disposition: SecurityPolicyViolationEventDisposition,
    #[webidl(name = "documentURI", default = "", converter = "usv_string")]
    pub document_uri: String,
    #[webidl(name = "effectiveDirective", default = webidl::DomString16(Vec::new()), converter = "raw")]
    pub effective_directive: webidl::DomString16,
    #[webidl(name = "lineNumber", default = 0)]
    pub line_number: u32,
    #[webidl(name = "originalPolicy", default = webidl::DomString16(Vec::new()), converter = "raw")]
    pub original_policy: webidl::DomString16,
    #[webidl(default = "", converter = "usv_string")]
    pub referrer: String,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    pub sample: webidl::DomString16,
    #[webidl(name = "sourceFile", default = "", converter = "usv_string")]
    pub source_file: String,
    #[webidl(name = "statusCode", converter = "unsigned_short", default = 0)]
    pub status_code: u16,
    #[webidl(name = "violatedDirective", default = webidl::DomString16(Vec::new()), converter = "raw")]
    pub violated_directive: webidl::DomString16,
}

pub(super) fn parse_security_policy_violation_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<SecurityPolicyViolationEventInit> {
    super::super::init::parse_event_dictionary(scope, args, "SecurityPolicyViolationEvent")
}

impl SecurityPolicyViolationEventInit {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }

    pub(super) fn initialize<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("SecurityPolicyViolationEvent state should initialize");
        crate::interfaces::initialize(scope, event, "SecurityPolicyViolationEvent")
            .expect("SecurityPolicyViolationEvent brand");
    }
}
