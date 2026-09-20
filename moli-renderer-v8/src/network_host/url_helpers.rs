use http::HeaderName;
use indexmap::IndexMap;

pub(crate) fn resolve_context_url(
    document_url: &url::Url,
    input: &str,
    base: Option<&str>,
) -> std::result::Result<url::Url, String> {
    let resolved_base = match base {
        Some(base) => url::Url::parse(base)
            .or_else(|_| document_url.join(base))
            .map_err(|error| format!("failed to resolve base url `{base}`: {error}"))?,
        None => document_url.clone(),
    };

    url::Url::parse(input)
        .or_else(|_| resolved_base.join(input))
        .map_err(|error| format!("failed to resolve url `{input}`: {error}"))
}

pub(in crate::network_host) fn merge_subresource_request_headers(
    context_headers: &[(String, String)],
    request_headers: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = IndexMap::<String, (String, String)>::new();
    for (name, value) in context_headers {
        merged
            .entry(header_name_key(name))
            .or_insert_with(|| (name.clone(), value.clone()));
    }
    for (name, value) in request_headers {
        let key = header_name_key(name);
        merged.shift_remove(&key);
        merged.insert(key, (name.clone(), value.clone()));
    }
    merged.into_values().collect()
}

/// Encode validated WebIDL headers once, then overlay them on UTF-8 defaults.
pub(crate) fn merge_byte_string_request_headers(
    context_headers: &[(String, String)],
    request_headers: &[(String, String)],
) -> moli_fetch::RequestHeaders {
    let mut headers = moli_fetch::RequestHeaders::from_utf8(context_headers.to_vec());
    headers.overlay(
        moli_fetch::RequestHeaders::from_byte_strings(request_headers)
            .expect("validated Fetch/XHR headers are ByteStrings"),
    );
    headers
}

fn header_name_key(name: &str) -> String {
    HeaderName::from_bytes(name.as_bytes())
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|_| name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::merge_subresource_request_headers;

    #[test]
    fn merge_subresource_request_headers_uses_header_name_keys_and_request_order() {
        let merged = merge_subresource_request_headers(
            &[
                ("X-Test".to_owned(), "context".to_owned()),
                ("Accept".to_owned(), "text/html".to_owned()),
            ],
            &[
                ("x-test".to_owned(), "request".to_owned()),
                ("X-New".to_owned(), "new".to_owned()),
            ],
        );

        assert_eq!(
            merged,
            vec![
                ("Accept".to_owned(), "text/html".to_owned()),
                ("x-test".to_owned(), "request".to_owned()),
                ("X-New".to_owned(), "new".to_owned()),
            ]
        );
    }
}
