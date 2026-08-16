use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use parking_lot::Mutex;
use serde_json::json;

use crate::{
    render_runtime::RenderRuntimeHandle,
    runtime::{
        RendererCommandTurnOutput, RendererInspectorCommandEnvelope, RendererInspectorCommandRoute,
        RendererInspectorIngressTicket, RendererInspectorPauseCommandEffect, RendererOwnerReply,
        RendererPageStateCapturePolicy, RendererPageToken, RendererRuntimeInspectorResponseSender,
    },
    script_vm::{
        inspector_pause::RendererInspectorPauseLoopWake,
        inspector_route::RendererInspectorSessionExecutorRouteId,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorMainCommandConsumer {
    Owner,
    Pause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererInspectorMainCommandClaim {
    Owner,
    Inspector,
    Canceled,
}

pub enum RendererRuntimeInspectorMainCommandCompletion {
    Owner(Box<RendererCommandTurnOutput>),
    Inspector,
    Canceled,
}

pub(crate) struct RendererInspectorMainCommand {
    command_id: u64,
    page_token: RendererPageToken,
    pub(crate) agent_token: RendererDevToolsAgentToken,
    capture_policy: RendererPageStateCapturePolicy,
    envelope: RendererInspectorCommandEnvelope,
    claim_tx: Option<tokio::sync::oneshot::Sender<RendererInspectorMainCommandClaim>>,
    owner_reply_tx: Option<tokio::sync::oneshot::Sender<anyhow::Result<RendererOwnerReply>>>,
    claimed_by: Option<RendererInspectorMainCommandConsumer>,
}

impl RendererInspectorMainCommand {
    pub(crate) fn command_id(&self) -> u64 {
        self.command_id
    }

    pub(crate) fn ticket(&self) -> &RendererInspectorIngressTicket {
        self.envelope.ticket()
    }

    pub(crate) fn first_dispatch_lifecycle(
        &self,
    ) -> crate::runtime::RendererInspectorFirstDispatchLifecycle {
        self.envelope.first_dispatch_lifecycle()
    }

    #[cfg(test)]
    pub(crate) fn raw_json(&self) -> &str {
        self.envelope.main_protocol_raw_json()
    }

    pub(crate) fn response(&self) -> &RendererRuntimeInspectorResponseSender {
        self.envelope.main_protocol_response()
    }

    pub(crate) fn pause_effect(&self) -> RendererInspectorPauseCommandEffect {
        self.envelope.pause_effect()
    }

    pub(crate) fn into_protocol_parts(
        self,
    ) -> (
        RendererInspectorIngressTicket,
        String,
        RendererRuntimeInspectorResponseSender,
    ) {
        self.envelope.into_main_protocol_parts()
    }

    #[cfg(test)]
    fn claimed_by(&self) -> Option<RendererInspectorMainCommandConsumer> {
        self.claimed_by
    }
}

pub struct RendererRuntimeInspectorMainCommandRoute {
    command_id: u64,
    ticket: RendererInspectorIngressTicket,
    claim_rx: Option<tokio::sync::oneshot::Receiver<RendererInspectorMainCommandClaim>>,
    owner_reply_rx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<RendererOwnerReply>>>,
    ingress: RendererInspectorMainIngress,
}

impl RendererRuntimeInspectorMainCommandRoute {
    pub fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub async fn wait_for_completion(
        mut self,
    ) -> anyhow::Result<RendererRuntimeInspectorMainCommandCompletion> {
        let claim = self
            .claim_rx
            .take()
            .expect("runtime inspector Main command claim receiver should only be awaited once")
            .await
            .map_err(|_| anyhow::anyhow!("runtime inspector Main command claim channel closed"))?;
        match claim {
            RendererInspectorMainCommandClaim::Owner => {
                let reply = self
                    .owner_reply_rx
                    .take()
                    .expect("an owner-claimed Main command must retain its owner reply receiver")
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("runtime inspector Main owner reply channel closed")
                    })??;
                match reply {
                    RendererOwnerReply::AsyncPageCommandRan(output) => {
                        Ok(RendererRuntimeInspectorMainCommandCompletion::Owner(output))
                    }
                    _ => Err(anyhow::anyhow!(
                        "runtime inspector Main owner returned an unexpected renderer reply"
                    )),
                }
            }
            RendererInspectorMainCommandClaim::Inspector => {
                Ok(RendererRuntimeInspectorMainCommandCompletion::Inspector)
            }
            RendererInspectorMainCommandClaim::Canceled => {
                Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled)
            }
        }
    }
}

