use std::collections::{BTreeMap, BTreeSet, VecDeque};

use moli_core::{RendererOutputCursor, RendererRuntimeCommandCausalIdentity};

use super::output_ingress::PreparedProtocolOutputs;
use crate::conn::{
    CdpConnection, CdpTurnOutcome, CommandDispatchContext, CommandOwnerScope,
    TargetPageResidenceIdentity,
};

/// The single-use authority for one Runtime command's causal output barrier.
///
/// The permit is deliberately move-only. Releasing or canceling it removes
/// exactly one command predecessor from held output; cloning it would make the
/// barrier terminal ambiguous.
#[derive(Debug)]
#[must_use = "a Runtime command output barrier must be released or canceled exactly once"]
pub struct RuntimeCommandOutputBarrierPermit {
    id: RuntimeCommandOutputBarrierId,
    command_id: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct RuntimeCommandOutputBarrierId(u64);

impl RuntimeCommandOutputBarrierId {
    fn checked_next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("Runtime command output barrier id overflow"),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RuntimeCommandOutputHoldId(u64);

impl RuntimeCommandOutputHoldId {
    fn checked_next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("Runtime command held-output id overflow"),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuntimeCommandCausalOwner {
    Page(TargetPageResidenceIdentity),
    Target {
        browser_context_id: String,
        target_id: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeCommandOutputRoute {
    delivery_scope: CommandOwnerScope,
    causal_owner: RuntimeCommandCausalOwner,
}

#[derive(Debug)]
struct ActiveRuntimeCommandOutputBarrier {
    command_id: u64,
    command_scope: CommandOwnerScope,
    causal_owner: RuntimeCommandCausalOwner,
    renderer_cause: RendererRuntimeCommandCausalIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeldOutputReleaseMode {
    All,
    OwnerActionsOnly,
}

#[derive(Debug)]
struct HeldRuntimeCommandOutput {
    id: RuntimeCommandOutputHoldId,
    renderer_output_cursor: Option<RendererOutputCursor>,
    predecessors: BTreeSet<RuntimeCommandOutputBarrierId>,
    release_mode: HeldOutputReleaseMode,
    outputs: PreparedProtocolOutputs,
}

#[derive(Debug)]
struct HeldRuntimeCommandOutputRoute {
    route: RuntimeCommandOutputRoute,
    outputs: VecDeque<HeldRuntimeCommandOutput>,
}

/// Typed terminal for one command barrier.
///
/// `Released` means the exact command owner still existed when its response
/// entered the protocol output sequence. `Superseded` means the command's
/// target/Page residence changed first. `Canceled` is an explicit command or
/// transport cancellation. Every terminal removes the predecessor once;
/// canceled/superseded output performs only browser-owner cleanup and does not
/// project protocol-only observations from the stale command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCommandOutputBarrierTerminal {
    Released,
    Canceled,
    Superseded,
}

/// One consumed barrier terminal plus the protocol/scheduler output produced
/// while releasing its held concrete work.
pub struct RuntimeCommandOutputBarrierCompletion {
    terminal: RuntimeCommandOutputBarrierTerminal,
    outcome: CdpTurnOutcome,
}

impl RuntimeCommandOutputBarrierCompletion {
    pub(crate) fn new(
        terminal: RuntimeCommandOutputBarrierTerminal,
        outcome: CdpTurnOutcome,
    ) -> Self {
        Self { terminal, outcome }
    }

    pub fn terminal(&self) -> RuntimeCommandOutputBarrierTerminal {
        self.terminal
    }

    pub fn into_outcome(self) -> CdpTurnOutcome {
        self.outcome
    }
}

/// Stable owner-local residence for concrete output held behind Runtime
/// command responses.
///
/// A held batch is frozen once by the renderer/protocol owner and is never
/// reconstructed from current state. Batches stay FIFO within their exact
/// delivery route. Each batch names the exact Runtime command that produced
/// it; another pending command on the same Page is not a predecessor.
#[derive(Debug, Default)]
pub struct RuntimeCommandOutputBarriers {
    next_barrier_id: RuntimeCommandOutputBarrierId,
    next_hold_id: RuntimeCommandOutputHoldId,
    active: BTreeMap<RuntimeCommandOutputBarrierId, ActiveRuntimeCommandOutputBarrier>,
    held_routes: Vec<HeldRuntimeCommandOutputRoute>,
}

impl RuntimeCommandCausalOwner {
    fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Option<Self> {
        if let Some(page) = conn.target_page_residence_identity_for_session(session_id) {
            return Some(Self::Page(page));
        }
        let (browser_context_id, target_id) = conn.target_owner_identity_for_session(session_id)?;
        Some(Self::Target {
            browser_context_id,
            target_id,
        })
    }
}

impl RuntimeCommandOutputRoute {
    fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Option<Self> {
        Some(Self {
            delivery_scope: CommandOwnerScope::capture(conn, session_id),
            causal_owner: RuntimeCommandCausalOwner::capture(conn, session_id)?,
        })
    }
}

impl RuntimeCommandOutputBarriers {
    pub fn admit(
        &mut self,
        conn: &CdpConnection,
        command_id: u64,
        session_id: Option<&str>,
    ) -> Option<RuntimeCommandOutputBarrierPermit> {
        let command_scope = CommandOwnerScope::capture(conn, session_id);
        let causal_owner = RuntimeCommandCausalOwner::capture(conn, session_id)?;
        // V8 Inspector commands use a session-local renderer call id. It is
        // deliberately independent from the frontend CDP request id, which may
        // be large or reused by another session. Admission therefore happens
        // only after dispatch has registered and rewritten the exact call.
        let renderer_cause =
            conn.renderer_runtime_command_cause_for_frontend(session_id, command_id)?;
        assert!(
            !self.active.values().any(|barrier| {
                barrier.causal_owner == causal_owner && barrier.renderer_cause == renderer_cause
            }),
            "an exact renderer Runtime command cause must have only one active response barrier"
        );
        let id = self.next_barrier_id;
        self.next_barrier_id = self.next_barrier_id.checked_next();
        let replaced = self.active.insert(
            id,
            ActiveRuntimeCommandOutputBarrier {
                command_id,
                command_scope,
                causal_owner,
                renderer_cause,
            },
        );
        assert!(
            replaced.is_none(),
            "Runtime command barrier ids must not be reused while active"
        );
        Some(RuntimeCommandOutputBarrierPermit { id, command_id })
    }

    pub(super) async fn route_publication_outputs(
        &mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        renderer_cause: Option<&RendererRuntimeCommandCausalIdentity>,
        renderer_output_cursor: Option<RendererOutputCursor>,
        mut outputs: PreparedProtocolOutputs,
        command_context: &mut CommandDispatchContext,
    ) {
        let Some(route) = RuntimeCommandOutputRoute::capture(conn, session_id) else {
            outputs
                .project_async(conn, session_id, command_context)
                .await;
            return;
        };
        let prepared_cause = outputs
            .top_level_location_navigation_runtime_command_cause()
            .cloned();
        let Some(renderer_cause) = prepared_cause.as_ref().or(renderer_cause) else {
            outputs
                .project_async(conn, session_id, command_context)
                .await;
            return;
        };
        let matching: Vec<_> = self
            .active
            .iter()
            .filter_map(|(id, barrier)| {
                (barrier.causal_owner == route.causal_owner
                    && &barrier.renderer_cause == renderer_cause)
                    .then_some(*id)
            })
            .collect();
        let [barrier_id] = matching.as_slice() else {
            assert!(
                matching.is_empty(),
                "one renderer Runtime command cause cannot match multiple active barriers"
            );
            outputs
                .project_async(conn, session_id, command_context)
                .await;
            return;
        };
        let causal_outputs = if prepared_cause.as_ref() == Some(renderer_cause) {
            let causal_outputs = outputs
                .take_top_level_location_navigation_for_runtime_command(renderer_cause)
                .expect("a prepared Runtime-command navigation cause must retain its exact action");
            outputs
                .project_async(conn, session_id, command_context)
                .await;
            causal_outputs
        } else {
            outputs
        };
        self.hold_for_exact_barrier(
            conn,
            session_id,
            route,
            *barrier_id,
            renderer_output_cursor,
            causal_outputs,
            command_context,
        )
        .await;
    }

    async fn hold_for_exact_barrier(
        &mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        route: RuntimeCommandOutputRoute,
        barrier_id: RuntimeCommandOutputBarrierId,
        renderer_output_cursor: Option<RendererOutputCursor>,
        outputs: PreparedProtocolOutputs,
        command_context: &mut CommandDispatchContext,
    ) {
        let Some(outputs) = outputs
            .project_before_command_response_and_hold_after(conn, session_id, command_context)
            .await
        else {
            return;
        };
        let id = self.next_hold_id;
        self.next_hold_id = self.next_hold_id.checked_next();
        let held = HeldRuntimeCommandOutput {
            id,
            renderer_output_cursor,
            predecessors: BTreeSet::from([barrier_id]),
            release_mode: HeldOutputReleaseMode::All,
            outputs,
        };
        if let Some(existing) = self
            .held_routes
            .iter_mut()
            .find(|existing| existing.route == route)
        {
            existing.outputs.push_back(held);
        } else {
            self.held_routes.push(HeldRuntimeCommandOutputRoute {
                route,
                outputs: VecDeque::from([held]),
            });
        }
    }

    pub(crate) async fn release(
        &mut self,
        conn: &mut CdpConnection,
        permit: RuntimeCommandOutputBarrierPermit,
        command_context: &mut CommandDispatchContext,
    ) -> RuntimeCommandOutputBarrierTerminal {
        let (barrier_id, barrier) = self.consume_permit(permit);
        let terminal = if self.command_owner_is_current(conn, &barrier) {
            RuntimeCommandOutputBarrierTerminal::Released
        } else {
            RuntimeCommandOutputBarrierTerminal::Superseded
        };
        self.finish_predecessor(conn, barrier_id, barrier, terminal, command_context)
            .await;
        terminal
    }

    pub(crate) async fn cancel(
        &mut self,
        conn: &mut CdpConnection,
        permit: RuntimeCommandOutputBarrierPermit,
        command_context: &mut CommandDispatchContext,
    ) -> RuntimeCommandOutputBarrierTerminal {
        let (barrier_id, barrier) = self.consume_permit(permit);
        let terminal = RuntimeCommandOutputBarrierTerminal::Canceled;
        self.finish_predecessor(conn, barrier_id, barrier, terminal, command_context)
            .await;
        terminal
    }

    fn consume_permit(
        &mut self,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> (
        RuntimeCommandOutputBarrierId,
        ActiveRuntimeCommandOutputBarrier,
    ) {
        let barrier = self
            .active
            .remove(&permit.id)
            .expect("Runtime command output barrier permit must name one active barrier");
        assert_eq!(
            barrier.command_id, permit.command_id,
            "Runtime command output barrier permit must retain its exact command identity"
        );
        (permit.id, barrier)
    }

    fn command_owner_is_current(
        &self,
        conn: &mut CdpConnection,
        barrier: &ActiveRuntimeCommandOutputBarrier,
    ) -> bool {
        let mut scope = barrier.command_scope.enter(conn);
        RuntimeCommandCausalOwner::capture(scope.conn_mut(), barrier.command_scope.session_id())
            .as_ref()
            == Some(&barrier.causal_owner)
    }

    async fn finish_predecessor(
        &mut self,
        conn: &mut CdpConnection,
        barrier_id: RuntimeCommandOutputBarrierId,
        barrier: ActiveRuntimeCommandOutputBarrier,
        terminal: RuntimeCommandOutputBarrierTerminal,
        command_context: &mut CommandDispatchContext,
    ) {
        for route in &mut self.held_routes {
            if route.route.causal_owner != barrier.causal_owner {
                continue;
            }
            for held in &mut route.outputs {
                let was_predecessor = held.predecessors.remove(&barrier_id);
                if was_predecessor && terminal != RuntimeCommandOutputBarrierTerminal::Released {
                    held.release_mode = HeldOutputReleaseMode::OwnerActionsOnly;
                }
            }
        }
        self.project_ready_outputs(conn, command_context).await;
    }

    async fn project_ready_outputs(
        &mut self,
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
    ) {
        let mut route_index = 0;
        while route_index < self.held_routes.len() {
            loop {
                let ready = self.held_routes[route_index]
                    .outputs
                    .front()
                    .is_some_and(|held| held.predecessors.is_empty());
                if !ready {
                    break;
                }
                let held = self.held_routes[route_index]
                    .outputs
                    .pop_front()
                    .expect("ready held output came from the route front");
                let route = self.held_routes[route_index].route.clone();
                let mut scope = route.delivery_scope.enter(conn);
                let conn = scope.conn_mut();
                if moli_trace::cdp_runtime_trace_enabled() {
                    tracing::info!(
                        target: "moli_cdp_runtime",
                        stage = "runtime_command_barrier_output_release",
                        hold_id = held.id.0,
                        renderer_output_stream_epoch = ?held.renderer_output_cursor.map(|cursor| cursor.stream().epoch().get()),
                        renderer_output_sequence = ?held.renderer_output_cursor.map(RendererOutputCursor::sequence),
                        release_mode = ?held.release_mode,
                    );
                }
                match held.release_mode {
                    HeldOutputReleaseMode::All => {
                        held.outputs
                            .project_async(conn, route.delivery_scope.session_id(), command_context)
                            .await;
                    }
                    HeldOutputReleaseMode::OwnerActionsOnly => {
                        held.outputs
                            .project_owner_actions_async(
                                conn,
                                route.delivery_scope.session_id(),
                                command_context,
                            )
                            .await;
                    }
                }
            }
            if self.held_routes[route_index].outputs.is_empty() {
                self.held_routes.remove(route_index);
            } else {
                route_index += 1;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn active_count(&self) -> usize {
        self.active.len()
    }

    #[cfg(test)]
    pub(super) fn held_output_count(&self) -> usize {
        self.held_routes
            .iter()
            .map(|route| route.outputs.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{
        RendererOwnerAction, RendererProtocolObservation, RendererRuntimeCommandCausalIdentity,
        page::{
            DevToolsSessionKey, RendererDocumentLifecycleIdentity,
            RendererDocumentSourcedSameDocumentNavigation,
            RendererDocumentSourcedTopLevelLocationNavigation, RendererDocumentToken,
            RendererFrameToken, RendererLifecycleEpoch, RendererPendingSameDocumentNavigation,
            RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
            SameDocumentHistoryUpdate,
        },
    };
    use serde_json::Value;

    use crate::conn::{
        BrowserContext, CdpConnection, CommandDispatchContext, RendererCommandDescriptor,
    };

    use super::{
        RuntimeCommandOutputBarrierPermit, RuntimeCommandOutputBarrierTerminal,
        RuntimeCommandOutputBarriers,
    };

    const SESSION_ID: &str = "SID-runtime-command-barrier";

    async fn connection_with_loaded_page() -> CdpConnection {
        let mut conn = CdpConnection::new();
        let page = conn
            .load_page_via_runtime_async("data:text/html,<title>runtime-command-barrier</title>")
            .await
            .expect("barrier test page should load");
        let mut browser_context = BrowserContext::new("BID-runtime-command-barrier".to_owned());
        browser_context.set_active_target_id("TID-runtime-command-barrier");
        browser_context.attach_active_session(SESSION_ID);
        browser_context.set_target_url(page.final_url().as_str().to_owned());
        let _ = browser_context
            .active_target
            .runtime_slot
            .replace_loaded_page(Some(page));
        conn.browser_context = Some(browser_context);
        conn
    }

    /// Returns the initial Document identity used by this minimal protocol
    /// fixture. The fixture installs a loaded Page directly, so it has no
    /// separate protocol-side root-Document binding to query.
    fn loaded_page_source_document(conn: &CdpConnection) -> RendererDocumentLifecycleIdentity {
        let page_id = conn
            .runtime_session_owner_slot(Some(SESSION_ID))
            .expect("loaded Page should retain its runtime slot")
            .loaded_page()
            .expect("runtime slot should retain the loaded Page")
            .renderer_page_id();
        RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, 1),
            epoch: RendererLifecycleEpoch(1),
        }
    }

    fn admit_registered_command(
        conn: &mut CdpConnection,
        barriers: &mut RuntimeCommandOutputBarriers,
        frontend_command_id: u64,
    ) -> RuntimeCommandOutputBarrierPermit {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(
            serde_json::json!({
                "id": frontend_command_id,
                "method": "Runtime.evaluate",
                "params": { "expression": "1" },
            })
            .to_string(),
        )
        .expect("test Runtime command descriptor should be valid");
        conn.try_register_renderer_call_for_session_owner(
            Some(SESSION_ID),
            frontend_command_id,
            None,
            descriptor,
        )
        .expect("test Runtime command should register its renderer call");
        barriers
            .admit(conn, frontend_command_id, Some(SESSION_ID))
            .expect("registered Runtime command should admit an exact output barrier")
    }

    fn renderer_cause_for_permit(
        conn: &CdpConnection,
        permit: &RuntimeCommandOutputBarrierPermit,
    ) -> RendererRuntimeCommandCausalIdentity {
        conn.renderer_runtime_command_cause_for_frontend(Some(SESSION_ID), permit.command_id)
            .expect("active test command should retain its exact renderer call identity")
    }

    async fn route_same_document_navigation(
        conn: &mut CdpConnection,
        barriers: &mut RuntimeCommandOutputBarriers,
        permit: Option<&super::RuntimeCommandOutputBarrierPermit>,
        fragment: &str,
        command_context: &mut CommandDispatchContext,
    ) {
        let source_document = loaded_page_source_document(conn);
        let outputs =
            super::super::output_ingress::PreparedProtocolOutputs::from_renderer_owner_action(
                conn,
                Some(SESSION_ID),
                RendererOwnerAction::SameDocumentNavigation(
                    RendererDocumentSourcedSameDocumentNavigation::new(
                        source_document,
                        RendererPendingSameDocumentNavigation {
                            url: format!("data:text/html,runtime-command-barrier#{fragment}"),
                            navigation_type: "fragment".to_owned(),
                            history_update: SameDocumentHistoryUpdate::Push,
                        },
                    ),
                ),
            )
            .await;
        let renderer_cause = permit.map(|permit| renderer_cause_for_permit(conn, permit));
        barriers
            .route_publication_outputs(
                conn,
                Some(SESSION_ID),
                renderer_cause.as_ref(),
                None,
                outputs,
                command_context,
            )
            .await;
    }

    async fn route_top_level_navigation_for_command(
        conn: &mut CdpConnection,
        barriers: &mut RuntimeCommandOutputBarriers,
        permit: &super::RuntimeCommandOutputBarrierPermit,
        marker: &str,
        command_context: &mut CommandDispatchContext,
    ) {
        let cause = renderer_cause_for_permit(conn, permit);
        let owner = conn
            .target_page_residence_identity_for_session(Some(SESSION_ID))
            .expect("loaded Page should expose its exact residence");
        let source_document = loaded_page_source_document(conn);
        let outputs =
            super::super::output_ingress::PreparedProtocolOutputs::from_top_level_location_navigation_for_test(
                owner,
                RendererDocumentSourcedTopLevelLocationNavigation::new_with_runtime_command_cause(
                    source_document,
                    format!("data:text/html,{marker}"),
                    Some(cause),
                ),
            );

        barriers
            .route_publication_outputs(conn, Some(SESSION_ID), None, None, outputs, command_context)
            .await;
    }

    fn protocol_messages(command_context: &mut CommandDispatchContext) -> Vec<Value> {
        command_context
            .take_protocol_events()
            .into_iter()
            .map(|event| event.into_protocol_message())
            .collect()
    }

    fn contains_same_document_navigation(messages: &[Value]) -> bool {
        messages
            .iter()
            .any(|message| message["method"] == "Page.navigatedWithinDocument")
    }

    fn contains_top_level_navigation(messages: &[Value]) -> bool {
        messages
            .iter()
            .any(|message| message["method"] == "Page.frameStartedNavigating")
    }

    async fn complete_published_top_level_navigation(
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
    ) {
        let [event]: [crate::conn::CdpSchedulerEvent; 1] = conn
            .take_scheduler_events()
            .try_into()
            .expect("barrier release should publish one concrete navigation action");
        let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
            panic!("barrier release must publish concrete protocol work");
        };
        assert!(work.is_top_level_location_navigation_owner_action());
        let (events, nested_scheduler_events) = conn
            .complete_ready_protocol_scheduler_work_turn(work)
            .await
            .into_protocol_event_parts();
        assert!(
            !nested_scheduler_events.iter().any(|event| {
                matches!(
                    event,
                    crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                        if work.is_top_level_location_navigation_owner_action()
                )
            }),
            "executing the concrete navigation must not republish its own owner action"
        );
        command_context.protocol_events_mut().extend(events);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_document_navigation_projects_before_exact_runtime_command_response() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 1);
        let mut command_context = CommandDispatchContext::default();

        route_same_document_navigation(
            &mut conn,
            &mut barriers,
            Some(&permit),
            "held",
            &mut command_context,
        )
        .await;

        assert_eq!(barriers.active_count(), 1);
        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            contains_same_document_navigation(&protocol_messages(&mut command_context)),
            "the completed history mutation must be reported before the exact command response"
        );

        let terminal = barriers
            .release(&mut conn, permit, &mut command_context)
            .await;

        assert_eq!(terminal, RuntimeCommandOutputBarrierTerminal::Released);
        assert_eq!(barriers.active_count(), 0);
        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            !contains_same_document_navigation(&protocol_messages(&mut command_context)),
            "releasing the barrier must not deliver the same navigation twice"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn debugger_transition_messages_wait_for_the_exact_command_response() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 2);
        let cause = renderer_cause_for_permit(&conn, &permit);
        let agent_token = conn
            .runtime_session_owner_slot(Some(SESSION_ID))
            .expect("loaded Page should retain its runtime slot")
            .loaded_page()
            .expect("runtime slot should retain the loaded Page")
            .renderer_devtools_agent_token();
        let observation = RendererProtocolObservation::RuntimeInspector(
            RendererRuntimeInspectorMessageBatch::new_after_command_response(
                agent_token,
                DevToolsSessionKey::Primary,
                vec![
                    RendererRuntimeInspectorMessage::protocol(serde_json::json!({
                        "method": "Debugger.resumed",
                        "params": {},
                    })),
                    RendererRuntimeInspectorMessage::protocol(serde_json::json!({
                        "method": "Debugger.paused",
                        "params": {"callFrames": [], "reason": "step"},
                    })),
                ],
            ),
        );
        let outputs =
            super::super::output_ingress::PreparedProtocolOutputs::from_renderer_observation(
                &mut conn,
                Some(SESSION_ID),
                agent_token,
                &observation,
            );
        let mut command_context = CommandDispatchContext::default();

        barriers
            .route_publication_outputs(
                &mut conn,
                Some(SESSION_ID),
                Some(&cause),
                None,
                outputs,
                &mut command_context,
            )
            .await;

        assert_eq!(barriers.held_output_count(), 1);
        assert!(
            protocol_messages(&mut command_context).is_empty(),
            "Debugger transition output must not overtake the matching response"
        );

        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        let messages = protocol_messages(&mut command_context);
        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            vec!["Debugger.resumed", "Debugger.paused"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn output_is_owned_by_one_exact_command_not_every_command_on_the_page() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let first = admit_registered_command(&mut conn, &mut barriers, 11);
        let second = admit_registered_command(&mut conn, &mut barriers, 12);
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut conn,
            &mut barriers,
            &first,
            "exact-predecessor",
            &mut command_context,
        )
        .await;
        assert_eq!(barriers.held_output_count(), 1);

        assert_eq!(
            barriers
                .release(&mut conn, second, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(barriers.active_count(), 1);
        assert_eq!(barriers.held_output_count(), 1);
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "an unrelated command response must not release another command's output"
        );

        assert_eq!(
            barriers
                .release(&mut conn, first, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(barriers.active_count(), 0);
        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "barrier release must publish, not execute, its concrete owner action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "the exact command response should release its held output for one scheduler turn"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn canceling_an_unrelated_command_does_not_downgrade_held_output() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let first = admit_registered_command(&mut conn, &mut barriers, 15);
        let second = admit_registered_command(&mut conn, &mut barriers, 16);
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut conn,
            &mut barriers,
            &first,
            "exact-predecessor-survives-unrelated-cancel",
            &mut command_context,
        )
        .await;
        assert_eq!(barriers.held_output_count(), 1);

        assert_eq!(
            barriers
                .cancel(&mut conn, second, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Canceled
        );
        assert_eq!(barriers.active_count(), 1);
        assert_eq!(barriers.held_output_count(), 1);
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "canceling an unrelated command must neither release nor downgrade another command's output"
        );

        assert_eq!(
            barriers
                .release(&mut conn, first, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(barriers.active_count(), 0);
        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "barrier release must publish, not execute, the retained owner action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "the exact command must retain and release its full protocol output after an unrelated cancel"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unattributed_same_page_output_is_not_held_by_a_pending_command() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 13);
        let mut command_context = CommandDispatchContext::default();

        route_same_document_navigation(
            &mut conn,
            &mut barriers,
            None,
            "independent-page-task",
            &mut command_context,
        )
        .await;

        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            contains_same_document_navigation(&protocol_messages(&mut command_context)),
            "output without the exact command identity must remain visible while that command awaits"
        );
        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concrete_runtime_navigation_cause_survives_an_earlier_untagged_wake() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 14);
        let cause = renderer_cause_for_permit(&conn, &permit);
        let owner = conn
            .target_page_residence_identity_for_session(Some(SESSION_ID))
            .expect("loaded Page should expose its exact residence");
        let source_document = loaded_page_source_document(&conn);
        let outputs =
            super::super::output_ingress::PreparedProtocolOutputs::from_top_level_location_navigation_for_test(
                owner,
                RendererDocumentSourcedTopLevelLocationNavigation::new_with_runtime_command_cause(
                    source_document,
                    "data:text/html,held-by-concrete-command-cause".to_owned(),
                    Some(cause),
                ),
            );
        let mut command_context = CommandDispatchContext::default();

        barriers
            .route_publication_outputs(
                &mut conn,
                Some(SESSION_ID),
                None,
                None,
                outputs,
                &mut command_context,
            )
            .await;

        assert_eq!(barriers.held_output_count(), 1);
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "an older untagged wake must not release a concrete command-caused navigation"
        );

        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "barrier release must publish, not execute, the concrete navigation action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "the exact response permit must release the concrete navigation for its scheduler turn"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replacement_supersedes_old_barrier_without_holding_new_page_output() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 21);
        conn.runtime_session_owner_slot_mut(Some(SESSION_ID))
            .expect("runtime slot should remain installed")
            .replace_page_attachment_id_for_test();
        let mut command_context = CommandDispatchContext::default();

        route_same_document_navigation(
            &mut conn,
            &mut barriers,
            Some(&permit),
            "replacement",
            &mut command_context,
        )
        .await;

        assert_eq!(
            barriers.held_output_count(),
            0,
            "output from the replacement Page must not be held by an old Page barrier"
        );
        assert!(
            contains_same_document_navigation(&protocol_messages(&mut command_context)),
            "one concrete replacement-Page record must project immediately instead of being \
             recaptured through the old command"
        );
        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Superseded
        );
        assert_eq!(barriers.active_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replacement_after_capture_retires_held_old_page_output() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 30);
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut conn,
            &mut barriers,
            &permit,
            "retired-source-page",
            &mut command_context,
        )
        .await;
        assert_eq!(barriers.held_output_count(), 1);

        conn.runtime_session_owner_slot_mut(Some(SESSION_ID))
            .expect("runtime slot should remain installed")
            .replace_page_attachment_id_for_test();
        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Superseded
        );
        assert_eq!(barriers.held_output_count(), 0);
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "a held old-Page navigation action must not project into the replacement Page"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_cancel_consumes_the_barrier_once() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 31);
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut conn,
            &mut barriers,
            &permit,
            "canceled-protocol-command",
            &mut command_context,
        )
        .await;
        assert_eq!(barriers.held_output_count(), 1);
        assert_eq!(
            barriers
                .cancel(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Canceled
        );
        assert_eq!(barriers.active_count(), 0);
        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            !contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "cancel must publish, not execute, the already-produced owner action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&protocol_messages(&mut command_context)),
            "canceling the protocol command must still settle its already-produced browser-owner action"
        );
    }
}
