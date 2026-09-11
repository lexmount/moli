use moli_cookie_jar::same_site_urls;
use moli_fetch::{RequestCredentialsMode, RequestMode};
use moli_url::{WebOrigin, origin_ascii_serialization, same_origin};
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

pub(crate) fn cors_request_origin_after_redirects<'a>(
    request_origin: &WebOrigin,
    redirects: impl IntoIterator<Item = (&'a url::Url, &'a url::Url)>,
) -> WebOrigin {
    if redirects
        .into_iter()
        .any(|(from, to)| !same_origin(from, to) && !request_origin.same_origin_url(from))
    {
        WebOrigin::Opaque
    } else {
        request_origin.clone()
    }
}

pub(crate) fn validate_cors_response(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    credentials_mode: RequestCredentialsMode,
) -> Result<(), String> {
    validate_cors_response_for_origin(
        &WebOrigin::from_url(document_url),
        response_url,
        response_headers,
        credentials_mode,
    )
}

pub(crate) fn validate_cors_response_for_origin(
    request_origin: &WebOrigin,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    credentials_mode: RequestCredentialsMode,
) -> Result<(), String> {
    if request_origin.same_origin_url(response_url) {
        return Ok(());
    }
    if !matches!(response_url.scheme(), "http" | "https") {
        return Ok(());
    }

    let origin = request_origin.ascii_serialization();
    let Some(allow_origin) = response_header_value(response_headers, "access-control-allow-origin")
    else {
        return Err(format!(
            "CORS check failed: no Access-Control-Allow-Origin for {origin}"
        ));
    };
    let allow_origin = allow_origin.trim();
    if allow_origin == "*" {
        if credentials_mode == RequestCredentialsMode::Include {
            return Err(format!(
                "CORS check failed: wildcard Access-Control-Allow-Origin does not allow credentialed requests from {origin}"
            ));
        }
        return Ok(());
    }
    if allow_origin != origin {
        return Err(format!(
            "CORS check failed: Access-Control-Allow-Origin `{allow_origin}` does not allow {origin}"
        ));
    }

    if credentials_mode == RequestCredentialsMode::Include {
        let allow_credentials =
            response_header_value(response_headers, "access-control-allow-credentials");
        if allow_credentials
            .as_deref()
            .is_none_or(|value| value.trim() != "true")
        {
            return Err(format!(
                "CORS check failed: credentialed requests from {origin} require Access-Control-Allow-Credentials: true"
            ));
        }
    }

    Ok(())
}

pub(crate) fn validate_fetch_response_security_policy(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    validate_fetch_response_headers_for_origin(
        document_url,
        &WebOrigin::from_url(document_url),
        response_url,
        response_headers,
        request_mode,
        credentials_mode,
        policy_context,
    )?;
    if request_mode == RequestMode::NoCors {
        validate_opaque_response_blocking(document_url, response_url, response_headers)
    } else {
        Ok(())
    }
}

/// Header policies precede response delivery; ORB gates the opaque internal
/// body separately so an unfinished download does not hold up fetch().
pub(crate) fn validate_fetch_response_headers_for_origin(
    document_url: &url::Url,
    request_origin: &WebOrigin,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    if request_mode == RequestMode::NoCors {
        validate_cross_origin_resource_policy(document_url, response_url, response_headers)?;
        validate_cross_origin_embedder_and_document_isolation_policy(
            document_url,
            response_url,
            response_headers,
            request_mode,
            credentials_mode,
            policy_context.cross_origin_embedder_policy,
            policy_context.document_isolation_policy,
        )
    } else {
        validate_cors_response_for_origin(
            request_origin,
            response_url,
            response_headers,
            credentials_mode,
        )
    }
}

