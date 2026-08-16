//! Nested pause-loop coordination and causal Inspector output routing.

use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Weak},
};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::devtools::target::RendererDevToolsTargetHandle;
#[cfg(test)]
use crate::runtime::RendererRuntimeInspectorResponseSender;
use crate::runtime::{
    PageId, PendingRendererOutputRecord, RendererInspectorIngressTicket,
    RendererInspectorPauseCommandEffect, RendererOutputResidenceIdentity,
    RendererProtocolObservation, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
    RendererTurnOutputJournal,
};

mod causality;
mod loop_state;

use causality::{
    RendererInspectorPauseCommandDispatch, RendererInspectorPauseCommandTransition,
    RendererInspectorPausePreface,
};
pub(crate) use causality::{
    RendererInspectorPauseCommandOutputRoute, RendererInspectorPauseNotificationRoute,
};
pub(crate) use loop_state::RendererInspectorPauseLoopPolicy;
use loop_state::RendererInspectorPausePhase;

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

#[must_use]
pub(crate) struct RendererInspectorPausePrefaceGuard {
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
pub(crate) struct RendererInspectorSessionOutboundRoute {
    target: RendererDevToolsTargetHandle,
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

    pub(crate) fn outbound_route(
        &self,
        target: RendererDevToolsTargetHandle,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) -> RendererInspectorSessionOutboundRoute {
        RendererInspectorSessionOutboundRoute {
            target,
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

    pub(crate) fn begin_command_dispatch(
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

    pub(crate) fn enter_pause(&self) -> Option<RendererInspectorPauseLoopPolicy> {
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

    pub(crate) fn leave_pause(&self) {
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

    pub(crate) fn record_v8_state_update(
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

impl RendererInspectorSessionOutboundRoute {
    pub(crate) fn route_notification(
        &self,
        message: &Value,
    ) -> RendererInspectorPauseNotificationRoute {
        self.target
            .pause_ref()
            .route_notification(self.agent_token, &self.session, message)
    }

    pub(crate) fn mark_command_response(&self, call_id: i32, succeeded: bool) {
        self.target.pause_ref().mark_command_response(
            self.session.wire_session_id(),
            call_id,
            succeeded,
        );
    }

    pub(crate) fn detach_session(&self) {
        self.target
            .pause_ref()
            .detach_session(self.agent_token, &self.session);
        self.target.detach_session(self.agent_token, &self.session);
    }

    pub(crate) fn stage_pause_preface(
        &self,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Option<RendererInspectorPausePrefaceGuard> {
        self.target.pause_ref().stage_pause_preface(
            self.agent_token,
            self.session.clone(),
            messages,
        )
    }
}

pub(crate) struct RendererInspectorPauseCommandDispatchGuard {
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
mod tests;
