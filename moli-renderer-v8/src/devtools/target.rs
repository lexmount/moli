use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use super::{
    ingress::{io::RendererInspectorIoIngress, main::RendererInspectorMainIngress},
    pause::{RendererInspectorPauseBridge, RendererInspectorSessionOutboundRoute},
};
use crate::runtime::PageId;

#[derive(Default)]
pub(crate) struct RendererDevToolsTargetShutdownRegistry {
    shared: Arc<Mutex<RendererDevToolsTargetShutdownRegistryState>>,
}

#[derive(Default)]
struct RendererDevToolsTargetShutdownRegistryState {
    terminal: bool,
    next_registration_id: u64,
    targets: HashMap<u64, RendererDevToolsTargetHandle>,
}

pub(crate) struct RendererDevToolsTargetShutdownRegistration {
    shared: Weak<Mutex<RendererDevToolsTargetShutdownRegistryState>>,
    registration_id: u64,
}

impl RendererDevToolsTargetShutdownRegistry {
    pub(crate) fn register(
        &self,
        target: RendererDevToolsTargetHandle,
    ) -> Result<RendererDevToolsTargetShutdownRegistration, &'static str> {
        let registration_id = {
            let mut state = self.shared.lock();
            if state.terminal {
                drop(state);
                target.close("Inspector target rejected after renderer owner shutdown");
                return Err("renderer owner is shut down");
            }
            state.next_registration_id = state
                .next_registration_id
                .checked_add(1)
                .ok_or("renderer DevTools target registration ID overflow")?;
            let registration_id = state.next_registration_id;
            let previous = state.targets.insert(registration_id, target);
            debug_assert!(previous.is_none());
            registration_id
        };
        Ok(RendererDevToolsTargetShutdownRegistration {
            shared: Arc::downgrade(&self.shared),
            registration_id,
        })
    }

    pub(crate) fn terminate_all(&self) {
        let targets = {
            let mut state = self.shared.lock();
            state.terminal = true;
            std::mem::take(&mut state.targets)
        };
        for target in targets.into_values() {
            target.close("Inspector target closed with its renderer owner");
        }
    }
}

impl std::fmt::Debug for RendererDevToolsTargetShutdownRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.lock();
        formatter
            .debug_struct("RendererDevToolsTargetShutdownRegistry")
            .field("terminal", &state.terminal)
            .field("target_count", &state.targets.len())
            .finish()
    }
}

impl Drop for RendererDevToolsTargetShutdownRegistration {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        shared.lock().targets.remove(&self.registration_id);
    }
}

/// Cloneable control-plane handle for one renderer DevTools target.
///
/// The handle is safe to retain outside the isolate owner stack. It exposes
/// only ingress and lifecycle coordination; all V8 session access remains in
/// the renderer-local Inspector executor.
#[derive(Clone)]
pub(crate) struct RendererDevToolsTargetHandle {
    pause: RendererInspectorPauseBridge,
    main: RendererInspectorMainIngress,
    io: RendererInspectorIoIngress,
}

impl RendererDevToolsTargetHandle {
    pub(crate) fn new(
        pause: RendererInspectorPauseBridge,
        main: RendererInspectorMainIngress,
        io: RendererInspectorIoIngress,
    ) -> Self {
        Self { pause, main, io }
    }

    pub(crate) fn pause(&self) -> RendererInspectorPauseBridge {
        self.pause.clone()
    }

    pub(crate) fn pause_ref(&self) -> &RendererInspectorPauseBridge {
        &self.pause
    }

    pub(crate) fn main_ref(&self) -> &RendererInspectorMainIngress {
        &self.main
    }

    pub(crate) fn io_ref(&self) -> &RendererInspectorIoIngress {
        &self.io
    }

    pub(crate) fn outbound_route(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) -> RendererInspectorSessionOutboundRoute {
        self.pause
            .outbound_route(self.clone(), agent_token, session)
    }

    pub(crate) fn detach_session(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        self.main.detach_session(agent_token, session);
        self.io.detach_session(agent_token, session);
    }

    pub(crate) fn close(&self, message: &str) -> bool {
        self.pause.close_target();
        self.main.close(message);
        self.io.close(message);
        self.io.terminate_execution_for_target_close()
    }

    /// Executes Chromium's terminal `Page.crash` IO control boundary.
    ///
    /// Unlike ordinary IO-agent commands, a crash must not wait for or occupy
    /// the target Inspector task runner. Seal both command receivers first,
    /// then interrupt any active JavaScript so the owner can retire the Page.
    pub(crate) fn crash_from_io(&self) {
        let _ = self.close("Inspector target crashed through Page.crash");
    }