impl Drop for RendererRuntimeInspectorMainCommandRoute {
    fn drop(&mut self) {
        self.ingress.cancel_queued_command(
            self.command_id,
            "Runtime inspector Main route was canceled before dispatch",
        );
    }
}

#[derive(Clone)]
pub(crate) struct RendererInspectorMainIngress {
    shared: Arc<RendererInspectorMainShared>,
}

struct RendererInspectorMainShared {
    state: Mutex<RendererInspectorMainState>,
    owner_wake_armed: AtomicBool,
    route_id: RendererInspectorSessionExecutorRouteId,
    pause_wake: RendererInspectorPauseLoopWake,
}

#[derive(Clone)]
pub(crate) struct RendererInspectorMainOwnerWake {
    route_id: RendererInspectorSessionExecutorRouteId,
}

impl RendererInspectorMainOwnerWake {
    pub(crate) fn route_id(&self) -> RendererInspectorSessionExecutorRouteId {
        self.route_id
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RendererInspectorMainSessionLaneKey {
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
}

#[derive(Default)]
struct RendererInspectorMainSessionLane {
    active_command_id: Option<u64>,
    queued: VecDeque<RendererInspectorMainCommand>,
    ready: bool,
    detached: bool,
}

struct RendererInspectorMainState {
    sessions: BTreeMap<RendererInspectorMainSessionLaneKey, RendererInspectorMainSessionLane>,
    ready_sessions: VecDeque<RendererInspectorMainSessionLaneKey>,
    owner_runtime: Option<RenderRuntimeHandle>,
    closed: bool,
}

pub(crate) struct RendererInspectorMainFirstDispatchGuard {
    ingress: RendererInspectorMainIngress,
    active: Option<(RendererInspectorMainSessionLaneKey, u64)>,
}

pub(crate) struct RendererInspectorMainPostDispatchWakeGuard {
    ingress: Option<RendererInspectorMainIngress>,
}

pub(crate) struct RendererInspectorMainOwnerDispatch {
    page_token: RendererPageToken,
    capture_policy: RendererPageStateCapturePolicy,
    envelope: RendererInspectorCommandEnvelope,
    reply_tx: tokio::sync::oneshot::Sender<anyhow::Result<RendererOwnerReply>>,
}

impl RendererInspectorMainOwnerDispatch {
    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererPageToken,
        RendererPageStateCapturePolicy,
        RendererInspectorCommandEnvelope,
        tokio::sync::oneshot::Sender<anyhow::Result<RendererOwnerReply>>,
    ) {
        (
            self.page_token,
            self.capture_policy,
            self.envelope,
            self.reply_tx,
        )
    }
}

impl Drop for RendererInspectorMainFirstDispatchGuard {
    fn drop(&mut self) {
        self.release();
    }
}

impl RendererInspectorMainFirstDispatchGuard {
    pub(crate) fn release(&mut self) {
        if self.release_lane() {
            self.ingress.notify_execution_opportunities();
        }
    }

    pub(crate) fn release_for_dispatch(&mut self) -> RendererInspectorMainPostDispatchWakeGuard {
        let has_ready = self.release_lane();
        RendererInspectorMainPostDispatchWakeGuard {
            ingress: has_ready.then(|| self.ingress.clone()),
        }
    }

