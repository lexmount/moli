use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::PreparedRuntimePageQuery;
use crate::conn::activity_source::{
    CompletedChildFrameLifecycleWork, PendingChildFrameLifecycleWork,
};
use crate::conn::{CommandOwnerScope, TargetRootDocumentProtocolAttachmentIdentity};
use crate::domains::command_output::CommandOutputPlan;
use moli_core::page::{CompletedPageCommand, Page, RendererCommandTurnOutput};

pub(crate) struct ChildNavigationDocument {
    pub(crate) loader_id: String,
    pub(crate) url: String,
}

pub(crate) struct StartedChildNavigationCompletion {
    pub(crate) output: Option<RendererCommandTurnOutput>,
    pub(crate) lifecycle: Result<Option<PendingChildFrameLifecycleWork>, CommandOutputPlan>,
}

impl ProtocolWorkflowState<'_> {
    pub(crate) fn start_child_navigation_completion(
        self,
        owner: CommandOwnerScope,
        binding: TargetRootDocumentProtocolAttachmentIdentity,
        completed: Result<CompletedPageCommand>,
        wait: bool,
        timeout: std::time::Duration,
    ) -> StartedChildNavigationCompletion {
        let conn = self.connection;
        let mut output = None;
        let lifecycle = (|| {
            if !conn.target_root_document_protocol_attachment_identity_is_current(&binding) {
                return Err(CommandOutputPlan::error(-31998, "NoSuchTarget"));
            }
            let completed =
                completed.map_err(|error| CommandOutputPlan::error(-32000, error.to_string()))?;
            let page = conn
                .loaded_page_mut_for_owner(&owner)
                .ok_or_else(|| CommandOutputPlan::error(-31998, "NoSuchTarget"))?;
            let (navigated, renderer_output) = page
                .finish_child_frame_navigation_to_url_command_turn(completed)
                .map_err(|error| CommandOutputPlan::error(-32000, error.to_string()))?;
            output = Some(renderer_output);
            if !navigated {
                return Err(CommandOutputPlan::error(-31998, "NoSuchTarget"));
            }
            if wait {
                conn.start_child_frame_lifecycle_work_for_owner(owner, timeout)
                    .map(Some)
                    .map_err(|error| CommandOutputPlan::error(-32000, error))
            } else {
                Ok(None)
            }
        })();
        StartedChildNavigationCompletion { output, lifecycle }
    }

    pub(crate) fn finish_child_navigation_lifecycle(
        self,
        binding: TargetRootDocumentProtocolAttachmentIdentity,
        frame_id: &str,
        completed: CompletedChildFrameLifecycleWork,
    ) -> Result<
        (
            bool,
            Option<ChildNavigationDocument>,
            RendererCommandTurnOutput,
        ),
        CommandOutputPlan,
    > {
        if !self
            .connection
            .target_root_document_protocol_attachment_identity_is_current(&binding)
        {
            return Err(CommandOutputPlan::error(-31998, "NoSuchTarget"));
        }
        let (loaded, output) = self
            .connection
            .complete_child_frame_lifecycle_work_command_turn_for_session_owner(completed)
            .map_err(|error| CommandOutputPlan::error(-32000, error))?;
        let frame = self
            .connection
            .browsing_context_owner(frame_id)
            .and_then(|frame| {
                Some(ChildNavigationDocument {
                    loader_id: frame.loader_id?,
                    url: frame.url,
                })
            });
        Ok((loaded, frame, output))
    }

    pub(crate) fn prepare_child_navigation_document_query(
        self,
        owner: &CommandOwnerScope,
        frame_id: String,
    ) -> Result<PreparedRuntimePageQuery<Option<ChildNavigationDocument>>> {
        PreparedRuntimePageQuery::prepare_resident(
            self.connection,
            owner,
            "child navigation identity lookup failed",
            |page| page.start_child_frame_tree_snapshot(),
            move |page, completion| {
                let mut pending = Page::finish_child_frame_tree_snapshot(page, completion)?;
                while let Some(frame) = pending.pop() {
                    if frame.frame_id == frame_id {
                        return Ok(Some(ChildNavigationDocument {
                            loader_id: frame.loader_id,
                            url: frame.url,
                        }));
                    }
                    pending.extend(frame.child_frames);
                }
                Ok(None)
            },
        )
    }
}
