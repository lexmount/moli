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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererRuntimeInspectorIoCommandClaim {
    Dispatched,
    SessionResponse {
        predecessor: crate::runtime::RendererOutputFence,
        response_succeeded: bool,
    },
    Canceled(String),
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
        self.envelope.response()
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
    session_response_settlement_rx: Option<
        tokio::sync::oneshot::Receiver<
            crate::runtime::RendererRuntimeInspectorSessionResponseSettlement,
        >,
    >,
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
        let claim = match self
            .first_dispatch_rx
            .take()
            .expect("runtime Inspector IO first dispatch should only be awaited once")
            .await
        {
            Ok(claim) => claim,
            Err(_) => {
                let Some(session_response_settlement_rx) =
                    self.session_response_settlement_rx.take()
                else {
                    return Err("runtime Inspector IO first-dispatch channel closed");
                };
                let (predecessor, response_succeeded) = session_response_settlement_rx
                    .await
                    .map_err(|_| "runtime Inspector IO first-dispatch channel closed")?
                    .into_parts();
                return Ok(RendererRuntimeInspectorIoCommandClaim::SessionResponse {
                    predecessor,
                    response_succeeded,
                });
            }
        };
        match claim {
            RendererRuntimeInspectorIoCommandClaim::Dispatched => {
                let Some(session_response_settlement_rx) =
                    self.session_response_settlement_rx.take()
                else {
                    return Ok(RendererRuntimeInspectorIoCommandClaim::Dispatched);
                };
                let (predecessor, response_succeeded) = session_response_settlement_rx
                    .await
                    .map_err(|_| "runtime Inspector IO session response was not published")?
                    .into_parts();
                Ok(RendererRuntimeInspectorIoCommandClaim::SessionResponse {
                    predecessor,
                    response_succeeded,
                })
            }
            RendererRuntimeInspectorIoCommandClaim::Canceled(message) => {
                let Some(session_response_settlement_rx) =
                    self.session_response_settlement_rx.take()
                else {
                    return Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(message));
                };
                match session_response_settlement_rx.await {
                    Ok(settlement) => {
                        let (predecessor, response_succeeded) = settlement.into_parts();
                        Ok(RendererRuntimeInspectorIoCommandClaim::SessionResponse {
                            predecessor,
                            response_succeeded,
                        })
                    }
                    Err(_) => Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(message)),
                }
            }
            response @ RendererRuntimeInspectorIoCommandClaim::SessionResponse { .. } => {
                Ok(response)
            }
        }
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
    // One count per lane is enough: every detach guard completes at most once,
    // and commands stay blocked until all outstanding guards have completed.
    session_detaches: BTreeMap<RendererDevToolsSessionLaneKey, usize>,
    closed: bool,
    owner_wake_tx: Option<tokio::sync::mpsc::UnboundedSender<RendererInspectorIoOwnerWake>>,
}

impl RendererInspectorIoState {
    fn has_ready(&self) -> bool {
        !self.closed
            && self.active_command_id.is_none()
            && self.commands.iter().any(|command| {
                !self
                    .session_detaches
                    .contains_key(&RendererDevToolsSessionLaneKey::new(
                        command.agent_token,
                        command.ticket().session().clone(),
                    ))
            })
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

/// Owns the target IO task slot only until the command reaches its executor.
/// A later interrupt command must not wait for an asynchronous response.
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
        let has_ready = self.finish_task(RendererRuntimeInspectorIoCommandClaim::Canceled(
            "Inspector IO command was abandoned before first dispatch".to_owned(),
        ));
        if has_ready {
            self.ingress.notify_execution_opportunities();
        }
    }
}

impl RendererInspectorIoFirstDispatchGuard {
    pub(crate) fn release(&mut self) {
        let has_ready = self.finish_task(RendererRuntimeInspectorIoCommandClaim::Dispatched);
        if has_ready {
            self.ingress.notify_execution_opportunities();
        }
    }

    /// Releases the receiver slot and publishes first-dispatch immediately
    /// before entering V8, but keeps the next execution wake behind the return
    /// from this dispatch. V8 may enter a nested debugger loop before the call
    /// returns, so the command's ingress lifecycle must already be settled.
    pub(crate) fn release_for_dispatch(&mut self) -> RendererInspectorIoPostDispatchWakeGuard {
        let has_ready = self.finish_task(RendererRuntimeInspectorIoCommandClaim::Dispatched);
        RendererInspectorIoPostDispatchWakeGuard {
            ingress: has_ready.then(|| self.ingress.clone()),
        }
    }

    pub(crate) fn reject(&mut self, message: impl Into<String>) {
        let has_ready = self.finish_task(RendererRuntimeInspectorIoCommandClaim::Canceled(
            message.into(),
        ));
        if has_ready {
            self.ingress.notify_execution_opportunities();
        }
    }

