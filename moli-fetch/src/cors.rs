use moli_url::WebOrigin;
use moli_web_mime::response_header_values;

use crate::{FetchUrlList, RedirectSource, RequestCredentialsMode, ResponseHead};

/// Validates a network response using the request's origin, credentials and
/// redirect history. Synthetic redirects affect the origin used by later hops.
pub fn validate_cors_response_chain(
    request_origin: impl Into<WebOrigin>,
    head: &ResponseHead,
    credentials_mode: RequestCredentialsMode,
    web_security_disabled: bool,
) -> Result<(), String> {
    if web_security_disabled {
        return Ok(());
    }
    let request_origin = request_origin.into();
    for (index, redirect) in head.redirect_chain.iter().enumerate() {
        // This response sees the origin established by earlier hops.
        let urls = FetchUrlList::new(&redirect.from_url, &head.redirect_chain[..index]);
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

fn combined_cors_response_header_value(
    headers: &[(String, Vec<u8>)],
    name: &str,
) -> Option<String> {
    // Fetch combines every matching field, including empty values. Neither
    // Allow-Origin nor Allow-Credentials permits a list.
    let values = response_header_values(headers, name);
    (!values.is_empty()).then(|| values.join(", "))
}

pub fn validate_cors_response_for_origin(
    origin: &str,
    response_headers: &[(String, Vec<u8>)],
    credentials_mode: RequestCredentialsMode,
) -> Result<(), String> {
    let Some(allow_origin) =
        combined_cors_response_header_value(response_headers, "access-control-allow-origin")
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
        let allow_credentials = combined_cors_response_header_value(
            response_headers,
            "access-control-allow-credentials",
        );
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
