use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::activity_source::{
    CompletedChildFrameLifecycleWork, PendingChildFrameLifecycleWork,
};
use crate::conn::{CommandOwnerScope, TargetRootDocumentProtocolAttachmentIdentity};
use crate::domains::command_output::CommandOutputPlan;
use moli_core::page::{CompletedPageCommand, RendererCommandTurnOutput};

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
        completed: CompletedChildFrameLifecycleWork,
    ) -> Result<(bool, RendererCommandTurnOutput), CommandOutputPlan> {
        if !self
            .connection
            .target_root_document_protocol_attachment_identity_is_current(&binding)
        {
            return Err(CommandOutputPlan::error(-31998, "NoSuchTarget"));
        }
        self.connection
            .complete_child_frame_lifecycle_work_command_turn_for_session_owner(completed)
            .map_err(|error| CommandOutputPlan::error(-32000, error))
    }
}
