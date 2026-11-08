use std::sync::Arc;

use parking_lot::Mutex;

use super::{RendererDocumentLifecycleIdentity, RendererWindowDocumentSource};

type FocusActivationCallback = Box<dyn FnOnce(bool) + Send>;

struct FocusActivationReply {
    callback: Mutex<Option<FocusActivationCallback>>,
    is_current: Box<dyn Fn() -> bool + Send + Sync>,
}

impl Drop for FocusActivationReply {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.get_mut().take() {
            callback(false);
        }
    }
}

/// A native Window-client focus request awaiting its browser's activation.
///
/// Cloning output retains the same one-shot reply. Discarding every copy
/// cancels the request; an absent browser owner must never manufacture focus.
#[derive(Clone)]
pub struct RendererPendingWindowClientFocus {
    pub source_document: RendererDocumentLifecycleIdentity,
    pub window: RendererWindowDocumentSource,
    reply: Arc<FocusActivationReply>,
}

impl RendererPendingWindowClientFocus {
    pub(crate) fn new(
        source_document: RendererDocumentLifecycleIdentity,
        window: RendererWindowDocumentSource,
        is_current: Box<dyn Fn() -> bool + Send + Sync>,
        callback: impl FnOnce(bool) + Send + 'static,
    ) -> Self {
        Self {
            source_document,
            window,
            reply: Arc::new(FocusActivationReply {
                callback: Mutex::new(Some(Box::new(callback))),
                is_current,
            }),
        }
    }

    /// Acknowledge actual browser activation, before native frame focusing.
    pub fn finish(&self, activated: bool) {
        let activated = activated && self.is_current();
        let callback = self.reply.callback.lock().take();
        if let Some(callback) = callback {
            callback(activated);
        }
    }

    /// Check the original native client, Document and worker run without
    /// entering a renderer realm or following a replacement Window.
    pub fn is_current(&self) -> bool {
        (self.reply.is_current)()
    }
}

impl PartialEq for RendererPendingWindowClientFocus {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.reply, &other.reply)
    }
}

impl std::fmt::Debug for RendererPendingWindowClientFocus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPendingWindowClientFocus")
            .field("source_document", &self.source_document)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(callback: impl FnOnce(bool) + Send + 'static) -> RendererPendingWindowClientFocus {
        let page = super::super::PageId::new_for_testing(1);
        RendererPendingWindowClientFocus::new(
            RendererDocumentLifecycleIdentity {
                frame: super::super::RendererFrameToken { page_id: page },
                document: super::super::RendererDocumentToken::new_for_testing(page, 1),
                epoch: super::super::RendererLifecycleEpoch(1),
            },
            RendererWindowDocumentSource::RootFrame,
            Box::new(|| true),
            callback,
        )
    }

    #[test]
    fn window_client_focus_activation_reply_is_once_across_output_clones() {
        let replies = Arc::new(Mutex::new(Vec::new()));
        let observed = replies.clone();
        let request = request(move |value| observed.lock().push(value));
        let clone = request.clone();
        request.finish(true);
        clone.finish(false);
        drop(request);
        drop(clone);
        assert_eq!(*replies.lock(), vec![true]);
    }

    #[test]
    fn discarded_window_client_focus_output_cancels_only_when_last_copy_retires() {
        let replies = Arc::new(Mutex::new(Vec::new()));
        let observed = replies.clone();
        let request = request(move |value| observed.lock().push(value));
        let clone = request.clone();
        drop(request);
        assert!(replies.lock().is_empty());
        drop(clone);
        assert_eq!(*replies.lock(), vec![false]);
    }

    #[test]
    fn window_client_focus_activation_rechecks_currentness_before_acknowledgement() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let template = request(|_| {});
        let current = Arc::new(AtomicBool::new(true));
        let probe = current.clone();
        let replies = Arc::new(Mutex::new(Vec::new()));
        let observed = replies.clone();
        let request = RendererPendingWindowClientFocus::new(
            template.source_document,
            template.window.clone(),
            Box::new(move || probe.load(Ordering::Acquire)),
            move |activated| observed.lock().push(activated),
        );
        assert!(request.is_current());
        let clone = request.clone();
        current.store(false, Ordering::Release);
        assert!(!clone.is_current());
        clone.finish(true);
        request.finish(true);
        drop(clone);
        drop(request);
        assert_eq!(*replies.lock(), vec![false]);
    }
}
