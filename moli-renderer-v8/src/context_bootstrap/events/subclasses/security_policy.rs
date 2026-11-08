use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyViolationEventFields,
};
use crate::context_bootstrap::events::EventInit;
use crate::webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(
    name = "SecurityPolicyViolationEventDisposition",
    rename_all = "kebab-case"
)]
enum SecurityPolicyViolationEventDisposition {
    Enforce,
    Report,
}

impl From<ContentSecurityPolicyDisposition> for SecurityPolicyViolationEventDisposition {
    fn from(disposition: ContentSecurityPolicyDisposition) -> Self {
        match disposition {
            ContentSecurityPolicyDisposition::Enforce => Self::Enforce,
            ContentSecurityPolicyDisposition::Report => Self::Report,
        }
    }
}

// Convert EventInit first, then the derived members in lexical order. Keep
// DOMString code units distinct from the four USVString URL fields.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SecurityPolicyViolationEventInit", snapshot)]
pub(super) struct SecurityPolicyViolationEventInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "blockedURI", default = "", converter = "usv_string")]
    blocked_uri: String,
    #[webidl(name = "columnNumber", default = 0)]
    column_number: u32,
    #[webidl(converter = "enum", default = SecurityPolicyViolationEventDisposition::Enforce)]
    disposition: SecurityPolicyViolationEventDisposition,
    #[webidl(name = "documentURI", default = "", converter = "usv_string")]
    document_uri: String,
    #[webidl(name = "effectiveDirective", default = webidl::DomString16(Vec::new()), converter = "raw")]
    effective_directive: webidl::DomString16,
    #[webidl(name = "lineNumber", default = 0)]
    line_number: u32,
    #[webidl(name = "originalPolicy", default = webidl::DomString16(Vec::new()), converter = "raw")]
    original_policy: webidl::DomString16,
    #[webidl(default = "", converter = "usv_string")]
    referrer: String,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    sample: webidl::DomString16,
    #[webidl(name = "sourceFile", default = "", converter = "usv_string")]
    source_file: String,
    #[webidl(name = "statusCode", converter = "unsigned_short", default = 0)]
    status_code: u16,
    #[webidl(name = "violatedDirective", default = webidl::DomString16(Vec::new()), converter = "raw")]
    violated_directive: webidl::DomString16,
}

pub(super) fn parse_security_policy_violation_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<SecurityPolicyViolationEventInit> {
    crate::context_bootstrap::events::parse_event_dictionary(
        scope,
        args,
        "SecurityPolicyViolationEvent",
    )
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
        crate::web_api_interfaces::initialize(scope, event, "SecurityPolicyViolationEvent")
            .expect("SecurityPolicyViolationEvent brand");
    }
}

/// Native reports use the same dictionary schema, without converting an author
/// object or consulting a public constructor.
pub(crate) fn initialize_native_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> Option<()> {
    let text = |value: &str| webidl::DomString16(value.encode_utf16().collect());
    SecurityPolicyViolationEventInit {
        base: EventInit {
            bubbles: true,
            cancelable: false,
            composed: true,
        },
        blocked_uri: fields.blocked_uri.to_owned(),
        column_number: fields.column_number.try_into().unwrap_or_default(),
        disposition: fields.disposition.into(),
        document_uri: fields.document_uri.to_owned(),
        effective_directive: text(fields.effective_directive),
        line_number: fields.line_number.try_into().unwrap_or_default(),
        original_policy: text(fields.original_policy),
        referrer: fields.referrer.to_owned(),
        sample: text(fields.sample),
        source_file: fields.source_file.to_owned(),
        status_code: fields.status_code.try_into().unwrap_or_default(),
        violated_directive: text(fields.violated_directive),
    }
    .initialize_snapshot(scope, event)?;
    crate::web_api_interfaces::initialize(scope, event, "SecurityPolicyViolationEvent").ok()
}
