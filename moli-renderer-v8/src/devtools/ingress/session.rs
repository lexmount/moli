use std::{collections::BTreeMap, sync::Arc};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use parking_lot::Mutex;

use super::{
    io::RendererInspectorIoIngress, lane::RendererDevToolsSessionLaneKey,
    main::RendererInspectorMainIngress,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionState {
    Connecting,
    Connected,
    Failed,
}

#[derive(Clone, Default)]
pub(crate) struct RendererInspectorSessionConnections {
    inner: Arc<Mutex<Connections>>,
}

/// Readiness belongs to the connection captured at command admission. A later
/// attach must not suspend older implicit-session work ahead of its own connect,
/// and a replacement connection must not authorize an old queued command.
#[derive(Clone)]
pub(crate) struct RendererInspectorSessionAdmission(Option<Arc<Mutex<ConnectionState>>>);

impl RendererInspectorSessionAdmission {
    pub(crate) fn accepts(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|state| *state.lock() != ConnectionState::Failed)
    }

    pub(crate) fn ready(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|state| *state.lock() == ConnectionState::Connected)
    }
}

#[derive(Default)]
struct Connections {
    closed: bool,
    sessions: BTreeMap<RendererDevToolsSessionLaneKey, Arc<Mutex<ConnectionState>>>,
}

impl RendererInspectorSessionConnections {
    pub(crate) fn confirm_connected(
        &self,
        agent: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) {
        let mut inner = self.inner.lock();
        if !inner.closed {
            let state = inner
                .sessions
                .entry(RendererDevToolsSessionLaneKey::new(agent, session))
                .or_insert_with(|| Arc::new(Mutex::new(ConnectionState::Connected)));
            let mut state = state.lock();
            if *state == ConnectionState::Connecting {
                *state = ConnectionState::Connected;
            }
        }
    }

    pub(crate) fn contains(&self, key: &RendererDevToolsSessionLaneKey) -> bool {
        self.inner.lock().sessions.contains_key(key)
    }

    pub(crate) fn admission(
        &self,
        key: &RendererDevToolsSessionLaneKey,
    ) -> RendererInspectorSessionAdmission {
        let inner = self.inner.lock();
        RendererInspectorSessionAdmission(if inner.closed {
            Some(Arc::new(Mutex::new(ConnectionState::Failed)))
        } else {
            inner.sessions.get(key).cloned()
        })
    }

    pub(crate) fn begin(
        &self,
        agent: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        main: RendererInspectorMainIngress,
        io: RendererInspectorIoIngress,
    ) -> Option<RendererInspectorSessionConnect> {
        let key = RendererDevToolsSessionLaneKey::new(agent, session);
        let mut inner = self.inner.lock();
        if inner.sessions.contains_key(&key) {
            return None;
        }
        let state = Arc::new(Mutex::new(ConnectionState::Connecting));
        if inner.closed {
            *state.lock() = ConnectionState::Failed;
        } else {
            inner.sessions.insert(key.clone(), state.clone());
        }
        Some(RendererInspectorSessionConnect {
            connections: self.clone(),
            key,
            state,
            main,
            io,
            completed: false,
        })
    }

    pub(crate) fn detach(&self, agent: RendererDevToolsAgentToken, session: &DevToolsSessionKey) {
        if let Some(state) = self
            .inner
            .lock()
            .sessions
            .remove(&RendererDevToolsSessionLaneKey::new(agent, session.clone()))
        {
            *state.lock() = ConnectionState::Failed;
        }
    }

    pub(crate) fn close(&self) {
        let mut inner = self.inner.lock();
        inner.closed = true;
        for state in std::mem::take(&mut inner.sessions).into_values() {
            *state.lock() = ConnectionState::Failed;
        }
    }

    pub(crate) fn retire_agent(&self, agent: RendererDevToolsAgentToken) {
        self.inner.lock().sessions.retain(|key, state| {
            if key.belongs_to_agent(agent) {
                *state.lock() = ConnectionState::Failed;
                false
            } else {
                true
            }
        });
    }
}

