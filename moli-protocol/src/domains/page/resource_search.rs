use std::sync::Arc;

use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererMainDocumentResource,
    RendererResourceContentBody, RendererResourceSearchRequest, RendererResourceTextSearchOutcome,
    RendererTextSearchMatch,
};
use serde::Deserialize;
use serde_json::json;

use super::{PageCommandTaskStep, PendingPageCommandDispatch, PendingPageCommandKind};
use crate::conn::{CapturedBody, CdpConnection, Cmd, CommandOwnerScope};
use crate::domains::command_output::CommandOutputPlan;

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

pub(super) struct PendingSearchInResourceCommand {
    pending: PendingPageCommand,
}

pub(super) struct CompletedSearchInResourceCommand {
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
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl RendererResourceContentBody for CapturedBody {
    fn read_bytes(&self, materialize_limit: usize) -> anyhow::Result<Vec<u8>> {
        self.materialize_bytes_limited(materialize_limit)
    }
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
        Some(false) => return complete_error("Agent is not enabled."),
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
    // The browser network owner retains the exact response body by loader.
    // Pass its immutable handle; source selection and decoding belong to the renderer.
    let main_document = conn
        .current_main_document_resource_for_session_owner(cmd.session_id)
        .map(|resource| RendererMainDocumentResource {
            frame_id: resource.frame_id,
            url: resource.url,
            response_headers: resource.response_headers,
            from_cache: resource.from_cache,
            body: resource
                .body
                .map(|body| Arc::new(body) as Arc<dyn RendererResourceContentBody>),
        });
    let request = RendererResourceSearchRequest {
        root_frame_id,
        frame_id: params.frame_id,
        url: params.url,
        query: params.query,
        case_sensitive: params.case_sensitive,
        is_regex: params.is_regex,
        materialize_limit: conn.response_body_materialize_limit(),
        main_document,
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return complete_error(CONTENT_UNAVAILABLE);
    };
    match page.start_resource_search_by_lines(request) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id: cmd.id,
            owner_scope: owner,
            kind: Box::new(PendingPageCommandKind::SearchInResource(
                PendingSearchInResourceCommand { pending },
            )),
        }),
        Err(error) => complete_error(format!("Failed to search resource: {error}")),
    }
}

pub(super) fn complete_search_in_resource_command(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completed: CompletedSearchInResourceCommand,
) -> PageCommandTaskStep {
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return complete_error(CONTENT_UNAVAILABLE);
    };
    let completion = match completed.completed {
        Ok(completion) => completion,
        Err(message) => return complete_error(format!("Failed to search resource: {message}")),
    };
    if page.renderer_agent_attachment_id() != completion.renderer_agent_attachment_id() {
        return complete_error(CONTENT_UNAVAILABLE);
    }
    match page.finish_resource_search_by_lines(completion) {
        Ok(RendererResourceTextSearchOutcome::Matches(matches)) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::result(json!({
                "result": matches.into_iter().map(|matched: RendererTextSearchMatch| json!({
                    "lineNumber": matched.line_number,
                    "lineContent": matched.line_content,
                })).collect::<Vec<_>>(),
            })))
        }
        Ok(RendererResourceTextSearchOutcome::FrameNotFound) => complete_error(FRAME_NOT_FOUND),
        Ok(RendererResourceTextSearchOutcome::ResourceNotFound) => {
            complete_error(RESOURCE_NOT_FOUND)
        }
        Ok(RendererResourceTextSearchOutcome::ContentUnavailable) => {
            complete_error(CONTENT_UNAVAILABLE)
        }
        Err(error) => complete_error(format!("Failed to search resource: {error}")),
    }
}

fn complete_error(message: impl Into<String>) -> PageCommandTaskStep {
    PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message))
}
