use super::super::fixture_support::child_upstream_fixture_text;
use super::super::{ChildBrowsingContextBootstrap, ChildBrowsingContextSnapshot, JsContextHost};
use super::configure_child_document_navigation_request;
use crate::document_runtime::{DocumentPolicyContainer, DomHandle};
use moli_encoding::decode_html_document_with_fallback;
use moli_fetch::Request;
use moli_web_mime::{
    is_dom_parser_xml_mime, is_html_document_mime, resource_mime_essence_for_url,
    response_document_content_type,
};
use url::Url;

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn apply_page_csp_bypass_to_child_snapshot(
        &self,
        mut snapshot: ChildBrowsingContextSnapshot,
    ) -> ChildBrowsingContextSnapshot {
        if self.bypass_content_security_policy() {
            snapshot
                .policy_container
                .clear_content_security_policy_for_bypass();
        }
        snapshot
    }

    pub(crate) fn materialize_local_child_snapshot_for_navigation_url(
        &self,
        handle: DomHandle,
        url: &Url,
    ) -> Option<ChildBrowsingContextSnapshot> {
        let snapshot = self.apply_page_csp_bypass_to_child_snapshot(
            self.materialize_local_child_snapshot_for_url(url)?,
        );
        if moli_url::is_about_blank(&snapshot.url) {
            Some(snapshot.with_fallback_base_url(self.document_base_url_for_child_context(handle)))
        } else {
            Some(snapshot)
        }
    }

    pub(crate) fn materialize_local_child_snapshot_for_url(
        &self,
        url: &Url,
    ) -> Option<ChildBrowsingContextSnapshot> {
        match url.scheme() {
            "about" if url.path() == "blank" => Some(ChildBrowsingContextSnapshot::html(
                url.clone(),
                "<!DOCTYPE html><html><head></head><body></body></html>".into(),
            )),
            "http" | "https" => child_upstream_fixture_text(url).map(|markup| {
                ChildBrowsingContextSnapshot::new(
                    url.clone(),
                    markup,
                    child_document_content_type_for_url(url),
                )
            }),
            "blob" => {
                let (body, mime_type) = crate::blob::object_url_body_and_type(url.as_str())?;
                if !mime_type.is_empty() && !is_html_document_mime(&mime_type) {
                    return None;
                }
                let character_set = self.child_document_fallback_character_set(Some("text/html"));
                Some(ChildBrowsingContextSnapshot::with_character_set(
                    url.clone(),
                    body,
                    child_document_content_type_from_header_value(&mime_type)
                        .or_else(|| Some("text/html".to_owned())),
                    character_set,
                ))
            }
            "data" => crate::network_host::local_url_response(url).map(|response| {
                let head = response.head();
                let content_type = child_document_content_type_from_headers(&head.headers);
                let fallback = self.child_document_fallback_character_set(content_type.as_deref());
                let (body, character_set) = decode_html_document_with_fallback(
                    response.body_bytes(),
                    &head.headers,
                    Some(&fallback),
                );
                let policy_container = DocumentPolicyContainer::from_navigation_response_headers(
                    &head.headers,
                    &head.final_url,
                );
                ChildBrowsingContextSnapshot::with_character_set(
                    head.final_url,
                    body,
                    content_type,
                    character_set,
                )
                .with_policy_container(policy_container)
            }),
            _ => None,
        }
    }

    pub(crate) fn materialize_child_snapshot_for_url_blocking(
        &self,
        owner_node: crate::document_runtime::DomHandle,
        url: &Url,
    ) -> Option<ChildBrowsingContextSnapshot> {
        if let Some(snapshot) = self.materialize_local_child_snapshot_for_url(url) {
            return Some(self.apply_page_csp_bypass_to_child_snapshot(snapshot));
        }
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let dispatch_scope = self
            .owner_dispatch_scope_for_node(owner_node)
            .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
        let loader = self
            .document_resource_loader_for_dispatch_scope(dispatch_scope)?
            .request_client()
            .clone();
        let initiator_url = self.document_url_for_child_context(owner_node);
        let browser_context = self.host_document().cookie_browser_context();
        let request = configure_child_document_navigation_request(
            Request::new("GET", url.as_str(), None, Vec::new()).ok()?,
            &initiator_url,
            &browser_context,
        )
        .with_page_network_policy();
        let response = loader.fetch_raw_for_blocking_boundary(request).ok()?;
        let head = response.head();
        let content_type = child_document_content_type_from_headers(&head.headers)
            .or_else(|| child_document_content_type_for_url(&head.final_url));
        let fallback = self.child_document_fallback_character_set(content_type.as_deref());
        let (markup, character_set) = decode_html_document_with_fallback(
            response.body_bytes(),
            &head.headers,
            Some(&fallback),
        );
        let policy_container = DocumentPolicyContainer::from_navigation_response_headers(
            &head.headers,
            &head.final_url,
        );
        Some(
            self.apply_page_csp_bypass_to_child_snapshot(
                ChildBrowsingContextSnapshot::with_character_set(
                    head.final_url,
                    markup,
                    content_type,
                    character_set,
                )
                .with_policy_container(policy_container),
            ),
        )
    }

    pub(in crate::native_bridge::context_host) fn materialize_local_child_snapshot_for_bootstrap(
        &self,
        handle: DomHandle,
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> Option<ChildBrowsingContextSnapshot> {
        match bootstrap {
            ChildBrowsingContextBootstrap::AboutBlank => {
                let policy_container =
                    self.initial_child_about_blank_policy_container_from_parent(handle);
                Some(
                    self.apply_page_csp_bypass_to_child_snapshot(
                        ChildBrowsingContextSnapshot::about_blank(
                            self.document_base_url_for_child_context(handle),
                        )
                        .with_policy_container(policy_container),
                    ),
                )
            }
            ChildBrowsingContextBootstrap::Srcdoc { base_url, markup } => Some(
                self.apply_page_csp_bypass_to_child_snapshot(ChildBrowsingContextSnapshot::srcdoc(
                    base_url.clone(),
                    markup.clone(),
                    self.document_character_set().to_owned(),
                )),
            ),
            ChildBrowsingContextBootstrap::Url(url) => {
                self.materialize_local_child_snapshot_for_navigation_url(handle, url)
            }
            ChildBrowsingContextBootstrap::Request(_) => None,
        }
    }

    pub(in crate::native_bridge::context_host) fn child_document_fallback_character_set(
        &self,
        content_type: Option<&str>,
    ) -> String {
        if content_type.is_some_and(is_dom_parser_xml_mime) {
            "UTF-8".to_owned()
        } else {
            self.document_character_set().to_owned()
        }
    }
}

