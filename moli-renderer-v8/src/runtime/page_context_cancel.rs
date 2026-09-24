use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RendererPageContextCancelReason {
    PageClosed,
    ContextDropped,
}

#[derive(Debug)]
struct RendererPageContextCancelState {
    reason: Mutex<Option<RendererPageContextCancelReason>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageContextCancelReceiver {
    rx: crossbeam_channel::Receiver<()>,
    state: Arc<RendererPageContextCancelState>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageContextCancelSender {
    tx: crossbeam_channel::Sender<()>,
    state: Arc<RendererPageContextCancelState>,
}

pub(crate) fn renderer_page_context_cancel_channel() -> (
    RendererPageContextCancelSender,
    RendererPageContextCancelReceiver,
) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let state = Arc::new(RendererPageContextCancelState {
        reason: Mutex::new(None),
    });
    (
        RendererPageContextCancelSender {
            tx,
            state: state.clone(),
        },
        RendererPageContextCancelReceiver { rx, state },
    )
}

impl RendererPageContextCancelSender {
    /// Serializes synchronous Inspector enqueue with Page retirement. Keeping
    /// the existing cancellation lock through enqueue prevents a command from
    /// entering after retirement has already drained that Page's receivers.
    pub(in crate::runtime) fn with_inspector_admission<T>(
        &self,
        enqueue: impl FnOnce() -> T,
    ) -> anyhow::Result<T> {
        let reason = self.state.reason.lock();
        if let Some(reason) = *reason {
            return Err(anyhow::anyhow!("Inspector Page is retired: {reason:?}"));
        }
        Ok(enqueue())
    }

    pub(crate) fn cancel(&self, reason: RendererPageContextCancelReason) {
        {
            let mut stored = self.state.reason.lock();
            if stored.is_none() {
                *stored = Some(reason);
            }
        }
        let _ = self.tx.send(());
    }
}

impl RendererPageContextCancelReceiver {
    pub(crate) fn reason(&self) -> Option<RendererPageContextCancelReason> {
        *self.state.reason.lock()
    }

    pub(crate) fn wake_receiver(&self) -> &crossbeam_channel::Receiver<()> {
        &self.rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_reason_is_replayable_after_wakeup_is_consumed() {
        let (tx, rx) = renderer_page_context_cancel_channel();
        let later_rx = rx.clone();

        tx.cancel(RendererPageContextCancelReason::PageClosed);
        rx.wake_receiver()
            .recv()
            .expect("first receiver should observe wakeup");

        assert_eq!(
            rx.reason(),
            Some(RendererPageContextCancelReason::PageClosed)
        );
        assert_eq!(
            later_rx.reason(),
            Some(RendererPageContextCancelReason::PageClosed)
        );
    }
}
