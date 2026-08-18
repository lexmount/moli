use std::{collections::VecDeque, sync::Arc};

use parking_lot::{Condvar, Mutex};
use tokio::sync::watch;

use super::page_surface::RendererPendingJavaScriptDialog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererJavaScriptDialogResult {
    pub accepted: bool,
    pub user_input: String,
}

#[derive(Clone, Debug)]
pub struct RendererJavaScriptDialogCompletion {
    inner: Arc<RendererJavaScriptDialogCompletionInner>,
}

#[derive(Debug)]
struct RendererJavaScriptDialogCompletionInner {
    result: Mutex<Option<RendererJavaScriptDialogResult>>,
    notify: Condvar,
}

impl RendererJavaScriptDialogCompletion {
    pub fn pending() -> Self {
        Self {
            inner: Arc::new(RendererJavaScriptDialogCompletionInner {
                result: Mutex::new(None),
                notify: Condvar::new(),
            }),
        }
    }

    pub fn finish(&self, accepted: bool, user_input: String) -> bool {
        let mut result = self.inner.result.lock();
        if result.is_some() {
            return false;
        }
        *result = Some(RendererJavaScriptDialogResult {
            accepted,
            user_input,
        });
        self.inner.notify.notify_all();
        true
    }

    pub fn wait(&self) -> RendererJavaScriptDialogResult {
        let mut result = self.inner.result.lock();
        loop {
            if let Some(result) = result.as_ref() {
                return result.clone();
            }
            self.inner.notify.wait(&mut result);
        }
    }
}

