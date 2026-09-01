use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_core::page::{
    CompletedPageCommand, Page, PendingPageCommand, RendererResourceTextSearchOutcome,
    RendererTextSearchMatch, SubresourceNetworkOutcome, SubresourceNetworkRecord,
    SubresourceResourceType,
};
use moli_encoding::{
    decode_classic_script_source, decode_html_document_with_fallback, decode_text_for_legacy_web,
    encoding_from_response_headers,
};
use moli_web_mime::{
    effective_response_mime_essence, is_dom_parser_xml_mime, is_html_document_mime,
    is_javascript_mime_essence, is_json_module_mime, is_text_mime_essence,
};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use super::{PageCommandTaskStep, PendingPageCommandDispatch, PendingPageCommandKind};
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::domains::command_output::CommandOutputPlan;

const AGENT_NOT_ENABLED: &str = "Agent is not enabled.";
const FRAME_NOT_FOUND: &str = "No frame for given id found";
const RESOURCE_NOT_FOUND: &str = "No resource with given URL found";
const CONTENT_UNAVAILABLE: &str = "Content unavailable. Resource was not cached";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchInResourceParams {
    frame_id: String,
    url: String,
    query: String,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(default)]
    is_regex: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceSearchSource {
    SelectedText,
    ChildDocument,
}

pub(super) struct PendingSearchInResourceCommand {
    params: SearchInResourceParams,
    source: ResourceSearchSource,
    pending: PendingPageCommand,
}

pub(super) struct CompletedSearchInResourceCommand {
    params: SearchInResourceParams,
    source: ResourceSearchSource,
    completed: Result<CompletedPageCommand, String>,
}

impl CompletedSearchInResourceCommand {
    pub(super) fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::renderer_output_predecessor)
    }
}

impl PendingSearchInResourceCommand {
    pub(super) async fn wait(self) -> CompletedSearchInResourceCommand {
        CompletedSearchInResourceCommand {
            params: self.params,
            source: self.source,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
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

pub(super) fn try_start_search_in_resource_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: SearchInResourceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    match conn.page_domain_enabled_for_session_owner(cmd.session_id) {
        Some(true) => {}
        Some(false) => return complete_error(AGENT_NOT_ENABLED),
        None => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "TargetNotLoaded",
            ));
        }
    }

    let Some((root_frame_id, _, _, _)) =
        conn.target_session_owner_frame_tree_identity(cmd.session_id)
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };
    let is_root_frame = params.frame_id == root_frame_id;
    if !is_root_frame
        && !conn
            .target_owner_has_attached_child_frame_id_for_session(cmd.session_id, &params.frame_id)
            .unwrap_or(false)
    {
        return complete_error(FRAME_NOT_FOUND);
    }
    let materialize_limit = conn.response_body_materialize_limit();
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);

    if is_root_frame {
        let main_document = conn.current_main_document_resource_for_session_owner(cmd.session_id);
        let selected = if main_document.as_ref().is_some_and(|resource| {
            resource.frame_id == root_frame_id
                && resource_urls_match(resource.url.as_str(), &params.url)
        }) {
            match main_document.and_then(|resource| {
                resource
                    .body
                    .map(|body| (body, resource.response_headers, resource.from_cache))
            }) {
                Some((body, headers, from_cache))
                    if resource_has_searchable_content(body.len(), from_cache) =>
                {
                    match body.materialize_bytes_limited(materialize_limit) {
                        Ok(bytes) => SelectedResource::Text(decode_resource_content(
                            &bytes,
                            &headers,
                            ResourceContentKind::MainDocument,
                        )),
                        Err(_) => SelectedResource::Unavailable,
                    }
                }
                Some(_) => SelectedResource::Unavailable,
                None => SelectedResource::Unavailable,
            }
        } else {
            let Some(page) = loaded_page(conn, &owner) else {
                return complete_error(CONTENT_UNAVAILABLE);
            };
            select_subresource(page, &root_frame_id, true, &params.url, materialize_limit)
        };
        return start_selected_resource_search(conn, cmd.id, owner, params, selected);
    }

    let Some(page) = loaded_page(conn, &owner) else {
        return complete_error(CONTENT_UNAVAILABLE);
    };
    match page.start_child_frame_resource_search_by_lines(
        params.frame_id.clone(),
        params.url.clone(),
        params.query.clone(),
        params.case_sensitive,
        params.is_regex,
    ) {
        Ok(pending) => pending_step(
            cmd.id,
            owner,
            PendingSearchInResourceCommand {
                params,
                source: ResourceSearchSource::ChildDocument,
                pending,
            },
        ),
        Err(error) => complete_error(format!("Failed to search resource: {error}")),
    }
}

