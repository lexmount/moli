use std::{
    collections::{BTreeMap, VecDeque},
    ffi::c_void,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use parking_lot::Mutex;
use serde_json::json;

use crate::{
    runtime::{
        RendererInspectorCommandEnvelope, RendererInspectorCommandRoute,
        RendererInspectorIngressTicket, RendererRuntimeInspectorResponseSender,
    },
    script_vm::{
        inspector_pause::RendererInspectorPauseLoopWake,
        inspector_route::RendererInspectorSessionExecutorRouteId,
    },
};

type RendererInspectorInterruptCallback =
    unsafe extern "C" fn(v8::UnsafeRawIsolatePtr, *mut c_void);

pub(crate) struct RendererInspectorInterruptTarget {
    route_id: RendererInspectorSessionExecutorRouteId,
}

impl RendererInspectorInterruptTarget {
    pub(crate) fn route_id(&self) -> RendererInspectorSessionExecutorRouteId {
        self.route_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorIoCommandConsumer {
    Owner,
    Interrupt,
    Pause,
}

pub(crate) struct RendererInspectorIoCommand {
    command_id: u64,
    pub(crate) agent_token: RendererDevToolsAgentToken,
    envelope: RendererInspectorCommandEnvelope,
    claim_tx: Option<tokio::sync::oneshot::Sender<RendererRuntimeInspectorCommandClaim>>,
    claimed_by: Option<RendererInspectorIoCommandConsumer>,
}

impl RendererInspectorIoCommand {
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

    pub(crate) fn raw_json(&self) -> &str {
        self.envelope.io_raw_json()
    }

    pub(crate) fn response(&self) -> Option<&RendererRuntimeInspectorResponseSender> {
        self.envelope.io_response()
    }

    pub(crate) fn take_response(&mut self) -> Option<RendererRuntimeInspectorResponseSender> {
        self.envelope.take_io_response()
    }

    #[cfg(test)]
    pub(crate) fn claimed_by(&self) -> Option<RendererInspectorIoCommandConsumer> {
        self.claimed_by
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererRuntimeInspectorCommandClaim {
    Inspector,
    Canceled,
}

pub struct RendererRuntimeInspectorCommandRoute {
    command_id: u64,
    ticket: RendererInspectorIngressTicket,
    claim_rx: Option<tokio::sync::oneshot::Receiver<RendererRuntimeInspectorCommandClaim>>,
    ingress: RendererInspectorIoIngress,
}

impl RendererRuntimeInspectorCommandRoute {
    pub fn command_id(&self) -> u64 {
        self.command_id
    }

    pub fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub async fn wait_for_claim(
        mut self,
    ) -> Result<RendererRuntimeInspectorCommandClaim, &'static str> {
        self.claim_rx
            .take()
            .expect("runtime inspector IO command claim receiver should only be awaited once")
            .await
            .map_err(|_| "runtime inspector IO command claim channel closed")
    }
}

impl Drop for RendererRuntimeInspectorCommandRoute {
    fn drop(&mut self) {
        self.ingress.cancel_queued_command(
            self.command_id,
            "Runtime inspector IO route was canceled before dispatch",
        );
    }
}

#[derive(Clone)]
pub(crate) struct RendererInspectorIoIngress {
    shared: Arc<RendererInspectorIoShared>,
}

struct RendererInspectorIoShared {
    state: Mutex<RendererInspectorIoState>,
    interrupt_armed: AtomicBool,
    interrupt_route: Option<RendererInspectorInterruptRoute>,
    pause_wake: RendererInspectorPauseLoopWake,
}

struct RendererInspectorInterruptRoute {
    isolate: v8::IsolateHandle,
    callback: RendererInspectorInterruptCallback,
    target: Arc<RendererInspectorInterruptTarget>,
}

#[derive(Clone)]
pub(crate) struct RendererInspectorIoOwnerWake {
    route_id: RendererInspectorSessionExecutorRouteId,
}

impl RendererInspectorIoOwnerWake {
    pub(crate) fn route_id(&self) -> RendererInspectorSessionExecutorRouteId {
        self.route_id
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RendererInspectorIoSessionLaneKey {
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
}

#[derive(Default)]
struct RendererInspectorIoSessionLane {
    active_command_id: Option<u64>,
    queued: VecDeque<RendererInspectorIoCommand>,
    ready: bool,
    detached: bool,
}

struct RendererInspectorIoState {
    next_command_id: u64,
    sessions: BTreeMap<RendererInspectorIoSessionLaneKey, RendererInspectorIoSessionLane>,
    ready_sessions: VecDeque<RendererInspectorIoSessionLaneKey>,
    owner_wake_tx: Option<tokio::sync::mpsc::UnboundedSender<RendererInspectorIoOwnerWake>>,
    closed: bool,
}

pub(crate) struct RendererInspectorIoFirstDispatchGuard {
    ingress: RendererInspectorIoIngress,
    active: Option<(RendererInspectorIoSessionLaneKey, u64)>,
    consumer: RendererInspectorIoCommandConsumer,
}

pub(crate) struct RendererInspectorIoPostDispatchWakeGuard {
    ingress: Option<RendererInspectorIoIngress>,
}

impl Drop for RendererInspectorIoFirstDispatchGuard {
    fn drop(&mut self) {
        self.release();
    }
}

impl RendererInspectorIoFirstDispatchGuard {
    pub(crate) fn release(&mut self) {
        let has_ready = self.release_lane();
        if has_ready {
            self.ingress.notify_execution_opportunities();
        }
    }

    /// Commits the first-dispatch lifecycle immediately before entering V8,
    /// but defers scheduling another callback until this dispatch returns. A
    /// nested pause entered by this dispatch still sees the advanced lane when
    /// it first polls the IO ingress.
    pub(crate) fn release_for_dispatch(&mut self) -> RendererInspectorIoPostDispatchWakeGuard {
        let has_ready = self.release_lane();
        RendererInspectorIoPostDispatchWakeGuard {
            ingress: has_ready.then(|| self.ingress.clone()),
        }
    }

    fn release_lane(&mut self) -> bool {
        let Some((lane, command_id)) = self.active.take() else {
            return false;
        };
        if self.consumer == RendererInspectorIoCommandConsumer::Interrupt {
            self.ingress
                .shared
                .interrupt_armed
                .store(false, Ordering::Release);
        }
        self.ingress.finish_first_dispatch(lane, command_id)
    }
}

impl Drop for RendererInspectorIoPostDispatchWakeGuard {
    fn drop(&mut self) {
        if let Some(ingress) = self.ingress.take() {
            ingress.notify_execution_opportunities();
        }
    }
}

impl RendererInspectorIoIngress {
    pub(crate) fn new(
        pause_wake: RendererInspectorPauseLoopWake,
        interrupt_route: Option<(
            v8::IsolateHandle,
            RendererInspectorInterruptCallback,
            RendererInspectorSessionExecutorRouteId,
        )>,
    ) -> Self {
        Self {
            shared: Arc::new(RendererInspectorIoShared {
                state: Mutex::new(RendererInspectorIoState {
                    next_command_id: 1,
                    sessions: BTreeMap::new(),
                    ready_sessions: VecDeque::new(),
                    owner_wake_tx: None,
                    closed: false,
                }),
                interrupt_armed: AtomicBool::new(false),
                interrupt_route: interrupt_route.map(|(isolate, callback, route_id)| {
                    RendererInspectorInterruptRoute {
                        isolate,
                        callback,
                        target: Arc::new(RendererInspectorInterruptTarget { route_id }),
                    }
                }),
                pause_wake,
            }),
        }
    }

    pub(crate) fn route_id(&self) -> Option<RendererInspectorSessionExecutorRouteId> {
        self.shared
            .interrupt_route
            .as_ref()
            .map(|route| route.target.route_id())
    }

    pub(crate) fn configure_owner_wake(
        &self,
        owner_wake_tx: tokio::sync::mpsc::UnboundedSender<RendererInspectorIoOwnerWake>,
    ) {
        let has_ready = {
            let mut state = self.shared.state.lock();
            state.owner_wake_tx = Some(owner_wake_tx);
            !state.ready_sessions.is_empty()
        };
        if has_ready {
            self.notify_execution_opportunities();
        }
    }

    pub(crate) fn enqueue_command(
        &self,
        agent_token: RendererDevToolsAgentToken,
        envelope: RendererInspectorCommandEnvelope,
    ) -> RendererRuntimeInspectorCommandRoute {
        assert_eq!(
            envelope.ticket().route(),
            RendererInspectorCommandRoute::Io,
            "only IO Inspector commands may enter RendererInspectorIoIngress"
        );
        let ticket = envelope.ticket().clone();
        let (claim_tx, claim_rx) = tokio::sync::oneshot::channel();
        let mut state = self.shared.state.lock();
        let command_id = state.next_command_id;
        state.next_command_id = state
            .next_command_id
            .checked_add(1)
            .expect("runtime inspector IO command ID overflow");
        let lane_key = RendererInspectorIoSessionLaneKey {
            agent_token,
            session: ticket.session().clone(),
        };
        let command = RendererInspectorIoCommand {
            command_id,
            agent_token,
            envelope,
            claim_tx: Some(claim_tx),
            claimed_by: None,
        };
        if state.closed {
            drop(state);
            fail_io_command(command, "Inspector IO target is closed");
            return RendererRuntimeInspectorCommandRoute {
                command_id,
                ticket,
                claim_rx: Some(claim_rx),
                ingress: self.clone(),
            };
        }
        let lane = state.sessions.entry(lane_key.clone()).or_default();
        if lane.detached {
            drop(state);
            fail_io_command(command, "Inspector IO session was detached");
            return RendererRuntimeInspectorCommandRoute {
                command_id,
                ticket,
                claim_rx: Some(claim_rx),
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
        RendererRuntimeInspectorCommandRoute {
            command_id,
            ticket,
            claim_rx: Some(claim_rx),
            ingress: self.clone(),
        }
    }

    pub(crate) fn claim_for_owner(&self) -> Option<RendererInspectorIoCommand> {
        self.claim_next(RendererInspectorIoCommandConsumer::Owner)
    }

    pub(crate) fn claim_for_interrupt(&self) -> Option<RendererInspectorIoCommand> {
        let command = self.claim_next(RendererInspectorIoCommandConsumer::Interrupt);
        if command.is_none() {
            self.shared.interrupt_armed.store(false, Ordering::Release);
            let has_ready = !self.shared.state.lock().ready_sessions.is_empty();
            if has_ready {
                self.request_interrupt();
            }
        }
        command
    }

    pub(crate) fn claim_for_pause(&self) -> Option<RendererInspectorIoCommand> {
        self.claim_next(RendererInspectorIoCommandConsumer::Pause)
    }

    #[cfg(test)]
    pub(crate) fn wait_and_claim_for_pause(
        &self,
        pause_bridge: &crate::script_vm::inspector_pause::RendererInspectorPauseBridge,
    ) -> Option<RendererInspectorIoCommand> {
        pause_bridge.wait_for_pause_work(|| self.claim_for_pause())
    }

    fn claim_next(
        &self,
        consumer: RendererInspectorIoCommandConsumer,
    ) -> Option<RendererInspectorIoCommand> {
        let mut state = self.shared.state.lock();
        if state.closed {
            return None;
        }
        while let Some(lane_key) = state.ready_sessions.pop_front() {
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
                let _ = claim_tx.send(RendererRuntimeInspectorCommandClaim::Inspector);
            }
            return Some(command);
        }
        None
    }

    pub(crate) fn first_dispatch_guard(
        &self,
        command: &RendererInspectorIoCommand,
    ) -> RendererInspectorIoFirstDispatchGuard {
        let lane_key = RendererInspectorIoSessionLaneKey {
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
            "a claimed Inspector IO command must own its session lane"
        );
        drop(state);
        RendererInspectorIoFirstDispatchGuard {
            ingress: self.clone(),
            active: Some((lane_key, command.command_id)),
            consumer: command
                .claimed_by
                .expect("a first-dispatch guard requires a claimed IO command"),
        }
    }

    fn finish_first_dispatch(
        &self,
        lane_key: RendererInspectorIoSessionLaneKey,
        command_id: u64,
    ) -> bool {
        {
            let mut state = self.shared.state.lock();
            let (make_ready, remove_lane) = {
                let lane = state
                    .sessions
                    .get_mut(&lane_key)
                    .expect("an active Inspector IO lane must still exist");
                assert_eq!(
                    lane.active_command_id.take(),
                    Some(command_id),
                    "only the active Inspector IO command may release its lane"
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
                .expect("located Inspector IO lane must remain present");
            let position = lane
                .queued
                .iter()
                .position(|command| command.command_id == command_id)
                .expect("located Inspector IO command must remain queued");
            let command = lane.queued.remove(position);
            if lane.queued.is_empty() && lane.active_command_id.is_none() {
                lane.ready = false;
                state.ready_sessions.retain(|ready| ready != &lane_key);
                state.sessions.remove(&lane_key);
            }
            command
        };
        if let Some(command) = command {
            fail_io_command(command, message);
        }
    }

    pub(crate) fn detach_session(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let commands = {
            let mut state = self.shared.state.lock();
            let lane_key = RendererInspectorIoSessionLaneKey {
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
                    .expect("located Inspector IO lane must remain present");
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
            fail_io_command(command, "Inspector IO session was detached");
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
            fail_io_command(command, message);
        }
    }

    pub(crate) fn cancel_all_queued(&self, message: &str) {
        let commands = drain_queued_commands(&mut self.shared.state.lock());
        for command in commands {
            fail_io_command(command, message);
        }
    }

    fn notify_execution_opportunities(&self) {
        let owner_wake = {
            let state = self.shared.state.lock();
            state.owner_wake_tx.clone().zip(self.route_id())
        };
        if let Some((owner_wake_tx, route_id)) = owner_wake {
            let _ = owner_wake_tx.send(RendererInspectorIoOwnerWake { route_id });
        }
        self.request_interrupt();
        self.shared.pause_wake.notify_one();
    }

    fn request_interrupt(&self) {
        let Some(route) = self.shared.interrupt_route.as_ref() else {
            return;
        };
        if self
            .shared
            .interrupt_armed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // Match Chromium's InspectorTaskRunner lifetime protocol: every V8
        // interrupt owns one strong callback target until V8 invokes it. A
        // late callback after executor teardown can therefore safely observe
        // that its TLS route disappeared without dereferencing stale state.
        let callback_target = Arc::into_raw(Arc::clone(&route.target));
        let callback_data = callback_target.cast_mut().cast::<c_void>();
        if !route
            .isolate
            .request_interrupt(route.callback, callback_data)
        {
            // SAFETY: `callback_target` came from `Arc::into_raw` immediately
            // above, and V8 rejected the request, so no callback can consume
            // this one strong reference.
            unsafe { drop(Arc::from_raw(callback_target)) };
            self.shared.interrupt_armed.store(false, Ordering::Release);
        }
    }
}

fn drain_queued_commands(state: &mut RendererInspectorIoState) -> Vec<RendererInspectorIoCommand> {
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

impl std::fmt::Debug for RendererInspectorIoIngress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        formatter
            .debug_struct("RendererInspectorIoIngress")
            .field("route_id", &self.route_id())
            .field("session_lanes", &state.sessions.len())
            .field("ready_sessions", &state.ready_sessions.len())
            .field(
                "interrupt_armed",
                &self.shared.interrupt_armed.load(Ordering::Acquire),
            )
            .field("closed", &state.closed)
            .finish()
    }
}

fn fail_io_command(mut command: RendererInspectorIoCommand, message: &str) {
    if let Some(claim_tx) = command.claim_tx.take() {
        let _ = claim_tx.send(RendererRuntimeInspectorCommandClaim::Canceled);
    }
    let Some(response) = command.take_response() else {
        return;
    };
    let call_id = response.call_id();
    let _ = response.send(json!({
        "id": call_id,
        "error": {
            "code": -32000,
            "message": message,
        },
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime::RendererInspectorIngressTicket,
        script_vm::inspector_pause::RendererInspectorPauseBridge,
    };

    fn ingress() -> RendererInspectorIoIngress {
        let pause_bridge = RendererInspectorPauseBridge::default();
        RendererInspectorIoIngress::new(pause_bridge.pause_loop_wake(), None)
    }

    fn enqueue(
        ingress: &RendererInspectorIoIngress,
        agent_token: RendererDevToolsAgentToken,
        session: Option<&str>,
        raw_json: &str,
    ) -> RendererRuntimeInspectorCommandRoute {
        ingress.enqueue_command(
            agent_token,
            RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    None,
                    session.map(str::to_owned),
                    RendererInspectorCommandRoute::Io,
                ),
                raw_json.to_owned(),
                None,
            ),
        )
    }

    #[test]
    fn owner_interrupt_and_pause_race_can_claim_one_command_only_once() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _route = enqueue(&ingress, agent, Some("session-a"), "first");

        let owner = ingress.claim_for_owner();
        let interrupt = ingress.claim_for_interrupt();
        let pause = ingress.claim_for_pause();
        assert_eq!(
            usize::from(owner.is_some())
                + usize::from(interrupt.is_some())
                + usize::from(pause.is_some()),
            1
        );
        assert_eq!(
            owner.and_then(|command| command.claimed_by()),
            Some(RendererInspectorIoCommandConsumer::Owner)
        );
    }

    #[test]
    fn one_session_is_fifo_while_another_session_is_independent() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _a1 = enqueue(&ingress, agent, Some("session-a"), "a1");
        let _a2 = enqueue(&ingress, agent, Some("session-a"), "a2");
        let _b1 = enqueue(&ingress, agent, Some("session-b"), "b1");

        let first = ingress.claim_for_owner().expect("first ready session");
        assert_eq!(first.raw_json(), "a1");
        let second = ingress
            .claim_for_interrupt()
            .expect("other session must remain independently ready");
        assert_eq!(second.raw_json(), "b1");
        assert!(ingress.claim_for_pause().is_none());

        ingress.first_dispatch_guard(&first).release();
        let third = ingress.claim_for_pause().expect("a2 after a1 dispatch");
        assert_eq!(third.raw_json(), "a2");
    }

    #[tokio::test]
    async fn detach_cancels_the_queue_while_an_active_first_dispatch_guard_finishes_safely() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let first_route = enqueue(&ingress, agent, Some("session-a"), "a1");
        let second_route = enqueue(&ingress, agent, Some("session-a"), "a2");

        let first = ingress
            .claim_for_interrupt()
            .expect("the session head should be claimable");
        let mut first_dispatch = ingress.first_dispatch_guard(&first);
        assert_eq!(
            first_route.wait_for_claim().await,
            Ok(RendererRuntimeInspectorCommandClaim::Inspector)
        );

        ingress.detach_session(agent, &DevToolsSessionKey::Attached("session-a".to_owned()));
        assert_eq!(
            second_route.wait_for_claim().await,
            Ok(RendererRuntimeInspectorCommandClaim::Canceled),
            "detach must cancel commands that have not been claimed"
        );

        first_dispatch.release();
        assert!(
            ingress.shared.state.lock().sessions.is_empty(),
            "the detached lane must retire after its active first dispatch releases"
        );
    }

    #[test]
    #[should_panic(expected = "only IO Inspector commands")]
    fn main_thread_command_cannot_enter_io_ingress() {
        let ingress = ingress();
        let page_command = crate::runtime::RendererPageCommand::dispatch_runtime_protocol_message(
            Some("session-a".to_owned()),
            RendererInspectorCommandRoute::MainThread,
            "main".to_owned(),
        );
        let crate::runtime::RendererPageCommand::Inspector(envelope) = page_command else {
            panic!("runtime protocol message must use an Inspector envelope");
        };
        let _ = ingress.enqueue_command(RendererDevToolsAgentToken::allocate(), envelope);
    }
}