impl PartialEq for RendererJavaScriptDialogCompletion {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for RendererJavaScriptDialogCompletion {}

#[derive(Debug, Default)]
struct RendererJavaScriptDialogBrokerState {
    pending: VecDeque<RendererPendingJavaScriptDialog>,
    open_count: usize,
}

#[derive(Debug)]
struct RendererJavaScriptDialogBrokerInner {
    state: Mutex<RendererJavaScriptDialogBrokerState>,
    open_signal_tx: watch::Sender<()>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererJavaScriptDialogBroker {
    inner: Arc<RendererJavaScriptDialogBrokerInner>,
}

impl Default for RendererJavaScriptDialogBroker {
    fn default() -> Self {
        let (open_signal_tx, _) = watch::channel(());
        Self {
            inner: Arc::new(RendererJavaScriptDialogBrokerInner {
                state: Mutex::new(RendererJavaScriptDialogBrokerState::default()),
                open_signal_tx,
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererJavaScriptDialogWatch {
    broker: RendererJavaScriptDialogBroker,
    open_signal_rx: watch::Receiver<()>,
}

impl RendererJavaScriptDialogBroker {
    fn open(&self, dialog: RendererPendingJavaScriptDialog) {
        {
            let mut state = self.inner.state.lock();
            state.open_count += 1;
            state.pending.push_back(dialog);
        }
        self.inner.open_signal_tx.send_modify(|_| {});
    }

    pub(crate) fn take_pending(&self) -> Vec<RendererPendingJavaScriptDialog> {
        self.inner.state.lock().pending.drain(..).collect()
    }

    pub(crate) fn dismiss_pending(&self) {
        let pending = self
            .inner
            .state
            .lock()
            .pending
            .drain(..)
            .collect::<Vec<_>>();
        for dialog in pending {
            let _ = dialog.finish(false, String::new());
        }
    }

    fn close(&self, completion: &RendererJavaScriptDialogCompletion) {
        let mut state = self.inner.state.lock();
        state
            .pending
            .retain(|dialog| !dialog.completion_matches(completion));
        assert!(
            state.open_count != 0,
            "closing a JavaScript dialog requires a matching open broker entry"
        );
        state.open_count -= 1;
    }

    pub(crate) fn watch(&self) -> RendererJavaScriptDialogWatch {
        RendererJavaScriptDialogWatch {
            broker: self.clone(),
            open_signal_rx: self.inner.open_signal_tx.subscribe(),
        }
    }

    fn has_open_dialog(&self) -> bool {
        self.inner.state.lock().open_count != 0
    }
}

impl RendererJavaScriptDialogWatch {
    pub(crate) async fn wait_until_open(mut self) {
        loop {
            if self.broker.has_open_dialog() {
                return;
            }
            if self.open_signal_rx.changed().await.is_err() {
                return;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RendererJavaScriptDialogRuntime {
    broker: RendererJavaScriptDialogBroker,
}

impl RendererJavaScriptDialogRuntime {
    pub(crate) fn broker(&self) -> RendererJavaScriptDialogBroker {
        self.broker.clone()
    }

    pub(crate) fn begin_modal(
        &self,
        mut dialog: RendererPendingJavaScriptDialog,
    ) -> (
        RendererPendingJavaScriptDialog,
        RendererModalJavaScriptDialog,
    ) {
        let completion = RendererJavaScriptDialogCompletion::pending();
        dialog.install_completion(completion.clone());
        self.broker.open(dialog.clone());
        (
            dialog,
            RendererModalJavaScriptDialog {
                broker: self.broker.clone(),
                completion,
            },
        )
    }
}

pub(crate) struct RendererModalJavaScriptDialog {
    broker: RendererJavaScriptDialogBroker,
    completion: RendererJavaScriptDialogCompletion,
}

impl RendererModalJavaScriptDialog {
    pub(crate) fn wait(self) -> RendererJavaScriptDialogResult {
        let result = self.completion.wait();
        self.broker.close(&self.completion);
        result
    }

    pub(crate) fn cancel(self) {
        let _ = self.completion.finish(false, String::new());
        self.broker.close(&self.completion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(message: &str) -> RendererPendingJavaScriptDialog {
        let page_id = crate::runtime::PageId::new_for_testing(91);
        RendererPendingJavaScriptDialog::new(
            crate::runtime::RendererJavaScriptDialogId::new(1),
            crate::runtime::RendererDocumentLifecycleIdentity {
                frame: crate::runtime::RendererFrameToken { page_id },
                document: crate::runtime::RendererDocumentToken::new_for_testing(page_id, 1),
                epoch: crate::runtime::RendererLifecycleEpoch(1),
            },
            crate::runtime::RendererJavaScriptDialogSource::RootFrame,
            "https://example.test/dialog".to_owned(),
            "confirm".to_owned(),
            message.to_owned(),
            String::new(),
            Some(RendererJavaScriptDialogCompletion::pending()),
        )
    }

    #[test]
    fn broker_drains_dialogs_in_fifo_order() {
        let broker = RendererJavaScriptDialogBroker::default();
        broker.open(dialog("first"));
        broker.open(dialog("second"));

        let dialogs = broker.take_pending();
        assert_eq!(
            dialogs
                .iter()
                .map(RendererPendingJavaScriptDialog::message)
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(broker.take_pending().is_empty());
    }

    #[test]
    fn dismiss_pending_unblocks_modal_completion() {
        let broker = RendererJavaScriptDialogBroker::default();
        let dialog = dialog("close");
        let completion = dialog.completion_for_test().unwrap();
        broker.open(dialog);

        broker.dismiss_pending();

        assert_eq!(
            completion.wait(),
            RendererJavaScriptDialogResult {
                accepted: false,
                user_input: String::new(),
            }
        );
        assert!(!completion.finish(true, "late".to_owned()));
    }

    #[tokio::test]
    async fn broker_watch_observes_a_dialog_after_the_pending_queue_is_drained() {
        let broker = RendererJavaScriptDialogBroker::default();
        let watch = broker.watch();
        let dialog = dialog("watched");
        let completion = dialog.completion_for_test().unwrap();
        broker.open(dialog);
        assert_eq!(broker.take_pending().len(), 1);

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            watch.wait_until_open(),
        )
        .await
        .expect("an open modal must interrupt renderer observation commands");

        completion.finish(false, String::new());
        broker.close(&completion);
        assert!(!broker.has_open_dialog());
    }
}
