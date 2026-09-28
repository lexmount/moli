use super::RendererInspectorPauseBridge;
use super::causality::{
    RendererInspectorPauseCommandDispatch, RendererInspectorPauseCommandTransition,
    RendererInspectorPauseNotificationRoute, RendererInspectorPausePreface,
};
use super::loop_state::{
    RendererInspectorPauseExitReason, RendererInspectorPauseLoopPolicy, RendererInspectorPausePhase,
};
use crate::devtools::target::RendererDevToolsTargetHandle;
use crate::runtime::{
    PendingRendererOutputRecord, RendererInspectorIngressTicket,
    RendererInspectorPauseCommandEffect, RendererProtocolObservation,
    RendererRuntimeCommandCausalIdentity, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorMessageBatch,
};
use moli_inspector_json::InspectorJson;
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

pub(super) struct PauseState {
    next_preface_id: u64,
    pub(super) phase: RendererInspectorPausePhase,
    pause_loop_policy: RendererInspectorPauseLoopPolicy,
    quit_requested: bool,
    session_detach_arms: usize,
    terminate_replacement_pause: bool,
    replacement_pauses_awaiting_resumed: HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
    pub(super) pending_prefaces: VecDeque<RendererInspectorPausePreface>,
    pub(super) paused_sessions_awaiting_resumed:
        HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
    // V8 dispatches one nested-loop command synchronously. A successful
    // resume/step response is emitted before dispatch returns; only then does
    // V8 leave the loop and report resumed to every session. A following
    // pause is likewise reported to every session before the next loop starts,
    // so active and pending transition ownership are each singular.
    active_command_dispatch: Option<RendererInspectorPauseCommandDispatch>,
    pub(super) pending_command_transition: Option<RendererInspectorPauseCommandTransition>,
}

impl Default for PauseState {
    fn default() -> Self {
        Self {
            next_preface_id: 1,
            phase: RendererInspectorPausePhase::Running,
            pause_loop_policy: RendererInspectorPauseLoopPolicy::MainAndIo,
            quit_requested: false,
            session_detach_arms: 0,
            terminate_replacement_pause: false,
            replacement_pauses_awaiting_resumed: HashSet::new(),
            pending_prefaces: VecDeque::new(),
            paused_sessions_awaiting_resumed: HashSet::new(),
            active_command_dispatch: None,
            pending_command_transition: None,
        }
    }
}

impl std::fmt::Debug for PauseState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PauseState")
            .field("phase", &self.phase)
            .field("pause_loop_policy", &self.pause_loop_policy)
            .field("quit_requested", &self.quit_requested)
            .field("session_detach_arms", &self.session_detach_arms)
            .field("pending_prefaces", &self.pending_prefaces.len())
            .field(
                "paused_sessions_awaiting_resumed",
                &self.paused_sessions_awaiting_resumed.len(),
            )
            .field(
                "has_active_command_dispatch",
                &self.active_command_dispatch.is_some(),
            )
            .field(
                "has_pending_command_transition",
                &self.pending_command_transition.is_some(),
            )
            .finish()
    }
}

impl PauseState {
    pub(super) fn is_active(&self) -> bool {
        self.phase != RendererInspectorPausePhase::Running
    }

    pub(super) fn finish_owner_turn(&mut self) {
        self.pending_command_transition = None;
    }

    pub(super) fn close_target(&mut self) {
        self.quit_requested = true;
        self.pending_prefaces.clear();
        self.paused_sessions_awaiting_resumed.clear();
        self.replacement_pauses_awaiting_resumed.clear();
        self.finish_owner_turn();
    }

    pub(super) fn detach_page(&mut self) -> bool {
        self.pending_prefaces.clear();
        self.paused_sessions_awaiting_resumed.clear();
        self.replacement_pauses_awaiting_resumed.clear();
        self.finish_owner_turn();
        match self.phase {
            RendererInspectorPausePhase::Running => {
                self.terminate_replacement_pause = false;
                false
            }
            RendererInspectorPausePhase::Entering => {
                self.phase = RendererInspectorPausePhase::Running;
                self.pause_loop_policy = RendererInspectorPauseLoopPolicy::MainAndIo;
                self.quit_requested = false;
                self.terminate_replacement_pause = false;
                false
            }
            RendererInspectorPausePhase::Paused => {
                self.quit_requested = true;
                true
            }
        }
    }
}

#[must_use]
pub(crate) struct RendererInspectorPausePrefaceGuard {
    bridge: RendererInspectorPauseBridge,
    id: u64,
}

