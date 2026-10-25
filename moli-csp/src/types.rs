use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentSecurityPolicyResourceKind {
    DocumentBase,
    DocumentConnect,
    DocumentFrame,
    DocumentFont,
    DocumentImage,
    DocumentManifest,
    DocumentMedia,
    DocumentScriptElement,
    DocumentStyleElement,
    WorkerConstructor,
    WorkerConnect,
    WorkerDynamicModuleImport,
    WorkerScript,
    WorkerStaticModuleImport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentSecurityPolicyNonUrlKind {
    DocumentInlineEventHandler,
    DocumentInlineNavigation,
    DocumentInlineScript,
    DocumentInlineStyleAttribute,
    DocumentInlineStyleElement,
    Eval,
    TrustedTypesEval,
    WasmEval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentSecurityPolicyRedirectStatus {
    NoRedirect,
    FollowedRedirect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentSecurityPolicyDisposition {
    Enforce,
    Report,
}

impl ContentSecurityPolicyDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Report => "report",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TrustedTypesForScriptRequirements {
    enforced: bool,
    report_only: bool,
}

impl TrustedTypesForScriptRequirements {
    pub const fn new(enforced: bool, report_only: bool) -> Self {
        Self {
            enforced,
            report_only,
        }
    }

    pub const fn enforced_only(enforced: bool) -> Self {
        Self::new(enforced, false)
    }

    pub const fn requires_conversion(self) -> bool {
        self.enforced || self.report_only
    }

    pub const fn is_enforced(self) -> bool {
        self.enforced
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentSecurityPolicyScriptElementRequest<'a> {
    pub nonce: Option<&'a str>,
    pub integrity: Option<&'a str>,
    pub parser_inserted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentSecurityPolicyStyleElementRequest<'a> {
    pub nonce: Option<&'a str>,
}

impl Default for ContentSecurityPolicyScriptElementRequest<'_> {
    fn default() -> Self {
        Self {
            nonce: None,
            integrity: None,
            parser_inserted: true,
        }
    }
}

impl<'a> ContentSecurityPolicyScriptElementRequest<'a> {
    pub fn parser_inserted_with_nonce(nonce: Option<&'a str>) -> Self {
        Self {
            nonce,
            integrity: None,
            parser_inserted: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentSecurityPolicyUrlViolation {
    pub effective_directive: &'static str,
    pub blocked_uri: String,
    pub document_uri: String,
    pub original_policy: String,
    pub disposition: ContentSecurityPolicyDisposition,
    pub report_uri_endpoints: Vec<String>,
    pub report_to_endpoints: Vec<String>,
    pub sample: String,
    pub source_file: String,
    pub line_number: i32,
    pub column_number: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentSecurityPolicyReportingEndpoints {
    endpoints: BTreeMap<String, String>,
}

impl ContentSecurityPolicyReportingEndpoints {
    pub fn endpoint_for_group(&self, group: &str) -> Option<&str> {
        self.endpoints.get(group).map(String::as_str)
    }

    pub(crate) fn insert(&mut self, group: impl Into<String>, endpoint: impl Into<String>) {
        self.endpoints.insert(group.into(), endpoint.into());
    }
}

#[derive(Clone, Copy)]
pub struct ContentSecurityPolicyViolationEventFields<'a> {
    pub document_uri: &'a str,
    pub referrer: &'a str,
    pub blocked_uri: &'a str,
    pub effective_directive: &'a str,
    pub violated_directive: &'a str,
    pub original_policy: &'a str,
    pub disposition: ContentSecurityPolicyDisposition,
    pub source_file: &'a str,
    pub sample: &'a str,
    pub line_number: i32,
    pub column_number: i32,
    pub status_code: i32,
}

impl<'a> ContentSecurityPolicyViolationEventFields<'a> {
    pub fn from_url_violation(violation: &'a ContentSecurityPolicyUrlViolation) -> Self {
        Self {
            document_uri: violation.document_uri.as_str(),
            referrer: "",
            blocked_uri: violation.blocked_uri.as_str(),
            effective_directive: violation.effective_directive,
            violated_directive: violation.effective_directive,
            original_policy: violation.original_policy.as_str(),
            disposition: violation.disposition,
            source_file: violation.source_file.as_str(),
            sample: violation.sample.as_str(),
            line_number: violation.line_number,
            column_number: violation.column_number,
            status_code: 0,
        }
    }
}

impl<'a> From<&'a ContentSecurityPolicyUrlViolation>
    for ContentSecurityPolicyViolationEventFields<'a>
{
    fn from(violation: &'a ContentSecurityPolicyUrlViolation) -> Self {
        Self::from_url_violation(violation)
    }
}
