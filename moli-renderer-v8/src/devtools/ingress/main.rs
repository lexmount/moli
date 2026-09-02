use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use parking_lot::Mutex;

use crate::{
    devtools::{
        ingress::lane::{
            RendererDevToolsIngressCommand, RendererDevToolsSessionLaneKey,
            RendererDevToolsSessionLanes,
        },
        pause::RendererInspectorPauseLoopWake,
        route::RendererInspectorSessionExecutorRouteId,
    },
    render_runtime::RenderRuntimeHandle,
    runtime::{
        RendererCommandTurnOutput, RendererDevToolsMainCommandEnvelope,
        RendererDevToolsMainNestedDispatch, RendererInspectorCommandEnvelope,
        RendererInspectorCommandRoute, RendererInspectorIngressTicket,
        RendererInspectorPauseCommandEffect, RendererOwnerReply, RendererPageCommand,
        RendererPageStateCapturePolicy, RendererPageToken, RendererRuntimeInspectorResponseSender,
        RendererRuntimeInspectorSessionResponseSettlement,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorMainCommandConsumer {
    Owner,
    Pause,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RendererInspectorMainCommandClaim {
    Owner,
    Inspector,
    Page,
    Canceled(String),
}

pub enum RendererRuntimeInspectorMainCommandCompletion {
    Owner(Box<RendererCommandTurnOutput>),
    OwnerSessionResponse {
        output: Box<RendererCommandTurnOutput>,
        response_succeeded: bool,
    },
    OwnerSessionErrorSettled(crate::runtime::RendererOutputFence),
    Inspector,
    InspectorSessionResponse {
        predecessor: crate::runtime::RendererOutputFence,
        response_succeeded: bool,
    },
    Page(Box<RendererCommandTurnOutput>),
    Canceled(String),
}

pub(crate) struct RendererInspectorMainCommand {
    command_id: u64,
    page_token: RendererPageToken,
    pub(crate) agent_token: RendererDevToolsAgentToken,
    capture_policy: RendererPageStateCapturePolicy,
    envelope: RendererDevToolsMainCommandEnvelope,
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

    #[cfg(test)]
    pub(crate) fn raw_json(&self) -> &str {
        self.envelope
            .inspector_envelope()
            .expect("a raw Inspector message requires an Inspector payload")
            .main_protocol_raw_json()
    }

    pub(crate) fn response(&self) -> &RendererRuntimeInspectorResponseSender {
        self.envelope
            .inspector_envelope()
            .expect("an Inspector response requires an Inspector payload")
            .main_protocol_response()
    }

    pub(crate) fn pause_effect(&self) -> RendererInspectorPauseCommandEffect {
        self.envelope
            .inspector_envelope()
            .expect("an Inspector pause effect requires an Inspector payload")
            .pause_effect()
    }

    pub(crate) fn nested_dispatch(&self) -> RendererDevToolsMainNestedDispatch {
        self.envelope.nested_dispatch()
    }

    pub(crate) fn into_protocol_parts(
        self,
    ) -> (
        RendererInspectorIngressTicket,
        String,
        RendererRuntimeInspectorResponseSender,
    ) {
        self.envelope
            .into_nested_inspector_envelope()
            .into_main_protocol_parts()
    }

    pub(crate) fn into_nested_page_parts(
        mut self,
    ) -> (
        RendererPageCommand,
        tokio::sync::oneshot::Sender<anyhow::Result<RendererOwnerReply>>,
    ) {
        let command = self.envelope.into_nested_page_command();
        let reply_tx = self
            .owner_reply_tx
            .take()
            .expect("a nested Page command must retain its completion sender");
        (command, reply_tx)
    }

    #[cfg(test)]
    fn claimed_by(&self) -> Option<RendererInspectorMainCommandConsumer> {
        self.claimed_by
    }
}

impl RendererDevToolsIngressCommand for RendererInspectorMainCommand {
    fn ingress_command_id(&self) -> u64 {
        self.command_id
    }
}

pub struct RendererRuntimeInspectorMainCommandRoute {
    command_id: u64,
    ticket: RendererInspectorIngressTicket,
    claim_rx: Option<tokio::sync::oneshot::Receiver<RendererInspectorMainCommandClaim>>,
    owner_reply_rx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<RendererOwnerReply>>>,
    session_response_settlement_rx:
        Option<tokio::sync::oneshot::Receiver<RendererRuntimeInspectorSessionResponseSettlement>>,
    ingress: RendererInspectorMainIngress,
}

impl RendererRuntimeInspectorMainCommandRoute {
    pub fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub async fn wait_for_completion(
        mut self,
    ) -> anyhow::Result<RendererRuntimeInspectorMainCommandCompletion> {
        let claim = match self
            .claim_rx
            .take()
            .expect("runtime inspector Main command claim receiver should only be awaited once")
            .await
        {
            Ok(claim) => claim,
            Err(_) => {
                if let Some(completion) = self.take_replayed_session_response().await {
                    return Ok(completion);
                }
                return Err(anyhow::anyhow!(
                    "runtime inspector Main command claim channel closed"
                ));
            }
        };
        match claim {
            RendererInspectorMainCommandClaim::Owner => {
                let reply = match self
                    .owner_reply_rx
                    .take()
                    .expect("an owner-claimed Main command must retain its owner reply receiver")
                    .await
                {
                    Ok(Ok(reply)) => reply,
                    Ok(Err(error)) => {
                        if let Some(completion) = self.take_replayed_session_response().await {
                            return Ok(completion);
                        }
                        return Err(error);
                    }
                    Err(_) => {
                        if let Some(completion) = self.take_replayed_session_response().await {
                            return Ok(completion);
                        }
                        return Err(anyhow::anyhow!(
                            "runtime inspector Main owner reply channel closed"
                        ));
                    }
                };
                match reply {
                    RendererOwnerReply::AsyncPageCommandRan(output) => {
                        Ok(RendererRuntimeInspectorMainCommandCompletion::Owner(output))
                    }
                    RendererOwnerReply::RuntimeInspectorSessionResponseSettled {
                        output,
                        response_succeeded,
                    } => Ok(
                        RendererRuntimeInspectorMainCommandCompletion::OwnerSessionResponse {
                            output,
                            response_succeeded,
                        },
                    ),
                    RendererOwnerReply::RuntimeInspectorSessionErrorSettled(predecessor) => Ok(
                        RendererRuntimeInspectorMainCommandCompletion::OwnerSessionErrorSettled(
                            predecessor,
                        ),
                    ),
                    _ => Err(anyhow::anyhow!(
                        "runtime inspector Main owner returned an unexpected renderer reply"
                    )),
                }
            }
            RendererInspectorMainCommandClaim::Inspector => {
                let Some(session_response_settlement_rx) =
                    self.session_response_settlement_rx.take()
                else {
                    return Ok(RendererRuntimeInspectorMainCommandCompletion::Inspector);
                };
                let (predecessor, response_succeeded) = session_response_settlement_rx
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("runtime Inspector Main session response was not published")
                    })?
                    .into_parts();
                Ok(
                    RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse {
                        predecessor,
                        response_succeeded,
                    },
                )
            }
            RendererInspectorMainCommandClaim::Page => {
                let reply = self
                    .owner_reply_rx
                    .take()
                    .expect("a pause-claimed Page command must retain its reply receiver")
                    .await
                    .map_err(|_| anyhow::anyhow!("nested Main Page reply channel closed"))??;
                match reply {
                    RendererOwnerReply::AsyncPageCommandRan(output) => {
                        Ok(RendererRuntimeInspectorMainCommandCompletion::Page(output))
                    }
                    _ => Err(anyhow::anyhow!(
                        "nested Main Page dispatch returned an unexpected renderer reply"
                    )),
                }
            }
            RendererInspectorMainCommandClaim::Canceled(message) => {
                if let Some(completion) = self.take_replayed_session_response().await {
                    Ok(completion)
                } else {
                    Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(
                        message,
                    ))
                }
            }
        }
    }

    async fn take_replayed_session_response(
        &mut self,
    ) -> Option<RendererRuntimeInspectorMainCommandCompletion> {
        let (predecessor, response_succeeded) = self
            .session_response_settlement_rx
            .take()?
            .await
            .ok()?
            .into_parts();
        Some(
            RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse {
                predecessor,
                response_succeeded,
            },
        )
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

struct RendererInspectorMainState {
    lanes: RendererDevToolsSessionLanes<RendererInspectorMainCommand>,
    owner_runtime: Option<RenderRuntimeHandle>,
}

pub(crate) struct RendererInspectorMainFirstDispatchGuard {
    ingress: RendererInspectorMainIngress,
    active: Option<(RendererDevToolsSessionLaneKey, u64)>,
    claim_tx: Option<tokio::sync::oneshot::Sender<RendererInspectorMainCommandClaim>>,
}

pub(crate) struct RendererInspectorMainPostDispatchWakeGuard {
    ingress: Option<RendererInspectorMainIngress>,
}

pub(crate) struct RendererInspectorMainOwnerDispatch {
    page_token: RendererPageToken,
    capture_policy: RendererPageStateCapturePolicy,
    envelope: RendererDevToolsMainCommandEnvelope,
    first_dispatch: RendererInspectorMainFirstDispatchGuard,
    reply_tx: tokio::sync::oneshot::Sender<anyhow::Result<RendererOwnerReply>>,
}

impl RendererInspectorMainOwnerDispatch {
    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererPageToken,
        RendererPageStateCapturePolicy,
        RendererDevToolsMainCommandEnvelope,
        RendererInspectorMainFirstDispatchGuard,
        tokio::sync::oneshot::Sender<anyhow::Result<RendererOwnerReply>>,
    ) {
        (
            self.page_token,
            self.capture_policy,
            self.envelope,
            self.first_dispatch,
            self.reply_tx,
        )
    }
}

impl Drop for RendererInspectorMainFirstDispatchGuard {
    fn drop(&mut self) {
        if let Some(claim_tx) = self.claim_tx.take() {
            let _ = claim_tx.send(RendererInspectorMainCommandClaim::Canceled(
                "Inspector Main command was abandoned before first dispatch".to_owned(),
            ));
        }
        if self.release_lane() {
            self.ingress.notify_execution_opportunities();
        }
    }
}

impl RendererInspectorMainFirstDispatchGuard {
    pub(crate) fn release(&mut self) {
        self.publish_first_dispatch();
        if self.release_lane() {
            self.ingress.notify_execution_opportunities();
        }
    }

    pub(crate) fn release_for_dispatch(&mut self) -> RendererInspectorMainPostDispatchWakeGuard {
        self.publish_first_dispatch();
        let has_ready = self.release_lane();
        RendererInspectorMainPostDispatchWakeGuard {
            ingress: has_ready.then(|| self.ingress.clone()),
        }
    }

    pub(crate) fn reject(&mut self, message: impl Into<String>) {
        if let Some(claim_tx) = self.claim_tx.take() {
            let _ = claim_tx.send(RendererInspectorMainCommandClaim::Canceled(message.into()));
        }
        if self.release_lane() {
            self.ingress.notify_execution_opportunities();
        }
    }

    fn publish_first_dispatch(&mut self) {
        if let Some(claim_tx) = self.claim_tx.take() {
            let _ = claim_tx.send(RendererInspectorMainCommandClaim::Inspector);
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
                    lanes: RendererDevToolsSessionLanes::default(),
                    owner_runtime: None,
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
            state.lanes.has_ready()
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
            RendererDevToolsMainCommandEnvelope::from_protocol_command(
                RendererPageCommand::Inspector(envelope),
            ),
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
        self.enqueue_with_policy(
            page_token,
            agent_token,
            RendererDevToolsMainCommandEnvelope::from_protocol_command(
                RendererPageCommand::Inspector(envelope),
            ),
            capture_policy,
        )
    }

    pub(crate) fn enqueue_protocol_page_command(
        &self,
        page_token: RendererPageToken,
        agent_token: RendererDevToolsAgentToken,
        command: RendererPageCommand,
        inspector_session_id: Option<String>,
        capture_policy: RendererPageStateCapturePolicy,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        self.enqueue_with_policy(
            page_token,
            agent_token,
            RendererDevToolsMainCommandEnvelope::from_protocol_command_in_session(
                command,
                inspector_session_id,
            ),
            capture_policy,
        )
    }

    fn enqueue_with_policy(
        &self,
        page_token: RendererPageToken,
        agent_token: RendererDevToolsAgentToken,
        envelope: RendererDevToolsMainCommandEnvelope,
        capture_policy: RendererPageStateCapturePolicy,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        assert_eq!(
            envelope.ticket().route(),
            RendererInspectorCommandRoute::MainThread,
            "only MainThread DevTools commands may enter RendererInspectorMainIngress"
        );
        let (claim_tx, claim_rx) = tokio::sync::oneshot::channel();
        let (owner_reply_tx, owner_reply_rx) = tokio::sync::oneshot::channel();
        let session_response_settlement_rx = envelope.response().and_then(
            RendererRuntimeInspectorResponseSender::take_session_response_settlement_receiver,
        );
        let mut state = self.shared.state.lock();
        let lane_key =
            RendererDevToolsSessionLaneKey::new(agent_token, envelope.ticket().session().clone());
        let ticket = envelope.ticket().clone();
        let command_id = ticket.sequence();
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
        if let Err(command) = state.lanes.enqueue(lane_key, command) {
            drop(state);
            fail_main_command(command, "Inspector Main target is closed");
        } else {
            drop(state);
            self.notify_execution_opportunities();
        }
        RendererRuntimeInspectorMainCommandRoute {
            command_id,
            ticket,
            claim_rx: Some(claim_rx),
            owner_reply_rx: Some(owner_reply_rx),
            session_response_settlement_rx,
            ingress: self.clone(),
        }
    }

    pub(crate) fn claim_for_owner(&self) -> Option<RendererInspectorMainCommand> {
        self.shared.owner_wake_armed.store(false, Ordering::Release);
        let command = self.claim_next(RendererInspectorMainCommandConsumer::Owner);
        if command.is_none() && self.shared.state.lock().lanes.has_ready() {
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
        let (_, mut command) = state.lanes.claim_next(|command| {
            consumer != RendererInspectorMainCommandConsumer::Pause
                || command.nested_dispatch() != RendererDevToolsMainNestedDispatch::OwnerOnly
        })?;
        command.claimed_by = Some(consumer);
        let claim = match consumer {
            RendererInspectorMainCommandConsumer::Owner => {
                Some(RendererInspectorMainCommandClaim::Owner)
            }
            RendererInspectorMainCommandConsumer::Pause => match command.nested_dispatch() {
                RendererDevToolsMainNestedDispatch::InspectorSession => None,
                RendererDevToolsMainNestedDispatch::PageAgent => {
                    Some(RendererInspectorMainCommandClaim::Page)
                }
                RendererDevToolsMainNestedDispatch::OwnerOnly => {
                    unreachable!("owner-only commands are skipped by pause claim")
                }
            },
        };
        if let Some(claim) = claim
            && let Some(claim_tx) = command.claim_tx.take()
        {
            let _ = claim_tx.send(claim);
        }
        if consumer == RendererInspectorMainCommandConsumer::Pause
            && command.nested_dispatch() == RendererDevToolsMainNestedDispatch::InspectorSession
        {
            command.owner_reply_tx.take();
        }
        Some(command)
    }

    pub(crate) fn first_dispatch_guard(
        &self,
        command: &mut RendererInspectorMainCommand,
    ) -> RendererInspectorMainFirstDispatchGuard {
        let lane_key = RendererDevToolsSessionLaneKey::new(
            command.agent_token,
            command.ticket().session().clone(),
        );
        let state = self.shared.state.lock();
        state.lanes.assert_active(
            &lane_key,
            command.command_id,
            "a claimed Inspector Main command must own its session lane",
        );
        drop(state);
        RendererInspectorMainFirstDispatchGuard {
            ingress: self.clone(),
            active: Some((lane_key, command.command_id)),
            claim_tx: command.claim_tx.take(),
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
        let mut command = command;
        let first_dispatch = self.first_dispatch_guard(&mut command);
        let RendererInspectorMainCommand {
            page_token,
            capture_policy,
            envelope,
            owner_reply_tx,
            ..
        } = command;
        RendererInspectorMainOwnerDispatch {
            page_token,
            capture_policy,
            envelope,
            first_dispatch,
            reply_tx: owner_reply_tx
                .expect("an owner-claimed Main command must retain its owner reply sender"),
        }
    }

    fn finish_first_dispatch(
        &self,
        lane_key: RendererDevToolsSessionLaneKey,
        command_id: u64,
    ) -> bool {
        self.shared.state.lock().lanes.finish_first_dispatch(
            lane_key,
            command_id,
            "only the active Inspector Main command may release its lane",
        )
    }

    pub(crate) fn cancel_queued_command(&self, command_id: u64, message: &str) {
        let command = self.shared.state.lock().lanes.cancel_queued(command_id);
        if let Some(command) = command {
            fail_main_command(command, message);
        }
    }

    pub(crate) fn begin_session_detach(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let lane_key = RendererDevToolsSessionLaneKey::new(agent_token, session.clone());
        let commands = self
            .shared
            .state
            .lock()
            .lanes
            .begin_session_detach(&lane_key);
        for command in commands {
            fail_main_command(command, "Inspector Main session was detached");
        }
    }

    pub(crate) fn finish_session_detach(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        let lane_key = RendererDevToolsSessionLaneKey::new(agent_token, session.clone());
        let has_ready = self
            .shared
            .state
            .lock()
            .lanes
            .finish_session_detach(&lane_key);
        if has_ready {
            self.notify_execution_opportunities();
        }
    }

    pub(crate) fn close(&self, message: &str) {
        let commands = { self.shared.state.lock().lanes.close_and_drain() };
        self.shared.pause_wake.notify_all();
        for command in commands {
            fail_main_command(command, message);
        }
    }

    pub(crate) fn cancel_all_queued(&self, message: &str) {
        let commands = self.shared.state.lock().lanes.drain_queued();
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

fn fail_main_command(command: RendererInspectorMainCommand, message: &str) {
    let RendererInspectorMainCommand {
        envelope,
        claim_tx,
        owner_reply_tx,
        ..
    } = command;
    if let Some(claim_tx) = claim_tx {
        let _ = claim_tx.send(RendererInspectorMainCommandClaim::Canceled(
            message.to_owned(),
        ));
    }
    if let Some(owner_reply_tx) = owner_reply_tx {
        let _ = owner_reply_tx.send(Err(anyhow::anyhow!(message.to_owned())));
    }
    drop(envelope);
}

impl std::fmt::Debug for RendererInspectorMainIngress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        formatter
            .debug_struct("RendererInspectorMainIngress")
            .field("route_id", &self.shared.route_id)
            .field("session_lanes", &state.lanes.session_count())
            .field("ready_sessions", &state.lanes.ready_count())
            .field("closed", &state.lanes.is_closed())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        devtools::pause::RendererInspectorPauseBridge,
        runtime::{
            RendererDevToolsSessionOutputHost, RendererInspectorIngressTicket,
            RendererOutputStreamIdentity, RendererPageReply, RendererPageState,
            RendererPerformanceMetricSnapshot, RendererRuntimeCommandOutput,
            RendererRuntimeInspectorAsyncCompletion, RendererRuntimeInspectorMessage,
            RendererRuntimeInspectorResponseChannel, RendererTurnOutputJournal,
            renderer_output_transport_channel,
        },
        types::ScriptExecutionReport,
    };

    fn page_state() -> Arc<RendererPageState> {
        let url = url::Url::parse("about:blank").expect("test URL");
        Arc::new(RendererPageState {
            requested_url: url.clone(),
            navigation_initiator_url: None,
            navigation_redirected: false,
            navigation_redirect_count: 0,
            navigation_redirect_chain: Vec::new(),
            final_url: url,
            document_title: String::new(),
            status: 200,
            headers: Vec::new(),
            script_execution: Arc::new(ScriptExecutionReport::default()),
            idle_override: None,
            service_worker_client_id: 0,
            dedicated_worker_running_worker_isolate_count: 0,
            performance_metric_snapshot: RendererPerformanceMetricSnapshot::default(),
        })
    }

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

        let mut first = ingress.claim_for_owner().expect("first ready Main session");
        assert!(first.raw_json().contains(r#""a1""#));
        let second = ingress
            .claim_for_pause()
            .expect("the other Main session remains independently ready");
        assert!(second.raw_json().contains(r#""b1""#));
        assert!(ingress.claim_for_owner().is_none());

        ingress.first_dispatch_guard(&mut first).release();
        let third = ingress.claim_for_pause().expect("a2 after a1 dispatch");
        assert!(third.raw_json().contains(r#""a2""#));
    }

    #[tokio::test]
    async fn overlapping_primary_detaches_hold_main_ingress_until_all_complete() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let _active_route = enqueue(
            &ingress,
            agent,
            None,
            r#"{"id":1,"method":"Runtime.getProperties","params":{"objectId":"active"}}"#,
        );
        let queued_route = enqueue(
            &ingress,
            agent,
            None,
            r#"{"id":2,"method":"Runtime.getProperties","params":{"objectId":"queued"}}"#,
        );
        let mut active = ingress
            .claim_for_owner()
            .expect("the session head should become active");
        let mut first_dispatch = ingress.first_dispatch_guard(&mut active);

        ingress.begin_session_detach(agent, &DevToolsSessionKey::Primary);
        assert!(matches!(
            queued_route.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
        ));

        let replacement_route = enqueue(
            &ingress,
            agent,
            None,
            r#"{"id":3,"method":"Runtime.getProperties","params":{"objectId":"replacement"}}"#,
        );
        ingress.begin_session_detach(agent, &DevToolsSessionKey::Primary);
        assert!(matches!(
            replacement_route.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(_))
        ));

        let post_detach_route = enqueue(
            &ingress,
            agent,
            None,
            r#"{"id":4,"method":"Runtime.getProperties","params":{"objectId":"post-detach"}}"#,
        );
        assert!(
            ingress.claim_for_owner().is_none(),
            "the next generation must wait for its own owner-side cleanup"
        );

        first_dispatch.release();
        assert!(
            ingress.claim_for_owner().is_none(),
            "releasing an old first dispatch must not bypass the detach barrier"
        );

        ingress.finish_session_detach(agent, &DevToolsSessionKey::Primary);
        assert!(
            ingress.claim_for_owner().is_none(),
            "the first cleanup must not bypass the second detach barrier"
        );

        ingress.finish_session_detach(agent, &DevToolsSessionKey::Primary);
        let replacement = ingress
            .claim_for_owner()
            .expect("replacement work should run after every owner cleanup");
        assert_eq!(replacement.command_id(), post_detach_route.command_id);
    }

    #[tokio::test]
    async fn direct_inspector_rejection_reports_error_before_releasing_next_main_command() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let rejected = enqueue(
            &ingress,
            agent,
            Some("session-reject"),
            r#"{"id":1,"method":"Runtime.getProperties","params":{"objectId":"missing"}}"#,
        );
        let _following = enqueue(
            &ingress,
            agent,
            Some("session-reject"),
            r#"{"id":2,"method":"Runtime.getProperties","params":{"objectId":"next"}}"#,
        );
        let mut command = ingress
            .claim_for_pause()
            .expect("the direct Inspector command should be claimable");
        assert!(
            ingress.claim_for_owner().is_none(),
            "the following command must remain behind the unresolved first-dispatch boundary"
        );
        let mut first_dispatch = ingress.first_dispatch_guard(&mut command);
        first_dispatch.reject("Inspector session is not available");

        assert!(matches!(
            rejected.wait_for_completion().await,
            Ok(RendererRuntimeInspectorMainCommandCompletion::Canceled(message))
                if message == "Inspector session is not available"
        ));
        assert!(
            ingress.claim_for_owner().is_some(),
            "rejecting first dispatch must release the next command in the same session"
        );
    }

    #[tokio::test]
    async fn claimed_owner_failure_waits_for_replacement_session_response() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let attachment = moli_page_types::RendererAgentAttachmentId::allocate();
        let session = DevToolsSessionKey::Attached("session-replay".to_owned());
        let (channel, response_rx) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::SessionSink,
        );
        assert!(response_rx.is_none());
        let old_response = channel.activate_sender(1, Some(attachment));
        let route = ingress.enqueue_command(
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(1)),
            agent,
            RendererInspectorCommandEnvelope::new_main_protocol(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    session.wire_session_id().map(str::to_owned),
                    RendererInspectorCommandRoute::MainThread,
                ),
                None,
                r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"1"}}"#.to_owned(),
                old_response,
            ),
        );
        let claimed = ingress
            .claim_for_owner()
            .expect("the original command should be claimed by the owner");
        let owner_dispatch = ingress.prepare_owner_dispatch(claimed);

        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
            crate::runtime::PageId::new_for_testing(2),
        );
        let (transport, _transport_rx) = renderer_output_transport_channel();
        let replacement_response = channel
            .activate_sender(2, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                agent,
                session,
                attachment,
                RendererTurnOutputJournal::new_with_transport(stream, transport),
            ));
        drop(owner_dispatch);
        replacement_response
            .send_output(RendererRuntimeCommandOutput::from_inspector_message(
                RendererRuntimeInspectorMessage::protocol(serde_json::json!({
                    "id": 2,
                    "result": {}
                })),
            ))
            .expect("the replacement route should publish the terminal response");

        assert!(matches!(
            route.wait_for_completion().await,
            Ok(
                RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse {
                    response_succeeded: true,
                    ..
                }
            )
        ));
    }

    #[tokio::test]
    async fn direct_inspector_error_completes_from_the_session_response() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let attachment = moli_page_types::RendererAgentAttachmentId::allocate();
        let session = DevToolsSessionKey::Attached("session-direct-error".to_owned());
        let (channel, response_rx) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::SessionSink,
        );
        assert!(response_rx.is_none());
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
            crate::runtime::PageId::new_for_testing(35),
        );
        let (transport, _transport_rx) = renderer_output_transport_channel();
        let response = channel
            .activate_sender(35, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                agent,
                session.clone(),
                attachment,
                RendererTurnOutputJournal::new_with_transport(stream, transport),
            ));
        let publisher = response.clone();
        let route = ingress.enqueue_command(
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(35)),
            agent,
            RendererInspectorCommandEnvelope::new_main_protocol(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    session.wire_session_id().map(str::to_owned),
                    RendererInspectorCommandRoute::MainThread,
                ),
                None,
                r#"{"id":35,"method":"Runtime.getProperties","params":{"objectId":"missing"}}"#
                    .to_owned(),
                response,
            ),
        );
        let mut command = ingress
            .claim_for_pause()
            .expect("the direct Inspector command must reach nested Main");
        ingress.first_dispatch_guard(&mut command).release();
        publisher
            .send_output(RendererRuntimeCommandOutput::from_inspector_message(
                RendererRuntimeInspectorMessage::protocol(serde_json::json!({
                    "id": 35,
                    "error": {
                        "code": -32000,
                        "message": "Cannot find object with given id",
                    },
                })),
            ))
            .expect("the direct Inspector error must publish");

        assert!(matches!(
            route.wait_for_completion().await,
            Ok(
                RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse {
                    response_succeeded: false,
                    ..
                }
            )
        ));
    }

    #[test]
    fn mixed_v8_and_page_agents_share_one_main_session_lane() {
        let ingress = ingress();
        let agent = RendererDevToolsAgentToken::allocate();
        let page_token =
            RendererPageToken::new_for_testing(crate::runtime::PageId::new_for_testing(1));
        let _page = ingress.enqueue_protocol_page_command(
            page_token,
            agent,
            RendererPageCommand::PerformanceMetricSnapshot,
            Some("session-a".to_owned()),
            RendererPageStateCapturePolicy::ProtocolTurn,
        );
        let _v8 = enqueue(
            &ingress,
            agent,
            Some("session-a"),
            r#"{"id":2,"method":"Runtime.getProperties","params":{"objectId":"v8"}}"#,
        );

        let mut page = ingress
            .claim_for_pause()
            .expect("the Page agent command should be first");
        assert_eq!(
            page.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::PageAgent
        );
        assert!(
            ingress.claim_for_pause().is_none(),
            "the V8 command must remain behind the Page agent first-dispatch boundary"
        );

        let output = RendererCommandTurnOutput::new(
            RendererPageReply::Unit,
            page_state(),
            RendererRuntimeCommandOutput::default(),
            None,
            None,
        )
        .expect("test Page-agent output")
        .hold_until_protocol_handoff(ingress.first_dispatch_guard(&mut page));
        assert!(
            ingress.claim_for_pause().is_none(),
            "settled Page output must retain the lane until protocol consumes it"
        );

        let (_completion, _predecessor) = output.into_completion_and_predecessor();
        let v8 = ingress
            .claim_for_pause()
            .expect("the V8 command should follow the Page protocol handoff");
        assert_eq!(
            v8.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::InspectorSession
        );
    }

    #[test]
    fn nested_main_accepts_runtime_evaluation_with_or_without_explicit_context() {
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
        let explicit_context = ingress
            .claim_for_pause()
            .expect("an Inspector-native context id should remain pumpable by nested Main");
        assert_eq!(
            explicit_context.claimed_by(),
            Some(RendererInspectorMainCommandConsumer::Pause)
        );
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
        expected = "only MainThread DevTools commands may enter RendererInspectorMainIngress"
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
