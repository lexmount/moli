use super::BrowserContext;
use moli_core::{
    browser::{CompletedContextPermissionUpdate, PendingContextPermissionUpdate},
    page::PermissionOverrideRegistration,
};

impl BrowserContext {
    pub(crate) fn set_permission_override(
        &mut self,
        registration: PermissionOverrideRegistration,
    ) -> Result<(), String> {
        self.browser_context.set_permission_override(registration)
    }

    pub(crate) fn clear_permission_overrides(&mut self) {
        self.browser_context.clear_permission_overrides();
    }

    pub(crate) fn permission_override_count(&self) -> usize {
        self.browser_context.permission_override_count()
    }

    pub(crate) fn permission_snapshot(&self) -> Vec<PermissionOverrideRegistration> {
        self.browser_context.permission_snapshot()
    }

    pub(crate) fn start_permission_update(
        &self,
    ) -> Result<Option<PendingContextPermissionUpdate>, String> {
        self.browser_context.start_permission_update()
    }

    pub(crate) fn finish_permission_update(
        &mut self,
        completed: CompletedContextPermissionUpdate,
    ) -> Result<(), String> {
        self.browser_context.finish_permission_update(completed)
    }
}