#[derive(Clone)]
pub(crate) struct RendererInspectorSessionOutboundRoute {
    target: RendererDevToolsTargetHandle,
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
}

pub(crate) struct RendererInspectorPauseCommandDispatchGuard {
    bridge: RendererInspectorPauseBridge,
    command_id: Option<u64>,
}

impl Drop for RendererInspectorPausePrefaceGuard {
    fn drop(&mut self) {
        self.bridge.cancel_pause_preface(self.id);
    }
}

impl RendererInspectorSessionOutboundRoute {
    pub(crate) fn route_notification(
        &self,
        message: &InspectorJson,
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

    pub(crate) fn deactivate_session(&self) {
        self.target
            .pause_ref()
            .detach_session(self.agent_token, &self.session);
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

impl Drop for RendererInspectorPauseCommandDispatchGuard {
    fn drop(&mut self) {
        if let Some(command_id) = self.command_id {
            self.bridge.finish_command_dispatch(command_id);
        }
    }
}

impl RendererInspectorPauseBridge {
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
        let awaiting_resumed = state.frontend.paused_sessions_awaiting_resumed.clone();
        assert!(
            state.frontend.active_command_dispatch.is_none(),
            "Inspector pause commands must dispatch serially in the nested loop"
        );
        state.frontend.active_command_dispatch = Some(RendererInspectorPauseCommandDispatch {
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
            .frontend
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
            state.frontend.pending_command_transition.is_none(),
            "one successful Inspector control transition must finish before the next nested-loop command"
        );
        state.frontend.pending_command_transition = Some(transition);
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
        if let Some(dispatch) = state.frontend.active_command_dispatch.as_mut()
            && matches(&dispatch.transition.causal_identity)
        {
            dispatch.transition.response_succeeded = succeeded;
        }
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
        let id = state.frontend.next_preface_id;
        state.frontend.next_preface_id = state
            .frontend
            .next_preface_id
            .checked_add(1)
            .expect("runtime inspector pause preface ID overflow");
        state
            .frontend
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
            .frontend
            .pending_prefaces
            .iter()
            .position(|preface| preface.id == id)
        {
            state.frontend.pending_prefaces.remove(position);
        }
    }

    pub(crate) fn arm_session_detach(&self) {
        let mut state = self.shared.state.lock();
        state.frontend.session_detach_arms = state
            .frontend
            .session_detach_arms
            .checked_add(1)
            .expect("runtime inspector session detach arm count overflow");
        if state.frontend.phase != RendererInspectorPausePhase::Running {
            self.shared.pause_loop_wake.notify_all();
        }
    }

    pub(crate) fn disarm_session_detach(&self) {
        let mut state = self.shared.state.lock();
        state.frontend.session_detach_arms = state
            .frontend
            .session_detach_arms
            .checked_sub(1)
            .expect("runtime inspector session detach arm count underflow");
    }

    pub(crate) fn enter_pause(&self) -> Option<RendererInspectorPauseLoopPolicy> {
        let mut state = self.shared.state.lock();
        if state.target_closed || state.frontend.phase != RendererInspectorPausePhase::Entering {
            return None;
        }
        state.frontend.phase = RendererInspectorPausePhase::Paused;
        Some(state.frontend.pause_loop_policy)
    }

    pub(crate) fn wait_for_pause_work<T>(
        &self,
        mut claim: impl FnMut() -> Option<T>,
    ) -> Result<T, RendererInspectorPauseExitReason> {
        let mut state = self.shared.state.lock();
        loop {
            if state.target_closed {
                return Err(RendererInspectorPauseExitReason::TargetClosed);
            }
            if (state.document_replacements != 0 && state.unload_callbacks == 0)
                || state.frontend.terminate_replacement_pause
            {
                return Err(RendererInspectorPauseExitReason::DocumentReplacement);
            }
            if state.frontend.session_detach_arms != 0 {
                return Err(RendererInspectorPauseExitReason::SessionDetached);
            }
            if state.frontend.quit_requested {
                return Err(RendererInspectorPauseExitReason::Resumed);
            }
            if let Some(work) = claim() {
                return Ok(work);
            }
            self.shared.pause_loop_wake.wait(&mut state);
        }
    }

    pub(crate) fn request_quit(&self) {
        let mut state = self.shared.state.lock();
        if state.frontend.phase != RendererInspectorPausePhase::Running {
            state.frontend.quit_requested = true;
            self.shared.pause_loop_wake.notify_all();
        }
    }

    pub(crate) fn leave_pause(&self) {
        let mut state = self.shared.state.lock();
        state.frontend.phase = RendererInspectorPausePhase::Running;
        state.frontend.pause_loop_policy = RendererInspectorPauseLoopPolicy::MainAndIo;
        state.frontend.quit_requested = false;
        state.frontend.terminate_replacement_pause = false;
        // Commands that lost the nested-loop race stay in their route-specific
        // ingress. Main retains its owner task; IO retains owner and interrupt
        // execution chances.
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
        message: &InspectorJson,
    ) -> RendererInspectorPauseNotificationRoute {
        let method = message.method();
        let is_paused_notification = method == Some("Debugger.paused");
        let is_resumed_notification = method == Some("Debugger.resumed");
        let session_route = (agent_token, session.clone());
        let mut state = self.shared.state.lock();
        if state.target_closed {
            return RendererInspectorPauseNotificationRoute::Drop;
        }
        if is_paused_notification
            && (state.route.is_none() || state.frontend.session_detach_arms != 0)
        {
            return RendererInspectorPauseNotificationRoute::Drop;
        }
        if is_resumed_notification
            && state
                .frontend
                .replacement_pauses_awaiting_resumed
                .remove(&session_route)
        {
            return RendererInspectorPauseNotificationRoute::Drop;
        }
        let replacement_pause = is_paused_notification
            && ((state.document_replacements != 0 && state.unload_callbacks == 0)
                || state.frontend.terminate_replacement_pause);
        let preface = if is_paused_notification {
            state
                .frontend
                .pending_prefaces
                .iter()
                .position(|preface| {
                    preface.agent_token == agent_token && preface.session == *session
                })
                .and_then(|position| state.frontend.pending_prefaces.remove(position))
                .map(|preface| preface.messages)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if is_paused_notification && !replacement_pause {
            state
                .frontend
                .paused_sessions_awaiting_resumed
                .insert(session_route.clone());
        }
        let resumes_reported_pause = is_resumed_notification
            && state
                .frontend
                .paused_sessions_awaiting_resumed
                .remove(&session_route);
        let (command_output, command_transition_complete) =
            if let Some(transition) = state.frontend.pending_command_transition.as_mut() {
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
            state.frontend.pending_command_transition = None;
        }
        if is_paused_notification {
            let is_instrumentation_pause = message
                .value()
                .get("params")
                .and_then(|params| params.get("reason"))
                .and_then(Value::as_str)
                == Some("instrumentation");
            if state.frontend.phase == RendererInspectorPausePhase::Running {
                state.frontend.phase = RendererInspectorPausePhase::Entering;
                state.frontend.pause_loop_policy = if is_instrumentation_pause {
                    RendererInspectorPauseLoopPolicy::IoOnly
                } else {
                    RendererInspectorPauseLoopPolicy::MainAndIo
                };
            } else if state.frontend.phase == RendererInspectorPausePhase::Entering
                && is_instrumentation_pause
            {
                // Multiple V8InspectorSessions observe the same isolate pause.
                // Any session identifying it as instrumentation tightens the
                // shared loop policy before V8 enters the client loop.
                state.frontend.pause_loop_policy = RendererInspectorPauseLoopPolicy::IoOnly;
            }
        }
        if replacement_pause {
            // The old document can pause again during commit. Publish its
            // preceding output and retain the decision to end the pause even
            // if the replacement is cancelled before V8 enters the loop.
            state.frontend.terminate_replacement_pause = true;
            state
                .frontend
                .replacement_pauses_awaiting_resumed
                .insert(session_route);
            return RendererInspectorPauseNotificationRoute::PublishPrefix { preface };
        }
        if state.frontend.phase == RendererInspectorPausePhase::Running && !resumes_reported_pause {
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
            .frontend
            .paused_sessions_awaiting_resumed
            .remove(&(agent_token, session.clone()));
        state
            .frontend
            .replacement_pauses_awaiting_resumed
            .remove(&(agent_token, session.clone()));
        state
            .frontend
            .pending_prefaces
            .retain(|preface| preface.agent_token != agent_token || &preface.session != session);
        let session_route = (agent_token, session.clone());
        if let Some(dispatch) = state.frontend.active_command_dispatch.as_mut() {
            dispatch.transition.awaiting_resumed.remove(&session_route);
            dispatch.transition.awaiting_repause.remove(&session_route);
        }
        if let Some(transition) = state.frontend.pending_command_transition.as_mut() {
            transition.awaiting_resumed.remove(&session_route);
            transition.awaiting_repause.remove(&session_route);
        }
        if state
            .frontend
            .pending_command_transition
            .as_ref()
            .is_some_and(RendererInspectorPauseCommandTransition::is_complete)
        {
            state.frontend.pending_command_transition = None;
        }
    }
}