pub(super) fn complete_search_in_resource_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completed: CompletedSearchInResourceCommand,
) -> PageCommandTaskStep {
    let materialize_limit = conn.response_body_materialize_limit();
    let Some(page) = loaded_page(conn, owner) else {
        return complete_error(CONTENT_UNAVAILABLE);
    };
    let completion = match completed.completed {
        Ok(completion) => completion,
        Err(message) => return complete_error(format!("Failed to search resource: {message}")),
    };
    let outcome = match page.finish_resource_search_by_lines(completion) {
        Ok(outcome) => outcome,
        Err(error) => return complete_error(format!("Failed to search resource: {error}")),
    };

    match (completed.source, outcome) {
        (_, RendererResourceTextSearchOutcome::Matches(matches)) => complete_matches(matches),
        (_, RendererResourceTextSearchOutcome::FrameNotFound) => complete_error(FRAME_NOT_FOUND),
        (_, RendererResourceTextSearchOutcome::ContentUnavailable) => {
            complete_error(CONTENT_UNAVAILABLE)
        }
        (
            ResourceSearchSource::SelectedText,
            RendererResourceTextSearchOutcome::ResourceNotFound,
        ) => complete_error(RESOURCE_NOT_FOUND),
        (
            ResourceSearchSource::ChildDocument,
            RendererResourceTextSearchOutcome::ResourceNotFound,
        ) => {
            let selected = select_subresource(
                page,
                &completed.params.frame_id,
                false,
                &completed.params.url,
                materialize_limit,
            );
            start_selected_resource_search(
                conn,
                command_id,
                owner.clone(),
                completed.params,
                selected,
            )
        }
    }
}

fn start_selected_resource_search(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    params: SearchInResourceParams,
    selected: SelectedResource,
) -> PageCommandTaskStep {
    let text = match selected {
        SelectedResource::Text(text) => text,
        SelectedResource::Unavailable => return complete_error(CONTENT_UNAVAILABLE),
        SelectedResource::Missing => return complete_error(RESOURCE_NOT_FOUND),
    };
    let Some(page) = loaded_page(conn, &owner) else {
        return complete_error(CONTENT_UNAVAILABLE);
    };
    match page.start_text_search_by_lines(
        text,
        params.query.clone(),
        params.case_sensitive,
        params.is_regex,
    ) {
        Ok(pending) => pending_step(
            command_id,
            owner,
            PendingSearchInResourceCommand {
                params,
                source: ResourceSearchSource::SelectedText,
                pending,
            },
        ),
        Err(error) => complete_error(format!("Failed to search resource: {error}")),
    }
}

fn pending_step(
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    pending: PendingSearchInResourceCommand,
) -> PageCommandTaskStep {
    PageCommandTaskStep::Pending(PendingPageCommandDispatch {
        command_id,
        owner_scope,
        kind: Box::new(PendingPageCommandKind::SearchInResource(pending)),
    })
}

fn complete_matches(matches: Vec<RendererTextSearchMatch>) -> PageCommandTaskStep {
    PageCommandTaskStep::Complete(CommandOutputPlan::result(json!({
        "result": matches
            .into_iter()
            .map(|matched| json!({
                "lineNumber": matched.line_number,
                "lineContent": matched.line_content,
            }))
            .collect::<Vec<_>>(),
    })))
}

fn complete_error(message: impl Into<String>) -> PageCommandTaskStep {
    PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message))
}

fn loaded_page<'a>(conn: &'a mut CdpConnection, owner: &CommandOwnerScope) -> Option<&'a mut Page> {
    conn.runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
        .ok()?
        .loaded_page_mut()
}

fn select_subresource(
    page: &Page,
    frame_id: &str,
    root_frame: bool,
    requested_url: &str,
    materialize_limit: usize,
) -> SelectedResource {
    let record = page
        .subresource_network_records()
        .iter()
        .rev()
        .find(|record| {
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
    if !resource_has_searchable_content(response_body.len(), record.from_cache()) {
        return SelectedResource::Unavailable;
    }
    if response_body.len() > materialize_limit {
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
    headers: &[(String, String)],
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
    if is_dom_parser_xml_mime(&mime) || is_json_module_mime(&mime) {
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
        let headers = vec![(
            "content-type".to_owned(),
            "text/html; charset=windows-1252".to_owned(),
        )];
        assert_eq!(
            decode_resource_content(b"<p>\x80</p>", &headers, ResourceContentKind::MainDocument,),
            "<p>\u{20ac}</p>"
        );
    }

    #[test]
    fn stylesheet_resource_rejects_invalid_response_charset_whitespace() {
        let headers = vec![(
            "content-type".to_owned(),
            "text/css; charset=\nshift_jis".to_owned(),
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
                    "application/octet-stream".to_owned(),
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
