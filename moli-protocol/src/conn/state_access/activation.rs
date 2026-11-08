use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{CompletedSessionPageUpdates, TargetSurfaceUpdate};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_window_client_focus(
        self,
        output: crate::domains::page::window_client_focus::WindowClientFocusOutput,
    ) -> Option<(
        crate::domains::target::activation::PreparedTargetActivation,
        crate::domains::page::window_client_focus::WindowClientFocusActivationGuard,
    )> {
        output.prepare(self.connection)
    }

    pub(crate) fn complete_window_client_focus(
        self,
        guard: crate::domains::page::window_client_focus::WindowClientFocusActivationGuard,
        result: Result<crate::domains::target::activation::TargetActivationOutput>,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        guard.complete(self.connection, result)
    }

    pub(crate) fn finish_target_surface_update(
        self,
        browser_context_id: String,
        target_id: String,
        completed: CompletedSessionPageUpdates,
    ) -> Result<()> {
        TargetSurfaceUpdate::finish(self.connection, &browser_context_id, &target_id, completed)
    }

    pub(crate) fn complete_target_activation_selection(
        self,
        selection: Option<crate::domains::target::activation::TargetActivationSelection>,
    ) -> Result<Vec<TargetSurfaceUpdate>> {
        crate::domains::target::activation::TargetActivationSelection::complete(
            selection,
            self.connection,
        )
    }
}
