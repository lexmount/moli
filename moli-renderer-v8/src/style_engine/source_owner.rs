use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Element, Node},
    style_engine::{StyleViewport, StyloStyleEnvironment, media_list::evaluate_media_query_list},
};

pub(crate) fn link_rel_qualifies_as_stylesheet(rel: Option<&str>, title: Option<&str>) -> bool {
    let Some(rel) = rel else {
        return false;
    };
    let includes_token = |token: &str| {
        rel.split_ascii_whitespace()
            .any(|candidate| candidate.eq_ignore_ascii_case(token))
    };
    includes_token("stylesheet")
        && (!includes_token("alternate") || title.is_some_and(|title| !title.is_empty()))
}

pub(crate) fn stylesheet_owner_type_is_supported(element: &Element) -> bool {
    let type_attribute = element.attribute("type");
    if element.is_html_element("style")
        || (element.namespace() == "http://www.w3.org/2000/svg" && element.local_name() == "style")
    {
        return moli_web_mime::is_css_style_element_type_attribute(type_attribute);
    }
    if element.is_html_element("link") {
        return moli_web_mime::is_css_stylesheet_type_hint(type_attribute);
    }
    false
}

pub(super) fn stylesheet_owner_is_in_style_scope(host: &DomHost, owner: DomHandle) -> bool {
    if let Some(root) = host.containing_shadow_root(owner) {
        return moli_selector::stylo_shadow_root_host_participates_in_style_scope(host, root);
    }
    let Some(document) = host.owner_document_handle(owner) else {
        return false;
    };
    // A document without a browsing context can still own an active stylesheet.
    let mut current = Some(owner);
    while let Some(handle) = current {
        if handle == document {
            return true;
        }
        current = host.node(handle).and_then(Node::parent_node);
    }
    false
}

pub(super) fn stylesheet_source_base_url(host: &DomHost, handle: DomHandle) -> url::Url {
    host.owner_document_handle(handle)
        .and_then(|document_handle| {
            host.node(document_handle)
                .and_then(Node::as_document)
                .map(|document| document.base_url().clone())
        })
        .or_else(|| host.document_base_url())
        .or_else(|| host.document_url().cloned())
        .unwrap_or_else(|| url::Url::parse("about:blank").expect("static about:blank parses"))
}

pub(super) fn stylesheet_source_media_matches(
    media_text: &str,
    environment: StyloStyleEnvironment,
    viewport: StyleViewport,
) -> bool {
    let media = media_text.trim();
    media.is_empty() || evaluate_media_query_list(media, environment, viewport)
}

#[cfg(test)]
mod tests {
    use super::link_rel_qualifies_as_stylesheet;

    #[test]
    fn alternate_stylesheet_links_require_a_present_non_empty_title() {
        assert!(link_rel_qualifies_as_stylesheet(Some("stylesheet"), None));
        assert!(link_rel_qualifies_as_stylesheet(
            Some("alternate stylesheet"),
            Some("contrast")
        ));
        assert!(link_rel_qualifies_as_stylesheet(
            Some("STYLESHEET alternate"),
            Some(" ")
        ));
        assert!(!link_rel_qualifies_as_stylesheet(
            Some("alternate stylesheet"),
            None
        ));
        assert!(!link_rel_qualifies_as_stylesheet(
            Some("alternate stylesheet"),
            Some("")
        ));
        assert!(!link_rel_qualifies_as_stylesheet(Some("alternate"), None));
    }
}
