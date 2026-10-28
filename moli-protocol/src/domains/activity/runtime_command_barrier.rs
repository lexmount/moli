use anyhow::Result;
use std::collections::{BTreeMap, VecDeque};

use moli_core::{RendererOutputCursor, RendererRuntimeCommandCausalIdentity};

use super::output_ingress::PreparedProtocolOutputs;
use crate::conn::{CdpConnection, CdpTurnOutcome, CommandOwnerScope, TargetPageResidenceIdentity};

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
    pub(crate) fn prepare_publication_projection(
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
        let cause = prepared_cause.as_ref().or(renderer_cause);
        let mut matching = self.active.iter().filter_map(|(id, barrier)| {
            (barrier.causal_owner == route.causal_owner && Some(&barrier.renderer_cause) == cause)
                .then_some(*id)
        });
        let barrier_id = matching.next();
        // A command response can release the predecessor before its owned
        // projection reaches the output boundary. Later after-response slots
        // still belong behind that original route front.
        if barrier_id.is_none() && !self.held_routes.iter().any(|held| held.route == route) {
            return unheld(owner, outputs);
        }
        assert!(
            matching.next().is_none(),
            "one Runtime cause has one output barrier"
        );
        let (prefix, outputs) = if let Some(cause) = prepared_cause.as_ref()
            && barrier_id.is_some()
        {
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
                    predecessor: barrier_id,
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

    pub(crate) fn complete_before_response_projection(
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

    pub(crate) fn prepare_completion(
        &mut self,
        conn: &CdpConnection,
        permit: RuntimeCommandOutputBarrierPermit,
        canceled: bool,
    ) -> (
        RuntimeCommandOutputBarrierTerminal,
        Vec<ReadyRuntimeCommandOutputProjection>,
    ) {
        let (id, barrier) = self.consume_permit(permit);
        let terminal = if canceled {
            RuntimeCommandOutputBarrierTerminal::Canceled
        } else if self.command_owner_is_current(conn, &barrier) {
            RuntimeCommandOutputBarrierTerminal::Released
        } else {
            RuntimeCommandOutputBarrierTerminal::Superseded
        };
        self.remove_predecessor(id, barrier, terminal);
        (terminal, self.take_ready_output_projections())
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

    fn remove_predecessor(
        &mut self,
        barrier_id: RuntimeCommandOutputBarrierId,
        barrier: ActiveRuntimeCommandOutputBarrier,
        terminal: RuntimeCommandOutputBarrierTerminal,
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

impl PreparedPublicationOutputProjection {
    pub(crate) fn prepare_until_wait(
        &mut self,
        conn: &mut CdpConnection,
        access: crate::conn::ProtocolStateHandle,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Option<super::PreparedProtocolOutputWait> {
        use super::ProtocolOutputProjectionMode;
        if let Some(prefix) = self.prefix.take() {
            let (prefix, pending) = prefix.prepare_next_projection_step(
                conn,
                self.owner.clone(),
                context,
                ProtocolOutputProjectionMode::All,
            );
            if pending.is_some() {
                self.prefix = Some(prefix);
                return pending;
            }
            let _ = prefix.finish_projection(
                conn,
                self.owner.clone(),
                context,
                ProtocolOutputProjectionMode::All,
            );
        }
        let mode = if self.hold.is_some() {
            ProtocolOutputProjectionMode::BeforeResponse
        } else {
            ProtocolOutputProjectionMode::All
        };
        let outputs = std::mem::replace(&mut self.outputs, PreparedProtocolOutputs::empty());
        let (outputs, pending) =
            outputs.prepare_next_projection_step(conn, self.owner.clone(), context, mode);
        if pending.is_some() {
            self.outputs = outputs;
            return pending;
        }
        let outputs = outputs.finish_projection(conn, self.owner.clone(), context, mode);
        context.materialize_projection_output();
        if let Some(hold) = self.hold.take() {
            let ready = conn.with_runtime_command_output_barriers(move |_, barriers| {
                barriers.complete_before_response_projection(hold, outputs)
            });
            conn.schedule_runtime_output_projections(ready, access);
        }
        None
    }

    #[cfg(test)]
    pub(crate) async fn run_using(
        mut self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        let mut first_error = None;
        loop {
            let access = driver.state_access();
            let (next, pending) = context.with_state(driver, move |state, context| {
                state.prepare_publication_projection_wait(self, access, context)
            })?;
            self = next;
            let Some(pending) = pending else {
                break;
            };
            if let Err(error) = pending.run(driver, context).await {
                first_error.get_or_insert(error);
            }
            context.materialize_projection_output();
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl ReadyRuntimeCommandOutputProjection {
    pub(crate) fn renderer_source(&self) -> Option<RendererOutputCursor> {
        self.renderer_output_cursor
    }

    pub(crate) fn target_id(&self, conn: &CdpConnection) -> Option<String> {
        conn.target_owner_identity_for_owner(&self.owner)?.1
    }

    pub(crate) async fn run_ready(
        ready: Vec<Self>,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        let mut first_error = None;
        for projection in ready {
            context
                .context_mut()
                .retain_runtime_output_projection_permit(projection.permit);
            let previous_source =
                driver.replace_renderer_output_source(projection.renderer_output_cursor);
            let result = if projection.owner_actions_only {
                projection
                    .outputs
                    .project_owner_actions_using(driver, projection.owner, context)
                    .await
            } else {
                projection
                    .outputs
                    .project_using(driver, projection.owner, context)
                    .await
            };
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
            driver.replace_renderer_output_source(previous_source);
        }
        first_error.map_or(Ok(()), Err)
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

    fn with_barriers<T: 'static>(
        ctx: &crate::testing::TestContext,
        operation: impl for<'a> FnOnce(&'a mut CdpConnection, &'a mut RuntimeCommandOutputBarriers) -> T,
    ) -> T {
        ctx.with_fixture(|conn| conn.with_runtime_command_output_barriers(operation))
    }

    async fn collect_projection_work(
        ctx: &mut crate::testing::TestContext,
        command: &mut CommandDispatchContext,
        events: Vec<crate::conn::CdpSchedulerEvent>,
    ) {
        let mut queue = std::collections::VecDeque::from(events);
        let mut retained = Vec::new();
        while let Some(event) = queue.pop_front() {
            match event {
                crate::conn::CdpSchedulerEvent::RendererOutputProjectionPrepared { pending } => {
                    let completed = ctx.drive_owned_wait(pending.wait()).await;
                    let output = ctx
                        .state
                        .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                        .unwrap();
                    let (events, scheduler_events) = output.into_protocol_event_parts();
                    command.protocol_events_mut().extend(events);
                    queue.extend(scheduler_events);
                }
                event => retained.push(event),
            }
        }
        ctx.with_fixture(|conn| conn.extend_scheduler_events(retained));
    }

    async fn finish_barrier(
        ctx: &mut crate::testing::TestContext,
        permit: RuntimeCommandOutputBarrierPermit,
        canceled: bool,
        command: &mut CommandDispatchContext,
    ) -> RuntimeCommandOutputBarrierTerminal {
        let completed = ctx
            .state
            .with_state(|state| {
                if canceled {
                    state.cancel_runtime_command_output_barrier_turn(permit)
                } else {
                    state.release_runtime_command_output_barrier_turn(permit)
                }
            })
            .unwrap();
        let terminal = completed.terminal();
        let (events, scheduler_events) = completed.into_outcome().into_protocol_event_parts();
        command.protocol_events_mut().extend(events);
        collect_projection_work(ctx, command, scheduler_events).await;
        terminal
    }

    async fn project_ready(
        ctx: &mut crate::testing::TestContext,
        command: &mut CommandDispatchContext,
        ready: Vec<super::ReadyRuntimeCommandOutputProjection>,
    ) {
        let events = ctx.with_fixture(|conn| {
            conn.schedule_runtime_output_projections(ready, conn.protocol_state_access());
            conn.take_scheduler_events()
        });
        collect_projection_work(ctx, command, events).await;
    }

    async fn project_publication(
        ctx: &mut crate::testing::TestContext,
        owner: CommandOwnerScope,
        cause: Option<RendererRuntimeCommandCausalIdentity>,
        cursor: Option<moli_core::RendererOutputCursor>,
        outputs: crate::domains::activity::PreparedProtocolOutputs,
        command: &mut CommandDispatchContext,
    ) {
        let projection = with_barriers(ctx, |conn, barriers| {
            barriers.prepare_publication_projection(conn, owner, cause.as_ref(), cursor, outputs)
        });
        ctx.run_fixture_projection(command, move |driver, context| {
            Box::pin(projection.run_using(driver, context))
        })
        .await
        .unwrap();
        let events = ctx.with_fixture(|conn| conn.take_scheduler_events());
        collect_projection_work(ctx, command, events).await;
    }

    const SESSION_ID: &str = "SID-runtime-command-barrier";

    async fn connection_with_loaded_page() -> crate::testing::TestContext {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let page = ctx
            .build_fixture_page("data:text/html,<title>runtime-command-barrier</title>")
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
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));
        ctx
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
        ctx: &mut crate::testing::TestContext,
        permit: Option<&super::RuntimeCommandOutputBarrierPermit>,
        fragment: &str,
        command_context: &mut CommandDispatchContext,
    ) {
        let (command_owner, renderer_cause, outputs) = ctx.with_fixture(|conn| {
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
                );
            let renderer_cause = permit.map(|permit| renderer_cause_for_permit(conn, permit));

            (command_owner, renderer_cause, outputs)
        });
        project_publication(
            ctx,
            command_owner,
            renderer_cause,
            None,
            outputs,
            command_context,
        )
        .await;
    }

    async fn route_top_level_navigation_for_command(
        ctx: &mut crate::testing::TestContext,
        permit: &super::RuntimeCommandOutputBarrierPermit,
        marker: &str,
        command_context: &mut CommandDispatchContext,
    ) {
        let (command_owner, outputs) = ctx.with_fixture(|conn| {
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


(command_owner, outputs)
        });
        project_publication(ctx, command_owner, None, None, outputs, command_context).await;
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
        ctx: &mut crate::testing::TestContext,
        command_context: &mut CommandDispatchContext,
    ) {
        let [event]: [crate::conn::CdpSchedulerEvent; 1] = ctx
            .with_fixture(|conn| conn.take_scheduler_events())
            .try_into()
            .expect("barrier release should publish one concrete navigation action");
        let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
            panic!("barrier release must publish concrete protocol work");
        };
        assert!(work.is_top_level_navigation_owner_action());
        let (events, nested_scheduler_events) = ctx
            .run_fixture_scheduler_work(work)
            .await
            .into_protocol_event_parts();
        assert!(
            !nested_scheduler_events.iter().any(|event| {
                matches!(
                    event,
                    crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                        if work.is_top_level_navigation_owner_action()
                )
            }),
            "executing the concrete navigation must not republish its own owner action"
        );
        command_context.protocol_events_mut().extend(events);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_document_navigation_projects_before_exact_runtime_command_response() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 1)
        });
        let mut command_context = CommandDispatchContext::default();

        route_same_document_navigation(&mut ctx, Some(&permit), "held", &mut command_context).await;

        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            1
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        assert!(
            contains_same_document_navigation(&produced_events(&mut command_context)),
            "the completed history mutation must be reported before the exact command response"
        );

        let terminal = finish_barrier(&mut ctx, permit, false, &mut command_context).await;

        assert_eq!(terminal, RuntimeCommandOutputBarrierTerminal::Released);
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            0
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        assert!(
            !contains_same_document_navigation(&produced_events(&mut command_context)),
            "releasing the barrier must not deliver the same navigation twice"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn debugger_transition_messages_wait_for_the_exact_command_response() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 2)
        });
        let cause = ctx.with_fixture(|conn| renderer_cause_for_permit(conn, &permit));
        let (agent_token, source_residence) = ctx.with_fixture(|conn| {
            let page = conn
                .runtime_session_owner_slot(Some(SESSION_ID))
                .unwrap()
                .loaded_page()
                .unwrap();
            (
                page.renderer_devtools_agent_token(),
                RendererOutputResidenceIdentity::Page {
                    owner_local_host_id: page.renderer_owner_local_host_id(),
                    page_id: page.renderer_page_id(),
                },
            )
        });
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
        let outputs = ctx.with_fixture(|conn| {
            super::super::output_ingress::PreparedProtocolOutputs::from_renderer_observation(
                conn,
                &crate::conn::CommandOwnerScope::for_session(SESSION_ID),
                source_residence,
                &observation,
            )
        });
        let mut command_context = CommandDispatchContext::default();
        let command_owner = CommandOwnerScope::for_session(SESSION_ID);

        project_publication(
            &mut ctx,
            command_owner.clone(),
            (Some(&cause)).cloned(),
            None,
            outputs,
            &mut command_context,
        )
        .await;

        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );
        assert!(
            produced_events(&mut command_context).is_empty(),
            "Debugger transition output must not overtake the matching response"
        );

        assert_eq!(
            finish_barrier(&mut ctx, permit, false, &mut command_context).await,
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
        let mut ctx = connection_with_loaded_page().await;
        let first = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 11)
        });
        let second = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 12)
        });
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut ctx,
            &first,
            "exact-predecessor",
            &mut command_context,
        )
        .await;
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );

        assert_eq!(
            finish_barrier(&mut ctx, second, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            1
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "an unrelated command response must not release another command's output"
        );

        assert_eq!(
            finish_barrier(&mut ctx, first, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            0
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "barrier release must publish, not execute, its concrete owner action"
        );
        complete_published_top_level_navigation(&mut ctx, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
            "the exact command response should release its held output for one scheduler turn"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn canceling_an_unrelated_command_does_not_downgrade_held_output() {
        let mut ctx = connection_with_loaded_page().await;
        let first = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 15)
        });
        let second = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 16)
        });
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut ctx,
            &first,
            "exact-predecessor-survives-unrelated-cancel",
            &mut command_context,
        )
        .await;
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );

        assert_eq!(
            finish_barrier(&mut ctx, second, true, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Canceled
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            1
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "canceling an unrelated command must neither release nor downgrade another command's output"
        );

        assert_eq!(
            finish_barrier(&mut ctx, first, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            0
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "barrier release must publish, not execute, the retained owner action"
        );
        complete_published_top_level_navigation(&mut ctx, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
            "the exact command must retain and release its full protocol output after an unrelated cancel"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unattributed_same_page_output_is_not_held_by_a_pending_command() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 13)
        });
        let mut command_context = CommandDispatchContext::default();

        route_same_document_navigation(
            &mut ctx,
            None,
            "independent-page-task",
            &mut command_context,
        )
        .await;

        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        assert!(
            contains_same_document_navigation(&produced_events(&mut command_context)),
            "output without the exact command identity must remain visible while that command awaits"
        );
        assert_eq!(
            finish_barrier(&mut ctx, permit, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concrete_runtime_navigation_cause_survives_an_earlier_untagged_wake() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 14)
        });
        let cause = ctx.with_fixture(|conn| renderer_cause_for_permit(conn, &permit));
        let owner = ctx.with_fixture(|conn| {
            conn.target_page_residence_identity_for_session(Some(SESSION_ID))
                .expect("loaded Page should expose its exact residence")
        });
        let source_document = ctx.with_fixture(|conn| loaded_page_source_document(conn));
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

        project_publication(
            &mut ctx,
            command_owner.clone(),
            None,
            None,
            outputs,
            &mut command_context,
        )
        .await;

        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "an older untagged wake must not release a concrete command-caused navigation"
        );

        assert_eq!(
            finish_barrier(&mut ctx, permit, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Released
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "barrier release must publish, not execute, the concrete navigation action"
        );
        complete_published_top_level_navigation(&mut ctx, &mut command_context).await;
        assert!(
            contains_top_level_navigation(&produced_events(&mut command_context)),
            "the exact response permit must release the concrete navigation for its scheduler turn"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replacement_supersedes_old_barrier_without_holding_new_page_output() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 21)
        });
        ctx.with_fixture(|conn| {
            conn.runtime_session_owner_slot_mut(Some(SESSION_ID))
                .expect("runtime slot should remain installed")
                .replace_page_attachment_id_for_test()
        });
        let mut command_context = CommandDispatchContext::default();

        route_same_document_navigation(
            &mut ctx,
            Some(&permit),
            "replacement",
            &mut command_context,
        )
        .await;

        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0,
            "output from the replacement Page must not be held by an old Page barrier"
        );
        assert!(
            contains_same_document_navigation(&produced_events(&mut command_context)),
            "one concrete replacement-Page record must project immediately instead of being \
             recaptured through the old command"
        );
        assert_eq!(
            finish_barrier(&mut ctx, permit, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Superseded
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replacement_after_capture_retires_held_old_page_output() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 30)
        });
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut ctx,
            &permit,
            "retired-source-page",
            &mut command_context,
        )
        .await;
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );

        ctx.with_fixture(|conn| {
            conn.runtime_session_owner_slot_mut(Some(SESSION_ID))
                .expect("runtime slot should remain installed")
                .replace_page_attachment_id_for_test()
        });
        assert_eq!(
            finish_barrier(&mut ctx, permit, false, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Superseded
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        complete_published_top_level_navigation(&mut ctx, &mut command_context).await;
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "a held old-Page navigation action must not project into the replacement Page"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_cancel_consumes_the_barrier_once() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 31)
        });
        let mut command_context = CommandDispatchContext::default();

        route_top_level_navigation_for_command(
            &mut ctx,
            &permit,
            "canceled-protocol-command",
            &mut command_context,
        )
        .await;
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            1
        );
        assert_eq!(
            finish_barrier(&mut ctx, permit, true, &mut command_context).await,
            RuntimeCommandOutputBarrierTerminal::Canceled
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
            0
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
        assert!(
            !contains_top_level_navigation(&produced_events(&mut command_context)),
            "cancel must publish, not execute, the already-produced owner action"
        );
        complete_published_top_level_navigation(&mut ctx, &mut command_context).await;
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
            let mut ctx = connection_with_loaded_page().await;
            let permit = with_barriers(&ctx, |conn, barriers| {
                admit_registered_command(conn, barriers, 71)
            });
            let projection = with_barriers(&ctx, |conn, barriers| {
                reserve_navigation_projection(
                    conn,
                    barriers,
                    &permit,
                    "projection-held-before-response",
                )
            });
            let mut context = CommandDispatchContext::default();
            let terminal = if canceled {
                finish_barrier(&mut ctx, permit, true, &mut context).await
            } else {
                finish_barrier(&mut ctx, permit, false, &mut context).await
            };
            assert_eq!(
                terminal,
                if canceled {
                    RuntimeCommandOutputBarrierTerminal::Canceled
                } else {
                    RuntimeCommandOutputBarrierTerminal::Released
                }
            );
            ctx.with_fixture(|conn| {
                assert!(
                    conn.take_scheduler_events().is_empty(),
                    "a response terminal cannot release an unfinished before-response projection"
                )
            });
            let ready = with_barriers(&ctx, |_conn, barriers| {
                barriers.complete_before_response_projection(
                    projection.hold.unwrap(),
                    projection.outputs,
                )
            });
            assert_eq!(ready.len(), 1);
            assert_eq!(ready[0].owner_actions_only, canceled);
            assert_eq!(
                with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
                1,
                "the running projection retains its route front"
            );
            project_ready(&mut ctx, &mut context, ready).await;
            assert_eq!(
                with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
                0
            );
            let events = ctx.with_fixture(|conn| conn.take_scheduler_events());
            assert_eq!(
                events.len(),
                1,
                "the original owner continuation is published exactly once"
            );
            assert!(
                matches!(&events[0], crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } if work.is_top_level_navigation_owner_action())
            );
        }
    }

    #[tokio::test]
    async fn released_command_successor_waits_for_its_running_output_projection() {
        let mut ctx = connection_with_loaded_page().await;
        let permit = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 80)
        });
        let cause = ctx.with_fixture(|conn| renderer_cause_for_permit(conn, &permit));
        let transition_outputs = |conn: &mut CdpConnection, method: &str| {
            let page = conn
                .runtime_session_owner_slot(Some(SESSION_ID))
                .unwrap()
                .loaded_page()
                .unwrap();
            let source = RendererOutputResidenceIdentity::Page {
                owner_local_host_id: page.renderer_owner_local_host_id(),
                page_id: page.renderer_page_id(),
            };
            let observation = RendererProtocolObservation::RuntimeInspector(
                RendererRuntimeInspectorMessageBatch::new_after_command_response(
                    page.renderer_devtools_agent_token(),
                    DevToolsSessionKey::Primary,
                    vec![RendererRuntimeInspectorMessage::protocol(
                        serde_json::json!({
                            "method": method, "params": {}
                        }),
                    )],
                ),
            );
            super::super::output_ingress::PreparedProtocolOutputs::from_renderer_observation(
                conn,
                &CommandOwnerScope::for_session(SESSION_ID),
                source,
                &observation,
            )
        };
        let outputs = ctx.with_fixture(|conn| transition_outputs(conn, "Debugger.resumed"));
        let first = with_barriers(&ctx, |conn, barriers| {
            barriers.prepare_publication_projection(
                conn,
                CommandOwnerScope::for_session(SESSION_ID),
                Some(&cause),
                None,
                outputs,
            )
        });
        assert!(
            with_barriers(&ctx, |_conn, barriers| barriers
                .complete_before_response_projection(
                    first.hold.unwrap(),
                    first.outputs
                ))
            .is_empty()
        );
        let (_, ready) = with_barriers(&ctx, |conn, barriers| {
            barriers.prepare_completion(conn, permit, false)
        });
        assert_eq!(ready.len(), 1);

        // The response has arrived, but its released output still owns the
        // route front. A later publication must retain that existing order.
        let outputs = ctx.with_fixture(|conn| transition_outputs(conn, "Debugger.paused"));
        let successor = with_barriers(&ctx, |conn, barriers| {
            barriers.prepare_publication_projection(
                conn,
                CommandOwnerScope::for_session(SESSION_ID),
                Some(&cause),
                None,
                outputs,
            )
        });
        assert!(
            with_barriers(&ctx, |_conn, barriers| barriers
                .complete_before_response_projection(
                    successor
                        .hold
                        .expect("the running projection retains its successor"),
                    successor.outputs,
                ))
            .is_empty()
        );
        let mut context = CommandDispatchContext::default();
        project_ready(&mut ctx, &mut context, ready).await;
        let messages = protocol_messages(&mut context);
        assert_eq!(
            messages
                .iter()
                .filter_map(|message| message["method"].as_str())
                .collect::<Vec<_>>(),
            ["Debugger.resumed", "Debugger.paused"]
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
    }

    #[tokio::test]
    async fn later_same_route_output_cannot_overtake_a_running_projection() {
        let mut ctx = connection_with_loaded_page().await;
        let first = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 81)
        });
        let second = with_barriers(&ctx, |conn, barriers| {
            admit_registered_command(conn, barriers, 82)
        });
        let first_projection = with_barriers(&ctx, |conn, barriers| {
            reserve_navigation_projection(conn, barriers, &first, "projection-first")
        });
        let second_projection = with_barriers(&ctx, |conn, barriers| {
            reserve_navigation_projection(conn, barriers, &second, "projection-second")
        });
        let mut context = CommandDispatchContext::default();
        assert!(
            with_barriers(&ctx, |_conn, barriers| barriers
                .complete_before_response_projection(
                    second_projection.hold.unwrap(),
                    second_projection.outputs
                ))
            .is_empty()
        );
        finish_barrier(&mut ctx, second, false, &mut context).await;
        finish_barrier(&mut ctx, first, false, &mut context).await;
        ctx.with_fixture(|conn| assert!(conn.take_scheduler_events().is_empty()));
        let first_ready = with_barriers(&ctx, |_conn, barriers| {
            barriers.complete_before_response_projection(
                first_projection.hold.unwrap(),
                first_projection.outputs,
            )
        });
        assert_eq!(first_ready.len(), 1);
        assert!(
            with_barriers(&ctx, |_conn, barriers| barriers
                .take_ready_output_projections())
            .is_empty(),
            "yielding during the first projection must keep the second resident"
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            2
        );
        project_ready(&mut ctx, &mut context, first_ready).await;
        let urls = ctx.with_fixture(|conn| conn.take_scheduler_events().into_iter().map(|event| {
            let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else { panic!("navigation owner action"); };
            let super::super::scheduler_work::ReadyProtocolSchedulerWork::TopLevelNavigationOwnerAction(action) = work.into_ready() else { panic!("original navigation owner action"); };
            match action.into_parts().2 {
                crate::conn::TopLevelNavigationRequest::Location(navigation) => navigation.url().to_owned(),
                crate::conn::TopLevelNavigationRequest::History(_) => panic!("expected location navigation"),
            }
        }).collect::<Vec<_>>());
        assert_eq!(
            urls,
            [
                "data:text/html,projection-first",
                "data:text/html,projection-second"
            ]
        );
        assert_eq!(
            with_barriers(&ctx, |_conn, barriers| barriers.held_output_count()),
            0
        );
    }

    #[tokio::test]
    async fn finite_connection_access_restores_the_original_barriers_after_panic() {
        let ctx = connection_with_loaded_page().await;
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(
            serde_json::json!({"id":91,"method":"Runtime.evaluate","params":{"expression":"1"}})
                .to_string(),
        )
        .unwrap();
        ctx.with_fixture(|conn| {
            conn.try_register_renderer_call_for_session_owner(
                Some(SESSION_ID),
                91,
                None,
                descriptor,
            )
            .unwrap()
        });
        let permit = ctx.with_fixture(|conn| {
            conn.admit_runtime_command_output_barrier(91, Some(SESSION_ID))
                .unwrap()
        });
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ctx.with_fixture(|conn| {
                conn.with_runtime_command_output_barriers::<()>(|_, _barriers| {
                    assert_eq!(
                        with_barriers(&ctx, |_conn, barriers| barriers.active_count()),
                        1
                    );
                    panic!("controlled finite barrier operation failure");
                })
            });
        }));
        assert!(failed.is_err());
        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.with_runtime_command_output_barriers(|_, barriers| barriers.active_count()),
                1
            )
        });
        assert_eq!(
            ctx.state
                .with_state(|state| state.release_runtime_command_output_barrier_turn(permit))
                .unwrap()
                .terminal(),
            RuntimeCommandOutputBarrierTerminal::Released
        );
        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.with_runtime_command_output_barriers(|_, barriers| barriers.active_count()),
                0
            )
        });
    }
}