    fn finish_task(&mut self, claim: RendererRuntimeInspectorIoCommandClaim) -> bool {
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
            let _ = first_dispatch_tx.send(claim);
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
                    session_detaches: BTreeMap::new(),
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
        let mut state = self.shared.state.lock();
        let (first_dispatch_tx, first_dispatch_rx) = tokio::sync::oneshot::channel();
        let session_response_settlement_rx = envelope.response().and_then(
            RendererRuntimeInspectorResponseSender::take_session_response_settlement_receiver,
        );
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
            session_response_settlement_rx,
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
        let position = state
            .commands
            .iter()
            .position(|command| {
                !state
                    .session_detaches
                    .contains_key(&RendererDevToolsSessionLaneKey::new(
                        command.agent_token,
                        command.ticket().session().clone(),
                    ))
            })
            .expect("a ready Inspector task runner must have an eligible command");
        let mut command = state
            .commands
            .remove(position)
            .expect("the located Inspector IO command must remain queued");
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

    pub(crate) fn begin_session_detach(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let lane_key = RendererDevToolsSessionLaneKey::new(agent_token, session.clone());
        let commands = {
            let mut state = self.shared.state.lock();
            if state.closed {
                Vec::new()
            } else {
                let pending_detaches = state.session_detaches.entry(lane_key).or_default();
                *pending_detaches = pending_detaches
                    .checked_add(1)
                    .expect("renderer Inspector IO pending-detach count overflow");
                state.drain_commands(|command| {
                    command.agent_token == agent_token && command.ticket().session() == session
                })
            }
        };
        for command in commands {
            fail_io_command(command, "Inspector IO session was detached");
        }
    }

    pub(crate) fn finish_session_detach(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let lane_key = RendererDevToolsSessionLaneKey::new(agent_token, session.clone());
        let mut state = self.shared.state.lock();
        let finished_last_detach = match state.session_detaches.get_mut(&lane_key) {
            Some(pending_detaches) => {
                *pending_detaches -= 1;
                *pending_detaches == 0
            }
            None => {
                debug_assert!(
                    state.closed,
                    "only a closed target may finish an unarmed IO detach"
                );
                false
            }
        };
        if finished_last_detach {
            state.session_detaches.remove(&lane_key);
        }
        drop(state);
        self.notify_execution_opportunities();
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
        let _ = first_dispatch_tx.send(RendererRuntimeInspectorIoCommandClaim::Canceled(
            message.to_owned(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        devtools::pause::RendererInspectorPauseBridge,
        runtime::{
            RendererDevToolsSessionOutputHost, RendererInspectorCommandEnvelope,
            RendererInspectorIngressTicket, RendererOutputStreamIdentity,
            RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage,
            RendererRuntimeInspectorResponseChannel, RendererTurnOutputJournal,
            renderer_output_transport_channel,
        },
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

    fn session_response_output(
        call_id: i32,
        result: serde_json::Value,
    ) -> RendererRuntimeCommandOutput {
        RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::protocol(serde_json::json!({
                "id": call_id,
                "result": result,
            })),
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
    async fn dispatched_io_route_completes_from_its_session_response() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let attachment = moli_page_types::RendererAgentAttachmentId::allocate();
        let session = DevToolsSessionKey::Attached("session-output".to_owned());
        let (channel, response_rx) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::SessionSink,
        );
        assert!(response_rx.is_none());
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
            crate::runtime::PageId::new_for_testing(31),
        );
        let (transport, _transport_rx) = renderer_output_transport_channel();
        let response = channel
            .activate_sender(31, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                agent,
                session.clone(),
                attachment,
                RendererTurnOutputJournal::new_with_transport(stream, transport),
            ));
        let publisher = response.clone();
        let route = ingress.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::inspector(RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    session.wire_session_id().map(str::to_owned),
                    RendererInspectorCommandRoute::Io,
                ),
                r#"{"id":31,"method":"Debugger.getScriptSource","params":{"scriptId":"1"}}"#
                    .to_owned(),
                Some(response),
            )),
        );
        let mut command = ingress
            .claim_for_interrupt()
            .expect("the IO command must reach first dispatch");
        ingress.first_dispatch_guard(&mut command).release();
        publisher
            .send_output(session_response_output(
                31,
                serde_json::json!({"scriptSource": "source"}),
            ))
            .expect("the session response must publish");

