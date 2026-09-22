use crate::dom::native::DomHost;

pub(crate) fn document_default_language_from_headers(
    headers: &[(String, Vec<u8>)],
) -> Option<String> {
    headers
        .iter()
        .rev()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-language"))
        .and_then(|(_, value)| {
            single_content_language_value(&moli_fetch::decode_header_value(value))
        })
}

pub(crate) fn document_default_language_from_meta(dom_host: &DomHost) -> Option<String> {
    let document = dom_host.document_handle();
    let head = dom_host
        .node(document)
        .and_then(|node| node.as_document())
        .and_then(|document| document.head_handle(dom_host.dom(), dom_host.document_handle()))?;
    for child in dom_host.child_handles(head) {
        let Some(element) = dom_host.node(child).and_then(|node| node.as_element()) else {
            continue;
        };
        if !element.is_html_element("meta") {
            continue;
        }
        let Some(http_equiv) = element.attribute("http-equiv") else {
            continue;
        };
        if !http_equiv.eq_ignore_ascii_case("content-language") {
            continue;
        }
        let Some(content) = element.attribute("content") else {
            continue;
        };
        if let Some(language) = single_content_language_value(content) {
            return Some(language);
        }
    }
    None
}

pub(crate) fn sync_document_default_language_from_meta(
    dom_host: &mut DomHost,
    fallback_language: Option<&str>,
) {
    let language = document_default_language_from_meta(dom_host)
        .or_else(|| fallback_language.map(str::to_owned));
    let document = dom_host.document_handle();
    let _ = dom_host.set_document_default_language_for_handle(document, language);
}

fn single_content_language_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.contains(',') {
        return None;
    }
    is_basic_language_tag(value).then(|| value.to_ascii_lowercase())
}

fn is_basic_language_tag(value: &str) -> bool {
    value
        .split('-')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_language_default_requires_single_language_tag() {
        assert_eq!(single_content_language_value(" ko ").as_deref(), Some("ko"));
        assert_eq!(
            single_content_language_value("en-US").as_deref(),
            Some("en-us")
        );
        assert_eq!(single_content_language_value("ko, zh"), None);
        assert_eq!(single_content_language_value(""), None);
        assert_eq!(single_content_language_value("ko@x"), None);
    }
}
