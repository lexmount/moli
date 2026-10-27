use std::collections::{BTreeMap, VecDeque};

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
    predecessor: Option<RuntimeCommandOutputBarrierId>,
    release_mode: HeldOutputReleaseMode,
    // None retains this exact FIFO position while its before-response
    // projection runs outside state. A released predecessor alone cannot
    // make that unfinished projection ready.
    outputs: Option<PreparedProtocolOutputs>,
}

pub(crate) struct PreparedPublicationOutputProjection {
    pub(super) prefix: Option<PreparedProtocolOutputs>,
    pub(super) outputs: PreparedProtocolOutputs,
    pub(super) owner: CommandOwnerScope,
    pub(super) hold: Option<RuntimeCommandBeforeResponseProjection>,
}

/// The before-response projection may fill only its own reserved FIFO slot.
/// Barrier release/cancellation can proceed while it waits, retaining the
/// original release mode rather than reconstructing it from current state.
pub(crate) struct RuntimeCommandBeforeResponseProjection {
    hold_id: RuntimeCommandOutputHoldId,
}

pub(crate) struct ReadyRuntimeCommandOutputProjection {
    permit: RuntimeCommandOutputProjectionPermit,
    renderer_output_cursor: Option<RendererOutputCursor>,
    pub(super) owner: CommandOwnerScope,
    pub(super) outputs: PreparedProtocolOutputs,
    pub(super) owner_actions_only: bool,
}

pub(crate) struct RuntimeCommandOutputProjectionPermit {
    hold_id: RuntimeCommandOutputHoldId,
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

    fn capture_for_scope(conn: &CdpConnection, scope: &CommandOwnerScope) -> Option<Self> {
        if let Some(page) = conn.target_page_residence_identity_for_owner(scope) {
            return Some(Self::Page(page));
        }
        let (browser_context_id, target_id) = conn.target_owner_identity_for_owner(scope)?;
        Some(Self::Target {
            browser_context_id,
            target_id,
        })
    }
}

impl RuntimeCommandOutputRoute {
    fn capture_for_scope(conn: &CdpConnection, delivery_scope: CommandOwnerScope) -> Option<Self> {
        Some(Self {
            causal_owner: RuntimeCommandCausalOwner::capture_for_scope(conn, &delivery_scope)?,
            delivery_scope,
        })
    }
}

impl RuntimeCommandOutputBarriers {
    pub(super) fn prepare_publication_projection(
        &mut self,
        conn: &CdpConnection,
        owner: CommandOwnerScope,
        renderer_cause: Option<&RendererRuntimeCommandCausalIdentity>,
        cursor: Option<RendererOutputCursor>,
        mut outputs: PreparedProtocolOutputs,
    ) -> PreparedPublicationOutputProjection {
        let unheld = |owner, outputs| PreparedPublicationOutputProjection {
            prefix: None,
            outputs,
            owner,
            hold: None,
        };
        let Some(route) = RuntimeCommandOutputRoute::capture_for_scope(conn, owner.clone()) else {
            return unheld(owner, outputs);
        };
        let prepared_cause = outputs
            .top_level_location_navigation_runtime_command_cause()
            .cloned();
        let Some(cause) = prepared_cause.as_ref().or(renderer_cause) else {
            return unheld(owner, outputs);
        };
        let mut matching = self.active.iter().filter_map(|(id, barrier)| {
            (barrier.causal_owner == route.causal_owner && &barrier.renderer_cause == cause)
                .then_some(*id)
        });
        let Some(barrier_id) = matching.next() else {
            return unheld(owner, outputs);
        };
        assert!(
            matching.next().is_none(),
            "one Runtime cause has one output barrier"
        );
        let (prefix, outputs) = if prepared_cause.as_ref() == Some(cause) {
            let causal = outputs
                .take_top_level_location_navigation_for_runtime_command(cause)
                .expect("a prepared navigation retains its original causal action");
            (Some(outputs), causal)
        } else {
            (None, outputs)
        };
        let hold = outputs.has_after_response_slots().then(|| {
            let hold_id = self.next_hold_id;
            self.next_hold_id = self.next_hold_id.checked_next();
            self.push_held_output(
                route.clone(),
                HeldRuntimeCommandOutput {
                    id: hold_id,
                    renderer_output_cursor: cursor,
                    predecessor: Some(barrier_id),
                    release_mode: HeldOutputReleaseMode::All,
                    outputs: None,
                },
            );
            RuntimeCommandBeforeResponseProjection { hold_id }
        });
        PreparedPublicationOutputProjection {
            prefix,
            outputs,
            owner: route.delivery_scope,
            hold,
        }
    }

