use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

use crate::{
    page_task_queue::RendererResourceCompletionSender,
    runtime::{PendingRendererOutputRecord, RendererOwnerAction, RendererTurnOutputJournal},
    types::AsyncSubresourceFetchEvent,
};

#[derive(Clone, Debug)]
pub(crate) struct ResourceInterceptionContext {
    pub(crate) source_document: crate::runtime::RendererDocumentLifecycleIdentity,
    pub(crate) output: InterceptionOutput,
}

/// Native network facts normally join the Document's networking task source.
/// During a synchronous wait only, their already-resolved output may cross the
/// protocol transport directly. This capability has no PageVm or JS handles.
#[derive(Clone, Debug)]
pub(crate) struct InterceptionOutput {
    inner: Arc<InterceptionOutputInner>,
}

#[derive(Debug)]
struct InterceptionOutputInner {
    journal: RendererTurnOutputJournal,
    completion: RendererResourceCompletionSender,
    state: Mutex<InterceptionOutputState>,
}

#[derive(Debug, Default)]
struct InterceptionOutputState {
    closed: bool,
    synchronous_wait: bool,
    wake_pending: bool,
    pending: VecDeque<PendingRendererOutputRecord>,
}

/// A coalesced networking task drains native facts only. Erase its captured
/// publisher so a queued wake does not expose the journal/queue's recursive
/// implementation types through every resource-completion envelope.
pub(crate) struct InterceptionOutputWake(Box<dyn FnOnce() + Send>);

impl std::fmt::Debug for InterceptionOutputWake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterceptionOutputWake")
            .finish_non_exhaustive()
    }
}

impl InterceptionOutputWake {
    pub(crate) fn drain(self) {
        (self.0)();
    }
}

pub(crate) struct SynchronousNetworkOutputScope(InterceptionOutput);

impl Drop for SynchronousNetworkOutputScope {
    fn drop(&mut self) {
        self.0.inner.state.lock().synchronous_wait = false;
    }
}

impl InterceptionOutput {
    pub(crate) fn new(
        journal: RendererTurnOutputJournal,
        completion: RendererResourceCompletionSender,
    ) -> Self {
        Self {
            inner: Arc::new(InterceptionOutputInner {
                journal,
                completion,
                state: Mutex::new(InterceptionOutputState::default()),
            }),
        }
    }

    pub(crate) fn publish(&self, action: RendererOwnerAction) -> bool {
        self.publish_record(PendingRendererOutputRecord::owner_action(None, action))
    }

    pub(crate) fn record_subresource_network(
        &self,
        source_document: crate::runtime::RendererDocumentLifecycleIdentity,
        record: crate::types::SubresourceNetworkRecord,
    ) -> bool {
        self.publish_network(
            source_document,
            moli_page_types::ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
        )
    }

    pub(crate) fn publish_network(
        &self,
        source_document: crate::runtime::RendererDocumentLifecycleIdentity,
        item: moli_page_types::ScriptNetworkOutputItem,
    ) -> bool {
        self.publish_record(PendingRendererOutputRecord::observation(
            None,
            crate::runtime::RendererProtocolObservation::Network {
                source_document,
                item,
            },
        ))
    }

    fn publish_record(&self, record: PendingRendererOutputRecord) -> bool {
        let mut state = self.inner.state.lock();
        if state.closed {
            return false;
        }
        if state.synchronous_wait {
            self.inner.journal.append(record);
            self.inner.journal.publish_pending();
            return true;
        }
        state.pending.push_back(record);
        if state.wake_pending {
            return true;
        }
        state.wake_pending = true;
        if self
            .inner
            .completion
            .send_async_subresource_event(AsyncSubresourceFetchEvent::InterceptionOutputReady({
                let output = Arc::downgrade(&self.inner);
                InterceptionOutputWake(Box::new(move || {
                    if let Some(inner) = output.upgrade() {
                        InterceptionOutput { inner }.drain();
                    }
                }))
            }))
            .is_err()
        {
            state.pending.clear();
            state.wake_pending = false;
            return false;
        }
        true
    }

    /// Retirement drops undelivered pause capabilities, waking their network
    /// waiters even if this Document's networking tasks will no longer run.
    pub(crate) fn close(&self) {
        let mut state = self.inner.state.lock();
        state.closed = true;
        state.pending.clear();
    }

    /// Called on the selected networking task, after earlier Page output.
    pub(crate) fn drain(&self) {
        let mut state = self.inner.state.lock();
        self.inner.journal.append_records(state.pending.drain(..));
        state.wake_pending = false;
    }

    /// The caller has published its command/related-context prefix. Future
    /// network facts can now publish while this Page's JS stack is suspended.
    pub(crate) fn enter_synchronous_wait(&self) -> SynchronousNetworkOutputScope {
        let mut state = self.inner.state.lock();
        assert!(
            !state.synchronous_wait,
            "synchronous network waits cannot overlap on one Page"
        );
        state.synchronous_wait = true;
        self.inner.journal.append_records(state.pending.drain(..));
        self.inner.journal.publish_pending();
        SynchronousNetworkOutputScope(self.clone())
    }
}
