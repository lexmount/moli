use std::{collections::HashSet, sync::Arc};

use crate::ContentSecurityPolicyDisposition;
use ::content_security_policy::{Policy, PolicyDisposition, PolicySource};

/// A delivered policy keeps its original text for reports and shares its parsed
/// directives with inherited Documents, workers, and request snapshots.
#[derive(Clone, Debug)]
pub struct ContentSecurityPolicy(Arc<ParsedPolicy>);

#[derive(Debug)]
struct ParsedPolicy {
    serialized: String,
    policy: Policy,
}

impl ContentSecurityPolicy {
    pub(crate) fn parse(
        serialized: String,
        source: PolicySource,
        disposition: PolicyDisposition,
    ) -> Self {
        let mut policy = Policy::parse(&serialized, source, disposition);

        // The upstream parser skips a non-ASCII directive before checking for
        // duplicates. Moli delivery reserves the first name even when its
        // value is invalid. Keep that rule without tokenizing valid values a
        // second time. ASCII invalid values are retained by the parser until
        // validation below, so they already suppress their later duplicates.
        let invalid_first_names = if serialized.is_ascii() {
            HashSet::new()
        } else {
            let mut names = HashSet::new();
            serialized
                .split(';')
                .filter_map(|directive| {
                    let name = directive
                        .split_ascii_whitespace()
                        .next()?
                        .to_ascii_lowercase();
                    (names.insert(name.clone()) && !directive.is_ascii()).then_some(name)
                })
                .collect()
        };
        policy.directive_set.retain(|directive| {
            directive.is_valid() && !invalid_first_names.contains(&directive.name)
        });

        Self(Arc::new(ParsedPolicy { serialized, policy }))
    }

    /// Parse one HTTP policy while retaining its delivered text for reports.
    pub fn from_header(serialized: String, disposition: ContentSecurityPolicyDisposition) -> Self {
        Self::parse(
            serialized,
            PolicySource::Header,
            match disposition {
                ContentSecurityPolicyDisposition::Enforce => PolicyDisposition::Enforce,
                ContentSecurityPolicyDisposition::Report => PolicyDisposition::Report,
            },
        )
    }

    pub fn from_meta(serialized: String) -> Self {
        Self::parse(serialized, PolicySource::Meta, PolicyDisposition::Enforce)
    }

    pub fn as_str(&self) -> &str {
        &self.0.serialized
    }

    fn directives(&self) -> &[::content_security_policy::Directive] {
        &self.0.policy.directive_set
    }

    pub(crate) fn sources(&self, name: &str) -> Option<&[String]> {
        self.directives()
            .iter()
            .find(|directive| directive.name == name)
            .map(|directive| directive.value.as_slice())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.directives().is_empty()
    }
}

impl From<String> for ContentSecurityPolicy {
    fn from(serialized: String) -> Self {
        Self::from_header(serialized, ContentSecurityPolicyDisposition::Enforce)
    }
}

impl From<&str> for ContentSecurityPolicy {
    fn from(serialized: &str) -> Self {
        Self::from(serialized.to_owned())
    }
}

impl PartialEq for ContentSecurityPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
            && self.0.policy.source == other.0.policy.source
            && self.0.policy.disposition == other.0.policy.disposition
    }
}

impl Eq for ContentSecurityPolicy {}

pub fn content_security_policy_headers(
    headers: &[(String, Vec<u8>)],
) -> Vec<ContentSecurityPolicy> {
    content_security_policy_header_values(headers, "content-security-policy")
}

pub fn content_security_policy_report_only_headers(
    headers: &[(String, Vec<u8>)],
) -> Vec<ContentSecurityPolicy> {
    content_security_policy_header_values(headers, "content-security-policy-report-only")
}

pub(crate) fn content_security_policy_header_values(
    headers: &[(String, Vec<u8>)],
    header_name: &str,
) -> Vec<ContentSecurityPolicy> {
    let headers = moli_header_field::headers_to_byte_strings(headers);
    // Each HTTP field can contain several independently enforced policies.
    // Commas are CSP list delimiters even inside quotes in directive values;
    // the quoted-string rules used by Reporting-Endpoints do not apply here.
    // https://w3c.github.io/webappsec-csp/#parse-response-csp
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(header_name))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim_ascii)
        .map(|policy| {
            ContentSecurityPolicy::from_header(
                policy.to_owned(),
                if header_name == "content-security-policy-report-only" {
                    ContentSecurityPolicyDisposition::Report
                } else {
                    ContentSecurityPolicyDisposition::Enforce
                },
            )
        })
        .filter(|policy| !policy.is_empty())
        .collect()
}

pub(crate) fn directive_source_list<'policy>(
    policy: &'policy ContentSecurityPolicy,
    name: &str,
) -> Option<Vec<&'policy str>> {
    policy
        .sources(name)
        .map(|sources| sources.iter().map(String::as_str).collect())
}
