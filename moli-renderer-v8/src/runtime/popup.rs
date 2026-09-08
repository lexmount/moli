use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

use super::{RendererPendingPopupActivation, RendererPopupOpening};

#[derive(Debug, Default)]
struct PopupInputs {
    pending: Vec<RendererPendingPopupActivation>,
    closed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPopupBroker {
    pending: Arc<Mutex<PopupInputs>>,
    changed: watch::Sender<()>,
}

impl Default for RendererPopupBroker {
    fn default() -> Self {
        Self {
            pending: Arc::default(),
            changed: watch::channel(()).0,
        }
    }
}

/// A move-owned native input receiver. Closing a Page ends new admission but
/// preserves requests already accepted by window.open until the owner drains them.
pub struct RendererPopupInputReceiver {
    pending: Arc<Mutex<PopupInputs>>,
    changed: watch::Receiver<()>,
}

impl RendererPopupBroker {
    pub(crate) fn accept(
        &self,
        request: RendererPendingPopupActivation,
    ) -> Option<Arc<RendererPopupOpening>> {
        let opening = request.opening();
        {
            let mut pending = self.pending.lock();
            if pending.closed {
                return None;
            }
            pending.pending.push(request);
        }
        self.changed.send_modify(|_| {});
        Some(opening)
    }

    pub(crate) fn observe(&self) -> RendererPopupInputReceiver {
        RendererPopupInputReceiver {
            pending: self.pending.clone(),
            changed: self.changed.subscribe(),
        }
    }

    pub(crate) fn close(&self) {
        self.pending.lock().closed = true;
        self.changed.send_modify(|_| {});
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending.lock().pending.len()
    }

    #[cfg(test)]
    pub(crate) fn take_pending(&self) -> Vec<RendererPendingPopupActivation> {
        std::mem::take(&mut self.pending.lock().pending)
    }
}

impl RendererPopupInputReceiver {
    pub async fn recv(&mut self) -> Option<Vec<RendererPendingPopupActivation>> {
        loop {
            self.changed.borrow_and_update();
            {
                let mut pending = self.pending.lock();
                if !pending.pending.is_empty() {
                    return Some(std::mem::take(&mut pending.pending));
                }
                if pending.closed {
                    return None;
                }
            }
            self.changed.changed().await.ok()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RendererPopupDisposition;

    fn request(name: &str) -> RendererPendingPopupActivation {
        RendererPendingPopupActivation::browser_context(
            None,
            "about:blank".into(),
            name.into(),
            RendererPopupDisposition::Background,
        )
    }

    #[tokio::test]
    async fn popup_close_seals_admission_but_drains_accepted_fifo_tail() {
        let broker = RendererPopupBroker::default();
        let first = broker.accept(request("first")).unwrap();
        let second = broker.accept(request("second")).unwrap();
        broker.close();
        assert!(broker.accept(request("too-late")).is_none());
        let mut receiver = broker.observe();
        let requests = receiver.recv().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.id())
                .collect::<Vec<_>>(),
            [first.id(), second.id()]
        );
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn popup_input_receiver_observes_acceptance_after_subscription() {
        let broker = RendererPopupBroker::default();
        let mut receiver = broker.observe();
        let opening = broker.accept(request("late")).unwrap();
        let received = receiver.recv().await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].id(), opening.id());
        assert_eq!(broker.pending_count(), 0);
    }
}