/// The native connect operation owns readiness, independently of its observer.
/// An abandoned operation rejects its queues instead of opening the IO lane.
#[doc(hidden)]
pub struct RendererInspectorSessionConnect {
    connections: RendererInspectorSessionConnections,
    key: RendererDevToolsSessionLaneKey,
    state: Arc<Mutex<ConnectionState>>,
    main: RendererInspectorMainIngress,
    io: RendererInspectorIoIngress,
    completed: bool,
}

impl RendererInspectorSessionConnect {
    pub(crate) fn is_current(&self) -> bool {
        let inner = self.connections.inner.lock();
        !inner.closed
            && inner
                .sessions
                .get(&self.key)
                .is_some_and(|state| Arc::ptr_eq(state, &self.state))
            && *self.state.lock() != ConnectionState::Failed
    }

    pub(crate) fn connected(mut self) {
        // The Renderer owner has created the native session and its output
        // route. Browser polling and Main binding replay are not dependencies.
        let connected = {
            let inner = self.connections.inner.lock();
            !inner.closed
                && inner.sessions.get(&self.key).is_some_and(|state| {
                    if !Arc::ptr_eq(state, &self.state) {
                        return false;
                    }
                    let mut state = state.lock();
                    if *state == ConnectionState::Failed {
                        return false;
                    }
                    *state = ConnectionState::Connected;
                    true
                })
        };
        self.completed = true;
        if connected {
            self.main.notify_execution_opportunities();
            self.io.notify_execution_opportunities();
        }
    }
}

impl Drop for RendererInspectorSessionConnect {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let failed = {
            let mut state = self.state.lock();
            if *state == ConnectionState::Connecting {
                *state = ConnectionState::Failed;
                true
            } else {
                false
            }
        };
        if failed {
            // Reject only commands carrying failed connection admissions. A
            // concurrent detach/reattach may already have reused this session
            // key, so cancellation must not create a key-wide detach barrier.
            self.main.reject_failed_connections();
            self.io.reject_failed_connections();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::io::RendererRuntimeInspectorIoCommandClaim;
    use super::super::main::RendererRuntimeInspectorMainCommandCompletion;
    use super::*;
    use crate::devtools::{
        pause::RendererInspectorPauseBridge, route::RendererInspectorSessionExecutorRouteId,
    };
    use crate::runtime::{
        RendererDevToolsIoCommandEnvelope, RendererInspectorCommandRoute,
        RendererInspectorIngressTicket, RendererPageCommand, RendererPageStateCapturePolicy,
        RendererPageToken,
    };

    fn receiver_target() -> crate::devtools::target::RendererDevToolsTargetHandle {
        let pause = RendererInspectorPauseBridge::default();
        let main = RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(99),
            pause.pause_loop_wake(),
        );
        let io = RendererInspectorIoIngress::new(pause.pause_loop_wake(), None);
        crate::devtools::target::RendererDevToolsTargetHandle::new(pause, main, io)
    }

    fn receivers() -> (
        RendererInspectorSessionConnections,
        RendererInspectorMainIngress,
        RendererInspectorIoIngress,
    ) {
        let target = receiver_target();
        (
            target.io_ref().session_connections(),
            target.main_ref().clone(),
            target.io_ref().clone(),
        )
    }