    pub(super) fn complete_before_response_projection(
        &mut self,
        permit: RuntimeCommandBeforeResponseProjection,
        outputs: PreparedProtocolOutputs,
    ) -> Vec<ReadyRuntimeCommandOutputProjection> {
        let held = self
            .held_routes
            .iter_mut()
            .flat_map(|route| &mut route.outputs)
            .find(|held| held.id == permit.hold_id)
            .expect("the original before-response projection retains its FIFO slot");
        assert!(
            held.outputs.is_none(),
            "a projection fills its reserved slot once"
        );
        held.outputs = Some(outputs);
        self.take_ready_output_projections()
    }

    fn push_held_output(
        &mut self,
        route: RuntimeCommandOutputRoute,
        held: HeldRuntimeCommandOutput,
    ) {
        if let Some(existing) = self
            .held_routes
            .iter_mut()
            .find(|entry| entry.route == route)
        {
            existing.outputs.push_back(held);
        } else {
            self.held_routes.push(HeldRuntimeCommandOutputRoute {
                route,
                outputs: VecDeque::from([held]),
            });
        }
    }

    fn take_ready_output_projections(&mut self) -> Vec<ReadyRuntimeCommandOutputProjection> {
        let mut ready = Vec::new();
        for route in &mut self.held_routes {
            if let Some(held) = route.outputs.front_mut()
                && held.predecessor.is_none()
                && let Some(outputs) = held.outputs.take()
            {
                ready.push(ReadyRuntimeCommandOutputProjection {
                    permit: RuntimeCommandOutputProjectionPermit { hold_id: held.id },
                    renderer_output_cursor: held.renderer_output_cursor,
                    owner: route.route.delivery_scope.clone(),
                    outputs,
                    owner_actions_only: held.release_mode
                        == HeldOutputReleaseMode::OwnerActionsOnly,
                });
            }
        }
        ready
    }

