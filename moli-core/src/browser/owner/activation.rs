use super::*;
use crate::browser::{BrowserEvent, BrowserEventRecord};

/// A reply subscription, not ownership of the Browser's activation work.
pub struct PendingWebContentsActivation {
    completion: oneshot::Receiver<Result<BrowserEventRecord, String>>,
}

impl PendingWebContentsActivation {
    pub async fn wait(self) -> Result<BrowserEventRecord, String> {
        self.completion
            .await
            .map_err(|_| "Browser stopped before completing activation".to_owned())?
    }
}

impl BrowserHandle {
    /// Commit selection and its semantic occurrence in one owner turn. Surface
    /// participants retain exact Documents and complete back on this same owner.
    pub fn activate_web_contents(
        &self,
        selected: WebContentsHandle,
    ) -> Result<PendingWebContentsActivation, String> {
        self.execute(move |browser| {
            let context = browser.context_mut(selected.context())?;
            let selected_has_dialog =
                context.web_contents_has_pending_javascript_dialog(selected)?;
            let previous = context.selected_web_contents_handle();
            let previous_has_dialog = previous
                .map(|handle| context.web_contents_has_pending_javascript_dialog(handle))
                .transpose()?
                .unwrap_or(false);
            if !context.select_web_contents(selected.id()) {
                return Err("WebContents unavailable".into());
            }
            let mut first_error = None;
            let mut updates = Vec::new();
            if previous != Some(selected) && !selected_has_dialog && !previous_has_dialog {
                for (handle, foreground) in
                    std::iter::once((selected, true)).chain(previous.map(|handle| (handle, false)))
                {
                    match context.start_web_contents_visibility_update(handle, foreground) {
                        Ok(Some(update)) => updates.push(update),
                        Ok(None) => {}
                        Err(error) => {
                            first_error.get_or_insert(error);
                        }
                    }
                }
            }
            let sequence = context
                .selected_web_contents_snapshot()
                .expect("selection just committed")
                .sequence;
            let event = browser.events.publish_committed(
                sequence,
                BrowserEvent::WebContentsActivated {
                    web_contents: selected,
                    previous,
                },
            );
            let (completion_tx, completion) = oneshot::channel();
            let local_sender = browser.local_sender.clone();
            tokio::task::spawn_local(async move {
                let mut completed = Vec::with_capacity(updates.len());
                for update in updates {
                    completed.push(update.wait().await);
                }
                let _ = local_sender.send(Box::new(move |browser| {
                    let result = browser.context_mut(selected.context()).and_then(|context| {
                        // The current selection may already have changed again.
                        // Only finish the captured Document operations; never
                        // write selection or resolve a replacement Document here.
                        for update in completed {
                            if let Err(error) = context.finish_document_policy_update(update) {
                                first_error.get_or_insert(error);
                            }
                        }
                        first_error.map_or(Ok(event), Err)
                    });
                    let _ = completion_tx.send(result);
                }));
            });
            Ok(PendingWebContentsActivation { completion })
        })?
    }
}
