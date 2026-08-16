use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Weak},
};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use crate::runtime::RendererRuntimeInspectorResponseSender;
use crate::runtime::{
    PageId, PendingRendererOutputRecord, RendererInspectorIngressTicket,
    RendererInspectorPauseCommandEffect, RendererOutputResidenceIdentity,
    RendererProtocolObservation, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
    RendererTurnOutputJournal,
};
use crate::script_vm::inspector_io::RendererInspectorIoIngress;
use crate::script_vm::inspector_main::RendererInspectorMainIngress;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererInspectorPausePhase {
    Running,
    Entering,
    Paused,
}

/// Selects which DevTools receivers may be pumped by V8's nested pause loop.
///
/// Chromium runs a nestable Main-thread message loop for ordinary debugger
/// pauses, but instrumentation pauses only process interrupting Inspector
/// work. The policy is captured from the `Debugger.paused` notification before
/// V8 calls `run_message_loop_on_pause`, so the loop never needs to inspect a
/// command method or infer priority from its queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RendererInspectorPauseLoopPolicy {
    MainAndIo,
    IoOnly,
}

struct RendererInspectorPauseBridgeState {
    next_preface_id: u64,
    phase: RendererInspectorPausePhase,
    pause_loop_policy: RendererInspectorPauseLoopPolicy,
    quit_requested: bool,
    session_detach_arms: usize,
    target_closed: bool,
    pending_prefaces: VecDeque<RendererInspectorPausePreface>,
    paused_sessions_awaiting_resumed: HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
    // V8 dispatches one nested-loop command synchronously. A successful
    // resume/step response is emitted before dispatch returns; only then does
    // V8 leave the loop and report resumed to every session. A following
    // pause is likewise reported to every session before the next loop starts,
    // so active and pending transition ownership are each singular.
    active_command_dispatch: Option<RendererInspectorPauseCommandDispatch>,
    pending_command_transition: Option<RendererInspectorPauseCommandTransition>,
    route: Option<RendererInspectorPauseRoute>,
}

struct RendererInspectorPauseCommandDispatch {
    command_id: u64,
    transition: RendererInspectorPauseCommandTransition,
}

struct RendererInspectorPauseCommandTransition {
    causal_identity: RendererRuntimeCommandCausalIdentity,
    effect: RendererInspectorPauseCommandEffect,
    response_succeeded: bool,
    awaiting_resumed: HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
    awaiting_repause: HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
}

impl RendererInspectorPauseCommandTransition {
    fn is_complete(&self) -> bool {
        match self.effect {
            RendererInspectorPauseCommandEffect::None => true,
            RendererInspectorPauseCommandEffect::Resume => self.awaiting_resumed.is_empty(),
            RendererInspectorPauseCommandEffect::Step => {
                self.awaiting_resumed.is_empty() && self.awaiting_repause.is_empty()
            }
        }
    }

    fn observe_notification(
        &mut self,
        session: &(RendererDevToolsAgentToken, DevToolsSessionKey),
        is_resumed_notification: bool,
        is_paused_notification: bool,
    ) -> bool {
        if self.effect == RendererInspectorPauseCommandEffect::None {
            return false;
        }
        if is_resumed_notification && self.awaiting_resumed.remove(session) {
            if self.effect == RendererInspectorPauseCommandEffect::Step {
                self.awaiting_repause.insert(session.clone());
            }
            return true;
        }
        is_paused_notification
            && self.effect == RendererInspectorPauseCommandEffect::Step
            && self.awaiting_repause.remove(session)
    }

    fn output_route(&self) -> RendererInspectorPauseCommandOutputRoute {
        RendererInspectorPauseCommandOutputRoute {
            causal_identity: self.causal_identity.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RendererInspectorPauseCommandOutputRoute {
    pub(super) causal_identity: RendererRuntimeCommandCausalIdentity,
}

struct RendererInspectorPausePreface {
    id: u64,
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
    messages: Vec<RendererRuntimeInspectorMessage>,
}

#[must_use]
pub(super) struct RendererInspectorPausePrefaceGuard {
    bridge: RendererInspectorPauseBridge,
    id: u64,
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

impl Drop for RendererInspectorPausePrefaceGuard {
    fn drop(&mut self) {
        self.bridge.cancel_pause_preface(self.id);
    }
}
#[derive(Clone)]
pub(super) struct RendererInspectorSessionOutboundRoute {
    bridge: RendererInspectorPauseBridge,
    main_ingress: RendererInspectorMainIngress,
    io_ingress: RendererInspectorIoIngress,
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
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
                next_preface_id: 1,
                phase: RendererInspectorPausePhase::Running,
                pause_loop_policy: RendererInspectorPauseLoopPolicy::MainAndIo,
                quit_requested: false,
                session_detach_arms: 0,
                target_closed: false,
                pending_prefaces: VecDeque::new(),
                paused_sessions_awaiting_resumed: HashSet::new(),
                active_command_dispatch: None,
                pending_command_transition: None,
                route: None,
            }),
            pause_loop_wake: Condvar::new(),
        });
        Self { shared }
    }
}

