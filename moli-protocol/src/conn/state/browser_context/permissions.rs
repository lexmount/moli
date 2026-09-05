use super::{BrowserContext, physical::BrowserContext as PhysicalBrowserContext};
use moli_core::{
    browser::{BrowserContextId, DocumentId, PermissionDefaults, WebContentsId},
    page::{CompletedPageCommand, PendingPageCommand, PermissionOverrideRegistration},
};

#[cfg(test)]
mod tests;

pub(crate) struct PendingContextPermissionUpdate {
    context_id: BrowserContextId,
    pages: Vec<PendingPermissionPageUpdate>,
}

struct PendingPermissionPageUpdate {
    contents_id: WebContentsId,
    document_id: DocumentId,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedContextPermissionUpdate {
    context_id: BrowserContextId,
    pages: Vec<CompletedPermissionPageUpdate>,
}

struct CompletedPermissionPageUpdate {
    contents_id: WebContentsId,
    document_id: DocumentId,
    completed: Result<CompletedPageCommand, String>,
}

impl PendingContextPermissionUpdate {
    pub(crate) async fn wait(self) -> CompletedContextPermissionUpdate {
        let mut pages = Vec::with_capacity(self.pages.len());
        for page in self.pages {
            pages.push(CompletedPermissionPageUpdate {
                contents_id: page.contents_id,
                document_id: page.document_id,
                completed: page.pending.wait().await.map_err(|error| error.to_string()),
            });
        }
        CompletedContextPermissionUpdate {
            context_id: self.context_id,
            pages,
        }
    }
}

impl CompletedContextPermissionUpdate {
    pub(crate) fn context_id(&self) -> BrowserContextId {
        self.context_id
    }
}

impl BrowserContext {
    pub(crate) fn set_permission_override(
        &mut self,
        defaults: &mut PermissionDefaults,
        registration: PermissionOverrideRegistration,
    ) {
        self.physical
            .permission_overrides
            .set(defaults, registration);
    }

    pub(crate) fn clear_permission_overrides(&mut self) {
        self.physical.permission_overrides.clear();
    }

    pub(crate) fn permission_override_count(&self) -> usize {
        self.physical.permission_overrides.override_count()
    }

    pub(crate) fn permission_snapshot(
        &self,
        defaults: &PermissionDefaults,
    ) -> Vec<PermissionOverrideRegistration> {
        self.physical.permission_overrides.snapshot(defaults)
    }

    pub(crate) fn start_permission_update(
        &self,
        defaults: &PermissionDefaults,
    ) -> Result<Option<PendingContextPermissionUpdate>, String> {
        self.physical.start_permission_update(defaults)
    }

    pub(crate) fn finish_permission_update(
        &mut self,
        completed: CompletedContextPermissionUpdate,
    ) -> Result<(), String> {
        self.physical.finish_permission_update(completed)
    }
}

impl PhysicalBrowserContext {
    fn start_permission_update(
        &self,
        defaults: &PermissionDefaults,
    ) -> Result<Option<PendingContextPermissionUpdate>, String> {
        let mut pages = Vec::new();
        let overrides = self.permission_overrides.snapshot(defaults);
        for (contents_id, contents) in &self.web_contents {
            let Some(document) = &contents.main_frame.current_document else {
                continue;
            };
            pages.push(PendingPermissionPageUpdate {
                contents_id: *contents_id,
                document_id: document.id,
                pending: document
                    .page
                    .start_set_permission_overrides(&overrides)
                    .map_err(|error| {
                        format!("failed to update page permission overrides: {error}")
                    })?,
            });
        }
        Ok(
            (!pages.is_empty()).then_some(PendingContextPermissionUpdate {
                context_id: self.id,
                pages,
            }),
        )
    }

    fn finish_permission_update(
        &mut self,
        completed: CompletedContextPermissionUpdate,
    ) -> Result<(), String> {
        if completed.context_id != self.id {
            return Err("Permission update belongs to a different BrowserContext".into());
        }
        for page in completed.pages {
            let completion = page.completed?;
            let document = self
                .web_contents
                .get_mut(&page.contents_id)
                .and_then(|contents| contents.main_frame.current_document.as_mut())
                .ok_or("NoDocumentLoaded")?;
            if document.id == page.document_id {
                document
                    .page
                    .finish_set_permission_overrides(completion)
                    .map_err(|error| error.to_string())?;
            } else {
                // The policy is Context-owned and will be inherited by the
                // replacement. Keep a frozen successful acknowledgement, but
                // never install the outgoing Document's snapshot into it.
                completion
                    .into_unit_page_command_turn()
                    .map(drop)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}
