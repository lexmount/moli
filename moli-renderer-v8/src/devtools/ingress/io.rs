use std::{
    collections::{BTreeSet, VecDeque},
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
    devtools::{
        ingress::lane::RendererDevToolsSessionLaneKey, pause::RendererInspectorPauseLoopWake,
        route::RendererInspectorSessionExecutorRouteId,
    },
    runtime::{
        RendererDevToolsIoCommandEnvelope, RendererDevToolsIoCommandKind,
        RendererDevToolsIoCommandPayload, RendererInspectorCommandEnvelope,
        RendererInspectorCommandRoute, RendererInspectorIngressTicket,
        RendererInspectorPauseCommandEffect, RendererRuntimeInspectorResponseSender,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererRuntimeInspectorIoCommandClaim {
    Dispatched,
    Canceled,
}

type RendererInspectorIoFirstDispatchSender =
    tokio::sync::oneshot::Sender<RendererRuntimeInspectorIoCommandClaim>;
type RendererInspectorIoFirstDispatchReceiver =
    tokio::sync::oneshot::Receiver<RendererRuntimeInspectorIoCommandClaim>;

pub(crate) struct RendererInspectorIoCommand {
    command_id: u64,
    pub(crate) agent_token: RendererDevToolsAgentToken,
    envelope: RendererDevToolsIoCommandEnvelope,
    first_dispatch_tx: Option<RendererInspectorIoFirstDispatchSender>,
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

    pub(crate) fn kind(&self) -> RendererDevToolsIoCommandKind {
        self.envelope.kind()
    }

    pub(crate) fn raw_json(&self) -> &str {
        self.envelope
            .inspector_envelope()
            .expect("only an Inspector IO payload has protocol JSON")
            .io_raw_json()
    }

    pub(crate) fn response(&self) -> Option<&RendererRuntimeInspectorResponseSender> {
        self.envelope
            .inspector_envelope()
            .and_then(RendererInspectorCommandEnvelope::io_response)
    }

    pub(crate) fn response_delivery(&self) -> moli_page_types::RendererInspectorResponseDelivery {
        self.envelope
            .inspector_envelope()
            .expect("only an Inspector IO payload has a response delivery")
            .io_response_delivery()
    }

    pub(crate) fn pause_effect(&self) -> RendererInspectorPauseCommandEffect {
        self.envelope
            .inspector_envelope()
            .map_or(RendererInspectorPauseCommandEffect::None, |envelope| {
                envelope.pause_effect()
            })
    }

    pub(crate) fn take_response(&mut self) -> Option<RendererRuntimeInspectorResponseSender> {
        self.envelope
            .inspector_envelope_mut()
            .and_then(RendererInspectorCommandEnvelope::take_io_response)
    }

    pub(crate) fn into_payload(self) -> RendererDevToolsIoCommandPayload {
        self.envelope.into_payload()
    }

    #[cfg(test)]
    pub(crate) fn claimed_by(&self) -> Option<RendererInspectorIoCommandConsumer> {
        self.claimed_by
    }
}

pub struct RendererRuntimeInspectorIoCommandRoute {
    command_id: u64,
    ticket: RendererInspectorIngressTicket,
    first_dispatch_rx: Option<RendererInspectorIoFirstDispatchReceiver>,
    ingress: RendererInspectorIoIngress,
}

impl RendererRuntimeInspectorIoCommandRoute {
    pub fn command_id(&self) -> u64 {
        self.command_id
    }

    pub fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub async fn wait_for_first_dispatch(
        mut self,
    ) -> Result<RendererRuntimeInspectorIoCommandClaim, &'static str> {
        self.first_dispatch_rx
            .take()
            .expect("runtime Inspector IO first dispatch should only be awaited once")
            .await
            .map_err(|_| "runtime Inspector IO first-dispatch channel closed")
    }
}

impl Drop for RendererRuntimeInspectorIoCommandRoute {
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
    owner_wake_armed: AtomicBool,
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

struct RendererInspectorIoState {
    commands: VecDeque<RendererInspectorIoCommand>,
    active_command_id: Option<u64>,
    detached_sessions: BTreeSet<RendererDevToolsSessionLaneKey>,
    closed: bool,
    owner_wake_tx: Option<tokio::sync::mpsc::UnboundedSender<RendererInspectorIoOwnerWake>>,
}

impl RendererInspectorIoState {
    fn has_ready(&self) -> bool {
        !self.closed && self.active_command_id.is_none() && !self.commands.is_empty()
    }

    fn drain_commands(
        &mut self,
        mut should_drain: impl FnMut(&RendererInspectorIoCommand) -> bool,
    ) -> Vec<RendererInspectorIoCommand> {
        let mut retained = VecDeque::with_capacity(self.commands.len());
        let mut drained = Vec::new();
        while let Some(command) = self.commands.pop_front() {
            if should_drain(&command) {
                drained.push(command);
            } else {
                retained.push_back(command);
            }
        }
        self.commands = retained;
        drained
    }
}

pub(crate) struct RendererInspectorIoFirstDispatchGuard {
    ingress: RendererInspectorIoIngress,
    active_command_id: Option<u64>,
    consumer: RendererInspectorIoCommandConsumer,
    first_dispatch_tx: Option<RendererInspectorIoFirstDispatchSender>,
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
        let has_ready = self.release_task();
        if has_ready {
            self.ingress.notify_execution_opportunities();
        }
    }

    /// Releases the receiver slot and publishes first-dispatch immediately
    /// before entering V8, but keeps the next execution wake behind the return
    /// from this dispatch. V8 may enter a nested debugger loop before the call
    /// returns, so the command's ingress lifecycle must already be settled.
    pub(crate) fn release_for_dispatch(&mut self) -> RendererInspectorIoPostDispatchWakeGuard {
        let has_ready = self.release_task();
        RendererInspectorIoPostDispatchWakeGuard {
            ingress: has_ready.then(|| self.ingress.clone()),
        }
    }

    fn release_task(&mut self) -> bool {
        let Some(command_id) = self.active_command_id.take() else {
            return false;
        };
        if self.consumer == RendererInspectorIoCommandConsumer::Interrupt {
            self.ingress
                .shared
                .interrupt_armed
                .store(false, Ordering::Release);
        }
        let has_ready = self.ingress.finish_first_dispatch(command_id);
        if let Some(first_dispatch_tx) = self.first_dispatch_tx.take() {
            let _ = first_dispatch_tx.send(RendererRuntimeInspectorIoCommandClaim::Dispatched);
        }
        has_ready
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
                    commands: VecDeque::new(),
                    active_command_id: None,
                    detached_sessions: BTreeSet::new(),
                    closed: false,
                    owner_wake_tx: None,
                }),
                interrupt_armed: AtomicBool::new(false),
                owner_wake_armed: AtomicBool::new(false),
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

    /// Breaks an active V8 call so target teardown can reach the Page owner.
    ///
    /// Closing the ingress prevents queued IO work from being claimed, but
    /// the owner may still be inside non-yielding JavaScript. Target close
    /// owns this isolate's lifetime, so teardown can terminate that execution
    /// directly instead of depending on another DevTools command.
    pub(crate) fn terminate_execution_for_target_close(&self) -> bool {
        self.shared
            .interrupt_route
            .as_ref()
            .is_some_and(|route| route.isolate.terminate_execution())
    }

    pub(crate) fn configure_owner_wake(
        &self,
        owner_wake_tx: tokio::sync::mpsc::UnboundedSender<RendererInspectorIoOwnerWake>,
    ) {
        let has_ready = {
            let mut state = self.shared.state.lock();
            state.owner_wake_tx = Some(owner_wake_tx);
            state.has_ready()
        };
        if has_ready {
            self.notify_execution_opportunities();
        }
    }

    pub(crate) fn enqueue_command(
        &self,
        agent_token: RendererDevToolsAgentToken,
        envelope: RendererDevToolsIoCommandEnvelope,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        assert_eq!(
            envelope.ticket().route(),
            RendererInspectorCommandRoute::Io,
            "only IO DevTools commands may enter RendererInspectorIoIngress"
        );
        let lane_key =
            RendererDevToolsSessionLaneKey::new(agent_token, envelope.ticket().session().clone());
        let mut state = self.shared.state.lock();
        let (first_dispatch_tx, first_dispatch_rx) = tokio::sync::oneshot::channel();
        let ticket = envelope.ticket().clone();
        let command_id = ticket.sequence();
        let command = RendererInspectorIoCommand {
            command_id,
            agent_token,
            envelope,
            first_dispatch_tx: Some(first_dispatch_tx),
            claimed_by: None,
        };
        let rejected = if state.closed {
            Some((command, "Inspector IO target is closed"))
        } else if state.detached_sessions.contains(&lane_key) {
            Some((command, "Inspector IO session was detached"))
        } else {
            state.commands.push_back(command);
            None
        };
        drop(state);
        if let Some((command, message)) = rejected {
            fail_io_command(command, message);
        } else {
            self.notify_execution_opportunities();
        }
        RendererRuntimeInspectorIoCommandRoute {
            command_id,
            ticket,
            first_dispatch_rx: Some(first_dispatch_rx),
            ingress: self.clone(),
        }
    }

    pub(crate) fn claim_for_owner(&self) -> Option<RendererInspectorIoCommand> {
        self.shared.owner_wake_armed.store(false, Ordering::Release);
        let command = self.claim_next(RendererInspectorIoCommandConsumer::Owner);
        if command.is_none() && self.shared.state.lock().has_ready() {
            self.notify_execution_opportunities();
        }
        command
    }

    pub(crate) fn claim_for_interrupt(&self) -> Option<RendererInspectorIoCommand> {
        let command = self.claim_next(RendererInspectorIoCommandConsumer::Interrupt);
        if command.is_none() {
            self.shared.interrupt_armed.store(false, Ordering::Release);
            let has_ready = self.shared.state.lock().has_ready();
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
        pause_bridge: &crate::devtools::pause::RendererInspectorPauseBridge,
    ) -> Option<RendererInspectorIoCommand> {
        pause_bridge.wait_for_pause_work(|| self.claim_for_pause())
    }

    fn claim_next(
        &self,
        consumer: RendererInspectorIoCommandConsumer,
    ) -> Option<RendererInspectorIoCommand> {
        let mut state = self.shared.state.lock();
        if !state.has_ready() {
            return None;
        }
        let mut command = state
            .commands
            .pop_front()
            .expect("a ready Inspector task runner must have a command");
        state.active_command_id = Some(command.command_id);
        command.claimed_by = Some(consumer);
        Some(command)
    }

    pub(crate) fn first_dispatch_guard(
        &self,
        command: &mut RendererInspectorIoCommand,
    ) -> RendererInspectorIoFirstDispatchGuard {
        let state = self.shared.state.lock();
        assert_eq!(
            command.first_dispatch_lifecycle(),
            crate::runtime::RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
        );
        assert_eq!(
            state.active_command_id,
            Some(command.command_id),
            "a claimed Inspector IO command must own the target task runner",
        );
        drop(state);
        RendererInspectorIoFirstDispatchGuard {
            ingress: self.clone(),
            active_command_id: Some(command.command_id),
            consumer: command
                .claimed_by
                .expect("a first-dispatch guard requires a claimed IO command"),
            first_dispatch_tx: command.first_dispatch_tx.take(),
        }
    }

    fn finish_first_dispatch(&self, command_id: u64) -> bool {
        let mut state = self.shared.state.lock();
        assert_eq!(
            state.active_command_id.take(),
            Some(command_id),
            "only the active Inspector IO command may release its target task runner"
        );
        state.has_ready()
    }

    pub(crate) fn cancel_queued_command(&self, command_id: u64, message: &str) {
        let command = {
            let mut state = self.shared.state.lock();
            state
                .commands
                .iter()
                .position(|command| command.command_id == command_id)
                .and_then(|position| state.commands.remove(position))
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
        let lane_key = RendererDevToolsSessionLaneKey::new(agent_token, session.clone());
        let commands = {
            let mut state = self.shared.state.lock();
            state.detached_sessions.insert(lane_key);
            state.drain_commands(|command| {
                command.agent_token == agent_token && command.ticket().session() == session
            })
        };
        for command in commands {
            fail_io_command(command, "Inspector IO session was detached");
        }
    }

    pub(crate) fn close(&self, message: &str) {
        let commands = {
            let mut state = self.shared.state.lock();
            state.closed = true;
            state.commands.drain(..).collect::<Vec<_>>()
        };
        self.shared.pause_wake.notify_all();
        for command in commands {
            fail_io_command(command, message);
        }
    }

    pub(crate) fn cancel_all_queued(&self, message: &str) {
        let commands = self
            .shared
            .state
            .lock()
            .commands
            .drain(..)
            .collect::<Vec<_>>();
        for command in commands {
            fail_io_command(command, message);
        }
    }

    fn notify_execution_opportunities(&self) {
        let owner_wake = {
            let state = self.shared.state.lock();
            state
                .has_ready()
                .then(|| state.owner_wake_tx.clone().zip(self.route_id()))
                .flatten()
        };
        if let Some((owner_wake_tx, route_id)) = owner_wake
            && self
                .shared
                .owner_wake_armed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            && owner_wake_tx
                .send(RendererInspectorIoOwnerWake { route_id })
                .is_err()
        {
            self.shared.owner_wake_armed.store(false, Ordering::Release);
        }
        if self.shared.state.lock().has_ready() {
            self.request_interrupt();
            self.shared.pause_wake.notify_one();
        }
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

impl std::fmt::Debug for RendererInspectorIoIngress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        formatter
            .debug_struct("RendererInspectorIoIngress")
            .field("route_id", &self.route_id())
            .field("queued_tasks", &state.commands.len())
            .field("active_command_id", &state.active_command_id)
            .field(
                "interrupt_armed",
                &self.shared.interrupt_armed.load(Ordering::Acquire),
            )
            .field("closed", &state.closed)
            .finish()
    }
}

fn fail_io_command(mut command: RendererInspectorIoCommand, message: &str) {
    if let Some(first_dispatch_tx) = command.first_dispatch_tx.take() {
        let _ = first_dispatch_tx.send(RendererRuntimeInspectorIoCommandClaim::Canceled);
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
        devtools::pause::RendererInspectorPauseBridge,
        runtime::{RendererInspectorCommandEnvelope, RendererInspectorIngressTicket},
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
    ) -> RendererRuntimeInspectorIoCommandRoute {
        ingress.enqueue_command(
            agent_token,
            RendererDevToolsIoCommandEnvelope::inspector(RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    None,
                    session.map(str::to_owned),
                    RendererInspectorCommandRoute::Io,
                ),
                raw_json.to_owned(),
                None,
                moli_page_types::RendererInspectorResponseDelivery::CommandReply,
            )),
        )
    }

    fn io_ticket(session: &str) -> RendererInspectorIngressTicket {
        RendererInspectorIngressTicket::new(
            None,
            Some(session.to_owned()),
            RendererInspectorCommandRoute::Io,
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
    fn concurrent_owner_interrupt_and_pause_claim_exactly_once_under_stress() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();

        for round in 0..128 {
            let route = enqueue(
                &ingress,
                agent,
                Some("session-race"),
                &format!("command-{round}"),
            );
            let barrier = Arc::new(std::sync::Barrier::new(4));
            let (owner, interrupt, pause) = std::thread::scope(|scope| {
                let owner_ingress = ingress.clone();
                let owner_barrier = Arc::clone(&barrier);
                let owner = scope.spawn(move || {
                    owner_barrier.wait();
                    owner_ingress.claim_for_owner()
                });
                let interrupt_ingress = ingress.clone();
                let interrupt_barrier = Arc::clone(&barrier);
                let interrupt = scope.spawn(move || {
                    interrupt_barrier.wait();
                    interrupt_ingress.claim_for_interrupt()
                });
                let pause_ingress = ingress.clone();
                let pause_barrier = Arc::clone(&barrier);
                let pause = scope.spawn(move || {
                    pause_barrier.wait();
                    pause_ingress.claim_for_pause()
                });
                barrier.wait();
                (
                    owner.join().expect("owner claimant thread"),
                    interrupt.join().expect("interrupt claimant thread"),
                    pause.join().expect("pause claimant thread"),
                )
            });
            let mut claimed = [owner, interrupt, pause]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            assert_eq!(
                claimed.len(),
                1,
                "round {round} must have one successful consumer"
            );
            let mut command = claimed.pop().expect("exactly one claimed command");
            assert_eq!(command.raw_json(), format!("command-{round}"));
            ingress.first_dispatch_guard(&mut command).release();
            drop(route);
        }

        assert!(
            {
                let state = ingress.shared.state.lock();
                state.commands.is_empty() && state.active_command_id.is_none()
            },
            "every stressed target task must retire"
        );
    }

    #[test]
    fn page_io_uses_one_target_fifo_across_sessions() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _a1 = enqueue(&ingress, agent, Some("session-a"), "a1");
        let _a2 = enqueue(&ingress, agent, Some("session-a"), "a2");
        let _b1 = enqueue(&ingress, agent, Some("session-b"), "b1");

        let mut first = ingress.claim_for_owner().expect("first target task");
        assert_eq!(first.raw_json(), "a1");
        assert!(
            ingress.claim_for_interrupt().is_none(),
            "only one target task may be active before first dispatch"
        );

        ingress.first_dispatch_guard(&mut first).release();
        let mut second = ingress
            .claim_for_interrupt()
            .expect("the second target task must follow first dispatch");
        assert_eq!(second.raw_json(), "a2");
        assert!(ingress.claim_for_pause().is_none());
        ingress.first_dispatch_guard(&mut second).release();
        let mut third = ingress
            .claim_for_pause()
            .expect("the third target task must follow second dispatch");
        assert_eq!(third.raw_json(), "b1");
        ingress.first_dispatch_guard(&mut third).release();
    }

    #[tokio::test]
    async fn replacement_io_ingress_does_not_wait_for_an_old_first_dispatch_receiver() {
        let agent = RendererDevToolsAgentToken::allocate();
        let first_attachment = ingress();
        let second_attachment = ingress();

        let first = enqueue(&first_attachment, agent, Some("session-a"), "first");
        let second = enqueue(&second_attachment, agent, Some("session-a"), "second");

        let mut first_command = first_attachment
            .claim_for_owner()
            .expect("first attachment command");
        first_attachment
            .first_dispatch_guard(&mut first_command)
            .release();
        let mut second_command = second_attachment
            .claim_for_owner()
            .expect("replacement attachment command");
        second_attachment
            .first_dispatch_guard(&mut second_command)
            .release();

        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                second.wait_for_first_dispatch()
            )
            .await
            .expect("a replacement capability must not wait for the old receiver"),
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );
        drop(first);
    }

    #[tokio::test]
    async fn target_fifo_orders_inspector_performance_and_emulation_first_dispatch() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let inspector = enqueue(&ingress, agent, Some("session-mixed"), "inspector");
        let performance = ingress.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(io_ticket("session-mixed")),
        );
        let emulation = ingress.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::set_script_execution_disabled(
                io_ticket("session-mixed"),
                crate::script_execution_control::RendererScriptExecutionControl::default(),
                true,
            ),
        );

        let mut first = ingress
            .claim_for_interrupt()
            .expect("Inspector must be the first mixed IO command");
        assert_eq!(first.kind(), RendererDevToolsIoCommandKind::Inspector);
        assert!(
            ingress.claim_for_owner().is_none(),
            "Performance must not overtake an active Inspector first dispatch"
        );
        ingress.first_dispatch_guard(&mut first).release();
        assert_eq!(
            inspector.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );

        let mut second = ingress
            .claim_for_owner()
            .expect("Performance must follow Inspector");
        assert_eq!(second.kind(), RendererDevToolsIoCommandKind::Performance);
        assert!(
            ingress.claim_for_pause().is_none(),
            "Emulation must not overtake an active Performance first dispatch"
        );
        ingress.first_dispatch_guard(&mut second).release();
        assert_eq!(
            performance.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );

        let mut third = ingress
            .claim_for_pause()
            .expect("Emulation must follow Performance");
        assert_eq!(third.kind(), RendererDevToolsIoCommandKind::Emulation);
        ingress.first_dispatch_guard(&mut third).release();
        assert_eq!(
            emulation.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );
    }

    #[tokio::test]
    async fn dropped_io_waiter_cannot_leave_a_completion_hole() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let abandoned = enqueue(&ingress, agent, Some("session-order"), "abandoned");
        let following = enqueue(&ingress, agent, Some("session-order"), "following");

        drop(abandoned);
        let mut command = ingress
            .claim_for_owner()
            .expect("the following command should remain queued");
        ingress.first_dispatch_guard(&mut command).release();

        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                following.wait_for_first_dispatch()
            )
            .await
            .expect("a dropped waiter must release the next publication"),
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );
    }

    #[tokio::test]
    async fn detach_cancels_all_queued_commands_while_active_first_dispatch_retires_safely() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let mut routes = (0..64)
            .map(|index| enqueue(&ingress, agent, Some("session-a"), &format!("a-{index}")))
            .collect::<Vec<_>>();
        let first_route = routes.remove(0);

        let mut first = ingress
            .claim_for_interrupt()
            .expect("the session head should be claimable");
        let mut first_dispatch = ingress.first_dispatch_guard(&mut first);
        first_dispatch.release();
        assert_eq!(
            first_route.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );

        ingress.detach_session(agent, &DevToolsSessionKey::Attached("session-a".to_owned()));
        for (index, route) in routes.into_iter().enumerate() {
            assert_eq!(
                route.wait_for_first_dispatch().await,
                Ok(RendererRuntimeInspectorIoCommandClaim::Canceled),
                "detach must cancel queued command {}",
                index + 1
            );
        }
        assert!(ingress.claim_for_owner().is_none());
        assert!(ingress.claim_for_interrupt().is_none());
        assert!(ingress.claim_for_pause().is_none());

        assert!(
            {
                let state = ingress.shared.state.lock();
                state.commands.is_empty() && state.active_command_id.is_none()
            },
            "the detached session's tasks must retire"
        );
    }

    #[tokio::test]
    async fn close_cancels_every_session_and_rejects_late_io_commands() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let a1_route = enqueue(&ingress, agent, Some("session-a"), "a1");
        let a2_route = enqueue(&ingress, agent, Some("session-a"), "a2");
        let b1_route = enqueue(&ingress, agent, Some("session-b"), "b1");
        let b2_route = enqueue(&ingress, agent, Some("session-b"), "b2");

        let mut active = ingress
            .claim_for_owner()
            .expect("one session head should become active");
        let mut first_dispatch = ingress.first_dispatch_guard(&mut active);
        first_dispatch.release();
        assert_eq!(
            a1_route.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched)
        );

        ingress.close("test target closed");
        for route in [a2_route, b1_route, b2_route] {
            assert_eq!(
                route.wait_for_first_dispatch().await,
                Ok(RendererRuntimeInspectorIoCommandClaim::Canceled),
                "close must cancel every unclaimed session command"
            );
        }
        assert!(ingress.claim_for_owner().is_none());
        assert!(ingress.claim_for_interrupt().is_none());
        assert!(ingress.claim_for_pause().is_none());

        assert!(
            {
                let state = ingress.shared.state.lock();
                state.commands.is_empty() && state.active_command_id.is_none()
            },
            "the active target task must retire safely after target close"
        );

        let late = enqueue(&ingress, agent, Some("session-late"), "late");
        assert_eq!(
            late.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled),
            "a closed target must reject late IO ingress"
        );
    }

    #[test]
    #[should_panic(expected = "must use the IO route")]
    fn main_thread_command_cannot_enter_io_ingress() {
        let ingress = ingress();
        let page_command = crate::runtime::RendererPageCommand::dispatch_runtime_protocol_message(
            Some("session-a".to_owned()),
            "main".to_owned(),
        );
        let crate::runtime::RendererPageCommand::Inspector(envelope) = page_command else {
            panic!("runtime protocol message must use an Inspector envelope");
        };
        let envelope = RendererDevToolsIoCommandEnvelope::inspector(envelope);
        let _ = ingress.enqueue_command(RendererDevToolsAgentToken::allocate(), envelope);
    }
}
