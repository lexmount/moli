use moli_fetch::validate_cors_response_for_origin;

use moli_cookie_jar::same_site_urls;
use moli_fetch::{RedirectSource, RequestCredentialsMode, RequestMode};
use moli_url::WebOrigin;
use moli_web_mime::{
    response_header_value, response_header_values, should_opaque_response_be_blocked_by_orb,
    should_opaque_response_be_blocked_by_orb_with_body,
};

use crate::cross_origin_isolation::{CrossOriginEmbedderPolicy, DocumentIsolationPolicy};

const CORS_SAFELISTED_RESPONSE_HEADER_NAMES: &[&str] = &[
    "cache-control",
    "content-language",
    "content-length",
    "content-type",
    "expires",
    "last-modified",
    "pragma",
];

#[derive(Debug)]
pub(crate) enum FetchResponseSecurityViolation {
    Rejected(String),
    OpaqueResponseBlocked(String),
}

impl FetchResponseSecurityViolation {
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::Rejected(message) | Self::OpaqueResponseBlocked(message) => message,
        }
    }
}

pub(crate) fn is_cors_policy_failure_message(message: &str) -> bool {
    message.contains("CORS check failed:") || message.contains("CORS preflight failed:")
}

/// Validates an already fetched network response and its network redirects.
/// Service worker and browser-generated redirects still contribute to taint,
/// but their synthetic response headers are not subject to the CORS check.
/// Once CORS-tainted, a redirect across origins changes the request origin to
/// null, even when the final URL returns to the initiating document's origin.
pub(crate) fn validate_cors_response_chain(
    request_origin: impl Into<WebOrigin>,
    head: &moli_fetch::ResponseHead,
    credentials_mode: RequestCredentialsMode,
) -> Result<(), String> {
    let request_origin = request_origin.into();
    for (index, redirect) in head.redirect_chain.iter().enumerate() {
        // This response precedes the redirect: only earlier hops contribute to
        // the Origin that was sent for it.
        let urls = moli_fetch::FetchUrlList::new(&redirect.from_url, &head.redirect_chain[..index]);
        if redirect.source == RedirectSource::Network && urls.has_cross_origin_url(&request_origin)
        {
            validate_cors_response_for_origin(
                &urls.serialized_origin(&request_origin),
                &redirect.headers,
                credentials_mode,
            )?;
        }
    }
    let urls = head.url_list();
    if matches!(head.final_url.scheme(), "http" | "https")
        && urls.has_cross_origin_url(&request_origin)
    {
        validate_cors_response_for_origin(
            &urls.serialized_origin(&request_origin),
            &head.headers,
            credentials_mode,
        )?;
    }
    Ok(())
}

pub(crate) fn validate_fetch_response_security_policy(
    request_origin: impl Into<WebOrigin>,
    head: &moli_fetch::ResponseHead,
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    let request_origin = request_origin.into();
    validate_fetch_response_headers(
        &request_origin,
        head,
        request_mode,
        credentials_mode,
        policy_context,
    )?;
    if request_mode == RequestMode::NoCors {
        validate_opaque_response_blocking(&request_origin, &head.final_url, &head.headers)
    } else {
        Ok(())
    }
}

/// Header policies precede response delivery; ORB gates the opaque internal
/// body separately so an unfinished download does not hold up fetch().
pub(crate) fn validate_fetch_response_headers(
    request_origin: &WebOrigin,
    head: &moli_fetch::ResponseHead,
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    head.url_list()
        .validate_request_mode(request_mode, request_origin)?;
    if request_mode == RequestMode::SameOrigin {
        return Ok(());
    }
    if request_mode == RequestMode::NoCors {
        validate_cross_origin_resource_policy(request_origin, &head.final_url, &head.headers)?;
        validate_cross_origin_embedder_and_document_isolation_policy(
            request_origin,
            &head.final_url,
            &head.headers,
            request_mode,
            credentials_mode,
            policy_context.cross_origin_embedder_policy,
            policy_context.document_isolation_policy,
        )
    } else {
        validate_cors_response_chain(request_origin, head, credentials_mode)
    }
}

pub(crate) fn fetch_response_needs_orb_body_validation(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
) -> bool {
    request_mode == RequestMode::NoCors
        && matches!(response_url.scheme(), "http" | "https")
        && !request_origin.into().same_origin_url(response_url)
        && should_opaque_response_be_blocked_by_orb(response_headers)
}

pub(crate) fn validated_opaque_response_body<'a>(
    response_headers: &[(String, String)],
    body: &'a crate::protocol_types::SubresourceResponseBody,
) -> Result<std::borrow::Cow<'a, [u8]>, FetchResponseSecurityViolation> {
    let bytes = body.try_bytes().map_err(|error| {
        FetchResponseSecurityViolation::Rejected(format!(
            "fetch: failed to read response body: {error}"
        ))
    })?;
    if should_opaque_response_be_blocked_by_orb_with_body(response_headers, &bytes) {
        return Err(FetchResponseSecurityViolation::OpaqueResponseBlocked(
            crate::network_host::ABORTED_ERROR_TEXT.to_owned(),
        ));
    }
    Ok(bytes)
}

pub(crate) fn validate_fetch_response_security_policy_with_body(
    request_origin: impl Into<WebOrigin>,
    head: &moli_fetch::ResponseHead,
    response_body: &[u8],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    let request_origin = request_origin.into();
    validate_fetch_response_security_policy_with_body_classified(
        &request_origin,
        head,
        response_body,
        request_mode,
        credentials_mode,
        policy_context,
    )
    .map_err(FetchResponseSecurityViolation::into_message)
}