    fn release_lane(&mut self) -> bool {
        let Some((lane, command_id)) = self.active.take() else {
            return false;
        };
        self.ingress.finish_first_dispatch(lane, command_id)
    }
}

impl Drop for RendererInspectorMainPostDispatchWakeGuard {
    fn drop(&mut self) {
        if let Some(ingress) = self.ingress.take() {
            ingress.notify_execution_opportunities();
        }
    }
}

impl RendererInspectorMainIngress {
    pub(crate) fn new(
        route_id: RendererInspectorSessionExecutorRouteId,
        pause_wake: RendererInspectorPauseLoopWake,
    ) -> Self {
        Self {
            shared: Arc::new(RendererInspectorMainShared {
                state: Mutex::new(RendererInspectorMainState {
                    sessions: BTreeMap::new(),
                    ready_sessions: VecDeque::new(),
                    owner_runtime: None,
                    closed: false,
                }),
                owner_wake_armed: AtomicBool::new(false),
                route_id,
                pause_wake,
            }),
        }
    }

    pub(crate) fn configure_owner_wake(&self, owner_runtime: RenderRuntimeHandle) {
        let has_ready = {
            let mut state = self.shared.state.lock();
            state.owner_runtime = Some(owner_runtime);
            !state.ready_sessions.is_empty()
        };
        if has_ready {
            self.notify_execution_opportunities();
        }
    }

    pub(crate) fn enqueue_command(
        &self,
        page_token: RendererPageToken,
        agent_token: RendererDevToolsAgentToken,
        envelope: RendererInspectorCommandEnvelope,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        self.enqueue_with_policy(
            page_token,
            agent_token,
            envelope,
            RendererPageStateCapturePolicy::ProtocolTurn,
        )
    }

    pub(crate) fn enqueue_owner_command(
        &self,
        page_token: RendererPageToken,
        agent_token: RendererDevToolsAgentToken,
        envelope: RendererInspectorCommandEnvelope,
        capture_policy: RendererPageStateCapturePolicy,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        self.enqueue_with_policy(page_token, agent_token, envelope, capture_policy)
    }

    fn enqueue_with_policy(
        &self,
        page_token: RendererPageToken,
        agent_token: RendererDevToolsAgentToken,
        envelope: RendererInspectorCommandEnvelope,
        capture_policy: RendererPageStateCapturePolicy,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        assert_eq!(
            envelope.ticket().route(),
            RendererInspectorCommandRoute::MainThread,
            "only MainThread Inspector commands may enter RendererInspectorMainIngress"
        );
        let ticket = envelope.ticket().clone();
        let (claim_tx, claim_rx) = tokio::sync::oneshot::channel();
        let (owner_reply_tx, owner_reply_rx) = tokio::sync::oneshot::channel();
        let mut state = self.shared.state.lock();
        let command_id = ticket.sequence();
        let lane_key = RendererInspectorMainSessionLaneKey {
            agent_token,
            session: ticket.session().clone(),
        };
        let command = RendererInspectorMainCommand {
            command_id,
            page_token,
            agent_token,
            capture_policy,
            envelope,
            claim_tx: Some(claim_tx),
            owner_reply_tx: Some(owner_reply_tx),
            claimed_by: None,
        };
        if state.closed {
            drop(state);
            fail_main_command(command, "Inspector Main target is closed");
            return RendererRuntimeInspectorMainCommandRoute {
                command_id,
                ticket,
                claim_rx: Some(claim_rx),
                owner_reply_rx: Some(owner_reply_rx),
                ingress: self.clone(),
            };
        }
        let lane = state.sessions.entry(lane_key.clone()).or_default();
        if lane.detached {
            drop(state);
            fail_main_command(command, "Inspector Main session was detached");
            return RendererRuntimeInspectorMainCommandRoute {
                command_id,
                ticket,
                claim_rx: Some(claim_rx),
                owner_reply_rx: Some(owner_reply_rx),
                ingress: self.clone(),
            };
        }
        lane.queued.push_back(command);
        if lane.active_command_id.is_none() && !lane.ready {
            lane.ready = true;
            state.ready_sessions.push_back(lane_key);
        }
        drop(state);
        self.notify_execution_opportunities();
        RendererRuntimeInspectorMainCommandRoute {
            command_id,
            ticket,
            claim_rx: Some(claim_rx),
            owner_reply_rx: Some(owner_reply_rx),
            ingress: self.clone(),
        }
    }

