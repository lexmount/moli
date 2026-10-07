use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyViolationEventDeclaration,
};
use crate::context_bootstrap::events::EventInit;
use crate::util::{v8_string, v8_string_from_utf16_units};
use crate::webidl;

// Convert EventInit first, then the derived members in lexical order. Keep
// DOMString code units distinct from the four USVString URL fields.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SecurityPolicyViolationEventInit")]
pub(super) struct SecurityPolicyViolationEventInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "blockedURI", default = "", converter = "usv_string")]
    blocked_uri: String,
    #[webidl(name = "columnNumber", default = 0)]
    column_number: u32,
    #[webidl(converter = "enum", default = ContentSecurityPolicyDisposition::Enforce)]
    disposition: ContentSecurityPolicyDisposition,
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

impl Default for SecurityPolicyViolationEventInit {
    fn default() -> Self {
        Self {
            base: EventInit::default(),
            blocked_uri: String::new(),
            column_number: 0,
            disposition: ContentSecurityPolicyDisposition::Enforce,
            document_uri: String::new(),
            effective_directive: webidl::DomString16(Vec::new()),
            line_number: 0,
            original_policy: webidl::DomString16(Vec::new()),
            referrer: String::new(),
            sample: webidl::DomString16(Vec::new()),
            source_file: String::new(),
            status_code: 0,
            violated_directive: webidl::DomString16(Vec::new()),
        }
    }
}

pub(super) fn parse_security_policy_violation_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<SecurityPolicyViolationEventInit> {
    let parsed = webidl::dictionary_arg(
        args,
        1,
        webidl::Context::argument("SecurityPolicyViolationEvent", 2),
    )
    .and_then(|object| match object {
        Some(object) => webidl::parse_dictionary_object(scope, object),
        None => Ok(SecurityPolicyViolationEventInit::default()),
    });
    match parsed {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
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
        let document_uri = v8_string(scope, &self.document_uri).expect("USVString documentURI");
        let referrer = v8_string(scope, &self.referrer).expect("USVString referrer");
        let blocked_uri = v8_string(scope, &self.blocked_uri).expect("USVString blockedURI");
        let effective_directive = v8_string_from_utf16_units(scope, &self.effective_directive.0)
            .expect("DOMString effectiveDirective");
        let violated_directive = v8_string_from_utf16_units(scope, &self.violated_directive.0)
            .expect("DOMString violatedDirective");
        let original_policy = v8_string_from_utf16_units(scope, &self.original_policy.0)
            .expect("DOMString originalPolicy");
        let disposition = v8_string(scope, self.disposition.as_str()).expect("disposition enum");
        let source_file = v8_string(scope, &self.source_file).expect("USVString sourceFile");
        let sample = v8_string_from_utf16_units(scope, &self.sample.0).expect("DOMString sample");
        ContentSecurityPolicyViolationEventDeclaration::new(
            document_uri,
            referrer,
            blocked_uri,
            effective_directive,
            violated_directive,
            original_policy,
            disposition,
            source_file,
            sample,
            self.line_number,
            self.column_number,
            self.status_code,
        )
        .initialize(scope, event)
        .expect("SecurityPolicyViolationEvent state should initialize");
    }
}
