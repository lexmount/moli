use std::{fmt::Debug, sync::Arc};

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_encoding::{
    decode_classic_script_source, decode_html_document_with_fallback, decode_text_for_legacy_web,
    encoding_from_response_headers,
};
use moli_page_types::{
    SubresourceNetworkOutcome, SubresourceNetworkRecord, SubresourceResourceType,
};
use moli_web_mime::{
    effective_response_mime_essence, is_dom_parser_xml_mime, is_html_document_mime,
    is_javascript_mime_essence, is_json_mime, is_text_mime_essence,
};
use url::Url;

use crate::runtime::RendererResourceTextSearchOutcome;

use super::PageVm;

#[derive(Debug, Clone)]
pub struct RendererResourceSearchRequest {
    pub root_frame_id: String,
    pub frame_id: String,
    pub url: String,
    pub query: String,
    pub case_sensitive: bool,
    pub is_regex: bool,
    pub materialize_limit: usize,
    pub main_document: Option<RendererMainDocumentResource>,
}

#[derive(Debug, Clone)]
pub struct RendererMainDocumentResource {
    pub frame_id: String,
    pub url: Url,
    pub response_headers: Vec<(String, Vec<u8>)>,
    pub from_cache: bool,
    pub body: Option<Arc<dyn RendererResourceContentBody>>,
}

/// Immutable original response bytes retained by the browser network owner.
/// Implementations must enforce the limit before materializing the body.
pub trait RendererResourceContentBody: Debug + Send + Sync {
    fn read_bytes(&self, materialize_limit: usize) -> Result<Vec<u8>>;
}

enum SelectedResource {
    Text(String),
    Unavailable,
    Missing,
}

#[derive(Clone, Copy)]
enum ResourceContentKind {
    MainDocument,
    Subresource(SubresourceResourceType),
}

impl PageVm {
    pub(crate) fn search_resource_by_lines(
        &mut self,
        request: RendererResourceSearchRequest,
    ) -> Result<RendererResourceTextSearchOutcome> {
        // Resource completions may already be visible to native CDP commands
        // before the protocol adapter has consumed their Page state snapshots.
        self.drain_network_output_into_report();

        let root_frame = request.frame_id == request.root_frame_id;
        if !root_frame {
            let outcome = self.vm_mut().search_child_frame_resource_by_lines(
                &request.frame_id,
                &request.url,
                &request.query,
                request.case_sensitive,
                request.is_regex,
            )?;
            if !matches!(outcome, RendererResourceTextSearchOutcome::ResourceNotFound) {
                return Ok(outcome);
            }
        }

        let main_document = request.main_document.as_ref().filter(|resource| {
            root_frame
                && resource.frame_id == request.frame_id
                && resource_urls_match(resource.url.as_str(), &request.url)
        });
        let selected = match main_document {
            Some(resource) => select_main_document(resource, request.materialize_limit),
            None => select_subresource(
                self.report.subresource_network_records(),
                &request.frame_id,
                root_frame,
                &request.url,
                request.materialize_limit,
            ),
        };
        let text = match selected {
            SelectedResource::Text(text) => text,
            SelectedResource::Unavailable => {
                return Ok(RendererResourceTextSearchOutcome::ContentUnavailable);
            }
            SelectedResource::Missing => {
                return Ok(RendererResourceTextSearchOutcome::ResourceNotFound);
            }
        };
        Ok(RendererResourceTextSearchOutcome::Matches(
            self.vm_mut().search_text_by_lines(
                &text,
                &request.query,
                request.case_sensitive,
                request.is_regex,
            )?,
        ))
    }
}

fn select_main_document(
    resource: &RendererMainDocumentResource,
    materialize_limit: usize,
) -> SelectedResource {
    let Some(body) = resource.body.as_ref() else {
        return SelectedResource::Unavailable;
    };
    let Ok(bytes) = body.read_bytes(materialize_limit) else {
        return SelectedResource::Unavailable;
    };
    if bytes.len() > materialize_limit
        || !resource_has_searchable_content(bytes.len(), resource.from_cache)
    {
        return SelectedResource::Unavailable;
    }
    SelectedResource::Text(decode_resource_content(
        &bytes,
        &resource.response_headers,
        ResourceContentKind::MainDocument,
    ))
}