pub(crate) fn validate_fetch_response_security_policy_with_body_classified(
    request_origin: impl Into<WebOrigin>,
    head: &moli_fetch::ResponseHead,
    response_body: &[u8],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), FetchResponseSecurityViolation> {
    let request_origin = request_origin.into();
    head.url_list()
        .validate_request_mode(request_mode, &request_origin)
        .map_err(FetchResponseSecurityViolation::Rejected)?;
    if request_mode == RequestMode::SameOrigin {
        return Ok(());
    }
    if request_mode == RequestMode::NoCors {
        validate_cross_origin_resource_policy(&request_origin, &head.final_url, &head.headers)
            .map_err(FetchResponseSecurityViolation::Rejected)?;
        validate_cross_origin_embedder_and_document_isolation_policy(
            &request_origin,
            &head.final_url,
            &head.headers,
            request_mode,
            credentials_mode,
            policy_context.cross_origin_embedder_policy,
            policy_context.document_isolation_policy,
        )
        .map_err(FetchResponseSecurityViolation::Rejected)?;
        validate_opaque_response_blocking_with_body(
            &request_origin,
            &head.final_url,
            &head.headers,
            response_body,
        )
        .map_err(FetchResponseSecurityViolation::OpaqueResponseBlocked)
    } else {
        validate_cors_response_chain(&request_origin, head, credentials_mode)
            .map_err(FetchResponseSecurityViolation::Rejected)
    }
}

pub(crate) fn validate_opaque_response_blocking(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    response_headers: &[(String, String)],
) -> Result<(), String> {
    let request_origin = request_origin.into();
    if !matches!(response_url.scheme(), "http" | "https")
        || request_origin.same_origin(&response_url.into())
        || !should_opaque_response_be_blocked_by_orb(response_headers)
    {
        return Ok(());
    }

    Err(format!(
        "OpaqueResponseBlocking check failed: {} cannot load {response_url} as an opaque no-cors response",
        request_origin.ascii_serialization()
    ))
}

pub(crate) fn validate_opaque_response_blocking_with_body(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &[u8],
) -> Result<(), String> {
    let request_origin = request_origin.into();
    if !matches!(response_url.scheme(), "http" | "https")
        || request_origin.same_origin(&response_url.into())
        || !should_opaque_response_be_blocked_by_orb_with_body(response_headers, response_body)
    {
        return Ok(());
    }

    Err(format!(
        "OpaqueResponseBlocking check failed: {} cannot load {response_url} as an opaque no-cors response",
        request_origin.ascii_serialization()
    ))
}

#[cfg(test)]
fn validate_cross_origin_embedder_policy(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    embedder_policy: CrossOriginEmbedderPolicy,
) -> Result<(), String> {
    let request_origin = request_origin.into();
    validate_cross_origin_embedder_and_document_isolation_policy(
        &request_origin,
        response_url,
        response_headers,
        request_mode,
        credentials_mode,
        embedder_policy,
        DocumentIsolationPolicy::None,
    )
}

pub(crate) fn validate_cross_origin_embedder_and_document_isolation_policy(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    embedder_policy: CrossOriginEmbedderPolicy,
    document_isolation_policy: DocumentIsolationPolicy,
) -> Result<(), String> {
    let request_origin = request_origin.into();
    if request_mode != RequestMode::NoCors
        || !matches!(response_url.scheme(), "http" | "https")
        || request_origin.same_origin(&response_url.into())
    {
        return Ok(());
    }

    let request_includes_credentials =
        request_includes_credentials(&request_origin, response_url, credentials_mode);
    let requires_corp_due_to_coep = match embedder_policy {
        CrossOriginEmbedderPolicy::None => false,
        CrossOriginEmbedderPolicy::RequireCorp => true,
        CrossOriginEmbedderPolicy::Credentialless => {
            request_mode == RequestMode::Navigate || request_includes_credentials
        }
    };
    let requires_corp_due_to_dip = match document_isolation_policy {
        DocumentIsolationPolicy::None => false,
        DocumentIsolationPolicy::IsolateAndRequireCorp => true,
        DocumentIsolationPolicy::IsolateAndCredentialless => {
            request_mode == RequestMode::Navigate || request_includes_credentials
        }
    };
    let requires_corp = requires_corp_due_to_coep || requires_corp_due_to_dip;
    if !requires_corp {
        return Ok(());
    }

    let policy_label = defaulted_corp_policy_label(
        requires_corp_due_to_coep,
        embedder_policy,
        requires_corp_due_to_dip,
        document_isolation_policy,
    );
    let Some(policy) = response_header_value(response_headers, "cross-origin-resource-policy")
    else {
        return Err(format!(
            "{policy_label} check failed: requires Cross-Origin-Resource-Policy for {} to load {response_url}",
            request_origin.ascii_serialization()
        ));
    };
    let policy = policy.trim().to_ascii_lowercase();
    match policy.as_str() {
        "same-origin" | "same-site" | "cross-origin" => {
            validate_cross_origin_resource_policy(&request_origin, response_url, response_headers)
        }
        _ => Err(format!(
            "{policy_label} check failed: treats invalid Cross-Origin-Resource-Policy `{policy}` as same-origin for {} to load {response_url}",
            request_origin.ascii_serialization()
        )),
    }
}