    pub(crate) fn claim_for_owner(&self) -> Option<RendererInspectorMainCommand> {
        self.shared.owner_wake_armed.store(false, Ordering::Release);
        let command = self.claim_next(RendererInspectorMainCommandConsumer::Owner);
        if command.is_none() && !self.shared.state.lock().ready_sessions.is_empty() {
            self.notify_execution_opportunities();
        }
        command
    }

    pub(crate) fn claim_for_pause(&self) -> Option<RendererInspectorMainCommand> {
        self.claim_next(RendererInspectorMainCommandConsumer::Pause)
    }

    fn claim_next(
        &self,
        consumer: RendererInspectorMainCommandConsumer,
    ) -> Option<RendererInspectorMainCommand> {
        let mut state = self.shared.state.lock();
        if state.closed {
            return None;
        }
        let ready_session_count = state.ready_sessions.len();
        for _ in 0..ready_session_count {
            let lane_key = state
                .ready_sessions
                .pop_front()
                .expect("the snapshotted Main ready-session count must remain available");
            let pause_dispatch_blocked = consumer == RendererInspectorMainCommandConsumer::Pause
                && state
                    .sessions
                    .get(&lane_key)
                    .and_then(|lane| lane.queued.front())
                    .is_some_and(|command| {
                        !command
                            .envelope
                            .can_dispatch_at_nested_inspector_session_boundary()
                    });
            if pause_dispatch_blocked {
                state.ready_sessions.push_back(lane_key);
                continue;
            }
            let Some(lane) = state.sessions.get_mut(&lane_key) else {
                continue;
            };
            lane.ready = false;
            if lane.active_command_id.is_some() {
                continue;
            }
            let Some(mut command) = lane.queued.pop_front() else {
                continue;
            };
            lane.active_command_id = Some(command.command_id);
            command.claimed_by = Some(consumer);
            if let Some(claim_tx) = command.claim_tx.take() {
                let claim = match consumer {
                    RendererInspectorMainCommandConsumer::Owner => {
                        RendererInspectorMainCommandClaim::Owner
                    }
                    RendererInspectorMainCommandConsumer::Pause => {
                        RendererInspectorMainCommandClaim::Inspector
                    }
                };
                let _ = claim_tx.send(claim);
            }
            if consumer == RendererInspectorMainCommandConsumer::Pause {
                command.owner_reply_tx.take();
            }
            return Some(command);
        }
        None
    }

    pub(crate) fn first_dispatch_guard(
        &self,
        command: &RendererInspectorMainCommand,
    ) -> RendererInspectorMainFirstDispatchGuard {
        let lane_key = RendererInspectorMainSessionLaneKey {
            agent_token: command.agent_token,
            session: command.ticket().session().clone(),
        };
        let state = self.shared.state.lock();
        assert_eq!(
            command.first_dispatch_lifecycle(),
            crate::runtime::RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
        );
        assert_eq!(
            state
                .sessions
                .get(&lane_key)
                .and_then(|lane| lane.active_command_id),
            Some(command.command_id),
            "a claimed Inspector Main command must own its session lane"
        );
        drop(state);
        RendererInspectorMainFirstDispatchGuard {
            ingress: self.clone(),
            active: Some((lane_key, command.command_id)),
        }
    }

