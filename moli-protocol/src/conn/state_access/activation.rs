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
        selection: Option<(String, String)>,
    ) -> Result<Vec<TargetSurfaceUpdate>> {
        if let Some((browser_context_id, target_id)) = selection
            && self
                .connection
                .browser_context_by_id(&browser_context_id)
                .is_some_and(|context| context.is_active_target(&target_id))
        {
            let restore = self
                .connection
                .browser_context
                .as_ref()
                .map(|context| context.id.clone());
            self.connection
                .activate_browser_context_by_id(&browser_context_id);
            // The old async refresh passed no loaded Page, so its entire
            // operation is this finite engine-policy/resource reset.
            self.connection.apply_active_engine_fetch_overrides();
            self.connection.invalidate_resource_runtime();
            self.connection.notify_target_host_activated(&target_id);
            if let Some(restore) = restore {
                self.connection.activate_browser_context_by_id(&restore);
            }
        }
        self.connection.start_browser_document_activity_updates()
    }
}
