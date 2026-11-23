use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};

use moli_layout::{LayoutCssImageReference, ScreenPaintLayout};
use tokio::{sync::watch, task::JoinHandle};

use crate::document_runtime::DomHandle;

/// The active CDP subscription owns the sender. Polls clone it while a
/// request is in flight. Closing the channel only wakes cache cleanup.
#[derive(Clone, Debug)]
pub struct RendererScreencastPaintSession(watch::Sender<()>);

impl Default for RendererScreencastPaintSession {
    fn default() -> Self {
        Self(watch::channel(()).0)
    }
}

impl PartialEq for RendererScreencastPaintSession {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_channel(&other.0)
    }
}

impl Eq for RendererScreencastPaintSession {}

pub(super) struct ScreenPaintEntry {
    pub(super) layout: Box<ScreenPaintLayout<DomHandle>>,
    pub(super) resource_generation: u64,
    pub(super) css_images: Vec<LayoutCssImageReference<DomHandle>>,
}

struct ScreenPaintConsumer {
    closed: watch::Receiver<()>,
    cleanup: JoinHandle<()>,
}

impl Drop for ScreenPaintConsumer {
    fn drop(&mut self) {
        self.cleanup.abort();
    }
}

#[derive(Default)]
struct PaintStore {
    consumers: Vec<ScreenPaintConsumer>,
    entry: Option<ScreenPaintEntry>,
}

impl PaintStore {
    fn release_closed_consumers(&mut self) {
        self.consumers
            .retain(|consumer| consumer.closed.has_changed().is_ok());
        if self.consumers.is_empty() {
            self.entry = None;
        }
    }
}

#[derive(Default)]
pub(super) struct ScreencastPaintState(OnceCell<Rc<RefCell<PaintStore>>>);

impl ScreencastPaintState {
    pub(super) fn attach(&self, session: &RendererScreencastPaintSession) {
        let store = self.0.get_or_init(Default::default);
        let mut paint = store.borrow_mut();
        paint.release_closed_consumers();
        let closed = session.0.subscribe();
        if paint
            .consumers
            .iter()
            .any(|consumer| consumer.closed.same_channel(&closed))
        {
            return;
        }

        // Release on the renderer's local thread even when the page is idle.
        // This task borrows only the paint store and can finish during any page turn.
        let weak_store = Rc::downgrade(store);
        let mut signal = closed.clone();
        let cleanup = tokio::task::spawn_local(async move {
            let _ = signal.changed().await;
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().release_closed_consumers();
            }
        });
        paint
            .consumers
            .push(ScreenPaintConsumer { closed, cleanup });
    }

    pub(super) fn is_active(&self) -> bool {
        self.0.get().is_some_and(|store| {
            store
                .borrow()
                .consumers
                .iter()
                .any(|consumer| consumer.closed.has_changed().is_ok())
        })
    }

    pub(super) fn read<T>(&self, read: impl FnOnce(&ScreenPaintEntry) -> T) -> Option<T> {
        self.0.get()?.borrow().entry.as_ref().map(read)
    }

    pub(super) fn publish(&self, entry: Option<ScreenPaintEntry>) {
        if let Some(store) = self.0.get() {
            let active = self.is_active();
            store.borrow_mut().entry = if active { entry } else { None };
        }
    }
}