pub(in crate::native_bridge::context_host) fn child_document_content_type_from_headers(
    headers: &[(String, String)],
) -> Option<String> {
    response_document_content_type(headers)
}

fn child_document_content_type_from_header_value(value: &str) -> Option<String> {
    moli_web_mime::mime_essence(value)
}

pub(in crate::native_bridge::context_host) fn child_document_content_type_for_url(
    url: &Url,
) -> Option<String> {
    resource_mime_essence_for_url(url.as_str(), url.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_document_content_type_routes_through_web_mime_helpers() {
        let headers = vec![
            ("content-type".to_owned(), "text/plain".to_owned()),
            (
                "Content-Type".to_owned(),
                " Image/SVG+XML ; charset=utf-8 ".to_owned(),
            ),
        ];
        assert_eq!(
            child_document_content_type_from_headers(&headers).as_deref(),
            Some("image/svg+xml")
        );
        assert_eq!(
            child_document_content_type_from_headers(&[(
                "Content-Type".to_owned(),
                " ".to_owned()
            )]),
            None
        );
        assert_eq!(
            child_document_content_type_for_url(
                &Url::parse("https://example.test/frame.PNG").unwrap()
            )
            .as_deref(),
            Some("image/png")
        );
        assert_eq!(
            child_document_content_type_for_url(
                &Url::parse("data:Image/SVG+XML,<svg></svg>").unwrap()
            )
            .as_deref(),
            Some("image/svg+xml")
        );
        let data_url = Url::parse("data:font/woff2;base64,AA==").unwrap();
        assert_eq!(
            child_document_content_type_for_url(&data_url).as_deref(),
            Some("font/woff2")
        );
        let video_url = Url::parse("https://example.test/media/clip.WEBM").unwrap();
        assert_eq!(
            child_document_content_type_for_url(&video_url).as_deref(),
            Some("video/webm")
        );
        assert_eq!(
            child_document_content_type_for_url(
                &Url::parse("https://example.test/frame.json").unwrap()
            ),
            None
        );
    }
}
