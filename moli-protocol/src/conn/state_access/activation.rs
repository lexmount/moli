use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{CompletedSessionPageUpdates, TargetSurfaceUpdate};

impl ProtocolWorkflowState<'_> {
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
