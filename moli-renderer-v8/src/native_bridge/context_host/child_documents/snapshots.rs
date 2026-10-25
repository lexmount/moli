use super::super::fixture_support::child_upstream_fixture_text;
use super::super::{ChildBrowsingContextBootstrap, ChildBrowsingContextSnapshot, JsContextHost};
use super::LocalChildDocumentPreparation;
use crate::document_runtime::{DocumentPolicyContainer, DomHandle};
use crate::navigation_document::{
    decode_navigation_document_response, navigation_response_content_type,
};
use moli_web_mime::{resource_mime_essence_for_url, response_document_content_type};
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
    ) -> LocalChildDocumentPreparation {
        let mut snapshot = match self.materialize_local_child_snapshot_for_url(url, Some(handle)) {
            LocalChildDocumentPreparation::Ready(snapshot) => *snapshot,
            other => return other,
        };
        if ChildBrowsingContextBootstrap::Url(url.clone()).content_security_policy_inherited() {
            snapshot.policy_container =
                self.initial_child_about_blank_policy_container_from_parent(handle);
        }
        let snapshot = self.apply_page_csp_bypass_to_child_snapshot(snapshot);
        if moli_url::is_about_blank(&snapshot.url) {
            LocalChildDocumentPreparation::Ready(Box::new(
                snapshot.with_fallback_base_url(self.document_base_url_for_child_context(handle)),
            ))
        } else {
            LocalChildDocumentPreparation::Ready(Box::new(snapshot))
        }
    }

    pub(crate) fn materialize_local_child_snapshot_for_url(
        &self,
        url: &Url,
        owner: Option<DomHandle>,
    ) -> LocalChildDocumentPreparation {
        match url.scheme() {
            "about" if url.path() == "blank" => {
                LocalChildDocumentPreparation::Ready(Box::new(ChildBrowsingContextSnapshot::html(
                    url.clone(),
                    "<!DOCTYPE html><html><head></head><body></body></html>".into(),
                )))
            }
            "http" | "https" => match child_upstream_fixture_text(url) {
                Some(markup) => LocalChildDocumentPreparation::Ready(Box::new(
                    ChildBrowsingContextSnapshot::new(
                        url.clone(),
                        markup,
                        child_document_content_type_for_url(url),
                    ),
                )),
                None => LocalChildDocumentPreparation::Unavailable,
            },
            "blob" | "data" => {
                let Some(response) = crate::network_host::local_url_response(url) else {
                    return LocalChildDocumentPreparation::Unavailable;
                };
                let head = response.head();
                let mime = navigation_response_content_type(
                    &head.final_url,
                    &head.headers,
                    response.body_bytes(),
                );
                if !moli_web_mime::is_supported_document_mime_type(&mime) {
                    return LocalChildDocumentPreparation::IgnoredNavigation;
                }
                let content_type = Some(mime);
                let policy_container = DocumentPolicyContainer::from_navigation_response_headers(
                    &head.headers,
                    &head.final_url,
                );
                let encoding_context = owner
                    .map(|handle| self.child_document_encoding_context(handle))
                    .unwrap_or_default();
                let fallback = encoding_context.fallback_encoding(
                    &head.final_url,
                    content_type.as_deref(),
                    policy_container.sandbox.forces_opaque_origin,
                );
                let (body, character_set) = decode_navigation_document_response(
                    response.body_bytes(),
                    &head.headers,
                    content_type.as_deref(),
                    &head.final_url,
                    fallback,
                );
                LocalChildDocumentPreparation::Ready(Box::new(
                    ChildBrowsingContextSnapshot::with_content(
                        head.final_url,
                        body,
                        content_type,
                        character_set,
                    )
                    .with_response_status(Some(head.status))
                    .with_policy_container(policy_container),
                ))
            }
            _ => LocalChildDocumentPreparation::Unavailable,
        }
    }

    pub(in crate::native_bridge::context_host) fn materialize_local_child_snapshot_for_bootstrap(
        &self,
        handle: DomHandle,
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> LocalChildDocumentPreparation {
        if let Some(snapshot) = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.pending_local_history_snapshot(bootstrap))
        {
            return LocalChildDocumentPreparation::Historical(Box::new(
                self.apply_page_csp_bypass_to_child_snapshot(snapshot),
            ));
        }
        let mut snapshot = match bootstrap {
            ChildBrowsingContextBootstrap::AboutBlank => ChildBrowsingContextSnapshot::about_blank(
                self.document_base_url_for_child_context(handle),
            ),
            ChildBrowsingContextBootstrap::Srcdoc { base_url, markup } => {
                ChildBrowsingContextSnapshot::srcdoc(base_url.clone(), markup.clone())
            }
            ChildBrowsingContextBootstrap::Url(url) => {
                return self.materialize_local_child_snapshot_for_navigation_url(handle, url);
            }
            ChildBrowsingContextBootstrap::Request(_) => {
                return LocalChildDocumentPreparation::Unavailable;
            }
        };
        if bootstrap.content_security_policy_inherited() {
            snapshot.policy_container =
                self.initial_child_about_blank_policy_container_from_parent(handle);
        }
        LocalChildDocumentPreparation::Ready(Box::new(
            self.apply_page_csp_bypass_to_child_snapshot(snapshot),
        ))
    }
}

pub(in crate::native_bridge::context_host) fn child_document_content_type_from_headers(
    headers: &[(String, Vec<u8>)],
) -> Option<String> {
    response_document_content_type(headers)
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
        let headers: Vec<(String, Vec<u8>)> = vec![
            ("content-type".to_owned(), b"text/plain".to_vec()),
            (
                "Content-Type".to_owned(),
                b" Image/SVG+XML ; charset=utf-8 ".to_vec(),
            ),
        ];
        assert_eq!(
            child_document_content_type_from_headers(&headers).as_deref(),
            Some("image/svg+xml")
        );
        assert_eq!(
            child_document_content_type_from_headers(&[("Content-Type".to_owned(), b" ".to_vec())]),
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