    pub(crate) fn prepare_owner_dispatch(
        &self,
        command: RendererInspectorMainCommand,
    ) -> RendererInspectorMainOwnerDispatch {
        assert_eq!(
            command.claimed_by,
            Some(RendererInspectorMainCommandConsumer::Owner),
            "only an owner-claimed Main command can enter a Page owner turn"
        );
        let first_dispatch = self.first_dispatch_guard(&command);
        let RendererInspectorMainCommand {
            page_token,
            capture_policy,
            mut envelope,
            owner_reply_tx,
            ..
        } = command;
        envelope.bind_main_ingress_first_dispatch(first_dispatch);
        RendererInspectorMainOwnerDispatch {
            page_token,
            capture_policy,
            envelope,
            reply_tx: owner_reply_tx
                .expect("an owner-claimed Main command must retain its owner reply sender"),
        }
    }

    fn finish_first_dispatch(
        &self,
        lane_key: RendererInspectorMainSessionLaneKey,
        command_id: u64,
    ) -> bool {
        let mut state = self.shared.state.lock();
        let (make_ready, remove_lane) = {
            let lane = state
                .sessions
                .get_mut(&lane_key)
                .expect("an active Inspector Main lane must still exist");
            assert_eq!(
                lane.active_command_id.take(),
                Some(command_id),
                "only the active Inspector Main command may release its lane"
            );
            let make_ready = !lane.detached && !lane.queued.is_empty() && !lane.ready;
            if make_ready {
                lane.ready = true;
            }
            (make_ready, lane.queued.is_empty())
        };
        if make_ready {
            state.ready_sessions.push_back(lane_key.clone());
        }
        if remove_lane {
            state.sessions.remove(&lane_key);
        }
        !state.ready_sessions.is_empty()
    }

    pub(crate) fn cancel_queued_command(&self, command_id: u64, message: &str) {
        let command = {
            let mut state = self.shared.state.lock();
            let lane_key = state.sessions.iter().find_map(|(key, lane)| {
                lane.queued
                    .iter()
                    .any(|command| command.command_id == command_id)
                    .then(|| key.clone())
            });
            let Some(lane_key) = lane_key else {
                return;
            };
            let lane = state
                .sessions
                .get_mut(&lane_key)
                .expect("located Inspector Main lane must remain present");
            let position = lane
                .queued
                .iter()
                .position(|command| command.command_id == command_id)
                .expect("located Inspector Main command must remain queued");
            let command = lane.queued.remove(position);
            if lane.queued.is_empty() && lane.active_command_id.is_none() {
                lane.ready = false;
                state.ready_sessions.retain(|ready| ready != &lane_key);
                state.sessions.remove(&lane_key);
            }
            command
        };
        if let Some(command) = command {
            fail_main_command(command, message);
        }
    }

    pub(crate) fn detach_session(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let commands = {
            let mut state = self.shared.state.lock();
            let lane_key = RendererInspectorMainSessionLaneKey {
                agent_token,
                session: session.clone(),
            };
            state.ready_sessions.retain(|ready| ready != &lane_key);
            let Some(_) = state.sessions.get(&lane_key) else {
                return;
            };
            let (commands, remove_lane) = {
                let lane = state
                    .sessions
                    .get_mut(&lane_key)
                    .expect("located Inspector Main lane must remain present");
                lane.ready = false;
                lane.detached = true;
                (
                    lane.queued.drain(..).collect::<Vec<_>>(),
                    lane.active_command_id.is_none(),
                )
            };
            if remove_lane {
                state.sessions.remove(&lane_key);
            }
            commands
        };
        for command in commands {
            fail_main_command(command, "Inspector Main session was detached");
        }
    }

    pub(crate) fn close(&self, message: &str) {
        let commands = {
            let mut state = self.shared.state.lock();
            state.closed = true;
            drain_queued_commands(&mut state)
        };
        self.shared.pause_wake.notify_all();
        for command in commands {
            fail_main_command(command, message);
        }
    }

    pub(crate) fn cancel_all_queued(&self, message: &str) {
        let commands = drain_queued_commands(&mut self.shared.state.lock());
        for command in commands {
            fail_main_command(command, message);
        }
    }