pub(crate) fn fetch_response_needs_orb_body_validation(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
) -> bool {
    request_mode == RequestMode::NoCors
        && matches!(response_url.scheme(), "http" | "https")
        && !same_origin(document_url, response_url)
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

pub(crate) fn validate_fetch_response_security_policy_for_origin(
    document_url: &url::Url,
    request_origin: &WebOrigin,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    if request_mode == RequestMode::NoCors {
        validate_fetch_response_security_policy(
            document_url,
            response_url,
            response_headers,
            request_mode,
            credentials_mode,
            policy_context,
        )
    } else {
        validate_cors_response_for_origin(
            request_origin,
            response_url,
            response_headers,
            credentials_mode,
        )
    }
}

pub(crate) fn validate_fetch_response_security_policy_with_body(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &[u8],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    validate_fetch_response_security_policy_with_body_classified(
        document_url,
        response_url,
        response_headers,
        response_body,
        request_mode,
        credentials_mode,
        policy_context,
    )
    .map_err(FetchResponseSecurityViolation::into_message)
}

pub(crate) fn validate_fetch_response_security_policy_with_body_classified(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &[u8],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), FetchResponseSecurityViolation> {
    if request_mode == RequestMode::NoCors {
        validate_cross_origin_resource_policy(document_url, response_url, response_headers)
            .map_err(FetchResponseSecurityViolation::Rejected)?;
        validate_cross_origin_embedder_and_document_isolation_policy(
            document_url,
            response_url,
            response_headers,
            request_mode,
            credentials_mode,
            policy_context.cross_origin_embedder_policy,
            policy_context.document_isolation_policy,
        )
        .map_err(FetchResponseSecurityViolation::Rejected)?;
        validate_opaque_response_blocking_with_body(
            document_url,
            response_url,
            response_headers,
            response_body,
        )
        .map_err(FetchResponseSecurityViolation::OpaqueResponseBlocked)
    } else {
        validate_cors_response(
            document_url,
            response_url,
            response_headers,
            credentials_mode,
        )
        .map_err(FetchResponseSecurityViolation::Rejected)
    }
}

pub(crate) fn validate_fetch_response_security_policy_with_body_for_origin(
    document_url: &url::Url,
    request_origin: &WebOrigin,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &[u8],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), String> {
    validate_fetch_response_security_policy_with_body_classified_for_origin(
        document_url,
        request_origin,
        response_url,
        response_headers,
        response_body,
        request_mode,
        credentials_mode,
        policy_context,
    )
    .map_err(FetchResponseSecurityViolation::into_message)
}

pub(crate) fn validate_fetch_response_security_policy_with_body_classified_for_origin(
    document_url: &url::Url,
    request_origin: &WebOrigin,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &[u8],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<(), FetchResponseSecurityViolation> {
    if request_mode == RequestMode::NoCors {
        validate_fetch_response_security_policy_with_body_classified(
            document_url,
            response_url,
            response_headers,
            response_body,
            request_mode,
            credentials_mode,
            policy_context,
        )
    } else {
        validate_cors_response_for_origin(
            request_origin,
            response_url,
            response_headers,
            credentials_mode,
        )
        .map_err(FetchResponseSecurityViolation::Rejected)
    }
}

pub(crate) fn validate_opaque_response_blocking(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
) -> Result<(), String> {
    if !matches!(response_url.scheme(), "http" | "https")
        || same_origin(document_url, response_url)
        || !should_opaque_response_be_blocked_by_orb(response_headers)
    {
        return Ok(());
    }

    Err(format!(
        "OpaqueResponseBlocking check failed: {} cannot load {response_url} as an opaque no-cors response",
        origin_ascii_serialization(document_url)
    ))
}

pub(crate) fn validate_opaque_response_blocking_with_body(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &[u8],
) -> Result<(), String> {
    if !matches!(response_url.scheme(), "http" | "https")
        || same_origin(document_url, response_url)
        || !should_opaque_response_be_blocked_by_orb_with_body(response_headers, response_body)
    {
        return Ok(());
    }

    Err(format!(
        "OpaqueResponseBlocking check failed: {} cannot load {response_url} as an opaque no-cors response",
        origin_ascii_serialization(document_url)
    ))
}

#[cfg(test)]
fn validate_cross_origin_embedder_policy(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    embedder_policy: CrossOriginEmbedderPolicy,
) -> Result<(), String> {
    validate_cross_origin_embedder_and_document_isolation_policy(
        document_url,
        response_url,
        response_headers,
        request_mode,
        credentials_mode,
        embedder_policy,
        DocumentIsolationPolicy::None,
    )
}

pub(crate) fn validate_cross_origin_embedder_and_document_isolation_policy(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    embedder_policy: CrossOriginEmbedderPolicy,
    document_isolation_policy: DocumentIsolationPolicy,
) -> Result<(), String> {
    if request_mode != RequestMode::NoCors
        || !matches!(response_url.scheme(), "http" | "https")
        || same_origin(document_url, response_url)
    {
        return Ok(());
    }

    let request_includes_credentials =
        request_includes_credentials(document_url, response_url, credentials_mode);
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
            origin_ascii_serialization(document_url)
        ));
    };
    let policy = policy.trim().to_ascii_lowercase();
    match policy.as_str() {
        "same-origin" | "same-site" | "cross-origin" => {
            validate_cross_origin_resource_policy(document_url, response_url, response_headers)
        }
        _ => Err(format!(
            "{policy_label} check failed: treats invalid Cross-Origin-Resource-Policy `{policy}` as same-origin for {} to load {response_url}",
            origin_ascii_serialization(document_url)
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
    document_url: &url::Url,
    response_url: &url::Url,
    credentials_mode: RequestCredentialsMode,
) -> bool {
    match credentials_mode {
        RequestCredentialsMode::Include => true,
        RequestCredentialsMode::Omit => false,
        RequestCredentialsMode::SameOrigin => same_origin(document_url, response_url),
    }
}

pub(crate) fn validate_cross_origin_resource_policy(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
) -> Result<(), String> {
    if !matches!(response_url.scheme(), "http" | "https") {
        return Ok(());
    }
    let Some(policy) = response_header_value(response_headers, "cross-origin-resource-policy")
    else {
        return Ok(());
    };
    let policy = policy.trim().to_ascii_lowercase();
    let allowed = match policy.as_str() {
        "same-origin" => same_origin(document_url, response_url),
        "same-site" => same_site_urls(document_url, response_url, true),
        "cross-origin" => true,
        _ => true,
    };
    if allowed {
        return Ok(());
    }
    Err(format!(
        "Cross-Origin-Resource-Policy check failed: `{policy}` does not allow {} to load {response_url}",
        origin_ascii_serialization(document_url)
    ))
}

#[cfg(test)]
pub(crate) fn cors_preflight_request_headers(
    document_url: &url::Url,
    request_url: &url::Url,
    method: &str,
    request_headers: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    cors_preflight_request_headers_for_origin(
        &WebOrigin::from_url(document_url),
        request_url,
        method,
        request_headers,
        false,
    )
}

pub(crate) fn cors_preflight_request_headers_for_origin(
    request_origin: &WebOrigin,
    request_url: &url::Url,
    method: &str,
    request_headers: &[(String, String)],
    use_cors_preflight: bool,
) -> Option<Vec<(String, String)>> {
    if request_origin.same_origin_url(request_url) {
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

#[cfg(test)]
pub(crate) fn validate_cors_preflight_response(
    document_url: &url::Url,
    response_url: &url::Url,
    requested_method: &str,
    request_headers: &[(String, String)],
    response_status: u16,
    response_headers: &[(String, String)],
) -> Result<(), String> {
    validate_cors_preflight_response_for_origin(
        &WebOrigin::from_url(document_url),
        response_url,
        requested_method,
        request_headers,
        response_status,
        response_headers,
        RequestCredentialsMode::SameOrigin,
    )
}

pub(crate) fn validate_cors_preflight_response_for_origin(
    request_origin: &WebOrigin,
    response_url: &url::Url,
    requested_method: &str,
    request_headers: &[(String, String)],
    response_status: u16,
    response_headers: &[(String, String)],
    credentials_mode: RequestCredentialsMode,
) -> Result<(), String> {
    if !(200..300).contains(&response_status) {
        return Err(format!(
            "CORS preflight failed: response status {response_status}"
        ));
    }
    validate_cors_response_for_origin(
        request_origin,
        response_url,
        response_headers,
        credentials_mode,
    )?;

    if !moli_fetch::is_cors_safelisted_method(requested_method) {
        let Some(allow_methods) =
            response_header_value(response_headers, "access-control-allow-methods")
        else {
            return Err(format!(
                "CORS preflight failed: no Access-Control-Allow-Methods for {requested_method}"
            ));
        };
        if !comma_separated_tokens(&allow_methods)
            .iter()
            .any(|method| method == requested_method)
        {
            return Err(format!(
                "CORS preflight failed: Access-Control-Allow-Methods `{allow_methods}` does not allow {requested_method}"
            ));
        }
    }

    let unsafe_header_names = moli_fetch::cors_unsafe_request_header_names(request_headers);
    if unsafe_header_names.is_empty() {
        return Ok(());
    }

    let Some(allow_headers) =
        response_header_value(response_headers, "access-control-allow-headers")
    else {
        return Err(format!(
            "CORS preflight failed: no Access-Control-Allow-Headers for {}",
            unsafe_header_names.join(",")
        ));
    };
    let allowed_header_names = comma_separated_tokens(&allow_headers)
        .into_iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    for header_name in unsafe_header_names {
        if !allowed_header_names.iter().any(|name| name == &header_name) {
            return Err(format!(
                "CORS preflight failed: Access-Control-Allow-Headers `{allow_headers}` does not allow {header_name}"
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
fn filter_cors_exposed_response_headers(
    document_url: &url::Url,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    credentials_mode: RequestCredentialsMode,
) -> Vec<(String, String)> {
    filter_cors_exposed_response_headers_for_origin(
        &WebOrigin::from_url(document_url),
        response_url,
        response_headers,
        credentials_mode,
    )
}

pub(crate) fn filter_cors_exposed_response_headers_for_origin(
    request_origin: &WebOrigin,
    response_url: &url::Url,
    response_headers: &[(String, String)],
    credentials_mode: RequestCredentialsMode,
) -> Vec<(String, String)> {
    if request_origin.same_origin_url(response_url) {
        return response_headers.to_vec();
    }
    if !matches!(response_url.scheme(), "http" | "https") {
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

fn comma_separated_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
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

    fn url(value: &str) -> url::Url {
        url::Url::parse(value).expect("valid URL")
    }

    #[test]
    fn cors_response_validation_uses_the_redirect_tainted_origin() {
        let home = url("https://page.test/a");
        let away = url("https://script.test/b");
        let elsewhere = url("https://other.test/c");
        let origin = WebOrigin::from_url(&home);
        assert_eq!(
            cors_request_origin_after_redirects(&origin, [(&home, &away)]),
            origin
        );
        for redirects in [
            vec![(&away, &elsewhere)],
            vec![(&home, &away), (&away, &home)],
        ] {
            let tainted = cors_request_origin_after_redirects(&origin, redirects);
            assert!(tainted.is_opaque());
            assert!(
                validate_cors_response_for_origin(
                    &tainted,
                    &home,
                    &[("access-control-allow-origin".into(), "null".into())],
                    RequestCredentialsMode::SameOrigin,
                )
                .is_ok()
            );
            assert!(
                validate_cors_response_for_origin(
                    &tainted,
                    &home,
                    &[],
                    RequestCredentialsMode::SameOrigin,
                )
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
            &url("http://example.test/page"),
            &url("http://other.test/data"),
            "PUT",
            &headers,
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
                &url("http://example.test/page"),
                &url("http://other.test/data"),
                "POST",
                &headers,
            ),
            None
        );
    }

    #[test]
    fn opaque_request_origin_requires_null_cors_opt_in_for_same_url_origin() {
        let response_url = url("https://example.test/data");

        assert!(
            validate_cors_response_for_origin(
                &WebOrigin::Opaque,
                &response_url,
                &[],
                RequestCredentialsMode::SameOrigin,
            )
            .is_err()
        );
        assert!(
            validate_cors_response_for_origin(
                &WebOrigin::Opaque,
                &response_url,
                &[("Access-Control-Allow-Origin".to_owned(), "null".to_owned(),)],
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
                &url("http://example.test/page"),
                &url("http://other.test/data"),
                "POST",
                &request_headers,
                204,
                &response_headers,
            ),
            Ok(())
        );
    }

    #[test]
    fn validate_cors_preflight_response_allows_safelisted_methods_without_allow_methods() {
        let document_url = url::Url::parse("https://origin.test/page").unwrap();
        let response_url = url::Url::parse("https://api.test/data").unwrap();
        let response_headers = vec![
            ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
            (
                "Access-Control-Allow-Headers".to_owned(),
                "content-type".to_owned(),
            ),
        ];

        for method in ["GET", "HEAD", "POST"] {
            validate_cors_preflight_response(
                &document_url,
                &response_url,
                method,
                &[("Content-Type".to_owned(), "custom/type".to_owned())],
                200,
                &response_headers,
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
        let response_url = url::Url::parse("https://api.test/data").unwrap();
        let response_headers = vec![
            ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
            (
                "Access-Control-Allow-Headers".to_owned(),
                "content-type".to_owned(),
            ),
        ];

        let error = validate_cors_preflight_response(
            &document_url,
            &response_url,
            "PUT",
            &[("Content-Type".to_owned(), "custom/type".to_owned())],
            200,
            &response_headers,
        )
        .expect_err("unsafelisted PUT preflight should require Access-Control-Allow-Methods");
        assert!(error.contains("no Access-Control-Allow-Methods for PUT"));
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
            &url("http://other.test/data"),
            &headers,
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
            &url("http://other.test/data"),
            &headers,
            RequestCredentialsMode::SameOrigin,
        );
        assert_eq!(non_credentialed, headers);

        let credentialed = filter_cors_exposed_response_headers(
            &url("http://example.test/page"),
            &url("http://other.test/data"),
            &headers,
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
            &url("http://example.test/data"),
            &headers,
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
