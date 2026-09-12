use moli_web_mime::response_header_values;

use crate::RequestCredentialsMode;

fn combined_cors_response_header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    // Fetch combines every matching field, including empty values. Neither
    // Allow-Origin nor Allow-Credentials permits a list.
    let values = response_header_values(headers, name);
    (!values.is_empty()).then(|| values.join(", "))
}

pub fn validate_cors_response_for_origin(
    origin: &str,
    response_headers: &[(String, String)],
    credentials_mode: RequestCredentialsMode,
) -> Result<(), String> {
    let Some(allow_origin) = combined_cors_response_header_value(response_headers, "access-control-allow-origin")
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
            combined_cors_response_header_value(response_headers, "access-control-allow-credentials");
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
