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
    pub(crate) navigation_id: Option<u64>,
    pub(crate) loader_id: String,
    pub(crate) url: String,
}

pub(crate) struct StartedChildNavigationCompletion {
    pub(crate) output: Option<RendererCommandTurnOutput>,
    pub(crate) lifecycle: Result<(u64, Option<PendingChildFrameLifecycleWork>), CommandOutputPlan>,
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
            let Some(navigation_id) = navigated else {
                return Err(CommandOutputPlan::error(-31998, "NoSuchTarget"));
            };
            let lifecycle = if wait {
                conn.start_child_frame_lifecycle_work_for_owner(owner, timeout)
                    .map(Some)
                    .map_err(|error| CommandOutputPlan::error(-32000, error))
            } else {
                Ok(None)
            }?;
            Ok((navigation_id, lifecycle))
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
        let (loaded, output) = self
            .connection
            .complete_child_frame_lifecycle_work_command_turn_for_session_owner(completed)
            .map_err(|error| CommandOutputPlan::error(-32000, error))?;
        Ok((loaded, output))
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
            move |page| page.start_child_frame_navigation_identity_snapshot(&frame_id),
            move |page, completion| {
                Ok(
                    Page::finish_child_frame_navigation_identity_snapshot(page, completion)?.map(
                        |frame| ChildNavigationDocument {
                            navigation_id: frame.navigation_id,
                            loader_id: frame.loader_id,
                            url: frame.url,
                        },
                    ),
                )
            },
        )
    }
}