    fn notify_execution_opportunities(&self) {
        let owner_runtime = self.shared.state.lock().owner_runtime.clone();
        if let Some(owner_runtime) = owner_runtime
            && self
                .shared
                .owner_wake_armed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            && owner_runtime
                .enqueue_inspector_main_receiver_wake(RendererInspectorMainOwnerWake {
                    route_id: self.shared.route_id,
                })
                .is_err()
        {
            self.shared.owner_wake_armed.store(false, Ordering::Release);
            self.close("Inspector Main owner receiver was closed");
            return;
        }
        self.shared.pause_wake.notify_one();
    }
}

fn drain_queued_commands(
    state: &mut RendererInspectorMainState,
) -> Vec<RendererInspectorMainCommand> {
    state.ready_sessions.clear();
    let commands = state
        .sessions
        .values_mut()
        .flat_map(|lane| lane.queued.drain(..))
        .collect();
    state
        .sessions
        .retain(|_, lane| lane.active_command_id.is_some());
    commands
}

fn fail_main_command(command: RendererInspectorMainCommand, message: &str) {
    let RendererInspectorMainCommand {
        envelope,
        claim_tx,
        owner_reply_tx,
        ..
    } = command;
    if let Some(claim_tx) = claim_tx {
        let _ = claim_tx.send(RendererInspectorMainCommandClaim::Canceled);
    }
    if let Some(owner_reply_tx) = owner_reply_tx {
        let _ = owner_reply_tx.send(Err(anyhow::anyhow!(message.to_owned())));
    }
    if !envelope.is_main_protocol_command_with_deferred_response() {
        return;
    }
    let (_, _, response) = envelope.into_main_protocol_parts();
    let call_id = response.call_id();
    let _ = response.send(json!({
        "id": call_id,
        "error": {
            "code": -32000,
            "message": message,
        },
    }));
}

impl std::fmt::Debug for RendererInspectorMainIngress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        formatter
            .debug_struct("RendererInspectorMainIngress")
            .field("route_id", &self.shared.route_id)
            .field("session_lanes", &state.sessions.len())
            .field("ready_sessions", &state.ready_sessions.len())
            .field("closed", &state.closed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime::{RendererInspectorIngressTicket, RendererRuntimeInspectorAsyncCompletion},
        script_vm::inspector_pause::RendererInspectorPauseBridge,
    };