fn defaulted_corp_policy_label(
    requires_corp_due_to_coep: bool,
    embedder_policy: CrossOriginEmbedderPolicy,
    requires_corp_due_to_dip: bool,
    document_isolation_policy: DocumentIsolationPolicy,
) -> String {
    match (requires_corp_due_to_coep, requires_corp_due_to_dip) {
        (true, true) => format!(
            "Cross-Origin-Embedder-Policy `{}` and Document-Isolation-Policy `{}`",
            embedder_policy.label(),
            document_isolation_policy.label()
        ),
        (true, false) => {
            format!("Cross-Origin-Embedder-Policy `{}`", embedder_policy.label())
        }
        (false, true) => format!(
            "Document-Isolation-Policy `{}`",
            document_isolation_policy.label()
        ),
        (false, false) => "No policy".to_owned(),
    }
}

fn request_includes_credentials(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    credentials_mode: RequestCredentialsMode,
) -> bool {
    let request_origin = request_origin.into();
    match credentials_mode {
        RequestCredentialsMode::Include => true,
        RequestCredentialsMode::Omit => false,
        RequestCredentialsMode::SameOrigin => request_origin.same_origin(&response_url.into()),
    }
}

pub(crate) fn validate_cross_origin_resource_policy(
    request_origin: impl Into<WebOrigin>,
    response_url: &url::Url,
    response_headers: &[(String, String)],
) -> Result<(), String> {
    let request_origin = request_origin.into();
    if !matches!(response_url.scheme(), "http" | "https") {
        return Ok(());
    }
    let Some(policy) = response_header_value(response_headers, "cross-origin-resource-policy")
    else {
        return Ok(());
    };
    let policy = policy.trim().to_ascii_lowercase();
    let allowed = match policy.as_str() {
        "same-origin" => request_origin.same_origin(&response_url.into()),
        "same-site" => url::Url::parse(request_origin.ascii_serialization())
            .is_ok_and(|origin| same_site_urls(&origin, response_url, true)),
        "cross-origin" => true,
        _ => true,
    };
    if allowed {
        return Ok(());
    }
    Err(format!(
        "Cross-Origin-Resource-Policy check failed: `{policy}` does not allow {} to load {response_url}",
        request_origin.ascii_serialization()
    ))
}

pub(crate) fn cors_preflight_request_headers(
    cors_tainted: bool,
    request_url: &url::Url,
    method: &str,
    request_headers: &[(String, String)],
    use_cors_preflight: bool,
) -> Option<Vec<(String, String)>> {
    if !cors_tainted {
        return None;
    }
    if !matches!(request_url.scheme(), "http" | "https") {
        return None;
    }

    let unsafe_header_names = moli_fetch::cors_unsafe_request_header_names(request_headers);
    let method_requires_preflight = !moli_fetch::is_cors_safelisted_method(method);
    if !use_cors_preflight && !method_requires_preflight && unsafe_header_names.is_empty() {
        return None;
    }

    let mut headers = vec![(
        "Access-Control-Request-Method".to_owned(),
        method.to_owned(),
    )];
    if !unsafe_header_names.is_empty() {
        headers.push((
            "Access-Control-Request-Headers".to_owned(),
            unsafe_header_names.join(","),
        ));
    }
    Some(headers)
}

pub(crate) fn validate_cors_preflight_response(
    origin: &str,
    credentials_mode: RequestCredentialsMode,
    requested_method: &str,
    request_headers: &[(String, String)],
    response_status: u16,
    response_headers: &[(String, String)],
    use_cors_preflight: bool,
) -> Result<(), String> {
    if !(200..300).contains(&response_status) {
        return Err(format!(
            "CORS preflight failed: response status {response_status}"
        ));
    }
    validate_cors_response_for_origin(origin, response_headers, credentials_mode)?;

    // Parse both complete lists before checking permissions, including for
    // safelisted methods and requests without unsafe header names.
    let mut allow_methods =
        parse_cors_preflight_allowlist(response_headers, "Access-Control-Allow-Methods")?;
    let allow_headers =
        parse_cors_preflight_allowlist(response_headers, "Access-Control-Allow-Headers")?;
    if allow_methods.is_none() && use_cors_preflight {
        allow_methods = Some(vec![requested_method.to_owned()]);
    }
    let wildcard_allowed = credentials_mode != RequestCredentialsMode::Include;

    if !moli_fetch::is_cors_safelisted_method(requested_method) {
        let Some(allow_methods) = allow_methods else {
            return Err(format!(
                "CORS preflight failed: no Access-Control-Allow-Methods for {requested_method}"
            ));
        };
        if !allow_methods
            .iter()
            .any(|method| method == requested_method || (wildcard_allowed && method == "*"))
        {
            return Err(format!(
                "CORS preflight failed: Access-Control-Allow-Methods `{}` does not allow {requested_method}",
                allow_methods.join(",")
            ));
        }
    }

    let unsafe_header_names = moli_fetch::cors_unsafe_request_header_names(request_headers);
    if unsafe_header_names.is_empty() {
        return Ok(());
    }

    let Some(allow_headers) = allow_headers else {
        return Err(format!(
            "CORS preflight failed: no Access-Control-Allow-Headers for {}",
            unsafe_header_names.join(",")
        ));
    };
    let wildcard_headers = wildcard_allowed && allow_headers.iter().any(|name| name == "*");
    for header_name in unsafe_header_names {
        // Authorization is a CORS non-wildcard request-header name, so it
        // always needs an explicit, case-insensitive match.
        if !allow_headers
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&header_name))
            && (!wildcard_headers || header_name == "authorization")
        {
            return Err(format!(
                "CORS preflight failed: Access-Control-Allow-Headers `{}` does not allow {header_name}",
                allow_headers.join(",")
            ));
        }
    }

    Ok(())
}

