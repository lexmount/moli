use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
    sync::{Arc, OnceLock, Weak},
};

use tokio::{sync::watch, task::JoinHandle};

use super::layout_state::DocumentLayoutState;
use crate::raster_worker::RendererRasterStream;

/// The active screen paint consumer owns the sender. Polls clone it while a
/// request is in flight. Closing the channel only wakes cache cleanup.
#[derive(Clone)]
pub struct RendererScreencastPaintSession(Arc<ScreencastPaintSession>);

struct ScreencastPaintSession {
    active: watch::Sender<()>,
    raster: OnceLock<Arc<RendererRasterStream>>,
}

impl Default for RendererScreencastPaintSession {
    fn default() -> Self {
        Self(Arc::new(ScreencastPaintSession {
            active: watch::Sender::new(()),
            raster: OnceLock::new(),
        }))
    }
}

impl std::fmt::Debug for RendererScreencastPaintSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererScreencastPaintSession")
            .finish_non_exhaustive()
    }
}

impl RendererScreencastPaintSession {
    pub(crate) fn raster_stream(&self) -> Weak<RendererRasterStream> {
        Arc::downgrade(self.0.raster.get_or_init(Arc::default))
    }
}

impl PartialEq for RendererScreencastPaintSession {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RendererScreencastPaintSession {}

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
pub(super) struct ScreenPaintConsumers(OnceCell<Rc<RefCell<Vec<ScreenPaintConsumer>>>>);

impl ScreenPaintConsumers {
    pub(super) fn attach(
        &self,
        session: &RendererScreencastPaintSession,
        document_layout: &Rc<RefCell<DocumentLayoutState>>,
    ) {
        let store = self.0.get_or_init(Default::default);
        let mut consumers = store.borrow_mut();
        consumers.retain(|consumer| consumer.closed.has_changed().is_ok());
        let closed = session.0.active.subscribe();
        if consumers
            .iter()
            .any(|consumer| consumer.closed.same_channel(&closed))
        {
            return;
        }

        // The task releases paint on the local thread even when the page is
        // idle. A weak lease set prevents an old layout policy from clearing
        // the world of a new publication after the policy has been replaced.
        let weak_consumers = Rc::downgrade(store);
        let weak_layout = Rc::downgrade(document_layout);
        let mut signal = closed.clone();
        let cleanup = tokio::task::spawn_local(async move {
            let _ = signal.changed().await;
            if let Some(consumers) = weak_consumers.upgrade() {
                let mut consumers = consumers.borrow_mut();
                consumers.retain(|consumer| consumer.closed.has_changed().is_ok());
                if consumers.is_empty()
                    && let Some(layout) = weak_layout.upgrade()
                {
                    layout.borrow_mut().discard_screen_paint();
                }
            }
        });
        consumers.push(ScreenPaintConsumer { closed, cleanup });
    }

    pub(super) fn is_active(&self) -> bool {
        self.0.get().is_some_and(|store| {
            store
                .borrow()
                .iter()
                .any(|consumer| consumer.closed.has_changed().is_ok())
        })
    }
}
