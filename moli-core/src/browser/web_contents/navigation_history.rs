use super::WebContents;
use super::{HistoryTraversalDestination, PageNavigationHistoryEntry, ResolvedHistoryTraversal};
use crate::{
    browser::{DocumentId, WebContentsId},
    page::{CompletedPageCommand, PendingPageCommand, SameDocumentHistoryUpdate},
};
use url::Url;

#[cfg(test)]
mod tests;

pub struct SameDocumentNavigationCommitted {
    pub web_contents: WebContentsId,
    pub document: DocumentId,
    pub url: Url,
}

impl WebContents {
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_navigation_history_for_test(&mut self, snapshot: (String, String)) {
        self.navigation.record_navigation_history_for_test(snapshot);
    }

    pub fn mark_renderer_crashed(&mut self) {
        self.crashed = true;
        self.navigation.clear_navigation_history();
    }

    pub fn resolve_history_traversal(
        &self,
        destination: HistoryTraversalDestination,
    ) -> Result<ResolvedHistoryTraversal, &'static str> {
        self.navigation.resolve_history_traversal(destination)
    }

    pub fn navigation_history_snapshot(&self) -> (usize, Vec<PageNavigationHistoryEntry>) {
        self.navigation.navigation_history_snapshot()
    }

    pub fn navigation_history_entry_url(&self, entry_id: i32) -> Option<String> {
        self.navigation.navigation_history_entry_url(entry_id)
    }

    pub fn commit_document_title(
        &mut self,
        change: &crate::RendererDocumentTitleChanged,
    ) -> Option<bool> {
        let document = self.main_frame.current_document.as_ref()?;
        let snapshot = document.lifecycle.snapshot()?;
        if (snapshot.frame, snapshot.document, snapshot.epoch)
            != (
                change.source_document.frame,
                change.source_document.document,
                change.source_document.epoch,
            )
        {
            return None;
        }
        Some(
            self.navigation
                .refresh_current_navigation_history_title(change.title.clone()),
        )
    }

    pub fn commit_same_document_navigation(
        &mut self,
        document_id: DocumentId,
        url: Url,
        history_update: SameDocumentHistoryUpdate,
    ) -> Option<SameDocumentNavigationCommitted> {
        let document = self.main_frame.current_document.as_ref()?;
        // A renderer document.open epoch does not undo a history operation
        // already performed on this Browser Document. Replacement does.
        if document.id != document_id || self.navigation.has_pending_document_navigation() {
            return None;
        }
        let title = self
            .navigation
            .current_history_title()
            .unwrap_or_default()
            .to_owned();
        if !self.navigation.record_same_document_navigation_history(
            url.to_string(),
            title,
            history_update,
        ) {
            return None;
        }
        Some(SameDocumentNavigationCommitted {
            web_contents: self.id(),
            document: document_id,
            url,
        })
    }

    pub fn start_reset_navigation_history(&self) -> Result<PendingPageCommand, String> {
        if !self.navigation.can_reset_navigation_history() {
            return Err("History cannot be pruned".to_owned());
        }
        self.main_frame
            .current_document
            .as_ref()
            .ok_or("NoDocumentLoaded")?
            .page
            .start_reset_navigation_history()
            .map_err(|error| error.to_string())
    }

    pub fn finish_reset_navigation_history(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool, String> {
        let document = self
            .main_frame
            .current_document
            .as_mut()
            .ok_or("NoDocumentLoaded")?;
        if !completion.is_from_page(&document.page) {
            return Err("stale history reset document".to_owned());
        }
        let pruned = document
            .page
            .finish_reset_navigation_history(completion)
            .map_err(|error| error.to_string())?;
        Ok(pruned && self.navigation.reset_navigation_history())
    }
}
