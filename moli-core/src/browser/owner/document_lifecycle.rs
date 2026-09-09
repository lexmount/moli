use super::Browser;
use crate::browser::{BrowserEvent, DocumentHandle, DocumentLifecycleSnapshot};
use crate::page::RendererDocumentLifecycleSnapshot;

impl Browser {
    pub(super) fn observe_document_lifecycle(&mut self, document: DocumentHandle) {
        let Ok(host) = self
            .context_mut(document.web_contents().context())
            .and_then(|context| context.document_mut(document))
        else {
            return;
        };
        let Some(mut renderer) = host.page.observe_document_lifecycle() else {
            return;
        };
        let Some(mut title) = host.page.observe_document_title() else {
            return;
        };
        let retirement = host.lifetime.observe();
        let snapshot = renderer.snapshot();
        self.commit_document_lifecycle(document, snapshot);
        self.commit_native_document_title(document, &title.borrow_and_update().clone());
        let sender = self.local_sender.clone();
        tokio::task::spawn_local(async move {
            let retirement = retirement.wait();
            tokio::pin!(retirement);
            loop {
                let snapshot = tokio::select! {
                    _ = &mut retirement => break,
                    changed = title.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        renderer.snapshot()
                    },
                    snapshot = renderer.changed() => match snapshot {
                        Some(snapshot) => snapshot,
                        None => break,
                    },
                };
                let title = title.borrow_and_update().clone();
                // One outstanding completion per physical Document. Renderer
                // progress coalesces while the Browser owner is busy.
                let (committed, completion) = tokio::sync::oneshot::channel();
                if sender
                    .send(Box::new(move |browser| {
                        browser.commit_document_lifecycle(document, snapshot);
                        browser.commit_native_document_title(document, &title);
                        let _ = committed.send(());
                    }))
                    .is_err()
                {
                    break;
                }
                if completion.await.is_err() {
                    break;
                }
            }
        });
    }

    fn commit_native_document_title(
        &mut self,
        document: DocumentHandle,
        change: &crate::RendererDocumentTitleChanged,
    ) {
        let Ok(context) = self.context_mut(document.web_contents().context()) else {
            return;
        };
        if context.ensure_document_current(document).is_ok()
            && context.commit_document_title(document.web_contents(), change) == Ok(Some(true))
        {
            self.events
                .publish(BrowserEvent::DocumentTitleChanged(document));
        }
    }

    pub(super) fn commit_document_lifecycle(
        &mut self,
        document: DocumentHandle,
        lifecycle: RendererDocumentLifecycleSnapshot,
    ) {
        let Ok(context) = self.context_mut(document.web_contents().context()) else {
            return;
        };
        if context.ensure_document_current(document).is_err() {
            return;
        }
        let mut dialogs = context.web_contents_javascript_dialog_snapshots(document.web_contents());
        if context
            .web_contents_mut(document.web_contents())
            .is_ok_and(|contents| contents.observe_native_document_lifecycle(lifecycle))
        {
            dialogs.retain(|dialog| {
                context
                    .document_javascript_dialog_snapshot(document, dialog.key)
                    .is_none()
            });
            self.events.publish(BrowserEvent::DocumentLifecycleChanged(
                DocumentLifecycleSnapshot {
                    document,
                    lifecycle,
                },
            ));
            self.publish_closed_javascript_dialogs(dialogs);
        }
    }
}
