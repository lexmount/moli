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
        let retirement = host.lifetime.observe();
        let snapshot = renderer.snapshot();
        self.commit_document_lifecycle(document, snapshot);
        let sender = self.local_sender.clone();
        tokio::task::spawn_local(async move {
            let retirement = retirement.wait();
            tokio::pin!(retirement);
            loop {
                let snapshot = tokio::select! {
                    _ = &mut retirement => break,
                    snapshot = renderer.changed() => match snapshot {
                        Some(snapshot) => snapshot,
                        None => break,
                    },
                };
                // One outstanding completion per physical Document. Renderer
                // progress coalesces while the Browser owner is busy.
                let (committed, completion) = tokio::sync::oneshot::channel();
                if sender
                    .send(Box::new(move |browser| {
                        browser.commit_document_lifecycle(document, snapshot);
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
        if context
            .web_contents_mut(document.web_contents())
            .is_ok_and(|contents| contents.observe_native_document_lifecycle(lifecycle))
        {
            self.events.publish(BrowserEvent::DocumentLifecycleChanged(
                DocumentLifecycleSnapshot {
                    document,
                    lifecycle,
                },
            ));
        }
    }
}