impl std::fmt::Debug for RendererInspectorPauseBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        formatter
            .debug_struct("RendererInspectorPauseBridge")
            .field("phase", &state.phase)
            .field("pause_loop_policy", &state.pause_loop_policy)
            .field("quit_requested", &state.quit_requested)
            .field("session_detach_arms", &state.session_detach_arms)
            .field("target_closed", &state.target_closed)
            .field("pending_prefaces", &state.pending_prefaces.len())
            .field(
                "paused_sessions_awaiting_resumed",
                &state.paused_sessions_awaiting_resumed.len(),
            )
            .field(
                "has_active_command_dispatch",
                &state.active_command_dispatch.is_some(),
            )
            .field(
                "has_pending_command_transition",
                &state.pending_command_transition.is_some(),
            )
            .finish()
    }
}

impl RendererInspectorPauseBridge {
    pub(crate) fn pause_loop_wake(&self) -> RendererInspectorPauseLoopWake {
        RendererInspectorPauseLoopWake {
            shared: Arc::downgrade(&self.shared),
        }
    }

    pub(super) fn outbound_route(
        &self,
        main_ingress: RendererInspectorMainIngress,
        io_ingress: RendererInspectorIoIngress,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) -> RendererInspectorSessionOutboundRoute {
        RendererInspectorSessionOutboundRoute {
            bridge: self.clone(),
            main_ingress,
            io_ingress,
            agent_token,
            session,
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
        self.shared.state.lock().phase != RendererInspectorPausePhase::Running
    }

    pub(super) fn begin_command_dispatch(
        &self,
        command_id: u64,
        ticket: &RendererInspectorIngressTicket,
        effect: RendererInspectorPauseCommandEffect,
        response_call_id: Option<i32>,
    ) -> RendererInspectorPauseCommandDispatchGuard {
        if effect == RendererInspectorPauseCommandEffect::None {
            return RendererInspectorPauseCommandDispatchGuard {
                bridge: self.clone(),
                command_id: None,
            };
        }
        let Some(call_id) = response_call_id else {
            return RendererInspectorPauseCommandDispatchGuard {
                bridge: self.clone(),
                command_id: None,
            };
        };
        let causal_identity = RendererRuntimeCommandCausalIdentity::new(
            ticket.session().wire_session_id().map(str::to_owned),
            call_id,
        );
        let mut state = self.shared.state.lock();
        let awaiting_resumed = state.paused_sessions_awaiting_resumed.clone();
        assert!(
            state.active_command_dispatch.is_none(),
            "Inspector pause commands must dispatch serially in the nested loop"
        );
        state.active_command_dispatch = Some(RendererInspectorPauseCommandDispatch {
            command_id,
            transition: RendererInspectorPauseCommandTransition {
                causal_identity,
                effect,
                response_succeeded: false,
                awaiting_resumed,
                awaiting_repause: HashSet::new(),
            },
        });
        RendererInspectorPauseCommandDispatchGuard {
            bridge: self.clone(),
            command_id: Some(command_id),
        }
    }

    fn finish_command_dispatch(&self, command_id: u64) {
        let mut state = self.shared.state.lock();
        let dispatch = state
            .active_command_dispatch
            .take()
            .expect("an Inspector pause command dispatch guard requires an active command");
        assert_eq!(
            dispatch.command_id, command_id,
            "the Inspector pause command guard must finish its active dispatch"
        );
        let transition = dispatch.transition;
        if !transition.response_succeeded || transition.is_complete() {
            return;
        }
        assert!(
            state.pending_command_transition.is_none(),
            "one successful Inspector control transition must finish before the next nested-loop command"
        );
        state.pending_command_transition = Some(transition);
    }

    fn mark_command_response(
        &self,
        inspector_session_id: Option<&str>,
        call_id: i32,
        succeeded: bool,
    ) {
        let mut state = self.shared.state.lock();
        let matches = |cause: &RendererRuntimeCommandCausalIdentity| {
            cause.call_id() == call_id && cause.inspector_session_id() == inspector_session_id
        };
        if let Some(dispatch) = state.active_command_dispatch.as_mut()
            && matches(&dispatch.transition.causal_identity)
        {
            dispatch.transition.response_succeeded = succeeded;
        }
    }

    /// Ends the bounded handoff from a resume/step command to the renderer turn
    /// it released. A step that reaches the end of its task may never enter a
    /// new pause; owner settlement is the terminal that prevents its cause from
    /// leaking into a later, unrelated pause.
    pub(crate) fn finish_owner_turn(&self) {
        self.shared.state.lock().pending_command_transition = None;
    }

    fn stage_pause_preface(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Option<RendererInspectorPausePrefaceGuard> {
        if messages.is_empty() {
            return None;
        }
        let mut state = self.shared.state.lock();
        if state.target_closed || state.route.is_none() {
            return None;
        }
        let id = state.next_preface_id;
        state.next_preface_id = state
            .next_preface_id
            .checked_add(1)
            .expect("runtime inspector pause preface ID overflow");
        state
            .pending_prefaces
            .push_back(RendererInspectorPausePreface {
                id,
                agent_token,
                session,
                messages,
            });
        Some(RendererInspectorPausePrefaceGuard {
            bridge: self.clone(),
            id,
        })
    }

    fn cancel_pause_preface(&self, id: u64) {
        let mut state = self.shared.state.lock();
        if let Some(position) = state
            .pending_prefaces
            .iter()
            .position(|preface| preface.id == id)
        {
            state.pending_prefaces.remove(position);
        }
    }

    pub(crate) fn arm_session_detach(&self) {
        let mut state = self.shared.state.lock();
        state.session_detach_arms = state
            .session_detach_arms
            .checked_add(1)
            .expect("runtime inspector session detach arm count overflow");
        if state.phase != RendererInspectorPausePhase::Running {
            self.shared.pause_loop_wake.notify_all();
        }
    }

    pub(crate) fn disarm_session_detach(&self) {
        let mut state = self.shared.state.lock();
        state.session_detach_arms = state
            .session_detach_arms
            .checked_sub(1)
            .expect("runtime inspector session detach arm count underflow");
    }

    pub(super) fn enter_pause(&self) -> Option<RendererInspectorPauseLoopPolicy> {
        let mut state = self.shared.state.lock();
        if state.target_closed || state.phase != RendererInspectorPausePhase::Entering {
            return None;
        }
        state.phase = RendererInspectorPausePhase::Paused;
        Some(state.pause_loop_policy)
    }

    pub(crate) fn wait_for_pause_work<T>(&self, mut claim: impl FnMut() -> Option<T>) -> Option<T> {
        let mut state = self.shared.state.lock();
        loop {
            if state.target_closed || state.quit_requested || state.session_detach_arms != 0 {
                return None;
            }
            if let Some(work) = claim() {
                return Some(work);
            }
            self.shared.pause_loop_wake.wait(&mut state);
        }
    }

    pub(crate) fn request_quit(&self) {
        let mut state = self.shared.state.lock();
        if state.phase != RendererInspectorPausePhase::Running {
            state.quit_requested = true;
            self.shared.pause_loop_wake.notify_all();
        }
    }

    pub(super) fn leave_pause(&self) {
        let mut state = self.shared.state.lock();
        state.phase = RendererInspectorPausePhase::Running;
        state.pause_loop_policy = RendererInspectorPauseLoopPolicy::MainAndIo;
        state.quit_requested = false;
        // Commands that lost the nested-loop race stay in their route-specific
        // ingress. Main retains its owner task; IO retains owner and interrupt
        // execution chances.
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
        state.pending_prefaces.clear();
        state.paused_sessions_awaiting_resumed.clear();
        state.pending_command_transition = None;
        match state.phase {
            RendererInspectorPausePhase::Running => {}
            RendererInspectorPausePhase::Entering => {
                state.phase = RendererInspectorPausePhase::Running;
                state.pause_loop_policy = RendererInspectorPauseLoopPolicy::MainAndIo;
                state.quit_requested = false;
            }
            RendererInspectorPausePhase::Paused => {
                state.quit_requested = true;
                self.shared.pause_loop_wake.notify_all();
            }
        }
        true
    }

    pub(crate) fn close_target(&self) {
        let mut state = self.shared.state.lock();
        state.target_closed = true;
        state.quit_requested = true;
        state.pending_prefaces.clear();
        state.paused_sessions_awaiting_resumed.clear();
        state.pending_command_transition = None;
        self.shared.pause_loop_wake.notify_all();
    }

    pub(super) fn record_v8_state_update(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        state_update: V8InspectorSessionState,
    ) {
        let route = {
            let state = self.shared.state.lock();
            if state.target_closed {
                return;
            }
            state.route.clone()
        };
        let Some(route) = route else {
            return;
        };
        let mut batch = RendererRuntimeInspectorMessageBatch::new(agent_token, session, Vec::new());
        batch.v8_state_update = Some(state_update);
        route.output_journal.publish_record(
            PendingRendererOutputRecord::observation(
                None,
                RendererProtocolObservation::RuntimeInspector(batch),
            )
            .resolve()
            .unwrap_or_else(|_| {
                panic!("Inspector state update must have resolved source identity")
            }),
        );
    }

    fn route_notification(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
        message: &Value,
    ) -> RendererInspectorPauseNotificationRoute {
        let method = message.get("method").and_then(Value::as_str);
        let is_paused_notification = method == Some("Debugger.paused");
        let is_resumed_notification = method == Some("Debugger.resumed");
        let session_route = (agent_token, session.clone());
        let mut state = self.shared.state.lock();
        if state.target_closed {
            return RendererInspectorPauseNotificationRoute::Drop;
        }
        if is_paused_notification && (state.route.is_none() || state.session_detach_arms != 0) {
            return RendererInspectorPauseNotificationRoute::Drop;
        }
        let preface = if is_paused_notification {
            state
                .pending_prefaces
                .iter()
                .position(|preface| {
                    preface.agent_token == agent_token && preface.session == *session
                })
                .and_then(|position| state.pending_prefaces.remove(position))
                .map(|preface| preface.messages)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if is_paused_notification {
            state
                .paused_sessions_awaiting_resumed
                .insert(session_route.clone());
        }
        let resumes_reported_pause = is_resumed_notification
            && state
                .paused_sessions_awaiting_resumed
                .remove(&session_route);
        let (command_output, command_transition_complete) =
            if let Some(transition) = state.pending_command_transition.as_mut() {
                let matched = transition.observe_notification(
                    &session_route,
                    is_resumed_notification,
                    is_paused_notification,
                );
                (
                    matched.then(|| transition.output_route()),
                    transition.is_complete(),
                )
            } else {
                (None, false)
            };
        if command_transition_complete {
            state.pending_command_transition = None;
        }
        if is_paused_notification {
            let is_instrumentation_pause = message
                .get("params")
                .and_then(|params| params.get("reason"))
                .and_then(Value::as_str)
                == Some("instrumentation");
            if state.phase == RendererInspectorPausePhase::Running {
                state.phase = RendererInspectorPausePhase::Entering;
                state.pause_loop_policy = if is_instrumentation_pause {
                    RendererInspectorPauseLoopPolicy::IoOnly
                } else {
                    RendererInspectorPauseLoopPolicy::MainAndIo
                };
            } else if state.phase == RendererInspectorPausePhase::Entering
                && is_instrumentation_pause
            {
                // Multiple V8InspectorSessions observe the same isolate pause.
                // Any session identifying it as instrumentation tightens the
                // shared loop policy before V8 enters the client loop.
                state.pause_loop_policy = RendererInspectorPauseLoopPolicy::IoOnly;
            }
        }
        if state.phase == RendererInspectorPausePhase::Running && !resumes_reported_pause {
            RendererInspectorPauseNotificationRoute::OrdinaryTurn
        } else {
            RendererInspectorPauseNotificationRoute::PublishImmediately {
                preface,
                command_output,
            }
        }
    }

    fn detach_session(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let mut state = self.shared.state.lock();
        state
            .paused_sessions_awaiting_resumed
            .remove(&(agent_token, session.clone()));
        state
            .pending_prefaces
            .retain(|preface| preface.agent_token != agent_token || &preface.session != session);
        let session_route = (agent_token, session.clone());
        if let Some(dispatch) = state.active_command_dispatch.as_mut() {
            dispatch.transition.awaiting_resumed.remove(&session_route);
            dispatch.transition.awaiting_repause.remove(&session_route);
        }
        if let Some(transition) = state.pending_command_transition.as_mut() {
            transition.awaiting_resumed.remove(&session_route);
            transition.awaiting_repause.remove(&session_route);
        }
        if state
            .pending_command_transition
            .as_ref()
            .is_some_and(RendererInspectorPauseCommandTransition::is_complete)
        {
            state.pending_command_transition = None;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum RendererInspectorPauseNotificationRoute {
    OrdinaryTurn,
    PublishImmediately {
        preface: Vec<RendererRuntimeInspectorMessage>,
        command_output: Option<RendererInspectorPauseCommandOutputRoute>,
    },
    Drop,
}

impl RendererInspectorSessionOutboundRoute {
    pub(super) fn route_notification(
        &self,
        message: &Value,
    ) -> RendererInspectorPauseNotificationRoute {
        self.bridge
            .route_notification(self.agent_token, &self.session, message)
    }

    pub(super) fn mark_command_response(&self, call_id: i32, succeeded: bool) {
        self.bridge
            .mark_command_response(self.session.wire_session_id(), call_id, succeeded);
    }

    pub(super) fn detach_session(&self) {
        self.bridge.detach_session(self.agent_token, &self.session);
        self.main_ingress
            .detach_session(self.agent_token, &self.session);
        self.io_ingress
            .detach_session(self.agent_token, &self.session);
    }

    pub(super) fn stage_pause_preface(
        &self,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Option<RendererInspectorPausePrefaceGuard> {
        self.bridge
            .stage_pause_preface(self.agent_token, self.session.clone(), messages)
    }
}

pub(super) struct RendererInspectorPauseCommandDispatchGuard {
    bridge: RendererInspectorPauseBridge,
    command_id: Option<u64>,
}

impl Drop for RendererInspectorPauseCommandDispatchGuard {
    fn drop(&mut self) {
        if let Some(command_id) = self.command_id {
            self.bridge.finish_command_dispatch(command_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RendererInspectorCommandRoute;

    fn io_ingress(
        bridge: &RendererInspectorPauseBridge,
    ) -> crate::script_vm::inspector_io::RendererInspectorIoIngress {
        crate::script_vm::inspector_io::RendererInspectorIoIngress::new(
            bridge.pause_loop_wake(),
            None,
        )
    }

    fn main_ingress(
        bridge: &RendererInspectorPauseBridge,
    ) -> crate::script_vm::inspector_main::RendererInspectorMainIngress {
        crate::script_vm::inspector_main::RendererInspectorMainIngress::new(
            crate::script_vm::inspector_route::RendererInspectorSessionExecutorRouteId::new(1),
            bridge.pause_loop_wake(),
        )
    }

    fn configure_page(bridge: &RendererInspectorPauseBridge, page_id: PageId) {
        bridge.configure_page_route(RendererTurnOutputJournal::new(
            crate::runtime::RendererOutputStreamIdentity::new_page(
                crate::runtime::RendererOwnerLocalHostId::new_for_testing(page_id.as_u64()),
                page_id,
                RendererDevToolsAgentToken::allocate(),
            ),
        ));
    }

    fn outbound_route(
        bridge: &RendererInspectorPauseBridge,
    ) -> RendererInspectorSessionOutboundRoute {
        outbound_route_with_io(bridge, io_ingress(bridge))
    }

    fn outbound_route_with_io(
        bridge: &RendererInspectorPauseBridge,
        io_ingress: crate::script_vm::inspector_io::RendererInspectorIoIngress,
    ) -> RendererInspectorSessionOutboundRoute {
        bridge.outbound_route(
            main_ingress(bridge),
            io_ingress,
            RendererDevToolsAgentToken::allocate(),
            DevToolsSessionKey::Primary,
        )
    }

    fn enqueue_command(
        ingress: &crate::script_vm::inspector_io::RendererInspectorIoIngress,
        agent_token: RendererDevToolsAgentToken,
        inspector_session_id: Option<String>,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> crate::script_vm::inspector_io::RendererRuntimeInspectorIoCommandRoute {
        ingress.enqueue_command(
            agent_token,
            crate::runtime::RendererInspectorCommandEnvelope::new_io(
                crate::runtime::RendererInspectorIngressTicket::new(
                    None,
                    inspector_session_id,
                    crate::runtime::RendererInspectorCommandRoute::Io,
                ),
                raw_json,
                Some(response),
            ),
        )
    }

    fn route_paused(
        bridge: &RendererInspectorPauseBridge,
    ) -> RendererInspectorPauseNotificationRoute {
        outbound_route(bridge).route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        }))
    }

    fn response_sender(
        call_id: i32,
    ) -> (
        RendererRuntimeInspectorResponseSender,
        tokio::sync::oneshot::Receiver<crate::runtime::RendererRuntimeInspectorAsyncCompletion>,
    ) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (RendererRuntimeInspectorResponseSender::new(call_id, tx), rx)
    }

    fn expect_immediate_preface(
        route: RendererInspectorPauseNotificationRoute,
    ) -> Vec<RendererRuntimeInspectorMessage> {
        match route {
            RendererInspectorPauseNotificationRoute::PublishImmediately { preface, .. } => preface,
            route => panic!("expected immediate pause publication, got {route:?}"),
        }
    }

    fn expect_immediate_command_output(
        route: RendererInspectorPauseNotificationRoute,
    ) -> Option<RendererInspectorPauseCommandOutputRoute> {
        match route {
            RendererInspectorPauseNotificationRoute::PublishImmediately {
                command_output, ..
            } => command_output,
            route => panic!("expected immediate pause publication, got {route:?}"),
        }
    }

    #[test]
    fn step_transition_keeps_the_exact_command_cause_through_repause() {
        let bridge = RendererInspectorPauseBridge::default();
        let io_ingress = io_ingress(&bridge);
        configure_page(&bridge, PageId::new_for_testing(1));
        let outbound = outbound_route_with_io(&bridge, io_ingress.clone());
        assert!(
            expect_immediate_preface(outbound.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {"callFrames": []},
            })))
            .is_empty()
        );
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::MainAndIo)
        );

        let (response, _response_rx) = response_sender(41);
        let command_route = enqueue_command(
            &io_ingress,
            RendererDevToolsAgentToken::allocate(),
            None,
            r#"{"id":41,"method":"Debugger.stepOut","params":{}}"#.to_owned(),
            response,
        );
        assert_eq!(
            command_route.ticket().route(),
            RendererInspectorCommandRoute::Io
        );
        let command = io_ingress
            .wait_and_claim_for_pause(&bridge)
            .expect("the nested pause loop should claim stepOut");
        let first_dispatch = io_ingress.first_dispatch_guard(&command);
        assert_eq!(command.ticket(), command_route.ticket());
        let dispatch = bridge.begin_command_dispatch(
            command.command_id(),
            command.ticket(),
            command.pause_effect(),
            command.response().map(|response| response.call_id()),
        );
        outbound.mark_command_response(41, true);
        drop(dispatch);
        drop(first_dispatch);
        bridge.leave_pause();
        let resumed = expect_immediate_command_output(
            outbound.route_notification(&json!({"method": "Debugger.resumed", "params": {}})),
        )
        .expect("the resumed event must retain the stepOut cause");
        assert_eq!(
            resumed.causal_identity,
            RendererRuntimeCommandCausalIdentity::new(None, 41)
        );

        let paused = expect_immediate_command_output(outbound.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        })))
        .expect("the following pause must retain the same stepOut cause");
        assert_eq!(paused.causal_identity, resumed.causal_identity);
        assert!(
            bridge
                .shared
                .state
                .lock()
                .pending_command_transition
                .is_none()
        );
        drop(command_route);
    }

    #[test]
    fn step_cause_ends_with_the_owner_turn_when_no_repause_occurs() {
        let bridge = RendererInspectorPauseBridge::default();
        let io_ingress = io_ingress(&bridge);
        configure_page(&bridge, PageId::new_for_testing(1));
        let outbound = outbound_route_with_io(&bridge, io_ingress.clone());
        assert!(
            expect_immediate_preface(outbound.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {"callFrames": []},
            })))
            .is_empty()
        );
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::MainAndIo)
        );

        let (response, _response_rx) = response_sender(43);
        let command_route = enqueue_command(
            &io_ingress,
            RendererDevToolsAgentToken::allocate(),
            None,
            r#"{"id":43,"method":"Debugger.stepOut","params":{}}"#.to_owned(),
            response,
        );
        let command = io_ingress
            .wait_and_claim_for_pause(&bridge)
            .expect("the nested pause loop should claim stepOut");
        let first_dispatch = io_ingress.first_dispatch_guard(&command);
        let dispatch = bridge.begin_command_dispatch(
            command.command_id(),
            command.ticket(),
            command.pause_effect(),
            command.response().map(|response| response.call_id()),
        );
        outbound.mark_command_response(43, true);
        drop(dispatch);
        drop(first_dispatch);
        bridge.leave_pause();
        assert!(
            expect_immediate_command_output(
                outbound.route_notification(&json!({"method": "Debugger.resumed", "params": {}})),
            )
            .is_some()
        );

        bridge.finish_owner_turn();
        assert!(
            expect_immediate_command_output(outbound.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {"callFrames": []},
            })))
            .is_none(),
            "an unrelated later pause must not inherit the completed turn's step cause"
        );
        drop(command_route);
    }

    #[test]
    fn failed_step_command_does_not_own_a_later_resume_transition() {
        let bridge = RendererInspectorPauseBridge::default();
        let io_ingress = io_ingress(&bridge);
        configure_page(&bridge, PageId::new_for_testing(1));
        let outbound = outbound_route_with_io(&bridge, io_ingress.clone());
        assert!(
            expect_immediate_preface(outbound.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {"callFrames": []},
            })))
            .is_empty()
        );
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::MainAndIo)
        );

        let (response, _response_rx) = response_sender(42);
        let command_route = enqueue_command(
            &io_ingress,
            RendererDevToolsAgentToken::allocate(),
            None,
            r#"{"id":42,"method":"Debugger.stepOut","params":{}}"#.to_owned(),
            response,
        );
        let command = io_ingress
            .wait_and_claim_for_pause(&bridge)
            .expect("the nested pause loop should claim stepOut");
        let first_dispatch = io_ingress.first_dispatch_guard(&command);
        let dispatch = bridge.begin_command_dispatch(
            command.command_id(),
            command.ticket(),
            command.pause_effect(),
            command.response().map(|response| response.call_id()),
        );
        outbound.mark_command_response(42, false);
        drop(dispatch);
        drop(first_dispatch);

        assert!(
            bridge
                .shared
                .state
                .lock()
                .pending_command_transition
                .is_none()
        );
        bridge.leave_pause();
        assert!(
            expect_immediate_command_output(
                outbound.route_notification(&json!({"method": "Debugger.resumed", "params": {}})),
            )
            .is_none(),
            "a failed step response must not own a later resumed event"
        );
        drop(command_route);
    }

    #[test]
    fn staged_pause_preface_is_claimed_at_paused_boundary() {
        let bridge = RendererInspectorPauseBridge::default();
        configure_page(&bridge, PageId::new_for_testing(1));
        let route = outbound_route(&bridge);
        let preface = json!({
            "method": "DOM.setChildNodes",
            "params": {"parentId": 1, "nodes": []}
        });
        let guard = route
            .stage_pause_preface(vec![RendererRuntimeInspectorMessage::protocol(
                preface.clone(),
            )])
            .expect("configured page should accept a pause preface");
        let messages = expect_immediate_preface(route.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"reason": "DOM", "callFrames": []},
        })));
        drop(guard);

        let values = messages
            .into_iter()
            .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
            .collect::<Vec<_>>();
        assert_eq!(values, vec![preface]);
        assert!(bridge.shared.state.lock().pending_prefaces.is_empty());
    }

    #[test]
    fn staged_pause_preface_is_discarded_when_no_pause_occurs() {
        let bridge = RendererInspectorPauseBridge::default();
        configure_page(&bridge, PageId::new_for_testing(1));
        let route = outbound_route(&bridge);
        let guard = route
            .stage_pause_preface(vec![RendererRuntimeInspectorMessage::protocol(json!({
                "method": "DOM.setChildNodes",
                "params": {"parentId": 1, "nodes": []}
            }))])
            .expect("configured page should accept a pause preface");
        assert_eq!(bridge.shared.state.lock().pending_prefaces.len(), 1);
        drop(guard);
        assert!(bridge.shared.state.lock().pending_prefaces.is_empty());
    }

    #[test]
    fn resumed_after_pause_loop_exit_stays_on_pause_bridge() {
        let bridge = RendererInspectorPauseBridge::default();
        configure_page(&bridge, PageId::new_for_testing(1));
        let route = outbound_route(&bridge);

        assert!(
            expect_immediate_preface(route.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {"callFrames": []},
            })))
            .is_empty(),
            "paused notification should publish at the pause boundary"
        );
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::MainAndIo)
        );
        bridge.leave_pause();
        assert_eq!(
            bridge.shared.state.lock().phase,
            RendererInspectorPausePhase::Running
        );

        assert!(
            expect_immediate_preface(
                route.route_notification(&json!({"method": "Debugger.resumed", "params": {}}))
            )
            .is_empty(),
            "the resumed notification paired with the reported pause must publish immediately"
        );
        assert!(
            bridge
                .shared
                .state
                .lock()
                .paused_sessions_awaiting_resumed
                .is_empty()
        );

        let unpaired = json!({"method": "Debugger.resumed", "params": {}});
        assert_eq!(
            route.route_notification(&unpaired),
            RendererInspectorPauseNotificationRoute::OrdinaryTurn
        );
    }

    #[test]
    fn instrumentation_pause_selects_io_only_nested_loop_policy() {
        let bridge = RendererInspectorPauseBridge::default();
        configure_page(&bridge, PageId::new_for_testing(1));
        let route = outbound_route(&bridge);

        assert!(
            expect_immediate_preface(route.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {
                    "reason": "instrumentation",
                    "callFrames": [],
                },
            })))
            .is_empty()
        );
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::IoOnly),
            "instrumentation pauses must not pump the Main DevTools receiver"
        );
        bridge.leave_pause();

        assert!(
            expect_immediate_preface(route.route_notification(&json!({
                "method": "Debugger.paused",
                "params": {
                    "reason": "other",
                    "callFrames": [],
                },
            })))
            .is_empty()
        );
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::MainAndIo),
            "ordinary pauses must restore the nestable Main receiver"
        );
        bridge.leave_pause();
    }

    #[tokio::test]
    async fn dropping_io_route_cancels_the_unclaimed_command() {
        let bridge = RendererInspectorPauseBridge::default();
        let io_ingress = io_ingress(&bridge);
        configure_page(&bridge, PageId::new_for_testing(1));
        let (response, response_rx) = response_sender(8);
        let route = enqueue_command(
            &io_ingress,
            RendererDevToolsAgentToken::allocate(),
            None,
            r#"{"id":8,"method":"Runtime.getIsolateId"}"#.to_owned(),
            response,
        );
        drop(route);

        assert!(
            io_ingress.claim_for_owner().is_none(),
            "a canceled frontend route must remove its queued IO command"
        );
        let completion = response_rx
            .await
            .expect("route cancellation should explicitly fail the deferred response");
        let response = completion
            .output
            .protocol_response(8)
            .expect("route cancellation response");
        assert_eq!(
            response["error"]["message"],
            json!("Runtime inspector IO route was canceled before dispatch")
        );
    }

    #[test]
    fn page_detach_does_not_close_target_persistent_bridge_or_new_page_route() {
        let bridge = RendererInspectorPauseBridge::default();
        let first_page_id = PageId::new_for_testing(1);
        let second_page_id = PageId::new_for_testing(2);
        configure_page(&bridge, first_page_id);

        assert!(expect_immediate_preface(route_paused(&bridge)).is_empty());
        assert!(bridge.detach_page(first_page_id));
        {
            let state = bridge.shared.state.lock();
            assert_eq!(state.phase, RendererInspectorPausePhase::Running);
            assert!(!state.target_closed);
            assert!(state.route.is_none());
        }

        configure_page(&bridge, second_page_id);
        assert!(!bridge.detach_page(first_page_id));
        {
            let state = bridge.shared.state.lock();
            assert_eq!(
                state.route.as_ref().and_then(|route| {
                    match route.output_journal.stream().residence() {
                        RendererOutputResidenceIdentity::Page { page_id, .. } => Some(page_id),
                        RendererOutputResidenceIdentity::SharedWorker { .. }
                        | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
                    }
                }),
                Some(second_page_id),
                "a stale page drop must not detach the replacement page"
            );
            assert!(!state.target_closed);
        }

        bridge.close_target();
        assert!(bridge.shared.state.lock().target_closed);
    }

    #[test]
    fn quit_requested_while_entering_survives_nested_loop_entry() {
        let bridge = RendererInspectorPauseBridge::default();
        configure_page(&bridge, PageId::new_for_testing(1));
        assert!(expect_immediate_preface(route_paused(&bridge)).is_empty());

        bridge.request_quit();
        assert_eq!(
            bridge.enter_pause(),
            Some(RendererInspectorPauseLoopPolicy::MainAndIo)
        );
        let io_ingress = io_ingress(&bridge);
        assert!(io_ingress.wait_and_claim_for_pause(&bridge).is_none());
        bridge.leave_pause();
        assert_eq!(
            bridge.shared.state.lock().phase,
            RendererInspectorPausePhase::Running
        );
    }

    #[test]
    fn session_detach_arm_prevents_a_new_pause_before_owner_dispatch() {
        let bridge = RendererInspectorPauseBridge::default();
        configure_page(&bridge, PageId::new_for_testing(1));
        bridge.arm_session_detach();

        assert_eq!(
            route_paused(&bridge),
            RendererInspectorPauseNotificationRoute::Drop
        );
        assert_eq!(bridge.enter_pause(), None);
        assert_eq!(
            bridge.shared.state.lock().phase,
            RendererInspectorPausePhase::Running
        );

        bridge.disarm_session_detach();
    }

    #[test]
    fn detached_page_cannot_enter_a_new_pause() {
        let bridge = RendererInspectorPauseBridge::default();
        let page_id = PageId::new_for_testing(1);
        configure_page(&bridge, page_id);
        bridge.detach_page(page_id);

        assert_eq!(
            route_paused(&bridge),
            RendererInspectorPauseNotificationRoute::Drop
        );
        assert_eq!(bridge.enter_pause(), None);
        assert_eq!(
            bridge.shared.state.lock().phase,
            RendererInspectorPausePhase::Running
        );
    }
}
