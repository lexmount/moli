use super::{CdpConnection, CompletedContextPermissionUpdate, PendingContextPermissionUpdate};
use moli_core::page::PermissionOverrideRegistration;

impl CdpConnection {
    pub fn effective_permission_overrides_for_browser_context_id(
        &self,
        browser_context_id: &str,
    ) -> Vec<PermissionOverrideRegistration> {
        self.browser_context_by_id(browser_context_id)
            .map(|context| context.permission_snapshot())
            .unwrap_or_else(|| self.browser.permission_defaults_snapshot())
    }

    pub(crate) fn set_permission_override(
        &mut self,
        context_id: Option<&str>,
        registration: PermissionOverrideRegistration,
    ) -> Result<(), String> {
        if let Some(id) = context_id {
            let context = self
                .browser_context
                .iter_mut()
                .chain(&mut self.inactive_browser_contexts)
                .find(|context| context.id == id)
                .ok_or("UnknownBrowserContextId")?;
            context.set_permission_override(registration)?;
        } else {
            self.browser.set_permission_default(registration);
        }
        Ok(())
    }

    pub(crate) fn clear_permission_overrides(
        &mut self,
        context_id: Option<&str>,
    ) -> Result<(), String> {
        if let Some(id) = context_id {
            self.browser_context_by_id_mut(id)
                .ok_or("UnknownBrowserContextId")?
                .clear_permission_overrides();
        } else {
            self.browser.clear_permission_defaults();
            for context in self
                .browser_context
                .iter_mut()
                .chain(&mut self.inactive_browser_contexts)
            {
                context.clear_permission_overrides();
            }
        }
        Ok(())
    }

    pub(crate) fn permission_override_count(&self) -> usize {
        self.browser.permission_default_count()
            + self
                .browser_contexts()
                .map(|context| context.permission_override_count())
                .sum::<usize>()
    }

    pub(crate) fn start_permission_updates(
        &self,
    ) -> Result<Vec<PendingContextPermissionUpdate>, String> {
        let mut pending = Vec::new();
        for context in self.browser_contexts() {
            if let Some(update) = context.start_permission_update()? {
                pending.push(update);
            }
        }
        Ok(pending)
    }

    pub(crate) fn finish_permission_update(
        &mut self,
        completed: CompletedContextPermissionUpdate,
    ) -> Result<(), String> {
        self.browser_context
            .iter_mut()
            .chain(&mut self.inactive_browser_contexts)
            .find(|context| context.browser_context_id() == completed.context_id())
            .ok_or("NoDocumentLoaded")?
            .finish_permission_update(completed)
    }
}