    pub(crate) fn detach_page(&self, page_id: PageId, message: &str) {
        if self.pause.detach_page(page_id) {
            self.main.cancel_all_queued(message);
            self.io.cancel_all_queued(message);
        }
    }
}

impl std::fmt::Debug for RendererDevToolsTargetHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererDevToolsTargetHandle")
            .field("pause", &self.pause)
            .field("main", &self.main)
            .field("io", &self.io)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        devtools::{
            command::{
                RendererDevToolsIoCommandEnvelope, RendererInspectorCommandEnvelope,
                RendererInspectorCommandRoute, RendererInspectorIngressTicket,
            },
            ingress::{
                io::{RendererInspectorIoIngress, RendererRuntimeInspectorIoCommandClaim},
                main::{
                    RendererInspectorMainIngress, RendererRuntimeInspectorMainCommandCompletion,
                },
            },
            route::RendererInspectorSessionExecutorRouteId,
        },
        runtime::{
            PageId, RendererPageToken, RendererRuntimeInspectorAsyncCompletion,
            RendererRuntimeInspectorResponseSender,
        },
    };

    #[tokio::test]
    async fn page_crash_is_terminal_and_bypasses_main_and_io_lanes() {
        let pause = RendererInspectorPauseBridge::default();
        let main = RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(1),
            pause.pause_loop_wake(),
        );
        let io = RendererInspectorIoIngress::new(pause.pause_loop_wake(), None);
        let target = RendererDevToolsTargetHandle::new(pause, main.clone(), io.clone());
        let agent = RendererDevToolsAgentToken::allocate();
        let session_id = Some("session-a".to_owned());

        let io_route = io.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    None,
                    session_id.clone(),
                    RendererInspectorCommandRoute::Io,
                ),
            ),
        );
        let (response_tx, _response_rx) =
            tokio::sync::oneshot::channel::<RendererRuntimeInspectorAsyncCompletion>();
        let main_route = main.enqueue_command(
            RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
            agent,
            RendererInspectorCommandEnvelope::new_main_protocol(
                RendererInspectorIngressTicket::new(
                    None,
                    session_id,
                    RendererInspectorCommandRoute::MainThread,
                ),
                None,
                r#"{"id":1,"method":"Runtime.evaluate"}"#.to_owned(),
                RendererRuntimeInspectorResponseSender::new(1, response_tx),
            ),
        );

        target.crash_from_io();
        assert!(matches!(
            io_route.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));
        assert!(matches!(
            main_route.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
        ));

        let late = io.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    None,
                    Some("session-late".to_owned()),
                    RendererInspectorCommandRoute::Io,
                ),
            ),
        );
        assert!(
            matches!(
                late.wait_for_first_dispatch().await,
                Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
            ),
            "terminal Page.crash must reject late ordinary IO work"
        );
    }

    #[tokio::test]
    async fn renderer_owner_shutdown_closes_every_registered_target() {
        let registry = RendererDevToolsTargetShutdownRegistry::default();
        let pause = RendererInspectorPauseBridge::default();
        let main = RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(2),
            pause.pause_loop_wake(),
        );
        let io = RendererInspectorIoIngress::new(pause.pause_loop_wake(), None);
        let target = RendererDevToolsTargetHandle::new(pause, main.clone(), io.clone());
        let registration = registry
            .register(target)
            .expect("live owner must register its Inspector target");
        assert_eq!(registry.shared.lock().targets.len(), 1);

        let pending = io.enqueue_command(
            RendererDevToolsAgentToken::allocate(),
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    None,
                    Some("session-owner-shutdown".to_owned()),
                    RendererInspectorCommandRoute::Io,
                ),
            ),
        );
        registry.terminate_all();
        assert!(matches!(
            pending.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));
        assert!(registry.shared.lock().terminal);
        assert!(registry.shared.lock().targets.is_empty());

        drop(registration);
        let late_pause = RendererInspectorPauseBridge::default();
        let late_main = RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(3),
            late_pause.pause_loop_wake(),
        );
        let late_io = RendererInspectorIoIngress::new(late_pause.pause_loop_wake(), None);
        let late_target = RendererDevToolsTargetHandle::new(late_pause, late_main, late_io.clone());
        assert!(registry.register(late_target).is_err());
        let rejected = late_io.enqueue_command(
            RendererDevToolsAgentToken::allocate(),
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    None,
                    Some("session-late-owner-shutdown".to_owned()),
                    RendererInspectorCommandRoute::Io,
                ),
            ),
        );
        assert!(matches!(
            rejected.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));
    }
}
