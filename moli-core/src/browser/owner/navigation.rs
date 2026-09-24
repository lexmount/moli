use tokio::sync::oneshot;

use super::{Browser, PendingDocumentRetirement};
use crate::browser::{WebContentsHandle, web_contents::PreparedDocumentNavigation};

#[cfg(test)]
mod tests;

pub(super) struct BrowserDocumentNavigationCommit {
    pub snapshot: crate::browser::web_contents::DocumentCommitSnapshot,
    pub retirement: PendingDocumentRetirement,
    pub post_response_continuation:
        Option<crate::page::RendererPageCommandPostResponseContinuation>,
}

impl Browser {
    pub(super) fn commit_navigation(
        &mut self,
        contents: WebContentsHandle,
        prepared: PreparedDocumentNavigation,
    ) -> Result<BrowserDocumentNavigationCommit, String> {
        if contents.id() != prepared.web_contents_id() {
            return Err("navigation document belongs to another WebContents".to_owned());
        }
        let context = contents.context();
        let dialogs = self
            .context(context)?
            .web_contents_javascript_dialog_snapshots(contents);
        let commit = self
            .context_mut(context)?
            .commit_document_navigation(prepared)?;
        let document = crate::browser::DocumentHandle::new(contents, commit.document);
        let snapshot = self.context(context)?.document_commit_snapshot(document)?;
        self.events.publish_committed(
            commit.lifecycle.browser_sequence,
            crate::browser::BrowserEvent::DocumentCommitted(document),
        );
        self.observe_document_lifecycle(document);
        self.publish_closed_javascript_dialogs(dialogs);
        self.observe_javascript_dialogs(document);
        self.observe_popup_inputs(document);
        let (completion_tx, completion) = oneshot::channel();
        tokio::task::spawn_local(async move {
            commit.retirement.close().await;
            let _ = completion_tx.send(());
        });
        Ok(BrowserDocumentNavigationCommit {
            snapshot,
            retirement: PendingDocumentRetirement { completion },
            post_response_continuation: commit.post_response_continuation,
        })
    }
}
