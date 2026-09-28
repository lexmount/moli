//! Nested pause-loop coordination and causal Inspector output routing.

use std::sync::{Arc, Weak};

use parking_lot::{Condvar, Mutex};

use crate::runtime::{PageId, RendererOutputResidenceIdentity, RendererTurnOutputJournal};

#[cfg(feature = "devtools")]
mod causality;
#[cfg(feature = "devtools")]
mod frontend;
#[cfg(feature = "devtools")]
mod loop_state;

#[cfg(feature = "devtools")]
pub(crate) use causality::{
    RendererInspectorPauseCommandOutputRoute, RendererInspectorPauseNotificationRoute,
};
#[cfg(feature = "devtools")]
pub(crate) use frontend::{
    RendererInspectorPausePrefaceGuard, RendererInspectorSessionOutboundRoute,
};
#[cfg(feature = "devtools")]
pub(crate) use loop_state::{RendererInspectorPauseExitReason, RendererInspectorPauseLoopPolicy};

#[derive(Clone)]
pub(crate) struct RendererInspectorPauseBridge {
    shared: Arc<RendererInspectorPauseBridgeShared>,
}

pub(crate) struct RendererInspectorPauseBridgeShared {
    state: Mutex<RendererInspectorPauseBridgeState>,
    pause_loop_wake: Condvar,
}

#[derive(Clone)]
struct RendererInspectorPauseRoute {
    output_journal: RendererTurnOutputJournal,
}

struct RendererInspectorPauseBridgeState {
    document_replacements: usize,
    unload_callbacks: usize,
    target_closed: bool,
    route: Option<RendererInspectorPauseRoute>,
    #[cfg(feature = "devtools")]
    frontend: frontend::PauseState,
}

#[must_use]
pub(crate) struct RendererInspectorUnloadCallbackGuard {
    bridge: RendererInspectorPauseBridge,
}

impl Drop for RendererInspectorUnloadCallbackGuard {
    fn drop(&mut self) {
        let mut state = self.bridge.shared.state.lock();
        state.unload_callbacks = state
            .unload_callbacks
            .checked_sub(1)
            .expect("unload callback scope underflow");
    }
}

#[derive(Clone)]
pub(crate) struct RendererInspectorPauseLoopWake {
    shared: Weak<RendererInspectorPauseBridgeShared>,
}

impl RendererInspectorPauseLoopWake {
    pub(crate) fn notify_one(&self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let _state = shared.state.lock();
        shared.pause_loop_wake.notify_one();
    }

    pub(crate) fn notify_all(&self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let _state = shared.state.lock();
        shared.pause_loop_wake.notify_all();
    }
}

impl Default for RendererInspectorPauseBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererInspectorPauseBridge {
    fn new() -> Self {
        let shared = Arc::new(RendererInspectorPauseBridgeShared {
            state: Mutex::new(RendererInspectorPauseBridgeState {
                document_replacements: 0,
                unload_callbacks: 0,
                target_closed: false,
                route: None,
                #[cfg(feature = "devtools")]
                frontend: frontend::PauseState::default(),
            }),
            pause_loop_wake: Condvar::new(),
        });
        Self { shared }
    }
}

impl std::fmt::Debug for RendererInspectorPauseBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        let mut debug = formatter.debug_struct("RendererInspectorPauseBridge");
        debug
            .field("document_replacements", &state.document_replacements)
            .field("target_closed", &state.target_closed);
        #[cfg(feature = "devtools")]
        debug.field("frontend", &state.frontend);
        debug.finish()
    }
}

impl RendererInspectorPauseBridge {
    pub(crate) fn pause_loop_wake(&self) -> RendererInspectorPauseLoopWake {
        RendererInspectorPauseLoopWake {
            shared: Arc::downgrade(&self.shared),
        }
    }

    pub(crate) fn configure_page_route(&self, output_journal: RendererTurnOutputJournal) {
        let RendererOutputResidenceIdentity::Page { .. } = output_journal.stream().residence()
        else {
            panic!("an Inspector pause route requires a Page output stream");
        };
        self.shared.state.lock().route = Some(RendererInspectorPauseRoute { output_journal });
    }

    pub(crate) fn is_pause_active(&self) -> bool {
        #[cfg(feature = "devtools")]
        {
            self.shared.state.lock().frontend.is_active()
        }
        #[cfg(not(feature = "devtools"))]
        {
            false
        }
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn has_announced_pause_for_agent(
        &self,
        agent: moli_page_types::RendererDevToolsAgentToken,
    ) -> bool {
        self.shared
            .state
            .lock()
            .frontend
            .paused_sessions_awaiting_resumed
            .iter()
            .any(|(owner, _)| *owner == agent)
    }

    /// Ends the bounded handoff from a resume/step command to the renderer turn
    /// it released. A step that reaches the end of its task may never enter a
    /// new pause; owner settlement is the terminal that prevents its cause from
    /// leaking into a later, unrelated pause.
    pub(crate) fn finish_owner_turn(&self) {
        #[cfg(feature = "devtools")]
        self.shared.state.lock().frontend.finish_owner_turn();
    }

    pub(crate) fn begin_document_replacement(&self) -> bool {
        let mut state = self.shared.state.lock();
        if state.target_closed {
            return false;
        }
        state.document_replacements += 1;
        self.shared.pause_loop_wake.notify_all();
        true
    }

    pub(crate) fn begin_unload_callbacks(&self) -> RendererInspectorUnloadCallbackGuard {
        let mut state = self.shared.state.lock();
        // Replacement first terminates suspended old work, then deliberately
        // runs pagehide/unload before committing. Those callbacks must remain
        // debuggable, including when a second preparation overlaps them.
        state.unload_callbacks = state
            .unload_callbacks
            .checked_add(1)
            .expect("unload callback scope overflow");
        RendererInspectorUnloadCallbackGuard {
            bridge: self.clone(),
        }
    }

    pub(crate) fn finish_document_replacement(&self) {
        let mut state = self.shared.state.lock();
        state.document_replacements = state
            .document_replacements
            .checked_sub(1)
            .expect("document replacement scope must be balanced");
    }

    pub(crate) fn detach_page(&self, page_id: PageId) -> bool {
        let mut state = self.shared.state.lock();
        let route_page_id = state.route.as_ref().and_then(|route| {
            match route.output_journal.stream().residence() {
                RendererOutputResidenceIdentity::Page { page_id, .. } => Some(page_id),
                RendererOutputResidenceIdentity::SharedWorker { .. }
                | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
            }
        });
        if route_page_id != Some(page_id) {
            return false;
        }
        state.route = None;
        #[cfg(feature = "devtools")]
        if state.frontend.detach_page() {
            self.shared.pause_loop_wake.notify_all();
        }
        true
    }

    pub(crate) fn close_target(&self) {
        let mut state = self.shared.state.lock();
        state.target_closed = true;
        #[cfg(feature = "devtools")]
        state.frontend.close_target();
        self.shared.pause_loop_wake.notify_all();
    }
}

#[cfg(all(test, feature = "devtools"))]
mod tests;