pub(crate) fn filter_cors_exposed_response_headers(
    request_origin: impl Into<WebOrigin>,
    head: &moli_fetch::ResponseHead,
    credentials_mode: RequestCredentialsMode,
) -> Vec<(String, String)> {
    let request_origin = request_origin.into();
    let response_headers = &head.headers;
    if !head.url_list().has_cross_origin_url(&request_origin) {
        return response_headers.to_vec();
    }
    if !matches!(head.final_url.scheme(), "http" | "https") {
        return response_headers.to_vec();
    }

    let mut exposed_names = CORS_SAFELISTED_RESPONSE_HEADER_NAMES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut wildcard = false;
    for expose_value in response_header_values(response_headers, "access-control-expose-headers") {
        for token in expose_value.split(',') {
            let name = token.trim().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            if name == "*" {
                wildcard = true;
            } else if !exposed_names.iter().any(|existing| existing == &name) {
                exposed_names.push(name);
            }
        }
    }

    if wildcard && credentials_mode != RequestCredentialsMode::Include {
        return response_headers
            .iter()
            .filter(|(name, _)| !is_forbidden_response_header_name(name))
            .cloned()
            .collect();
    }

    response_headers
        .iter()
        .filter(|(name, _)| {
            !is_forbidden_response_header_name(name)
                && exposed_names
                    .iter()
                    .any(|exposed| name.eq_ignore_ascii_case(exposed))
        })
        .cloned()
        .collect()
}

fn parse_cors_preflight_allowlist(
    headers: &[(String, String)],
    name: &str,
) -> Result<Option<Vec<String>>, String> {
    let values = response_header_values(headers, name);
    if values.is_empty() {
        return Ok(None);
    }
    let mut tokens = Vec::new();
    for value in values {
        for token in value.split(',') {
            let token = token.trim_matches([' ', '\t']);
            if token.is_empty() {
                continue;
            }
            // Both method and field-name use HTTP token syntax. Preserve case
            // because method permissions require an exact match.
            if http::Method::from_bytes(token.as_bytes()).is_err() {
                return Err(format!(
                    "CORS preflight failed: invalid {name} value `{value}`"
                ));
            }
            tokens.push(token.to_owned());
        }
    }
    Ok(Some(tokens))
}

fn is_forbidden_response_header_name(name: &str) -> bool {
    parsed_header_name(name)
        .is_some_and(|name| moli_fetch::is_forbidden_response_header_name(name.as_str()))
}