        assert!(matches!(
            route.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::SessionResponse {
                response_succeeded: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn canceled_io_route_accepts_a_replayed_session_response() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let attachment = moli_page_types::RendererAgentAttachmentId::allocate();
        let session = DevToolsSessionKey::Attached("session-replayed-output".to_owned());
        let (channel, response_rx) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::SessionSink,
        );
        assert!(response_rx.is_none());
        let original_response = channel.activate_sender(32, Some(attachment));
        let route = ingress.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::inspector(RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    session.wire_session_id().map(str::to_owned),
                    RendererInspectorCommandRoute::Io,
                ),
                r#"{"id":32,"method":"Debugger.getScriptSource","params":{"scriptId":"2"}}"#
                    .to_owned(),
                Some(original_response),
            )),
        );
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
            crate::runtime::PageId::new_for_testing(32),
        );
        let (transport, _transport_rx) = renderer_output_transport_channel();
        let replayed_response = channel
            .activate_sender(33, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                agent,
                session,
                attachment,
                RendererTurnOutputJournal::new_with_transport(stream, transport),
            ));

        ingress.close("old attachment closed");
        replayed_response
            .send_output(session_response_output(
                33,
                serde_json::json!({"scriptSource": "replacement"}),
            ))
            .expect("the replayed session response must publish");

        assert!(matches!(
            route.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::SessionResponse {
                response_succeeded: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn canceled_io_route_keeps_cancellation_when_session_replay_closes() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let attachment = moli_page_types::RendererAgentAttachmentId::allocate();
        let (channel, response_rx) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::SessionSink,
        );
        assert!(response_rx.is_none());
        let route = ingress.enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::inspector(RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    Some("session-canceled-output".to_owned()),
                    RendererInspectorCommandRoute::Io,
                ),
                r#"{"id":34,"method":"Debugger.getScriptSource","params":{"scriptId":"3"}}"#
                    .to_owned(),
                Some(channel.activate_sender(34, Some(attachment))),
            )),
        );

        ingress.close("attachment closed before replay");

        assert!(matches!(
            route.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(message))
                if message == "attachment closed before replay"
        ));
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
    async fn claimed_io_rejection_preserves_error_and_releases_target_fifo() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let rejected = enqueue(&ingress, agent, Some("session-reject"), "rejected");
        let _following = enqueue(&ingress, agent, Some("session-reject"), "following");
        let mut command = ingress
            .claim_for_owner()
            .expect("the first IO command should be claimable");
        assert!(
            ingress.claim_for_pause().is_none(),
            "the target FIFO must remain occupied until rejection is published"
        );
        let mut first_dispatch = ingress.first_dispatch_guard(&mut command);
        first_dispatch.reject("Inspector session is not available");

        assert!(matches!(
            rejected.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(message))
                if message == "Inspector session is not available"
        ));
        assert!(
            ingress.claim_for_pause().is_some(),
            "rejecting the active command must release the next target task"
        );
    }

    #[tokio::test]
    async fn overlapping_primary_detaches_hold_io_ingress_until_all_complete() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let mut routes = (0..64)
            .map(|index| enqueue(&ingress, agent, None, &format!("a-{index}")))
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

        ingress.begin_session_detach(agent, &DevToolsSessionKey::Primary);
        for (index, route) in routes.into_iter().enumerate() {
            assert!(
                matches!(
                    route.wait_for_first_dispatch().await,
                    Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
                ),
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

        let replacement = enqueue(&ingress, agent, None, "replacement");
        ingress.begin_session_detach(agent, &DevToolsSessionKey::Primary);
        assert!(matches!(
            replacement.wait_for_first_dispatch().await,
            Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
        ));

        let post_detach = enqueue(&ingress, agent, None, "post-detach");
        let peer = enqueue(&ingress, agent, Some("session-b"), "peer");
        let mut peer_command = ingress
            .claim_for_owner()
            .expect("an unrelated session must bypass the suspended session");
        assert_eq!(peer_command.command_id(), peer.command_id);
        ingress.first_dispatch_guard(&mut peer_command).release();
        assert!(
            ingress.claim_for_owner().is_none(),
            "replacement IO work must remain behind owner-side cleanup"
        );

        ingress.finish_session_detach(agent, &DevToolsSessionKey::Primary);
        assert!(
            ingress.claim_for_owner().is_none(),
            "the first cleanup must not bypass the second IO detach barrier"
        );

        ingress.finish_session_detach(agent, &DevToolsSessionKey::Primary);
        let replacement_command = ingress
            .claim_for_owner()
            .expect("replacement IO work should run after owner cleanup");
        assert_eq!(replacement_command.command_id(), post_detach.command_id);
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
            assert!(
                matches!(
                    route.wait_for_first_dispatch().await,
                    Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
                ),
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
        assert!(
            matches!(
                late.wait_for_first_dispatch().await,
                Ok(RendererRuntimeInspectorIoCommandClaim::Canceled(_))
            ),
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
