use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyViolationEventFields,
};
use crate::context_bootstrap::events::EventInit;
use crate::webidl;
use moli_webapi::events::subclasses::security_policy::{
    SecurityPolicyViolationEventDisposition, SecurityPolicyViolationEventInit,
};
use webidl::WebIdlDictionarySerialize;

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
        disposition: match fields.disposition {
            ContentSecurityPolicyDisposition::Enforce => {
                SecurityPolicyViolationEventDisposition::Enforce
            }
            ContentSecurityPolicyDisposition::Report => {
                SecurityPolicyViolationEventDisposition::Report
            }
        },
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
