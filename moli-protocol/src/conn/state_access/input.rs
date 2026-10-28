use super::ProtocolWorkflowState;
use crate::domains::input::file_chooser::PreparedFileChooserActivation;
use crate::{
    automation::DevToolsRemoteHandleId,
    conn::{BackgroundProtocolEvent, CommandOwnerScope, TargetPageResidenceIdentity},
};
use anyhow::Result;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererCommandTurnOutput,
    RendererDomBidiNodeSharedIdResolution, RendererPendingDownloadActivation,
    RendererPendingFileChooserActivation,
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn settle_input_page_command(
        self,
        owner: &TargetPageResidenceIdentity,
        host: Option<&crate::conn::ProtocolPageHostScope>,
        completed: CompletedPageCommand,
    ) -> RendererCommandTurnOutput {
        if host.is_some_and(|host| !host.is_current(self.connection)) {
            return completed.into_output();
        }
        self.connection
            .settle_page_command_turn_for_owner(owner, completed)
    }
    pub(crate) fn prepare_input_download(
        self,
        owner: &CommandOwnerScope,
        page_owner: &TargetPageResidenceIdentity,
        host: Option<&crate::conn::ProtocolPageHostScope>,
        activation: RendererPendingDownloadActivation,
    ) -> Result<Option<crate::conn::downloads::PreparedDownloadActivationDelivery>> {
        if host.is_some_and(|host| !host.is_current(self.connection))
            || self
                .connection
                .target_page_residence_identity_for_owner(owner)
                .as_ref()
                != Some(page_owner)
        {
            return Ok(None);
        }
        self.connection
            .prepare_pending_download_activation_delivery(owner, activation, true)
            .map(Some)
    }
    pub(crate) fn capture_input_file_chooser(
        self,
        owner: &CommandOwnerScope,
        page_owner: TargetPageResidenceIdentity,
        host: Option<&crate::conn::ProtocolPageHostScope>,
        activation: RendererPendingFileChooserActivation,
    ) -> Option<PreparedFileChooserActivation> {
        if host.is_some_and(|host| !host.is_current(self.connection)) {
            return None;
        }
        PreparedFileChooserActivation::capture(self.connection, owner, page_owner, activation)
    }
    pub(crate) fn start_input_file_chooser_query(
        self,
        owner: &CommandOwnerScope,
        activation: &PreparedFileChooserActivation,
    ) -> Option<Result<PendingPageCommand>> {
        activation.start_query(self.connection, owner)
    }
    pub(crate) fn finish_input_file_chooser_query(
        self,
        activation: &PreparedFileChooserActivation,
        completed: CompletedPageCommand,
    ) -> Result<RendererDomBidiNodeSharedIdResolution> {
        activation.finish_query(self.connection, completed)
    }
    pub(crate) fn start_input_file_chooser_binding(
        self,
        owner: &CommandOwnerScope,
        activation: &PreparedFileChooserActivation,
        shared_id: String,
    ) -> Result<PendingPageCommand> {
        activation.start_binding(self.connection, owner, shared_id)
    }
    pub(crate) fn finish_input_file_chooser_binding(
        self,
        activation: &PreparedFileChooserActivation,
        completed: CompletedPageCommand,
    ) -> Result<()> {
        activation.finish_binding(self.connection, completed)
    }
    pub(crate) fn finish_input_file_chooser_activation(
        self,
        session_id: Option<&str>,
        activation: PreparedFileChooserActivation,
        shared_id: DevToolsRemoteHandleId,
    ) -> Option<BackgroundProtocolEvent> {
        activation.finish_event(self.connection, session_id, shared_id)
    }
}