    pub(crate) fn complete_output_projection(
        &mut self,
        permit: RuntimeCommandOutputProjectionPermit,
    ) -> Vec<ReadyRuntimeCommandOutputProjection> {
        let route = self
            .held_routes
            .iter_mut()
            .find(|route| {
                route
                    .outputs
                    .front()
                    .is_some_and(|held| held.id == permit.hold_id)
            })
            .expect("a running projection retains its original route front");
        let held = route
            .outputs
            .pop_front()
            .expect("the running route front exists");
        assert!(
            held.predecessor.is_none() && held.outputs.is_none(),
            "only the original released projection finishes its route front"
        );
        self.held_routes.retain(|route| !route.outputs.is_empty());
        self.take_ready_output_projections()
    }

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
        owner: &CommandOwnerScope,
        cause: Option<&RendererRuntimeCommandCausalIdentity>,
        cursor: Option<RendererOutputCursor>,
        outputs: PreparedProtocolOutputs,
        context: &mut CommandDispatchContext,
    ) {
        let prepared =
            self.prepare_publication_projection(conn, owner.clone(), cause, cursor, outputs);
        if let Some(prefix) = prepared.prefix {
            prefix.project_async(conn, &prepared.owner, context).await;
        }
        if let Some(hold) = prepared.hold {
            let after = prepared
                .outputs
                .project_before_command_response_and_hold_after(conn, &prepared.owner, context)
                .await
                .expect("a reserved projection retains its after-response slots");
            let ready = self.complete_before_response_projection(hold, after);
            self.project_ready_batches(conn, context, ready).await;
        } else {
            prepared
                .outputs
                .project_async(conn, &prepared.owner, context)
                .await;
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
        conn: &CdpConnection,
        barrier: &ActiveRuntimeCommandOutputBarrier,
    ) -> bool {
        RuntimeCommandCausalOwner::capture_for_scope(conn, &barrier.command_scope).as_ref()
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
                if held.predecessor == Some(barrier_id) {
                    held.predecessor = None;
                    if terminal != RuntimeCommandOutputBarrierTerminal::Released {
                        held.release_mode = HeldOutputReleaseMode::OwnerActionsOnly;
                    }
                }
            }
        }
        self.project_ready_outputs(conn, command_context).await;
    }

    async fn project_ready_outputs(
        &mut self,
        conn: &mut CdpConnection,
        context: &mut CommandDispatchContext,
    ) {
        let ready = self.take_ready_output_projections();
        self.project_ready_batches(conn, context, ready).await;
    }

    async fn project_ready_batches(
        &mut self,
        conn: &mut CdpConnection,
        context: &mut CommandDispatchContext,
        ready: Vec<ReadyRuntimeCommandOutputProjection>,
    ) {
        let mut ready = VecDeque::from(ready);
        while let Some(projection) = ready.pop_front() {
            if moli_trace::cdp_runtime_trace_enabled() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "runtime_command_barrier_output_release",
                    hold_id = projection.permit.hold_id.0,
                    renderer_output_stream_epoch = ?projection.renderer_output_cursor.map(|cursor| cursor.stream().epoch().get()),
                    renderer_output_sequence = ?projection.renderer_output_cursor.map(RendererOutputCursor::sequence),
                    owner_actions_only = projection.owner_actions_only,
                );
            }
            if projection.owner_actions_only {
                projection
                    .outputs
                    .project_owner_actions_async(conn, &projection.owner, context)
                    .await;
            } else {
                projection
                    .outputs
                    .project_async(conn, &projection.owner, context)
                    .await;
            }
            ready.extend(self.complete_output_projection(projection.permit));
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
        RendererOutputResidenceIdentity, RendererOwnerAction, RendererProtocolObservation,
        RendererRuntimeCommandCausalIdentity,
        page::{
            DevToolsSessionKey, RendererDocumentLifecycleIdentity,
            RendererDocumentSourcedSameDocumentNavigation,
            RendererDocumentSourcedTopLevelLocationNavigation, RendererDocumentToken,
            RendererFrameToken, RendererLifecycleEpoch, RendererPendingSameDocumentNavigation,
            RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
        },
    };

    use crate::conn::{
        BrowserContext, CdpConnection, CommandDispatchContext, CommandOwnerScope,
        RendererCommandDescriptor,
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
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        conn.install_browser_context_fixture_for_test(browser_context);
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
        let command_owner = CommandOwnerScope::for_session(SESSION_ID);
        let source_document = loaded_page_source_document(conn);
        let outputs =
            super::super::output_ingress::PreparedProtocolOutputs::from_renderer_owner_action(
                conn,
                &command_owner,
                RendererOwnerAction::SameDocumentNavigation(
                    RendererDocumentSourcedSameDocumentNavigation::new(
                        source_document,
                        RendererPendingSameDocumentNavigation {
                            url: format!("data:text/html,runtime-command-barrier#{fragment}"),
                            navigation_type: "fragment".to_owned(),
                        },
                    ),
                ),
            )
            .await;
        let renderer_cause = permit.map(|permit| renderer_cause_for_permit(conn, permit));
        barriers
            .route_publication_outputs(
                conn,
                &command_owner,
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
        let command_owner = CommandOwnerScope::for_session(SESSION_ID);
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
            .route_publication_outputs(conn, &command_owner, None, None, outputs, command_context)
            .await;
    }

    fn produced_events(
        command_context: &mut CommandDispatchContext,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        command_context.take_protocol_events()
    }

    fn contains_same_document_navigation(events: &[crate::conn::BackgroundProtocolEvent]) -> bool {
        events.iter().cloned().any(|event| {
            matches!(
                event.into_parts().1,
                Some(crate::automation::AutomationEvent::SameDocumentNavigation(
                    _
                ))
            )
        })
    }

    fn contains_top_level_navigation(events: &[crate::conn::BackgroundProtocolEvent]) -> bool {
        events.iter().cloned().any(|event| matches!(event.into_parts().1, Some(crate::automation::AutomationEvent::NavigationFrame(frame))
            if frame.kind == crate::automation::NavigationFrameEventKind::StartedNavigating))
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
            contains_same_document_navigation(&produced_events(&mut command_context)),
            "the completed history mutation must be reported before the exact command response"
        );

        let terminal = barriers
            .release(&mut conn, permit, &mut command_context)
            .await;

        assert_eq!(terminal, RuntimeCommandOutputBarrierTerminal::Released);
        assert_eq!(barriers.active_count(), 0);
        assert_eq!(barriers.held_output_count(), 0);
        assert!(
            !contains_same_document_navigation(&produced_events(&mut command_context)),
            "releasing the barrier must not deliver the same navigation twice"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn debugger_transition_messages_wait_for_the_exact_command_response() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let permit = admit_registered_command(&mut conn, &mut barriers, 2);
        let cause = renderer_cause_for_permit(&conn, &permit);
        let page = conn
            .runtime_session_owner_slot(Some(SESSION_ID))
            .expect("loaded Page should retain its runtime slot")
            .loaded_page()
            .expect("runtime slot should retain the loaded Page");
        let agent_token = page.renderer_devtools_agent_token();
        let source_residence = RendererOutputResidenceIdentity::Page {
            owner_local_host_id: page.renderer_owner_local_host_id(),
            page_id: page.renderer_page_id(),
        };
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
                &crate::conn::CommandOwnerScope::for_session(SESSION_ID),
                source_residence,
                &observation,
            );
        let mut command_context = CommandDispatchContext::default();
        let command_owner = CommandOwnerScope::for_session(SESSION_ID);

        barriers
            .route_publication_outputs(
                &mut conn,
                &command_owner,
                Some(&cause),
                None,
                outputs,
                &mut command_context,
            )
            .await;

        assert_eq!(barriers.held_output_count(), 1);
        assert!(
            produced_events(&mut command_context).is_empty(),
            "Debugger transition output must not overtake the matching response"
        );

        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        let messages = produced_events(&mut command_context);
        assert_eq!(
            messages
                .iter()
                .filter_map(crate::conn::BackgroundProtocolEvent::protocol_method)
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
            !contains_top_level_navigation(&produced_events(&mut command_context)),
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
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "barrier release must publish, not execute, its concrete owner action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
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
            !contains_top_level_navigation(&produced_events(&mut command_context)),
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
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "barrier release must publish, not execute, the retained owner action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
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
            contains_same_document_navigation(&produced_events(&mut command_context)),
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
        let command_owner = CommandOwnerScope::for_session(SESSION_ID);

        barriers
            .route_publication_outputs(
                &mut conn,
                &command_owner,
                None,
                None,
                outputs,
                &mut command_context,
            )
            .await;

        assert_eq!(barriers.held_output_count(), 1);
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "an older untagged wake must not release a concrete command-caused navigation"
        );

        assert_eq!(
            barriers
                .release(&mut conn, permit, &mut command_context)
                .await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "barrier release must publish, not execute, the concrete navigation action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
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
            contains_same_document_navigation(&produced_events(&mut command_context)),
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
            !contains_top_level_navigation(&produced_events(&mut command_context)),
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
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "cancel must publish, not execute, the already-produced owner action"
        );
        complete_published_top_level_navigation(&mut conn, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
            "canceling the protocol command must still settle its already-produced browser-owner action"
        );
    }
    fn reserve_navigation_projection(
        conn: &CdpConnection,
        barriers: &mut RuntimeCommandOutputBarriers,
        permit: &RuntimeCommandOutputBarrierPermit,
        marker: &str,
    ) -> super::PreparedPublicationOutputProjection {
        let cause = renderer_cause_for_permit(conn, permit);
        let owner = conn
            .target_page_residence_identity_for_session(Some(SESSION_ID))
            .unwrap();
        let outputs = super::super::output_ingress::PreparedProtocolOutputs::from_top_level_location_navigation_for_test(
            owner,
            RendererDocumentSourcedTopLevelLocationNavigation::new_with_runtime_command_cause(
                loaded_page_source_document(conn), format!("data:text/html,{marker}"), Some(cause),
            ),
        );
        barriers.prepare_publication_projection(
            conn,
            CommandOwnerScope::for_session(SESSION_ID),
            None,
            None,
            outputs,
        )
    }

    #[tokio::test]
    async fn response_release_during_before_projection_retains_its_original_terminal() {
        for canceled in [false, true] {
            let mut conn = connection_with_loaded_page().await;
            let mut barriers = RuntimeCommandOutputBarriers::default();
            let permit = admit_registered_command(&mut conn, &mut barriers, 71);
            let projection = reserve_navigation_projection(
                &conn,
                &mut barriers,
                &permit,
                "projection-held-before-response",
            );
            let mut context = CommandDispatchContext::default();
            let terminal = if canceled {
                barriers.cancel(&mut conn, permit, &mut context).await
            } else {
                barriers.release(&mut conn, permit, &mut context).await
            };
            assert_eq!(
                terminal,
                if canceled {
                    RuntimeCommandOutputBarrierTerminal::Canceled
                } else {
                    RuntimeCommandOutputBarrierTerminal::Released
                }
            );
            assert!(
                conn.take_scheduler_events().is_empty(),
                "a response terminal cannot release an unfinished before-response projection"
            );
            let ready = barriers
                .complete_before_response_projection(projection.hold.unwrap(), projection.outputs);
            assert_eq!(ready.len(), 1);
            assert_eq!(ready[0].owner_actions_only, canceled);
            assert_eq!(
                barriers.held_output_count(),
                1,
                "the running projection retains its route front"
            );
            barriers
                .project_ready_batches(&mut conn, &mut context, ready)
                .await;
            assert_eq!(barriers.held_output_count(), 0);
            let events = conn.take_scheduler_events();
            assert_eq!(
                events.len(),
                1,
                "the original owner continuation is published exactly once"
            );
            assert!(
                matches!(&events[0], crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } if work.is_top_level_location_navigation_owner_action())
            );
        }
    }

    #[tokio::test]
    async fn later_same_route_output_cannot_overtake_a_running_projection() {
        let mut conn = connection_with_loaded_page().await;
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let first = admit_registered_command(&mut conn, &mut barriers, 81);
        let second = admit_registered_command(&mut conn, &mut barriers, 82);
        let first_projection =
            reserve_navigation_projection(&conn, &mut barriers, &first, "projection-first");
        let second_projection =
            reserve_navigation_projection(&conn, &mut barriers, &second, "projection-second");
        let mut context = CommandDispatchContext::default();
        assert!(
            barriers
                .complete_before_response_projection(
                    second_projection.hold.unwrap(),
                    second_projection.outputs
                )
                .is_empty()
        );
        barriers.release(&mut conn, second, &mut context).await;
        barriers.release(&mut conn, first, &mut context).await;
        assert!(conn.take_scheduler_events().is_empty());
        let first_ready = barriers.complete_before_response_projection(
            first_projection.hold.unwrap(),
            first_projection.outputs,
        );
        assert_eq!(first_ready.len(), 1);
        assert!(
            barriers.take_ready_output_projections().is_empty(),
            "yielding during the first projection must keep the second resident"
        );
        assert_eq!(barriers.held_output_count(), 2);
        barriers
            .project_ready_batches(&mut conn, &mut context, first_ready)
            .await;
        let urls = conn.take_scheduler_events().into_iter().map(|event| {
            let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else { panic!("navigation owner action"); };
            let super::super::scheduler_work::ReadyProtocolSchedulerWork::TopLevelLocationNavigationOwnerAction(action) = work.into_ready() else { panic!("original navigation owner action"); };
            action.into_parts().2.url().to_owned()
        }).collect::<Vec<_>>();
        assert_eq!(
            urls,
            [
                "data:text/html,projection-first",
                "data:text/html,projection-second"
            ]
        );
        assert_eq!(barriers.held_output_count(), 0);
    }

    #[tokio::test]
    async fn finite_connection_access_restores_the_original_barriers_after_panic() {
        let mut conn = connection_with_loaded_page().await;
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(
            serde_json::json!({"id":91,"method":"Runtime.evaluate","params":{"expression":"1"}})
                .to_string(),
        )
        .unwrap();
        conn.try_register_renderer_call_for_session_owner(Some(SESSION_ID), 91, None, descriptor)
            .unwrap();
        let permit = conn
            .admit_runtime_command_output_barrier(91, Some(SESSION_ID))
            .unwrap();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            conn.with_runtime_command_output_barriers::<()>(|_, barriers| {
                assert_eq!(barriers.active_count(), 1);
                panic!("controlled finite barrier operation failure");
            });
        }));
        assert!(failed.is_err());
        assert_eq!(
            conn.with_runtime_command_output_barriers(|_, barriers| barriers.active_count()),
            1
        );
        assert_eq!(
            conn.release_runtime_command_output_barrier_turn_async(permit)
                .await
                .terminal(),
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(
            conn.with_runtime_command_output_barriers(|_, barriers| barriers.active_count()),
            0
        );
    }
}