fn select_subresource(
    records: &[SubresourceNetworkRecord],
    frame_id: &str,
    root_frame: bool,
    requested_url: &str,
    materialize_limit: usize,
) -> SelectedResource {
    let record = records.iter().rev().find(|record| {
        resource_belongs_to_frame(record, frame_id, root_frame)
            && subresource_url_matches(record, requested_url)
    });
    let Some(record) = record else {
        return SelectedResource::Missing;
    };
    let SubresourceNetworkOutcome::Success {
        response_headers,
        response_body,
        ..
    } = record.outcome()
    else {
        return SelectedResource::Unavailable;
    };
    if !resource_has_searchable_content(response_body.len(), record.from_cache())
        || response_body.len() > materialize_limit
    {
        return SelectedResource::Unavailable;
    }
    let Ok(bytes) = response_body.materialize_bytes() else {
        return SelectedResource::Unavailable;
    };
    SelectedResource::Text(decode_resource_content(
        &bytes,
        response_headers,
        ResourceContentKind::Subresource(record.resource_type()),
    ))
}

fn resource_has_searchable_content(body_len: usize, from_cache: bool) -> bool {
    body_len != 0 || from_cache
}

fn resource_belongs_to_frame(
    record: &SubresourceNetworkRecord,
    frame_id: &str,
    root_frame: bool,
) -> bool {
    record.frame_id() == Some(frame_id) || (root_frame && record.frame_id().is_none())
}

fn subresource_url_matches(record: &SubresourceNetworkRecord, requested_url: &str) -> bool {
    if resource_urls_match(record.url().as_str(), requested_url) {
        return true;
    }
    match record.outcome() {
        SubresourceNetworkOutcome::Success { final_url, .. } => {
            resource_urls_match(final_url.as_str(), requested_url)
        }
        SubresourceNetworkOutcome::Failure { .. } => false,
    }
}

fn decode_resource_content(
    bytes: &[u8],
    headers: &[(String, Vec<u8>)],
    kind: ResourceContentKind,
) -> String {
    let mime = effective_response_mime_essence(headers, None).unwrap_or_default();
    if matches!(kind, ResourceContentKind::MainDocument)
        && (mime.is_empty() || is_html_document_mime(&mime))
    {
        return decode_html_document_with_fallback(bytes, headers, Some("utf-8")).0;
    }
    if matches!(
        kind,
        ResourceContentKind::Subresource(SubresourceResourceType::Script)
    ) || is_javascript_mime_essence(&mime)
    {
        return decode_classic_script_source(bytes, headers, None, None);
    }
    let response_charset = encoding_from_response_headers(headers).map(|encoding| encoding.name());
    if matches!(
        kind,
        ResourceContentKind::Subresource(SubresourceResourceType::Stylesheet)
    ) {
        return decode_text_for_legacy_web(bytes, response_charset);
    }
    if is_dom_parser_xml_mime(&mime) || is_json_mime(&mime) {
        return decode_text_for_legacy_web(bytes, response_charset);
    }
    if is_text_mime_essence(&mime) {
        return decode_text_for_legacy_web(bytes, response_charset.or(Some("windows-1252")));
    }
    BASE64_STANDARD.encode(bytes)
}

fn resource_urls_match(left: &str, right: &str) -> bool {
    match (url_without_fragment(left), url_without_fragment(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

fn url_without_fragment(value: &str) -> Option<Url> {
    Url::parse(value).ok().map(|mut url| {
        url.set_fragment(None);
        url
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_url_identity_ignores_fragments() {
        assert!(resource_urls_match(
            "https://example.test/page#one",
            "https://example.test/page#two"
        ));
        assert!(!resource_urls_match(
            "https://example.test/page",
            "https://example.test/other"
        ));
    }

    #[test]
    fn html_resource_decoding_observes_declared_charset() {
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "content-type".to_owned(),
            b"text/html; charset=windows-1252".to_vec(),
        )];
        assert_eq!(
            decode_resource_content(b"<p>\x80</p>", &headers, ResourceContentKind::MainDocument,),
            "<p>\u{20ac}</p>"
        );
    }

    #[test]
    fn stylesheet_resource_rejects_invalid_response_charset_whitespace() {
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "content-type".to_owned(),
            b"text/css; charset=\nshift_jis".to_vec(),
        )];

        assert_eq!(
            decode_resource_content(
                "body::after { content: '目次'; }".as_bytes(),
                &headers,
                ResourceContentKind::Subresource(SubresourceResourceType::Stylesheet),
            ),
            "body::after { content: '目次'; }"
        );
    }

    #[test]
    fn binary_resource_searches_chromium_style_base64_content() {
        assert_eq!(
            decode_resource_content(
                &[0, 255],
                &[(
                    "content-type".to_owned(),
                    b"application/octet-stream".to_vec(),
                )],
                ResourceContentKind::Subresource(SubresourceResourceType::Image),
            ),
            "AP8="
        );
    }

    #[test]
    fn uncached_empty_resource_is_unavailable_but_cached_empty_resource_is_searchable() {
        assert!(!resource_has_searchable_content(0, false));
        assert!(resource_has_searchable_content(0, true));
        assert!(resource_has_searchable_content(1, false));
    }
}