    fn ingress() -> RendererInspectorMainIngress {
        let pause_bridge = RendererInspectorPauseBridge::default();
        RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(1),
            pause_bridge.pause_loop_wake(),
        )
    }

    fn enqueue(
        ingress: &RendererInspectorMainIngress,
        agent_token: RendererDevToolsAgentToken,
        session: Option<&str>,
        raw_json: &str,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        enqueue_with_action(ingress, agent_token, session, None, raw_json)
    }

    fn enqueue_with_action(
        ingress: &RendererInspectorMainIngress,
        agent_token: RendererDevToolsAgentToken,
        session: Option<&str>,
        action: Option<&str>,
        raw_json: &str,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        let (response_tx, _response_rx) =
            tokio::sync::oneshot::channel::<RendererRuntimeInspectorAsyncCompletion>();
        ingress.enqueue_command(
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(1)),
            agent_token,
            RendererInspectorCommandEnvelope::new_main_protocol(
                RendererInspectorIngressTicket::new(
                    None,
                    session.map(str::to_owned),
                    RendererInspectorCommandRoute::MainThread,
                ),
                action.map(str::to_owned),
                raw_json.to_owned(),
                RendererRuntimeInspectorResponseSender::new(1, response_tx),
            ),
        )
    }

    #[test]
    fn owner_and_pause_can_claim_one_main_command_only_once() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _route = enqueue(
            &ingress,
            agent,
            Some("session-a"),
            r#"{"id":1,"method":"Runtime.getProperties","params":{"objectId":"first"}}"#,
        );

        let pause = ingress.claim_for_pause();
        let owner = ingress.claim_for_owner();
        assert_eq!(
            usize::from(pause.is_some()) + usize::from(owner.is_some()),
            1
        );
        assert_eq!(
            pause.and_then(|command| command.claimed_by()),
            Some(RendererInspectorMainCommandConsumer::Pause)
        );
    }

    #[test]
    fn main_ingress_is_fifo_per_session_and_independent_across_sessions() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _a1 = enqueue(
            &ingress,
            agent,
            Some("session-a"),
            r#"{"id":1,"method":"Runtime.getProperties","params":{"objectId":"a1"}}"#,
        );
        let _a2 = enqueue(
            &ingress,
            agent,
            Some("session-a"),
            r#"{"id":2,"method":"Runtime.getProperties","params":{"objectId":"a2"}}"#,
        );
        let _b1 = enqueue(
            &ingress,
            agent,
            Some("session-b"),
            r#"{"id":3,"method":"Runtime.getProperties","params":{"objectId":"b1"}}"#,
        );

        let first = ingress.claim_for_owner().expect("first ready Main session");
        assert!(first.raw_json().contains(r#""a1""#));
        let second = ingress
            .claim_for_pause()
            .expect("the other Main session remains independently ready");
        assert!(second.raw_json().contains(r#""b1""#));
        assert!(ingress.claim_for_owner().is_none());

        ingress.first_dispatch_guard(&first).release();
        let third = ingress.claim_for_pause().expect("a2 after a1 dispatch");
        assert!(third.raw_json().contains(r#""a2""#));
    }

    #[test]
    fn nested_main_accepts_runtime_evaluation_but_not_page_owner_context_rewrite() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _default_evaluate = enqueue_with_action(
            &ingress,
            agent,
            Some("session-default"),
            Some("evaluate"),
            r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"1 + 1"}}"#,
        );
        let nested = ingress
            .claim_for_pause()
            .expect("default-world Runtime.evaluate should be pumpable by nested Main");
        assert_eq!(
            nested.claimed_by(),
            Some(RendererInspectorMainCommandConsumer::Pause)
        );

        let _context_evaluate = enqueue_with_action(
            &ingress,
            agent,
            Some("session-context"),
            Some("evaluate"),
            r#"{"id":2,"method":"Runtime.evaluate","params":{"contextId":41,"expression":"2 + 2"}}"#,
        );
        assert!(
            ingress.claim_for_pause().is_none(),
            "context-id rewriting belongs to Page owner dispatch"
        );
        assert!(ingress.claim_for_owner().is_some());
    }

    #[test]
    fn owner_only_main_command_blocks_its_session_lane_from_pause_claim() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let crate::runtime::RendererPageCommand::Inspector(envelope) =
            crate::runtime::RendererPageCommand::runtime_enable_events(None)
        else {
            panic!("Runtime.enable events must be a Main Inspector envelope");
        };
        let _route = ingress.enqueue_owner_command(
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(1)),
            agent,
            envelope,
            RendererPageStateCapturePolicy::ProtocolTurn,
        );

        assert!(ingress.claim_for_pause().is_none());
        let owner = ingress
            .claim_for_owner()
            .expect("the ordinary owner receiver must claim owner-only Main work");
        assert_eq!(
            owner.claimed_by(),
            Some(RendererInspectorMainCommandConsumer::Owner)
        );
    }

    #[test]
    #[should_panic(
        expected = "only MainThread Inspector commands may enter RendererInspectorMainIngress"
    )]
    fn io_command_cannot_enter_main_ingress() {
        let ingress = ingress();
        let _route = ingress.enqueue_command(
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(1)),
            RendererDevToolsAgentToken::allocate(),
            RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io),
                r#"{"id":1,"method":"Runtime.terminateExecution"}"#.to_owned(),
                None,
            ),
        );
    }
}