    fn queue_io(
        io: &RendererInspectorIoIngress,
        agent: RendererDevToolsAgentToken,
        id: &str,
    ) -> super::super::io::RendererRuntimeInspectorIoCommandRoute {
        io.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    None,
                    Some(id.to_owned()),
                    RendererInspectorCommandRoute::Io,
                ),
            ),
        )
    }

    fn queue_main(
        main: &RendererInspectorMainIngress,
        agent: RendererDevToolsAgentToken,
        id: &str,
    ) -> super::super::main::RendererRuntimeInspectorMainCommandRoute {
        main.enqueue_protocol_page_command(
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(99)),
            agent,
            RendererPageCommand::ChildFrameTreeSnapshot,
            Some(id.to_owned()),
            RendererPageStateCapturePolicy::ProtocolTurn,
        )
    }

    #[tokio::test]
    async fn document_replacement_reclaims_only_the_retired_agents_connections() {
        let target = receiver_target();
        let main = target.main_ref();
        let io = target.io_ref();
        let connections = io.session_connections();
        let live_agent = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Attached("same-session".into());
        connections
            .begin(live_agent, session.clone(), main.clone(), io.clone())
            .unwrap()
            .connected();
        let live_main = queue_main(main, live_agent, "same-session");
        let live_io = queue_io(io, live_agent, "same-session");
        for _ in 0..4096 {
            let agent = RendererDevToolsAgentToken::allocate();
            let stale = connections
                .begin(agent, session.clone(), main.clone(), io.clone())
                .unwrap();
            let retired_main = queue_main(main, agent, "same-session");
            let retired_io = queue_io(io, agent, "same-session");
            target.retire_agent_connections(agent);
            assert!(!stale.is_current());
            drop(stale);
            assert!(matches!(
                retired_main.wait_for_completion().await,
                Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
            ));
            assert!(matches!(
                retired_io.wait_for_first_dispatch().await,
                Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
            ));
            assert_eq!(connections.inner.lock().sessions.len(), 1);
            assert!(
                connections
                    .admission(&RendererDevToolsSessionLaneKey::new(
                        live_agent,
                        session.clone()
                    ))
                    .ready()
            );
        }
        let mut command = main.claim_for_owner().expect("live agent Main survives");
        main.first_dispatch_guard(&mut command).release();
        let mut command = io.claim_for_owner().expect("live agent IO survives");
        io.first_dispatch_guard(&mut command).release();
        assert!(matches!(
            live_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        ));
        drop(live_main);
    }

    #[tokio::test]
    async fn connecting_a_session_preserves_work_admitted_before_its_initialization() {
        let (connections, main, io) = receivers();
        let agent = RendererDevToolsAgentToken::allocate();
        let earlier_main = queue_main(&main, agent, "existing");
        let earlier_io = queue_io(&io, agent, "existing");
        let connect = connections
            .begin(
                agent,
                DevToolsSessionKey::Attached("existing".into()),
                main.clone(),
                io.clone(),
            )
            .unwrap();
        let later_main = queue_main(&main, agent, "existing");
        let later_io = queue_io(&io, agent, "existing");

        // Native activation can already be in the Primary lane when Browser
        // attach installs its connection. Blocking that earlier work would
        // also strand the connect operation queued behind it.
        let mut command = main
            .claim_for_owner()
            .expect("a later connection cannot gate already admitted Main work");
        main.first_dispatch_guard(&mut command).release();
        drop(command);
        assert!(main.claim_for_owner().is_none());
        let mut command = io
            .claim_for_owner()
            .expect("a later connection cannot gate already admitted IO work");
        io.first_dispatch_guard(&mut command).release();
        drop(command);
        assert!(io.claim_for_owner().is_none());
        assert!(matches!(
            earlier_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        ));

        connect.connected();
        let mut command = main.claim_for_owner().expect("later Main is now connected");
        main.first_dispatch_guard(&mut command).release();
        let mut command = io.claim_for_owner().expect("later IO is now connected");
        io.first_dispatch_guard(&mut command).release();
        assert!(matches!(
            later_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        ));
        drop((earlier_main, later_main));
    }

    #[tokio::test]
    async fn native_connect_releases_io_before_main_configuration_or_browser_observation() {
        let (connections, main, io) = receivers();
        let agent = RendererDevToolsAgentToken::allocate();
        let connect = connections
            .begin(
                agent,
                DevToolsSessionKey::Attached("new".into()),
                main.clone(),
                io.clone(),
            )
            .unwrap();
        let pending_main = queue_main(&main, agent, "new");
        let pending_io = queue_io(&io, agent, "new");
        assert!(main.claim_for_owner().is_none());
        assert!(io.claim_for_owner().is_none());
        let peer = queue_io(&io, agent, "peer");
        let mut command = io
            .claim_for_owner()
            .expect("a connecting session cannot hold another session's IO");
        io.first_dispatch_guard(&mut command).release();
        assert!(matches!(
            peer.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        ));
        connect.connected();
        let configuration = main
            .claim_for_owner()
            .expect("Main may begin after native connect");
        let (_token, _policy, _envelope, _dispatch, held_reply) =
            main.prepare_owner_dispatch(configuration).into_parts();
        // Main configuration is still active, and no Browser completion has
        // been consumed. IO nevertheless has its native output/session route.
        let mut command = io
            .claim_for_owner()
            .expect("IO must not wait for the Main configuration reply");
        io.first_dispatch_guard(&mut command).release();
        assert!(matches!(
            pending_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        ));
        drop(held_reply);
        drop(pending_main);
    }

    #[tokio::test]
    async fn abandoned_connect_rejects_both_lanes_and_stale_attempt_cannot_open_replacement() {
        let (connections, main, io) = receivers();
        let agent = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Attached("named".into());
        let connect = connections
            .begin(agent, session.clone(), main.clone(), io.clone())
            .unwrap();
        let queued_main = queue_main(&main, agent, "named");
        let queued_io = queue_io(&io, agent, "named");
        drop(connect);
        assert!(matches!(
            queued_main.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
        ));
        assert!(matches!(
            queued_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));
        assert!(matches!(
            queue_io(&io, agent, "named")
                .wait_for_first_dispatch()
                .await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));
        connections.detach(agent, &session);
        let old = connections
            .begin(agent, session.clone(), main.clone(), io.clone())
            .unwrap();
        let old_main = queue_main(&main, agent, "named");
        let old_io = queue_io(&io, agent, "named");
        let key = RendererDevToolsSessionLaneKey::new(agent, session.clone());
        let old_admission = connections.admission(&key);
        connections.detach(agent, &session);
        let replacement = connections
            .begin(agent, session.clone(), main.clone(), io.clone())
            .unwrap();
        let replacement_main = queue_main(&main, agent, "named");
        let replacement_io = queue_io(&io, agent, "named");
        old.connected();
        // A failed attempt can reach cancellation cleanup after this session
        // ID has been reused. Only the old connection's queued work is rejected.
        main.reject_failed_connections();
        io.reject_failed_connections();
        assert!(matches!(
            old_main.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
        ));
        assert!(matches!(
            old_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));
        assert!(
            !connections.admission(&key).ready(),
            "old cancellation cannot unlock the replacement"
        );
        replacement.connected();
        assert!(connections.admission(&key).ready());
        assert!(
            !old_admission.ready(),
            "a replacement connection cannot authorize old admitted work"
        );
        let mut command = main
            .claim_for_owner()
            .expect("old cancellation must preserve replacement Main work");
        main.first_dispatch_guard(&mut command).release();
        let mut command = io
            .claim_for_owner()
            .expect("old cancellation must preserve replacement IO work");
        io.first_dispatch_guard(&mut command).release();
        assert!(matches!(
            replacement_io.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        ));
        drop(replacement_main);
        connections.detach(agent, &session);
        assert!(connections.inner.lock().sessions.is_empty());
    }

    #[tokio::test]
    async fn frontend_revocation_preserves_only_admitted_internal_cleanup() {
        let (_connections, main, _io) = receivers();
        let agent = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Attached("cleanup".into());
        let frontend = queue_main(&main, agent, "cleanup");
        let cleanup = queue_main(&main, agent, "cleanup");
        cleanup.cleanup_control().retain();
        main.begin_session_detach(agent, &session);
        assert!(matches!(
            frontend.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
        ));
        let command = main
            .claim_for_owner()
            .expect("revocation must preserve internal cleanup authority");
        let (_, _, _, mut dispatch, reply) = main.prepare_owner_dispatch(command).into_parts();
        dispatch.release();
        let replacement = queue_main(&main, agent, "cleanup");
        assert!(
            main.claim_for_owner().is_none(),
            "replacement frontend waits for native retirement"
        );
        main.finish_session_detach(agent, &session);
        assert!(main.claim_for_owner().is_some());
        drop(reply);
        drop(cleanup);
        drop(replacement);
    }
}
