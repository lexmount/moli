use crate::{
    ContentSecurityPolicy, ContentSecurityPolicyReportingEndpoints,
    ContentSecurityPolicyViolationEventFields,
};
use serde_json::json;
use url::Url;

pub fn content_security_policy_report_uri_endpoints(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
) -> Vec<String> {
    if policy.sources("report-to").is_some() {
        return Vec::new();
    }
    policy
        .sources("report-uri")
        .into_iter()
        .flatten()
        .filter_map(|endpoint| protected_url.join(endpoint).ok())
        .filter(|endpoint| matches!(endpoint.scheme(), "http" | "https"))
        .map(|endpoint| endpoint.to_string())
        .collect()
}

pub fn content_security_policy_reporting_endpoints_from_headers(
    headers: &[(String, Vec<u8>)],
    protected_url: &Url,
) -> ContentSecurityPolicyReportingEndpoints {
    let mut endpoints = ContentSecurityPolicyReportingEndpoints::default();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("reporting-endpoints"))
    {
        parse_reporting_endpoints_header(
            &moli_header_field::decode_header_value(value),
            protected_url,
            &mut endpoints,
        );
    }
    endpoints
}

fn parse_reporting_endpoints_header(
    value: &str,
    protected_url: &Url,
    endpoints: &mut ContentSecurityPolicyReportingEndpoints,
) {
    for member in split_quoted_header_list(value) {
        let Some((raw_group, raw_endpoint)) = member.split_once('=') else {
            continue;
        };
        let group = raw_group.trim();
        if group.is_empty() {
            continue;
        }
        let Some(endpoint) = parse_quoted_header_value(raw_endpoint.trim()) else {
            continue;
        };
        let Ok(endpoint) = protected_url.join(&endpoint) else {
            continue;
        };
        if !matches!(endpoint.scheme(), "http" | "https") {
            continue;
        }
        endpoints.insert(group, endpoint.to_string());
    }
}

fn split_quoted_header_list(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quote = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                let part = value[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let part = value[start..].trim();
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

fn parse_quoted_header_value(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            output.push(chars.next()?);
        } else {
            output.push(ch);
        }
    }
    Some(output)
}

pub fn content_security_policy_report_to_endpoints(
    policy: &ContentSecurityPolicy,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Vec<String> {
    let Some(sources) = policy.sources("report-to") else {
        return Vec::new();
    };
    let Some(group) = sources.first() else {
        return Vec::new();
    };
    reporting_endpoints
        .endpoint_for_group(group)
        .map(|endpoint| vec![endpoint.to_owned()])
        .unwrap_or_default()
}

pub fn content_security_policy_violation_report_body(
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> String {
    json!({
        "csp-report": {
            "document-uri": fields.document_uri,
            "referrer": fields.referrer,
            "violated-directive": fields.violated_directive,
            "effective-directive": fields.effective_directive,
            "original-policy": fields.original_policy,
            "disposition": fields.disposition.as_str(),
            "blocked-uri": fields.blocked_uri,
            "source-file": fields.source_file,
            "status-code": fields.status_code,
            "script-sample": fields.sample,
        }
    })
    .to_string()
}

pub fn content_security_policy_reporting_api_report_body(
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> String {
    let mut body = serde_json::Map::new();
    body.insert("documentURL".to_owned(), json!(fields.document_uri));
    if !fields.referrer.is_empty() {
        body.insert("referrer".to_owned(), json!(fields.referrer));
    }
    if !fields.blocked_uri.is_empty() {
        body.insert("blockedURL".to_owned(), json!(fields.blocked_uri));
    }
    body.insert(
        "effectiveDirective".to_owned(),
        json!(fields.effective_directive),
    );
    body.insert("originalPolicy".to_owned(), json!(fields.original_policy));
    if !fields.source_file.is_empty() {
        body.insert("sourceFile".to_owned(), json!(fields.source_file));
    }
    if !fields.sample.is_empty() {
        body.insert("sample".to_owned(), json!(fields.sample));
    }
    body.insert("disposition".to_owned(), json!(fields.disposition.as_str()));
    body.insert("statusCode".to_owned(), json!(fields.status_code));
    if fields.line_number != 0 {
        body.insert("lineNumber".to_owned(), json!(fields.line_number));
    }
    if fields.column_number != 0 {
        body.insert("columnNumber".to_owned(), json!(fields.column_number));
    }
    json!([{
        "age": 0,
        "type": "csp-violation",
        "url": fields.document_uri,
        "body": body,
    }])
    .to_string()
}

/// Strip a protected document URL for inclusion in a CSP violation report.
pub fn csp_url_for_report(url: &Url) -> String {
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return url.scheme().to_owned();
    }
    let mut stripped = url.clone();
    let _ = stripped.set_username("");
    let _ = stripped.set_password(None);
    stripped.set_fragment(None);
    stripped.to_string()
}

/// Strip a script source URL before exposing it through a CSP violation report.
///
/// This implements the URL-shaping portion of CSP's "Strip URL for use in
/// reports" algorithm. Script source locations may expose HTTP(S)/WS(S) paths,
/// but never credentials, queries, or fragments. Other valid schemes are
/// reduced to the scheme name.
pub fn content_security_policy_source_file_for_report(source_file: &str) -> String {
    let Ok(mut source_url) = Url::parse(source_file) else {
        return String::new();
    };
    if !matches!(source_url.scheme(), "http" | "https" | "ws" | "wss") {
        return source_url.scheme().to_owned();
    }
    let _ = source_url.set_username("");
    let _ = source_url.set_password(None);
    source_url.set_query(None);
    source_url.set_fragment(None);
    source_url.to_string()
}
