use super::{Browser, BrowserHandle};
use crate::browser::{
    BrowserEvent, DocumentHandle, JavaScriptDialogOpened, RendererPageResidenceIdentity,
};
use crate::page::RendererJavaScriptDialogOpening;

impl Browser {
    pub(super) fn observe_javascript_dialogs(&mut self, document: DocumentHandle) {
        let Ok(host) = self
            .context_mut(document.web_contents().context())
            .and_then(|context| context.document_mut(document))
        else {
            return;
        };
        let mut renderer = host.page.observe_javascript_dialogs();
        let retirement = host.lifetime.observe();
        self.commit_javascript_dialogs(document);
        let sender = self.local_sender.clone();
        tokio::task::spawn_local(async move {
            let retirement = retirement.wait();
            tokio::pin!(retirement);
            loop {
                tokio::select! {
                    _ = &mut retirement => break,
                    changed = renderer.changed() => if !changed { break; },
                }
                // Wakeups coalesce, requests do not. Drain the original Page's
                // broker only inside its Browser owner turn, never from CDP.
                let (committed, completion) = tokio::sync::oneshot::channel();
                if sender
                    .send(Box::new(move |browser| {
                        browser.commit_javascript_dialogs(document);
                        let _ = committed.send(());
                    }))
                    .is_err()
                    || completion.await.is_err()
                {
                    break;
                }
            }
        });
    }

    pub(super) fn commit_javascript_dialogs(&mut self, document: DocumentHandle) {
        let Ok(context) = self.context_mut(document.web_contents().context()) else {
            return;
        };
        let Ok(host) = context.document(document) else {
            return;
        };
        let dialogs = host.page.take_pending_javascript_dialogs();
        for dialog in dialogs {
            let opening = dialog.opening();
            if let Ok(Some(key)) = self
                .context_mut(document.web_contents().context())
                .and_then(|context| context.install_document_javascript_dialog(document, dialog))
            {
                self.events
                    .publish(BrowserEvent::DialogOpened(JavaScriptDialogOpened {
                        document,
                        key,
                        opening,
                    }));
            }
        }
    }

    pub(super) fn publish_closed_javascript_dialogs(
        &mut self,
        dialogs: Vec<JavaScriptDialogOpened>,
    ) {
        for dialog in dialogs {
            self.events.publish(BrowserEvent::DialogClosed {
                document: dialog.document,
                key: dialog.key,
            });
        }
    }
}

impl BrowserHandle {
    /// Observe admission of this exact concrete renderer fact. The caller
    /// cannot install a request or reselect a replacement Page after awaiting.
    pub async fn wait_for_renderer_javascript_dialog(
        &self,
        renderer: RendererPageResidenceIdentity,
        opening: std::sync::Arc<RendererJavaScriptDialogOpening>,
    ) -> Option<JavaScriptDialogOpened> {
        let (document, mut admission) = self
            .execute(move |browser| {
                let document = browser
                    .contexts
                    .values()
                    .find_map(|context| context.document_for_renderer(renderer))?;
                let context = browser
                    .context_mut(document.web_contents().context())
                    .ok()?;
                let admission = context
                    .web_contents_mut(document.web_contents())
                    .ok()?
                    .javascript_dialogs
                    .observe_admission();
                Some((document, admission))
            })
            .ok()??;
        loop {
            if *admission.borrow_and_update() >= opening.id.sequence() {
                return Some(JavaScriptDialogOpened {
                    document,
                    key: crate::browser::web_contents::JavaScriptDialogKey::new(
                        document.id(),
                        opening.source_document,
                        opening.id,
                    ),
                    opening,
                });
            }
            admission.changed().await.ok()?;
        }
    }
}