fn parsed_header_name(name: &str) -> Option<http::HeaderName> {
    http::HeaderName::from_bytes(name.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_url::origin_ascii_serialization;

    fn url(value: &str) -> url::Url {
        url::Url::parse(value).expect("valid URL")
    }

    fn header_response(
        final_url: url::Url,
        headers: Vec<(String, String)>,
    ) -> moli_fetch::ResponseHead {
        moli_fetch::ResponseHead {
            final_url,
            status: 200,
            status_text: None,
            headers,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }
    }

    #[test]
    fn cors_response_chain_checks_network_redirects_without_extra_info() {
        for from_cache in [false, true] {
            let mut head = moli_fetch::ResponseHead {
                final_url: url("https://final.test/script.js"),
                status: 200,
                status_text: None,
                headers: vec![("Access-Control-Allow-Origin".to_owned(), "*".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: true,
                redirect_chain: vec![moli_fetch::RedirectInfo {
                    source: RedirectSource::Network,
                    from_url: url("https://redirect.test/script.js"),
                    to_url: url("https://final.test/script.js"),
                    status: 302,
                    headers: Vec::new(),
                    network_extra_info_available: false,
                    request_extra_info: None,
                    response_extra_info: None,
                    redirect_has_extra_info: false,
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    from_cache,
                    negotiated_http_version: None,
                }],
                from_cache: false,
                negotiated_http_version: None,
            };
            let document_url = url("https://document.test/page.html");
            validate_cors_response_chain(&document_url, &head, RequestCredentialsMode::SameOrigin)
                .expect_err("HTTP redirects require ACAO even without network ExtraInfo");

            head.redirect_chain[0].headers.push((
                "Access-Control-Allow-Origin".to_owned(),
                "https://document.test".to_owned(),
            ));
            validate_cors_response_chain(&document_url, &head, RequestCredentialsMode::SameOrigin)
                .expect("authorized HTTP redirect should pass");
        }
    }

    #[test]
    fn cors_response_fields_require_a_single_origin_value() {
        let response_url = url("https://other.test/data");
        for (origin, matching) in [
            (
                WebOrigin::from_url(&url("https://page.test/a")),
                "https://page.test",
            ),
            (WebOrigin::Opaque, "null"),
        ] {
            for mode in [
                RequestCredentialsMode::Omit,
                RequestCredentialsMode::SameOrigin,
                RequestCredentialsMode::Include,
            ] {
                for (values, allowed) in [
                    (vec![], false),
                    (vec![""], false),
                    (vec![matching], true),
                    (vec!["*"], mode != RequestCredentialsMode::Include),
                    (vec!["https://wrong.test"], false),
                    (vec![matching, matching], false),
                    (vec![matching, ""], false),
                    (vec!["", matching], false),
                    (vec!["*", matching], false),
                    (vec!["*", ""], false),
                    (vec!["*, *"], false),
                ] {
                    let mut headers = vec![(
                        "Access-Control-Allow-Credentials".to_owned(),
                        "true".to_owned(),
                    )];
                    headers.extend(values.iter().enumerate().map(|(index, value)| {
                        (
                            if index == 0 {
                                "Access-Control-Allow-Origin"
                            } else {
                                "access-control-allow-origin"
                            }
                            .to_owned(),
                            (*value).to_owned(),
                        )
                    }));
                    assert_eq!(
                        validate_cors_response_for_origin(&origin, &response_url, &headers, mode)
                            .is_ok(),
                        allowed,
                        "origin={matching}, mode={mode:?}, values={values:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn cors_response_fields_check_credentials_only_when_included() {
        let document_url = url("https://page.test/a");
        let response_url = url("https://other.test/data");
        for values in [
            vec![],
            vec![""],
            vec!["true"],
            vec!["TRUE"],
            vec!["True"],
            vec!["false"],
            vec!["true, true"],
            vec!["true", "true"],
            vec!["true", "false"],
            vec!["true", ""],
            vec!["", "true"],
        ] {
            let mut headers = vec![(
                "Access-Control-Allow-Origin".to_owned(),
                "https://page.test".to_owned(),
            )];
            headers.extend(values.iter().enumerate().map(|(index, value)| {
                (
                    if index == 0 {
                        "Access-Control-Allow-Credentials"
                    } else {
                        "ACCESS-CONTROL-ALLOW-CREDENTIALS"
                    }
                    .to_owned(),
                    (*value).to_owned(),
                )
            }));
            for mode in [
                RequestCredentialsMode::Omit,
                RequestCredentialsMode::SameOrigin,
                RequestCredentialsMode::Include,
            ] {
                assert_eq!(
                    validate_cors_response_for_origin(&WebOrigin::from_url(&document_url), &response_url, &headers, mode).is_ok(),
                    mode != RequestCredentialsMode::Include || values == ["true"],
                    "mode={mode:?}, values={values:?}"
                );
                assert!(
                    validate_cors_response_for_origin(&WebOrigin::from_url(&document_url), &document_url, &headers, mode).is_ok(),
                    "same-origin responses do not require CORS permission"
                );
            }
        }
    }

    #[test]
    fn cors_response_validation_uses_the_redirect_tainted_origin() {
        let home = url("https://page.test/a");
        let away = url("https://script.test/b");
        let elsewhere = url("https://other.test/c");
        let origin = WebOrigin::from_url(&home);
        for (redirects, serialized_origin) in [
            (vec![(&home, &away)], origin.ascii_serialization()),
            (vec![(&away, &elsewhere)], "null"),
            (vec![(&home, &away), (&away, &home)], "null"),
        ] {
            let final_url = redirects.last().expect("redirect target").1.clone();
            let mut head = header_response(
                final_url,
                vec![(
                    "Access-Control-Allow-Origin".to_owned(),
                    serialized_origin.to_owned(),
                )],
            );
            head.redirected = true;
            head.redirect_chain = redirects
                .into_iter()
                .map(|(from, to)| moli_fetch::RedirectInfo {
                    source: RedirectSource::Network,
                    from_url: from.clone(),
                    to_url: to.clone(),
                    status: 302,
                    headers: vec![(
                        "Access-Control-Allow-Origin".to_owned(),
                        origin.ascii_serialization().to_owned(),
                    )],
                    network_extra_info_available: false,
                    request_extra_info: None,
                    response_extra_info: None,
                    redirect_has_extra_info: false,
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                })
                .collect();
            assert_eq!(
                head.url_list().serialized_origin(&origin),
                serialized_origin
            );
            validate_cors_response_chain(&origin, &head, RequestCredentialsMode::SameOrigin)
                .expect("each response must be authorized against its own request hop origin");
            head.headers.clear();
            assert!(
                validate_cors_response_chain(&origin, &head, RequestCredentialsMode::SameOrigin)
                    .is_err(),
                "returning to the initial origin must not bypass CORS"
            );
        }
    }

    #[test]
    fn cors_preflight_request_headers_detect_unsafe_method_and_headers() {
        let headers = vec![
            ("Accept".to_owned(), "*/*".to_owned()),
            (
                "Content-Type".to_owned(),
                "text/plain;charset=UTF-8".to_owned(),
            ),
            ("X-Test".to_owned(), "yes".to_owned()),
            ("x-test".to_owned(), "again".to_owned()),
            ("X-Other".to_owned(), "ok".to_owned()),
        ];

        let preflight = cors_preflight_request_headers(
            true,
            &url("http://other.test/data"),
            "PUT",
            &headers,
            false,
        );

        assert_eq!(
            preflight,
            Some(vec![
                ("Access-Control-Request-Method".to_owned(), "PUT".to_owned()),
                (
                    "Access-Control-Request-Headers".to_owned(),
                    "x-other,x-test".to_owned()
                ),
            ])
        );
    }

    #[test]
    fn cors_preflight_request_headers_skip_simple_cross_origin_request() {
        let headers = vec![
            ("Accept".to_owned(), "*/*".to_owned()),
            ("Range".to_owned(), "bytes=0-1".to_owned()),
            (
                "Content-Type".to_owned(),
                "application/x-www-form-urlencoded;charset=UTF-8".to_owned(),
            ),
        ];

        assert_eq!(
            cors_preflight_request_headers(
                true,
                &url("http://other.test/data"),
                "POST",
                &headers,
                false
            ),
            None
        );
    }

    #[test]
    fn opaque_request_origin_requires_null_cors_opt_in_for_same_url_origin() {
        let response_url = url("https://example.test/data");
        let response = header_response(response_url.clone(), Vec::new());

        assert!(
            validate_cors_response_chain(
                &WebOrigin::Opaque,
                &response,
                RequestCredentialsMode::SameOrigin,
            )
            .is_err()
        );

        let allowed_response = header_response(
            response_url,
            vec![("Access-Control-Allow-Origin".to_owned(), "null".to_owned())],
        );
        assert!(
            validate_cors_response_chain(
                &WebOrigin::Opaque,
                &allowed_response,
                RequestCredentialsMode::SameOrigin,
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_cors_preflight_response_checks_method_and_headers() {
        let request_headers = vec![
            ("X-Test".to_owned(), "yes".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ];
        let response_headers = vec![
            (
                "Access-Control-Allow-Origin".to_owned(),
                "http://example.test".to_owned(),
            ),
            (
                "Access-Control-Allow-Methods".to_owned(),
                "POST, PUT".to_owned(),
            ),
            (
                "Access-Control-Allow-Headers".to_owned(),
                "content-type, x-test".to_owned(),
            ),
        ];

        assert_eq!(
            validate_cors_preflight_response(
                "http://example.test",
                RequestCredentialsMode::SameOrigin,
                "POST",
                &request_headers,
                204,
                &response_headers,
                false,
            ),
            Ok(())
        );
    }

    #[test]
    fn validate_cors_preflight_response_allows_safelisted_methods_without_allow_methods() {
        let document_url = url::Url::parse("https://origin.test/page").unwrap();
        let response_headers = vec![
            ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
            (
                "Access-Control-Allow-Headers".to_owned(),
                "content-type".to_owned(),
            ),
        ];

        for method in ["GET", "HEAD", "POST"] {
            validate_cors_preflight_response(
                &origin_ascii_serialization(&document_url),
                RequestCredentialsMode::SameOrigin,
                method,
                &[("Content-Type".to_owned(), "custom/type".to_owned())],
                200,
                &response_headers,
                false,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "safelisted {method} preflight should not require Access-Control-Allow-Methods: {error}"
                )
            });
        }
    }

    #[test]
    fn validate_cors_preflight_response_rejects_unsafelisted_method_without_allow_methods() {
        let document_url = url::Url::parse("https://origin.test/page").unwrap();
        let response_headers = vec![
            ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
            (
                "Access-Control-Allow-Headers".to_owned(),
                "content-type".to_owned(),
            ),
        ];

        let error = validate_cors_preflight_response(
            &origin_ascii_serialization(&document_url),
            RequestCredentialsMode::SameOrigin,
            "PUT",
            &[("Content-Type".to_owned(), "custom/type".to_owned())],
            200,
            &response_headers,
            false,
        )
        .expect_err("unsafelisted PUT preflight should require Access-Control-Allow-Methods");
        assert!(error.contains("no Access-Control-Allow-Methods for PUT"));
    }

    fn preflight_permissions(
        method: &str,
        request_headers: &[(&str, &str)],
        permissions: &[(&str, &str)],
        credentials_mode: RequestCredentialsMode,
    ) -> Result<(), String> {
        let request_headers = request_headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>();
        let mut response_headers = vec![
            (
                "Access-Control-Allow-Origin".to_owned(),
                "https://origin.test".to_owned(),
            ),
            (
                "Access-Control-Allow-Credentials".to_owned(),
                "true".to_owned(),
            ),
        ];
        response_headers.extend(
            permissions
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        let origin = WebOrigin::from_url(&url("https://origin.test/page"));
        validate_cors_preflight_response(
            origin.ascii_serialization(),
            credentials_mode,
            method,
            &request_headers,
            204,
            &response_headers,
            false,
        )
    }

    #[test]
    fn cors_preflight_permissions_combine_fields_without_folding_method_case() {
        let permissions = [
            ("Access-Control-Allow-Methods", "POST,,"),
            ("access-control-allow-methods", "\t patcH, \t"),
            ("Access-Control-Allow-Headers", "X-First"),
            ("ACCESS-CONTROL-ALLOW-HEADERS", ",\t X-SECOND,,"),
        ];
        for credentials in [
            RequestCredentialsMode::Omit,
            RequestCredentialsMode::Include,
        ] {
            let headers = [("x-first", "1"), ("x-second", "2")];
            assert_eq!(
                preflight_permissions("patcH", &headers, &permissions, credentials),
                Ok(())
            );
            assert!(preflight_permissions("PATCH", &headers, &permissions, credentials).is_err());
        }
    }

    #[test]
    fn cors_preflight_permissions_validate_both_complete_lists_before_safelists() {
        for field in [
            "Access-Control-Allow-Methods",
            "Access-Control-Allow-Headers",
        ] {
            for invalid in [
                "Bad value",
                "\"GET\"",
                "GET:POST",
                "GET;POST",
                "GET\u{00a0}",
                "\u{000b}GET",
                "GET\r\n",
                "GÉT",
            ] {
                let fields = [(field, "GET, X-Test"), (field, invalid)];
                assert!(
                    preflight_permissions("GET", &[], &fields, RequestCredentialsMode::Omit)
                        .is_err(),
                    "a later malformed {field} must reject even a safelisted request: {invalid:?}"
                );
                assert!(
                    preflight_permissions(
                        "GET",
                        &[("X-Test", "1")],
                        &fields,
                        RequestCredentialsMode::Omit
                    )
                    .is_err(),
                    "an earlier matching token must not hide a malformed {field}: {invalid:?}"
                );
            }
        }
    }

    #[test]
    fn cors_preflight_permissions_wildcards_respect_credentials_and_authorization() {
        let wildcards = [
            ("Access-Control-Allow-Methods", "*"),
            ("Access-Control-Allow-Headers", "*"),
        ];
        for credentials in [
            RequestCredentialsMode::Omit,
            RequestCredentialsMode::SameOrigin,
            RequestCredentialsMode::Include,
        ] {
            assert_eq!(
                preflight_permissions("PUT", &[("X-Test", "1")], &wildcards, credentials).is_ok(),
                credentials != RequestCredentialsMode::Include
            );
            assert_eq!(
                preflight_permissions("*", &[("*", "1")], &wildcards, credentials),
                Ok(())
            );
            assert!(
                preflight_permissions(
                    "POST",
                    &[("aUtHoRiZaTiOn", "secret")],
                    &wildcards,
                    credentials
                )
                .is_err()
            );
            assert_eq!(
                preflight_permissions(
                    "POST",
                    &[("Authorization", "secret")],
                    &[
                        ("Access-Control-Allow-Headers", "*"),
                        ("Access-Control-Allow-Headers", "AUTHORIZATION"),
                    ],
                    credentials
                ),
                Ok(())
            );
        }
    }

    #[test]
    fn cors_preflight_permissions_accept_http_tokens_and_empty_lists() {
        let token = "!#$%&'*+-.^_`|~0123456789AZaz";
        assert_eq!(
            preflight_permissions(
                token,
                &[(token, "1")],
                &[
                    ("Access-Control-Allow-Methods", token),
                    ("Access-Control-Allow-Headers", token),
                ],
                RequestCredentialsMode::Include
            ),
            Ok(())
        );
        for empty in ["", " \t ", ",, \t,"] {
            let fields = [
                ("Access-Control-Allow-Methods", empty),
                ("Access-Control-Allow-Headers", empty),
            ];
            assert_eq!(
                preflight_permissions("GET", &[], &fields, RequestCredentialsMode::Omit),
                Ok(())
            );
            assert!(
                preflight_permissions("PUT", &[], &fields, RequestCredentialsMode::Omit).is_err()
            );
            assert!(
                preflight_permissions(
                    "GET",
                    &[("X-Test", "1")],
                    &fields,
                    RequestCredentialsMode::Omit
                )
                .is_err()
            );
        }
    }

    #[test]
    fn cors_exposed_headers_keep_safelisted_and_explicit_names() {
        let headers = vec![
            ("Content-Type".to_owned(), "text/plain".to_owned()),
            ("Content-Language".to_owned(), "en".to_owned()),
            ("X-Visible".to_owned(), "yes".to_owned()),
            ("X-Hidden".to_owned(), "no".to_owned()),
            (
                "Access-Control-Expose-Headers".to_owned(),
                "X-Visible".to_owned(),
            ),
            ("Set-Cookie".to_owned(), "secret=1".to_owned()),
        ];

        let filtered = filter_cors_exposed_response_headers(
            &url("http://example.test/page"),
            &header_response(url("http://other.test/data"), headers.clone()),
            RequestCredentialsMode::SameOrigin,
        );

        assert_eq!(
            filtered,
            vec![
                ("Content-Type".to_owned(), "text/plain".to_owned()),
                ("Content-Language".to_owned(), "en".to_owned()),
                ("X-Visible".to_owned(), "yes".to_owned()),
            ]
        );
    }

    #[test]
    fn cors_exposed_headers_wildcard_does_not_apply_to_credentials_include() {
        let headers = vec![
            ("Content-Type".to_owned(), "text/plain".to_owned()),
            ("X-Wildcard".to_owned(), "yes".to_owned()),
            ("Access-Control-Expose-Headers".to_owned(), "*".to_owned()),
        ];

        let non_credentialed = filter_cors_exposed_response_headers(
            &url("http://example.test/page"),
            &header_response(url("http://other.test/data"), headers.clone()),
            RequestCredentialsMode::SameOrigin,
        );
        assert_eq!(non_credentialed, headers);

        let credentialed = filter_cors_exposed_response_headers(
            &url("http://example.test/page"),
            &header_response(url("http://other.test/data"), headers.clone()),
            RequestCredentialsMode::Include,
        );
        assert_eq!(
            credentialed,
            vec![("Content-Type".to_owned(), "text/plain".to_owned())]
        );
    }

    #[test]
    fn cors_exposed_headers_same_origin_keeps_existing_surface() {
        let headers = vec![
            ("X-Internal".to_owned(), "ok".to_owned()),
            (
                "Set-Cookie".to_owned(),
                "kept-for-current-surface".to_owned(),
            ),
        ];

        let filtered = filter_cors_exposed_response_headers(
            &url("http://example.test/page"),
            &header_response(url("http://example.test/data"), headers.clone()),
            RequestCredentialsMode::Include,
        );

        assert_eq!(filtered, headers);
    }

    #[test]
    fn cors_response_header_lookup_uses_http_header_names() {
        let headers = vec![
            ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
            ("Bad Header".to_owned(), "ignored".to_owned()),
        ];

        assert_eq!(
            response_header_value(&headers, "access-control-allow-origin"),
            Some("*".to_owned())
        );
        assert_eq!(response_header_value(&headers, "Bad Header"), None);
        assert!(is_forbidden_response_header_name("Set-Cookie"));
        assert!(is_forbidden_response_header_name("set-cookie2"));
    }

    #[test]
    fn corp_same_origin_blocks_cross_origin_no_cors_response() {
        let error = validate_cross_origin_resource_policy(
            &url("https://example.test/page"),
            &url("https://cdn.test/data"),
            &[(
                "Cross-Origin-Resource-Policy".to_owned(),
                "same-origin".to_owned(),
            )],
        )
        .expect_err("cross-origin response should be blocked");

        assert!(error.contains("Cross-Origin-Resource-Policy"));
    }

    #[test]
    fn corp_same_site_uses_schemeful_site_comparison() {
        let same_site = validate_cross_origin_resource_policy(
            &url("https://app.example.test/page"),
            &url("https://cdn.example.test/data"),
            &[(
                "Cross-Origin-Resource-Policy".to_owned(),
                "same-site".to_owned(),
            )],
        );
        assert!(same_site.is_ok());

        let cross_scheme = validate_cross_origin_resource_policy(
            &url("https://app.example.test/page"),
            &url("http://cdn.example.test/data"),
            &[(
                "Cross-Origin-Resource-Policy".to_owned(),
                "same-site".to_owned(),
            )],
        );
        assert!(cross_scheme.is_err());
    }

    #[test]
    fn coep_credentialless_requires_corp_only_when_request_includes_credentials() {
        let document_url = url("https://example.test/page");
        let response_url = url("https://cdn.test/data");

        let non_credentialed = validate_cross_origin_embedder_policy(
            &document_url,
            &response_url,
            &[],
            RequestMode::NoCors,
            RequestCredentialsMode::SameOrigin,
            CrossOriginEmbedderPolicy::Credentialless,
        );
        assert!(non_credentialed.is_ok());

        let credentialed = validate_cross_origin_embedder_policy(
            &document_url,
            &response_url,
            &[],
            RequestMode::NoCors,
            RequestCredentialsMode::Include,
            CrossOriginEmbedderPolicy::Credentialless,
        )
        .expect_err("credentialed no-cors response should require CORP");
        assert!(credentialed.contains("credentialless"));
    }

    #[test]
    fn document_isolation_policy_requires_corp_for_cross_origin_no_cors_responses() {
        let document_url = url("https://example.test/page");
        let response_url = url("https://cdn.test/data");

        let error = validate_cross_origin_embedder_and_document_isolation_policy(
            &document_url,
            &response_url,
            &[],
            RequestMode::NoCors,
            RequestCredentialsMode::SameOrigin,
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::IsolateAndRequireCorp,
        )
        .expect_err("DIP isolate-and-require-corp should require CORP");
        assert!(error.contains("Document-Isolation-Policy"));
        assert!(error.contains("isolate-and-require-corp"));
    }

    #[test]
    fn document_isolation_policy_credentialless_requires_corp_only_with_credentials() {
        let document_url = url("https://example.test/page");
        let response_url = url("https://cdn.test/data");

        let non_credentialed = validate_cross_origin_embedder_and_document_isolation_policy(
            &document_url,
            &response_url,
            &[],
            RequestMode::NoCors,
            RequestCredentialsMode::SameOrigin,
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::IsolateAndCredentialless,
        );
        assert!(non_credentialed.is_ok());

        let credentialed = validate_cross_origin_embedder_and_document_isolation_policy(
            &document_url,
            &response_url,
            &[],
            RequestMode::NoCors,
            RequestCredentialsMode::Include,
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::IsolateAndCredentialless,
        )
        .expect_err("DIP isolate-and-credentialless should require CORP for credentials");
        assert!(credentialed.contains("Document-Isolation-Policy"));
        assert!(credentialed.contains("isolate-and-credentialless"));
    }

    #[test]
    fn orb_blocks_cross_origin_no_cors_blocklisted_mime_types() {
        let error = validate_opaque_response_blocking(
            &url("https://example.test/page"),
            &url("https://cdn.test/data.json"),
            &[("Content-Type".to_owned(), "application/json".to_owned())],
        )
        .expect_err("cross-origin opaque JSON response should be blocked");

        assert!(error.contains("OpaqueResponseBlocking"));
    }

    #[test]
    fn orb_body_validation_allows_mislabeled_png_and_javascript() {
        assert!(
            validate_opaque_response_blocking_with_body(
                &url("https://example.test/page"),
                &url("https://cdn.test/image"),
                &[("Content-Type".to_owned(), "text/html".to_owned())],
                b"\x89PNG\r\n\x1A\nrest",
            )
            .is_ok()
        );
        assert!(
            validate_opaque_response_blocking_with_body(
                &url("https://example.test/page"),
                &url("https://cdn.test/script"),
                &[("Content-Type".to_owned(), "application/json".to_owned())],
                b"function fn() { return 42; }",
            )
            .is_ok()
        );
        assert!(
            validate_opaque_response_blocking_with_body(
                &url("https://example.test/page"),
                &url("https://cdn.test/data.json"),
                &[("Content-Type".to_owned(), "application/json".to_owned())],
                br#"{"hello":"world"}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn orb_allows_same_origin_and_safelisted_mime_types() {
        assert!(
            validate_opaque_response_blocking(
                &url("https://example.test/page"),
                &url("https://example.test/data.json"),
                &[("Content-Type".to_owned(), "application/json".to_owned())],
            )
            .is_ok()
        );
        assert!(
            validate_opaque_response_blocking(
                &url("https://example.test/page"),
                &url("https://cdn.test/image.png"),
                &[("Content-Type".to_owned(), "image/png".to_owned())],
            )
            .is_ok()
        );
    }
}
