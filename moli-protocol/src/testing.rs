//! Test helpers for exercising the CDP connection and its scheduler.
//!
//! `TestContext` owns a `ProtocolStateOwner`, accepts JSON commands, and exposes
//! focused assertions over emitted results, events, and errors.

use anyhow::Result;
use futures_util::{
    FutureExt as _, StreamExt as _, future::LocalBoxFuture, stream::FuturesUnordered,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;

use super::conn::{
    BackgroundProtocolEvent, CdpCommandTaskStep, CdpConnection, CdpInitialStoragePartition,
    CdpSchedulerEvent, CommandDispatchContext, CommandResponseFlushPermit,
    LoadedNavigationRendererAttachmentCommit, ParsedCdpCommand, PendingCdpCommandDispatch,
    RuntimeInspectorResponseReady,
};
use crate::automation::{AutomationCommand, AutomationResult, DevToolsError};
use crate::domains::activity::{
    ProtocolSchedulerWork, ProtocolSchedulerWorkKind, RuntimeCommandOutputBarrierCompletion,
    RuntimeCommandOutputBarrierPermit,
};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererOutputFence, RendererOutputStreamControl,
    RendererOutputStreamIdentity, RendererOutputTransportMessage, runtime::NavigationRuntimeConfig,
};
use moli_fetch::FetchConfig;
#[cfg(test)]
use std::net::SocketAddr;
#[cfg(test)]
use tokio::io::AsyncWriteExt;
#[cfg(test)]
use tokio::net::TcpListener;
#[cfg(test)]
use tokio::task::JoinHandle;

#[cfg(test)]
// Full-workspace runs execute CPU-heavy crypto vectors beside protocol tests.
// Keep this as a bounded diagnostic guard, but leave enough headroom for a
// real renderer-owner wake to be scheduled under that contention.
const TEST_SCHEDULER_INPUT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TestContext {
    pub state: crate::ProtocolStateOwner,
    pub sent: Vec<Value>,
    pending_runtime_deferred_replies: VecDeque<PendingTestRuntimeDeferredReply>,
    pending_protocol_scheduler_work: VecDeque<ProtocolSchedulerWork>,
    pending_owned_protocol_work:
        FuturesUnordered<LocalBoxFuture<'static, crate::CompletedProtocolSchedulerWork>>,
    runtime_inspector_response_ready_rx:
        tokio::sync::mpsc::UnboundedReceiver<RuntimeInspectorResponseReady>,
    renderer_publication_rx: moli_core::RendererOutputTransportReceiver,
    pub(crate) renderer_publication_tx: moli_core::RendererOutputTransportSender,
    background_event_tx: tokio::sync::mpsc::UnboundedSender<BackgroundProtocolEvent>,
    background_event_rx: tokio::sync::mpsc::UnboundedReceiver<BackgroundProtocolEvent>,
    background_navigation_completion_tx:
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
    background_navigation_completion_rx:
        tokio::sync::mpsc::UnboundedReceiver<crate::domains::page::BackgroundNavigationCompletion>,
    background_navigation_scheduler_enabled: bool,
    renderer_insertions: std::collections::HashMap<RendererOutputStreamIdentity, u64>,
    renderer_insertion_output: VecDeque<(
        moli_core::RendererOutputCursor,
        Vec<BackgroundProtocolEvent>,
    )>,
}

pub(crate) struct HeldFixtureRenderer {
    release: Option<std::sync::mpsc::Sender<()>>,
    pending: Option<moli_core::page::PendingPageCommand>,
    response: Option<moli_core::RendererNativeCommandResponseGuard>,
}

impl Drop for HeldFixtureRenderer {
    fn drop(&mut self) {
        self.response.take();
        self.release.take();
        self.pending.take();
    }
}

struct PendingTestRuntimeDeferredReply {
    pending: PendingCdpCommandDispatch,
    command_context: CommandDispatchContext,
    runtime_output_barrier: Option<RuntimeCommandOutputBarrierPermit>,
}

impl PendingTestRuntimeDeferredReply {
    fn new(
        pending: PendingCdpCommandDispatch,
        command_context: CommandDispatchContext,
        runtime_output_barrier: Option<RuntimeCommandOutputBarrierPermit>,
    ) -> Self {
        Self {
            pending,
            command_context,
            runtime_output_barrier,
        }
    }

    fn command_id(&self) -> Option<u64> {
        self.pending.command_id()
    }
}

enum TestSchedulerWork {
    ProtocolWorkCompleted(crate::CompletedProtocolSchedulerWork),
    ProtocolEvents(Vec<BackgroundProtocolEvent>),
    SchedulerEvents(Vec<CdpSchedulerEvent>),
    BackgroundEvent(BackgroundProtocolEvent),
    BackgroundNavigationCompletion(crate::domains::page::BackgroundNavigationCompletion),
    RuntimeDeferredReplyReady(RuntimeInspectorResponseReady),
    RendererPublication(RendererOutputTransportMessage),
    ReleaseRuntimeOutputBarrier(RuntimeCommandOutputBarrierPermit),
    CancelRuntimeOutputBarrier(RuntimeCommandOutputBarrierPermit),
}

#[must_use = "the held test command response must be released exactly once"]
pub(crate) struct TestCommandResponseFlushPermit {
    response_flush: CommandResponseFlushPermit,
    runtime_output_barrier: Option<RuntimeCommandOutputBarrierPermit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestSchedulerTurnOutcome {
    Idle,
    Processed(TestSchedulerInputKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestSchedulerInputKind {
    ProtocolWork,
    StateEvents,
    BackgroundEvent,
    BackgroundNavigationCompletion,
    RuntimeDeferredReply,
    RendererPublication,
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

fn set_test_target_discovery(conn: &mut CdpConnection, enabled: bool) {
    conn.set_root_target_discovery_enabled(enabled);
}

fn real_layout_test_runtime_config(
    optional_resource_fetch_mask: OptionalResourceFetchMask,
) -> NavigationRuntimeConfig {
    NavigationRuntimeConfig::new(
        FetchConfig::default(),
        optional_resource_fetch_mask,
        true,
        LayoutPolicy::OnDemand,
    )
    // Debugger tests exercise actual pauses; production defaults are tested separately.
    .with_debugger_enabled(true)
}

pub(crate) fn real_layout_test_connection() -> CdpConnection {
    CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        CdpInitialStoragePartition::memory(),
        real_layout_test_runtime_config(OptionalResourceFetchMask::NONE),
    )
}

impl TestContext {
    /// Finite fixture setup or assertions. The state borrow cannot escape;
    /// asynchronous commands always use the production owned workflows.
    pub(crate) fn with_fixture<T: 'static>(
        &self,
        operation: impl for<'state> FnOnce(&'state mut CdpConnection) -> T,
    ) -> T {
        self.state
            .with_state(|state| {
                state.fixture(|conn| {
                    conn.bind_fixture_renderer_output_owners_for_test();
                    let result = operation(conn);
                    conn.bind_fixture_renderer_output_owners_for_test();
                    result
                })
            })
            .expect("live test state owner")
    }

    pub(crate) fn handle(&self) -> crate::ProtocolStateHandle {
        self.state.handle()
    }

    pub(crate) async fn drive_owned_wait<T>(
        &mut self,
        wait: impl std::future::Future<Output = T>,
    ) -> T {
        let mut wait = Box::pin(wait);
        loop {
            tokio::select! {
                biased;
                ready = &mut wait => return ready,
                turn = Box::pin(self.wait_for_one_test_scheduler_turn()) => {
                    assert!(matches!(turn, TestSchedulerTurnOutcome::Processed(_)), "owned wait lost scheduler inputs");
                }
            }
        }
    }

    pub(crate) async fn hold_fixture_renderer(
        &self,
        session_id: Option<&str>,
    ) -> HeldFixtureRenderer {
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (pending, response) = self.with_fixture(|conn| {
            conn.loaded_page_mut_for_session_owner(session_id)
                .expect("held fixture renderer should remain resident")
                .start_cdp_call(
                    9_000_199,
                    None,
                    moli_core::RendererNativeOperation::then(
                        moli_core::RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            moli_core::RendererNativeOperationStep::Complete(
                                moli_core::RendererNativeProtocolResponse::success(json!({})),
                            )
                        },
                    ),
                )
                .unwrap()
        });
        let gate = HeldFixtureRenderer {
            release: Some(release),
            pending: Some(pending),
            response: Some(response),
        };
        tokio::time::timeout(Duration::from_secs(30), waiting)
            .await
            .unwrap()
            .unwrap();
        gate
    }

    pub(crate) async fn project_fixture_outputs(
        &mut self,
        outputs: crate::domains::activity::PreparedProtocolOutputs,
        owner: crate::conn::CommandOwnerScope,
        context: &mut CommandDispatchContext,
    ) -> Result<()> {
        self.run_fixture_projection(context, move |driver, context| {
            Box::pin(outputs.project_using(driver, owner, context))
        })
        .await
    }

    pub(crate) async fn run_fixture_projection(
        &mut self,
        command: &mut CommandDispatchContext,
        run: impl for<'a> FnOnce(
            &'a mut crate::conn::ProtocolWorkflowDriver,
            &'a mut crate::conn::CommandWorkflowContext,
        ) -> LocalBoxFuture<'a, Result<()>>,
    ) -> Result<()> {
        let mut driver = crate::conn::ProtocolWorkflowDriver::new(self.handle());
        let (mut context, _scope) =
            crate::conn::CommandWorkflowContext::take_from(command, None, None);
        let result = self.drive_owned_wait(run(&mut driver, &mut context)).await;
        self.with_fixture(|conn| context.finish_projection(conn, command));
        result
    }

    pub(crate) async fn route_fixture_background_turns(
        &mut self,
        completions: &mut tokio::sync::mpsc::UnboundedReceiver<
            crate::domains::page::BackgroundNavigationCompletion,
        >,
        initial: crate::conn::CdpRendererOwnerTurnOutcome,
    ) -> Vec<Value> {
        use crate::domains::activity::ProtocolSchedulerWorkKind;
        let mut turns = std::collections::VecDeque::from([initial]);
        let mut messages = Vec::new();
        loop {
            if let Some(turn) = turns.pop_front() {
                let (events, scheduler_events) =
                    self.route_completed_command_outcome_for_test(turn).await;
                messages.extend(events);
                let mut other_events = Vec::new();
                for event in scheduler_events {
                    match event {
                    crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                        if work.kind() == ProtocolSchedulerWorkKind::NavigationProtocolCompletionOwnerAction => {
                        let pending = self.with_fixture(|conn| conn.try_start_owned_protocol_scheduler_work(work)
                            .unwrap_or_else(|_| panic!("navigation uses its existing supervisor")).unwrap());
                        let mut wait = Box::pin(pending.wait());
                        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                            (&mut wait).await
                        }).await.unwrap();
                        let handoff = self.with_fixture(|conn| conn.complete_owned_protocol_scheduler_work(completed));
                        turns.push_back(handoff.with_renderer_output_predecessor(None));
                    }
                    event => other_events.push(event),
                }
                }
                self.route_direct_command_output_for_test(Vec::new(), other_events)
                    .await;
            } else if let Ok(completion) = completions.try_recv() {
                let turn = self
                    .with_fixture(|conn| conn.admit_background_navigation_completion(completion));
                turns.push_back(turn);
            } else {
                return messages;
            }
        }
    }

    pub(crate) async fn run_fixture_scheduler_work(
        &mut self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> crate::conn::CdpTurnOutcome {
        match self
            .state
            .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
            .expect("live work owner")
        {
            Ok(Some(pending)) => {
                let completed = self.drive_owned_wait(pending.wait()).await;
                self.state
                    .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                    .expect("live completion owner")
            }
            Ok(None) => {
                crate::conn::CdpTurnOutcome::new_with_protocol_events(Vec::new(), Vec::new())
            }
            Err(work) => self
                .state
                .with_state(|state| state.try_complete_protocol_scheduler_work_turn(*work))
                .expect("live finite owner")
                .expect("remaining scheduler work is finite"),
        }
    }

    pub(crate) async fn apply_fixture_navigation_completion(
        &mut self,
        completion: crate::domains::page::MaterializedNavigationCompletion,
        command: &mut CommandDispatchContext,
    ) -> Vec<Value> {
        let sent_start = self.sent.len();
        let (token, navigation, outcome) = completion.into_parts();
        self.run_fixture_projection(command, move |driver, context| {
            Box::pin(async move {
                let prepared = context.with_state(driver, move |state, context| {
                    state.fixture(|conn| {
                crate::domains::page::PreparedRendererNavigationResult::prepare_materialized(
                    conn, token, navigation, outcome, context,
                    crate::domains::page::NavigationCommandOutput::Background, Vec::new(),
                )
            })
                })?;
                prepared.run(driver, context).await
            })
        })
        .await
        .expect("fixture navigation completion");
        let step = self
            .state
            .with_state(|state| {
                state.finish_fixture_command_output_plan(
                    crate::domains::command_output::CommandOutputPlan::default(),
                    command,
                    None,
                    None,
                )
            })
            .unwrap();
        let (messages, scheduler) = self
            .complete_command_task_step_with_context_for_test(step, command)
            .await;
        let mut output: Vec<_> = self.sent.drain(sent_start..).collect();
        output.extend(messages);
        self.route_direct_command_output_for_test(Vec::new(), scheduler)
            .await;
        output.extend(self.sent.drain(sent_start..));
        output
    }

    pub(crate) async fn run_fixture_workflow(
        &mut self,
        workflow: crate::conn::PreparedCommandWorkflow,
        command_context: &mut CommandDispatchContext,
        command_id: Option<u64>,
        session_id: Option<String>,
    ) -> crate::domains::command_output::CommandOutputPlan {
        let pending = workflow.start(command_context, command_id, session_id);
        let completed = self.drive_owned_wait(pending.wait()).await;
        self.state
            .with_state(|state| state.finish_command_workflow(completed, command_context))
            .expect("live workflow owner")
    }

    pub(crate) async fn execute_owned_automation_command_for_test(
        &mut self,
        command: AutomationCommand,
    ) -> crate::conn::AutomationCommandDispatchOutcome {
        let mut step = self
            .state
            .with_state(|state| state.start_automation_command_dispatch(command))
            .expect("live test owner");
        loop {
            match step {
                crate::AutomationCommandTaskStep::Complete(outcome) => return *outcome,
                crate::AutomationCommandTaskStep::Pending(pending) => {
                    let completed = self.drive_owned_wait(pending.wait()).await;
                    step = self
                        .state
                        .with_state(|state| state.complete_automation_command_dispatch(completed))
                        .expect("live test owner");
                }
            }
        }
    }

    pub(crate) async fn dispatch_raw_command(&mut self, raw: &str) -> Vec<Value> {
        let sent_start = self.sent.len();
        let handle = self.handle();
        let outcome = self
            .drive_owned_wait(handle.process_message_for_test(raw))
            .await
            .expect("owned protocol command must finish");
        let (messages, scheduler_events) =
            self.route_completed_command_outcome_for_test(outcome).await;
        self.sent.extend(messages);
        self.route_direct_command_output_for_test(Vec::new(), scheduler_events)
            .await;
        self.sent.drain(sent_start..).collect()
    }

    pub(crate) async fn build_fixture_page(
        &mut self,
        raw_url: &str,
    ) -> Result<moli_core::page::Page> {
        let load = self.with_fixture(|conn| conn.prepare_page_fixture_load(raw_url));
        let (navigation, _) = load
            .run_at_boundary(None, moli_core::runtime::RendererReplyBoundary::Stage)
            .await;
        Ok(self.commit_fixture_response(navigation?, None).await?.page)
    }

    pub(crate) async fn run_fixture_page_query<T: 'static>(
        &mut self,
        session_id: Option<&str>,
        operation: &'static str,
        start: impl FnOnce(&mut moli_core::page::Page) -> Result<moli_core::page::PendingPageCommand>,
        finish: impl FnOnce(
            &mut moli_core::page::Page,
            moli_core::page::CompletedPageCommand,
        ) -> Result<T>
        + 'static,
    ) -> Result<T> {
        let prepared = self.with_fixture(|conn| {
            let owner = crate::conn::CommandOwnerScope::capture(conn, session_id);
            crate::conn::PreparedRuntimePageQuery::prepare_resident(
                conn, &owner, operation, start, finish,
            )
        })?;
        self.finish_fixture_query(prepared).await
    }

    /// Capture a Native source for projection tests. The caller owns its fence
    /// and decides when to admit the original publication to protocol ingress.
    pub(crate) async fn stage_fixture_page_query<T: 'static>(
        &self,
        session_id: Option<&str>,
        operation: &'static str,
        start: impl FnOnce(&mut moli_core::page::Page) -> Result<moli_core::page::PendingPageCommand>,
        finish: impl FnOnce(
            &mut moli_core::page::Page,
            moli_core::page::CompletedPageCommand,
        ) -> Result<T>
        + 'static,
    ) -> Result<(T, Option<moli_core::RendererOutputFence>)> {
        let prepared = self.with_fixture(|conn| {
            let owner = crate::conn::CommandOwnerScope::capture(conn, session_id);
            crate::conn::PreparedRuntimePageQuery::prepare_resident(
                conn, &owner, operation, start, finish,
            )
        })?;
        let mut predecessor = None;
        let result = prepared
            .run(
                &mut crate::conn::ProtocolWorkflowDriver::new(self.handle()),
                &mut predecessor,
            )
            .await?;
        Ok((result, predecessor))
    }

    pub(crate) async fn finish_fixture_query<T: 'static>(
        &mut self,
        prepared: crate::conn::PreparedRuntimePageQuery<T>,
    ) -> Result<T> {
        let mut driver = crate::conn::ProtocolWorkflowDriver::new(self.handle());
        let mut predecessor = None;
        let result = self
            .drive_owned_wait(prepared.run(&mut driver, &mut predecessor))
            .await;
        if let Some(predecessor) = predecessor {
            self.route_direct_command_renderer_predecessor_for_test(predecessor)
                .await;
        }
        result
    }

    pub(crate) fn with_fixture_page<T: 'static>(
        &self,
        session_id: Option<&str>,
        operation: impl for<'page> FnOnce(&'page mut moli_core::page::Page) -> T,
    ) -> T {
        self.with_fixture(|conn| {
            operation(
                conn.loaded_page_mut_for_protocol_access(session_id)
                    .expect("fixture Page should be loaded"),
            )
        })
    }

    pub(crate) async fn finish_fixture_page_updates(
        &mut self,
        owner: crate::conn::CommandOwnerScope,
        updates: crate::conn::SessionPageUpdates,
    ) -> Result<()> {
        let completed = self.drive_owned_wait(updates.wait()).await;
        let result =
            self.with_fixture(|conn| completed.finish(conn.loaded_page_mut_for_owner(&owner)));
        if let Some(predecessor) = result.predecessor {
            self.route_direct_command_renderer_predecessor_for_test(predecessor)
                .await;
        }
        result.first_error.map_or(Ok(()), Err)
    }

    pub(crate) async fn select_fixture_page_target(&mut self, target_id: &str) -> Result<bool> {
        let updates = self.with_fixture(|conn| {
            conn.browser_context
                .as_mut()
                .expect("fixture browser context")
                .start_select_page_target(target_id)
        });
        let Some(updates) = updates else {
            return Ok(false);
        };
        let mut driver = crate::conn::ProtocolWorkflowDriver::new(self.handle());
        self.drive_owned_wait(
            crate::domains::target::activation::wait_for_surface_updates(&mut driver, updates),
        )
        .await?;
        Ok(true)
    }

    pub(crate) async fn activate_fixture_target(
        &mut self,
        target_id: &str,
    ) -> Result<Vec<crate::conn::BackgroundProtocolEvent>, DevToolsError> {
        let prepared = self.with_fixture(|conn| {
            crate::domains::target::activation::prepare_target_activation_for_target(
                conn, target_id,
            )
        })?;
        let Some(prepared) = prepared else {
            return Ok(Vec::new());
        };
        let mut driver = crate::conn::ProtocolWorkflowDriver::new(self.handle());
        let output = self
            .drive_owned_wait(prepared.run(&mut driver))
            .await
            .map_err(|error| {
                DevToolsError::new(crate::automation::DevToolsErrorKind::Internal, error)
            })?;
        Ok(self.with_fixture(|conn| output.finish(conn)))
    }

    pub(crate) async fn remove_active_fixture_target(&mut self) -> bool {
        let host = self.with_fixture(|conn| {
            let context = conn.browser_context.as_mut()?;
            let target = context.active_target_id_owned()?;
            context.take_page_target_for_close(&target)
        });
        let Some(mut host) = host else {
            return false;
        };
        host.close_page_async().await;
        true
    }

    pub(crate) async fn fixture_default_execution_context(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<i64>> {
        self.run_fixture_page_query(
            session_id,
            "fixture default execution context",
            |page| page.start_default_execution_context_id(),
            |page, completed| page.finish_default_execution_context_id(completed),
        )
        .await
    }

    pub(crate) async fn fixture_isolated_inspector_context(
        &mut self,
        session_id: Option<&str>,
        context_id: i64,
    ) -> Result<Option<i64>> {
        self.run_fixture_page_query(
            session_id,
            "fixture isolated world attachment",
            |page| page.start_ensure_isolated_worlds_attached_to_inspector(),
            |page, completed| {
                page.finish_unit_runtime_page_command(
                    completed,
                    "ensure isolated worlds attached to inspector",
                )
            },
        )
        .await?;
        self.run_fixture_page_query(
            session_id,
            "fixture isolated execution context",
            |page| page.start_inspector_execution_context_id_for_isolated_context(context_id),
            |page, completed| {
                page.finish_inspector_execution_context_id_for_isolated_context(completed)
            },
        )
        .await
    }

    pub(crate) async fn serialize_fixture_document(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<String> {
        self.run_fixture_page_query(
            session_id,
            "serialize fixture document",
            |page| page.start_serialize_html(),
            |page, completed| page.finish_serialize_html(completed),
        )
        .await
    }

    pub(crate) async fn evaluate_fixture_page_expression(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        self.run_fixture_page_query(
            session_id,
            "fixture renderer expression",
            |page| page.start_evaluate_runtime_expression(expression, await_promise),
            |page, completed| page.finish_evaluate_runtime_expression(completed),
        )
        .await
    }

    pub(crate) async fn evaluate_fixture_expression(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, DevToolsError> {
        const ID: u64 = 9_000_101;
        self.process_and_wait_for_response_async(json!({"id": ID, "method":"Runtime.evaluate", "params":{"expression":expression, "awaitPromise":await_promise, "returnByValue":true}})).await;
        let response = self.take_response_by_id(ID);
        if let Some(error) = response.get("error") {
            return Err(crate::domains::command_output::devtools_error_from_cdp_error_value(error));
        }
        Ok(response["result"]["result"].clone())
    }

    #[cfg(test)]
    pub(crate) fn take_renderer_transport_for_owned_state_test(
        &mut self,
    ) -> moli_core::RendererOutputTransportReceiver {
        let (_, replacement) = moli_core::renderer_output_transport_channel();
        std::mem::replace(&mut self.renderer_publication_rx, replacement)
    }

    /// Explicitly publish fixture geometry before geometry-dependent commands.
    pub(crate) async fn capture_fixture_layout(&mut self, session_id: Option<&str>) {
        let mut request = json!({"id":9_000_000,"method":"Page.captureScreenshot"});
        if let Some(session_id) = session_id {
            request["sessionId"] = json!(session_id);
        }
        self.process_async(request).await;
        let response = self.take_response_by_id(9_000_000);
        assert!(
            response["result"]["data"].is_string(),
            "fixture screenshot: {response}"
        );
    }

    /// Build the default CDP test harness used by older internal unit tests.
    ///
    /// Historically those tests treated `Target.targetCreated` as a baseline
    /// event after `Target.createTarget`. Real CDP clients only receive target
    /// discovery events after enabling discovery, so keep that convenience in
    /// `TestContext` instead of changing `CdpConnection::new()`.
    pub fn new() -> Self {
        Self::new_with_target_discovery(true)
    }

    /// Build a CDP test harness with explicit Target discovery state.
    ///
    /// Use `true` for internal tests that assert Target-domain event payloads
    /// without spelling out the setup command. Use `false` for Chromium-parity
    /// tests that need to verify the default protocol behavior before
    /// `Target.setDiscoverTargets(true)` is called.
    pub fn new_with_target_discovery(target_discovery_enabled: bool) -> Self {
        let mut conn = real_layout_test_connection();
        set_test_target_discovery(&mut conn, target_discovery_enabled);
        Self::from_conn(conn)
    }

    pub fn new_with_layout_policy(layout_policy: LayoutPolicy) -> Self {
        let conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
            CdpInitialStoragePartition::memory(),
            NavigationRuntimeConfig::new(
                FetchConfig::default(),
                OptionalResourceFetchMask::NONE,
                true,
                layout_policy,
            ),
        );
        Self::from_conn(conn)
    }

    pub fn new_with_target_discovery_and_image_fetch(
        target_discovery_enabled: bool,
        image_fetch_enabled: bool,
    ) -> Self {
        Self::new_with_target_discovery_and_optional_resource_fetch_mask(
            target_discovery_enabled,
            if image_fetch_enabled {
                OptionalResourceFetchMask::IMAGE
            } else {
                OptionalResourceFetchMask::NONE
            },
        )
    }

    pub fn new_with_target_discovery_and_optional_resource_fetch_mask(
        target_discovery_enabled: bool,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
    ) -> Self {
        let mut conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
            CdpInitialStoragePartition::memory(),
            real_layout_test_runtime_config(optional_resource_fetch_mask),
        );
        set_test_target_discovery(&mut conn, target_discovery_enabled);
        Self::from_conn(conn)
    }

    pub fn from_conn(mut conn: CdpConnection) -> Self {
        conn.bind_fixture_renderer_output_owners_for_test();
        let (renderer_publication_tx, renderer_publication_rx) =
            moli_core::renderer_output_transport_channel();
        let (runtime_inspector_response_ready_tx, runtime_inspector_response_ready_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (background_event_tx, background_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (background_navigation_completion_tx, background_navigation_completion_rx) =
            tokio::sync::mpsc::unbounded_channel();
        conn.set_renderer_publication_sender(renderer_publication_tx.clone());
        conn.set_runtime_inspector_response_ready_sender(runtime_inspector_response_ready_tx);
        Self {
            state: crate::ProtocolStateOwner::new(conn),
            sent: Vec::new(),
            pending_runtime_deferred_replies: VecDeque::new(),
            pending_protocol_scheduler_work: VecDeque::new(),
            pending_owned_protocol_work: FuturesUnordered::new(),
            runtime_inspector_response_ready_rx,
            renderer_publication_rx,
            renderer_publication_tx,
            background_event_tx,
            background_event_rx,
            background_navigation_completion_tx,
            background_navigation_completion_rx,
            background_navigation_scheduler_enabled: false,
            renderer_insertions: Default::default(),
            renderer_insertion_output: VecDeque::new(),
        }
    }

    /// Enables the same asynchronous navigation channels used by the socket
    /// scheduler.
    ///
    /// Most protocol unit tests intentionally dispatch a domain command to
    /// completion without owning an actor. Chromium-ordering and lifecycle
    /// tests must opt into this production boundary: Page.navigate emits its
    /// start/early response first, while the later renderer Page commit arrives
    /// independently and is joined by its exact concrete-output cursor.
    pub(crate) fn enable_background_navigation_scheduler_for_test(&mut self) {
        if self.background_navigation_scheduler_enabled {
            return;
        }
        self.with_fixture(|conn| {
            conn.set_background_event_sender(self.background_event_tx.clone())
        });
        self.with_fixture(|conn| {
            conn.set_background_navigation_completion_sender(
                self.background_navigation_completion_tx.clone(),
            )
        });
        self.background_navigation_scheduler_enabled = true;
    }

    /// Loads and installs one production-shaped navigation fixture for the
    /// exact Page target addressed by `session_id`.
    ///
    /// Tests that expect owner-produced lifecycle or child-frame output must
    /// not insert a bare `Page` into the runtime slot: production installs the
    /// renderer Page and its exact root-Document lifecycle binding together.
    /// Keeping that invariant in one helper prevents protocol tests from
    /// accidentally depending on a state that the real navigation path never
    /// exposes.
    pub(crate) async fn install_navigation_fixture_for_session_owner(
        &mut self,
        raw_url: &str,
        session_id: Option<&str>,
    ) {
        let navigation = self
            .build_fixture_navigation(raw_url, session_id)
            .await
            .expect("navigation fixture should load");
        self.install_loaded_navigation_fixture_for_session_owner(navigation, session_id)
            .await;
    }

    /// Installs an in-memory response through the same target/Page ownership
    /// transaction as a real navigation.
    ///
    /// Tests that need a non-fetchable origin (for example an IndexedDB
    /// fixture at `https://example.test`) use this entry point. Building the
    /// renderer Page first and assigning it directly to `TargetRuntimeSlot`
    /// skips the concrete-output owner binding and is not a state production
    /// can expose.
    pub(crate) async fn install_buffered_navigation_fixture_for_session_owner(
        &mut self,
        requested_url: url::Url,
        response_body: String,
        session_id: Option<&str>,
    ) {
        let load = self
            .with_fixture(|conn| {
                conn.commit_declared_session_fixtures_for_test();
                conn.prepare_buffered_navigation_fixture(session_id, requested_url, response_body)
            })
            .expect("buffered navigation fixture should prepare");
        let navigation = load
            .run()
            .await
            .expect("buffered navigation fixture should build");
        let navigation = self
            .commit_fixture_response(navigation, session_id)
            .await
            .expect("buffered fixture Native commit");
        self.install_loaded_navigation_fixture_for_session_owner(navigation, session_id)
            .await;
    }

    pub(crate) async fn build_fixture_navigation(
        &mut self,
        raw_url: &str,
        session_id: Option<&str>,
    ) -> Result<crate::conn::LoadedNavigation> {
        let load = self.with_fixture(|conn| {
            conn.commit_declared_session_fixtures_for_test();
            conn.prepare_navigation_fixture_load(session_id, raw_url)
        })?;
        let (navigation, _) = load
            .run_at_boundary(None, moli_core::runtime::RendererReplyBoundary::Stage)
            .await;
        self.commit_fixture_response(navigation?, session_id).await
    }

    async fn commit_fixture_response(
        &mut self,
        navigation: crate::conn::NavigationLoadOutcome,
        session_id: Option<&str>,
    ) -> Result<crate::conn::LoadedNavigation> {
        let crate::conn::NavigationLoadOutcome::ResponseCommitReady(navigation) = navigation else {
            panic!("navigation fixture requires a Document response");
        };
        let configuration = self.with_fixture(|conn| {
            let owner = crate::conn::CommandOwnerScope::capture(conn, session_id);
            conn.prepared_document_commit_configuration_for_owner(&owner, navigation.final_url())
        })?;
        let permit = navigation.issue_commit_permit();
        navigation.commit(permit, configuration).await
    }

    async fn install_loaded_navigation_fixture_for_session_owner(
        &mut self,
        navigation: crate::conn::LoadedNavigation,
        session_id: Option<&str>,
    ) {
        let owner =
            self.with_fixture(|conn| crate::conn::CommandOwnerScope::capture(conn, session_id));
        let (_, target_id) = self.with_fixture(|conn| {
            conn.target_owner_identity_for_owner(&owner)
                .expect("navigation fixture requires an installed browser context")
        });
        let target_id = target_id.expect("navigation fixture requires an exact target");
        let renderer_output_predecessor = navigation.renderer_output_predecessor;
        let page_creation_artifacts = navigation.page_creation_artifacts;
        let final_url = navigation.final_url;
        let main_document_commit = navigation
            .main_document_commit
            .expect("navigation fixture must retain its frozen Document commit identity");
        let installed = self
            .with_fixture(|conn| {
                conn.install_loaded_navigation_page_for_owner(
                    &owner,
                    navigation.page,
                    LoadedNavigationRendererAttachmentCommit::Prepare(None),
                    &final_url,
                )
            })
            .expect("navigation fixture target must remain installed")
            .expect("navigation fixture Page commit must succeed");
        let page_commit = installed.close_previous().await;
        assert!(
            page_commit
                .committed_document_post_response_continuation
                .is_none(),
            "lifecycle-target fixture must not retain a DocumentCommit response gate"
        );
        let _ = self.with_fixture(|conn| {
            conn.commit_loaded_navigation_target_identity_for_owner(
                &owner,
                &main_document_commit,
                &final_url,
            )
        });
        let (binding, _) = self.with_fixture(|conn| {
            conn.bind_renderer_document_lifecycle_for_owner(
                &owner,
                page_creation_artifacts,
                None,
                target_id,
                crate::domains::page::LOADER_ID.to_owned(),
            )
        });
        let binding =
            binding.expect("navigation fixture must install its exact renderer Document binding");
        self.with_fixture(|conn| {
            assert_eq!(
                conn.target_root_document_lifecycle_identity_for_owner(&owner,),
                Some(binding.renderer_document_identity()),
                "navigation fixture must retain its exact renderer Document binding"
            )
        });
        if let Some(predecessor) = renderer_output_predecessor {
            // Production does not expose a completed navigation response until
            // the Page-creation cursor has crossed ordered protocol ingress.
            // Mirror that boundary here so a later enable command observes
            // the already-loaded target tail instead of racing publications
            // that merely happen to remain queued in the test transport.
            self.route_renderer_output_predecessor_before_command_response(predecessor)
                .await;
        }
    }

    /// Feed a JSON-serialisable message through the async CDP entrypoint and
    /// route the scheduler work directly requested by that command. Tests
    /// waiting for later external lifecycle input must use the event-wait
    /// helpers, which consume one renderer wake or runtime reply at a time.
    pub async fn process_async(&mut self, msg: impl serde::Serialize) {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        let command_id = command.request().id();
        let session_id = command.request().session_id().map(str::to_owned);
        let command_awaits_promise = match command.request().method() {
            "Runtime.awaitPromise" => true,
            "Runtime.evaluate" | "Runtime.callFunctionOn" | "Runtime.runScript" => command
                .request()
                .params()
                .and_then(|params| params.get("awaitPromise"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            _ => false,
        };
        let response_start = self.sent.len();
        Box::pin(self.process_parsed_command_like_scheduler(&command, true)).await;
        Box::pin(self.route_ready_test_command_response(command_id, response_start)).await;
        if self.with_fixture(|conn| {
            conn.renderer_runtime_command_cause_for_frontend(session_id.as_deref(), command_id)
                .is_some()
        }) && !(command_awaits_promise
            && self.with_fixture(|conn| {
                conn.has_pending_inspector_awaits_for_session_owner(session_id.as_deref())
            }))
            && !self
                .pending_runtime_deferred_replies
                .iter()
                .any(|pending| pending.command_id() == Some(command_id))
        {
            Box::pin(self.wait_for_test_command_response(command_id, response_start)).await;
        }
    }

    /// Navigate as a CDP client that subscribes to Page notifications.
    /// Tests using Page events as completion boundaries must opt in explicitly.
    #[cfg(test)]
    pub(crate) async fn navigate_with_page_events(&mut self, message: Value) {
        assert_eq!(message["method"], "Page.navigate");
        self.enable_page_events_for_test(message["sessionId"].as_str());
        self.process_async(message).await;
    }

    /// Dispatch one command and keep running the real scheduler inputs until
    /// that command's response is routed. This mirrors Chromium's synchronous
    /// DevTools test client without draining unrelated page work to idle.
    pub async fn process_and_wait_for_response_async(&mut self, msg: impl serde::Serialize) {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        let command_id = command.request().id();
        let response_start = self.sent.len();
        Box::pin(self.process_parsed_command_like_scheduler(&command, true)).await;
        Box::pin(self.wait_for_test_command_response(command_id, response_start)).await;
    }

    #[cfg(test)]
    pub(crate) fn enable_page_events_for_test(&mut self, session_id: Option<&str>) {
        self.with_fixture(|conn| {
            conn.commit_declared_session_fixtures_for_test();
            assert!(
                conn.set_page_domain_enabled_for_session_owner(session_id, true),
                "test Page subscription requires a loaded target session owner"
            )
        });
    }

    #[cfg(test)]
    pub(crate) fn enable_dom_events_for_test(&mut self, session_id: Option<&str>) {
        self.with_fixture(|conn| {
            assert!(
                conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
                    state.dom_session_state.enabled = true;
                })
                .is_some(),
                "test DOM subscription requires a loaded target session owner"
            )
        });
    }

    /// Wait for and remove one protocol message produced by a real scheduler
    /// input. It does not synthesize a capture or ask a command path to advance
    /// renderer work: it only routes renderer publications and deferred
    /// inspector replies already published by the production owner scheduler.
    #[cfg(test)]
    pub(crate) async fn wait_for_scheduler_message(
        &mut self,
        description: &str,
        mut matches: impl FnMut(&Value) -> bool,
    ) -> Value {
        let message = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            loop {
                if let Some(position) = self.sent.iter().position(&mut matches) {
                    return self.sent.remove(position);
                }
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "test scheduler lost all external input while waiting for {description}"
                );
            }
        })
        .await;

        match message {
            Ok(message) => message,
            Err(_) => panic!(
                "timed out waiting for {description} from a real scheduler input; sent={:?}",
                self.sent
            ),
        }
    }

    /// Wait until routing real scheduler input makes connection state satisfy
    /// `predicate`. This observes owner-published wakes only; it does not
    /// synthesize a capture or ask protocol code to advance renderer lifecycle.
    #[cfg(test)]
    pub(crate) async fn wait_until_scheduler_state(
        &mut self,
        description: &str,
        predicate: impl Fn(&CdpConnection) -> bool,
    ) {
        let waited = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            loop {
                if self.with_fixture(|conn| predicate(conn)) {
                    return;
                }
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "test scheduler lost all external input while waiting for {description}"
                );
            }
        })
        .await;

        if waited.is_err() {
            panic!(
                "timed out waiting for {description} from a real scheduler input; sent={:?}; diagnostics={}",
                self.sent,
                self.with_fixture(|conn| conn.moli_memory_diagnostics())
            );
        }
    }

    /// Feed one command through the async CDP entrypoint without completing
    /// deferred protocol residences afterwards, returning the scheduler events
    /// that production would handle after the command response turn.
    ///
    /// Use this for protocol sequences that intentionally send another command
    /// before idle/runtime work gets a chance to run. The production socket
    /// actor has a client-turn boundary between command output and deferred
    /// CDP activity; eager test draining would otherwise make the test observe
    /// a stronger, more synchronous ordering than real clients get.
    #[cfg(test)]
    pub(crate) async fn process_command_only_async(
        &mut self,
        msg: impl serde::Serialize,
    ) -> Vec<CdpSchedulerEvent> {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        Box::pin(self.process_parsed_command_like_scheduler(&command, false)).await
    }

    /// Dispatch one command through the production response-flush boundary,
    /// but leave that boundary held so tests can inspect causally later page
    /// work before the wire response is considered flushed.
    #[cfg(test)]
    pub(crate) async fn process_command_holding_response_flush_for_test(
        &mut self,
        msg: impl serde::Serialize,
    ) -> TestCommandResponseFlushPermit {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        let command_id = command.request().id();
        let response_start = self.sent.len();
        let (response_flush_permit, response_flush_context) =
            self.with_fixture(|conn| conn.begin_command_response_flush_permit());
        let mut command_context = CommandDispatchContext::new(response_flush_context);
        let step = self.with_fixture(|conn| {
            conn.start_parsed_command_dispatch_with_context(&command, &mut command_context)
        });
        let mut runtime_output_barrier = self.admit_runtime_output_barrier(&command);
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut runtime_output_barrier,
        ))
        .await;
        assert!(
            completed,
            "held-flush test helper requires an immediate command response boundary"
        );
        protocol_events.extend(command_context.take_protocol_events());
        protocol_events.extend(command_context.take_post_response_events());
        scheduler_events.extend(self.with_fixture(|conn| conn.take_scheduler_events()));
        Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events)).await;
        Box::pin(self.route_ready_test_command_response(command_id, response_start)).await;
        TestCommandResponseFlushPermit {
            response_flush: response_flush_permit,
            runtime_output_barrier,
        }
    }

    /// Releases a response boundary held by
    /// [`Self::process_command_holding_response_flush_for_test`].
    ///
    /// The response-flush permit becomes visible before command-owned
    /// after-response output, matching the production scheduler's
    /// `finish_command_dispatch_output_flush` ordering.
    pub(crate) async fn finish_held_command_response_flush_for_test(
        &mut self,
        permit: TestCommandResponseFlushPermit,
    ) {
        permit.response_flush.finish();
        if let Some(runtime_output_barrier) = permit.runtime_output_barrier {
            Box::pin(
                self.release_runtime_output_barrier_like_scheduler(runtime_output_barrier, true),
            )
            .await;
        }
    }

    /// Collects the renderer events routed during completion together with
    /// the command messages. Domain tests need that prefix to inspect native
    /// notifications rather than observing only the terminal response.
    #[cfg(test)]
    pub(crate) async fn complete_command_task_step_with_events_for_test(
        &mut self,
        step: CdpCommandTaskStep,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        let sent_start = self.sent.len();
        let (messages, scheduler_events) = self.complete_command_task_step_for_test(step).await;
        let mut published: Vec<_> = self.sent.drain(sent_start..).collect();
        published.extend(messages);
        (published, scheduler_events)
    }

    #[cfg(test)]
    pub(crate) async fn complete_command_task_step_for_test(
        &mut self,
        step: CdpCommandTaskStep,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        self.complete_command_task_step_with_context_for_test(
            step,
            &mut CommandDispatchContext::default(),
        )
        .await
    }

    pub(crate) async fn complete_command_task_step_with_context_for_test(
        &mut self,
        step: CdpCommandTaskStep,
        command_context: &mut CommandDispatchContext,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        let sent_start = self.sent.len();
        let command_id = match &step {
            CdpCommandTaskStep::Pending(pending) => pending.command_id(),
            CdpCommandTaskStep::Complete(_) => None,
        };
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let mut runtime_output_barrier = None;
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut runtime_output_barrier,
        ))
        .await;
        if completed {
            let mut messages = protocol_events_into_messages(protocol_events);
            if let Some(command_id) = command_id
                && !messages
                    .iter()
                    .any(|message| message.get("id").and_then(Value::as_u64) == Some(command_id))
            {
                // Synchronous renderer-owned Runtime commands publish their
                // terminal response through the DevTools-session output
                // transport. The command completion only proves that the
                // publication was committed; admit that independently
                // transported response just as `process_async` does, while
                // leaving sibling renderer events in the scheduler queue.
                Box::pin(self.route_ready_test_command_response(command_id, sent_start)).await;
                if let Some(position) = self.sent.get(sent_start..).and_then(|sent| {
                    sent.iter()
                        .position(|message| {
                            message.get("id").and_then(Value::as_u64) == Some(command_id)
                        })
                        .map(|position| sent_start + position)
                }) {
                    messages.push(self.sent.remove(position));
                }
            }
            let mut emitted: Vec<_> = self.sent.drain(sent_start..).collect();
            emitted.extend(messages);
            return (emitted, scheduler_events);
        }

        Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events)).await;
        if !self.pending_runtime_deferred_replies.is_empty() {
            let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
        }
        let messages = self.sent.drain(sent_start..).collect();
        (messages, Vec::new())
    }

    /// Routes the output of one direct protocol-neutral command through the
    /// stateful test scheduler.
    ///
    /// Direct `AutomationCommand` fixtures bypass parsed CDP dispatch but can
    /// still publish protocol work whose exact owner action is not ready yet.
    /// Keeping that work resident here mirrors the production scheduler;
    /// callers must use the scheduler wait helpers for later renderer input.
    #[cfg(test)]
    pub(crate) async fn route_direct_command_output_for_test(
        &mut self,
        protocol_events: Vec<BackgroundProtocolEvent>,
        scheduler_events: Vec<CdpSchedulerEvent>,
    ) {
        Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events)).await;
    }

    /// Waits until every concrete scheduler work item published by a direct
    /// command has completed.
    ///
    /// This is the protocol-neutral counterpart of waiting for a CDP event:
    /// WebDriver commands can require the same owner action without enabling a
    /// CDP domain, so their tests must synchronize with the work residence
    /// itself rather than manufacture a frontend subscription.
    #[cfg(test)]
    pub(crate) async fn wait_for_direct_command_work_completion_for_test(
        &mut self,
        description: &str,
    ) {
        let waited = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            while !self.pending_protocol_scheduler_work.is_empty()
                || !self.pending_owned_protocol_work.is_empty()
            {
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "test scheduler lost all external input while waiting for {description}"
                );
            }
        })
        .await;
        if waited.is_err() {
            panic!(
                "timed out waiting for {description}; pending protocol work={:?}",
                self.pending_protocol_scheduler_work
            );
        }
    }

    async fn process_parsed_command_like_scheduler(
        &mut self,
        command: &ParsedCdpCommand,
        drain_after_command: bool,
    ) -> Vec<CdpSchedulerEvent> {
        // Some low-level fixtures mutate target-side session declarations
        // after installing their BrowserContext. Materialize those declarations
        // as committed attachments at the test scheduler boundary; production
        // route lookup remains control-plane-only.
        self.with_fixture(|conn| conn.commit_declared_session_fixtures_for_test());
        let output_session_id = command.command_output_session_id().map(str::to_owned);
        let output_owner = self.with_fixture(|conn| {
            crate::conn::CommandOwnerScope::capture(conn, output_session_id.as_deref())
        });
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let mut command_context = crate::conn::CommandDispatchContext::default();
        let step = self.with_fixture(|conn| {
            conn.start_parsed_command_dispatch_with_context(command, &mut command_context)
        });
        let mut runtime_output_barrier = self.admit_runtime_output_barrier(command);
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut runtime_output_barrier,
        ))
        .await;

        if drain_after_command && completed {
            self.with_fixture(|conn| {
                crate::domains::activity::project_protocol_local_command_outputs(
                    conn,
                    &output_owner,
                    &mut command_context,
                )
            });
            protocol_events.extend(command_context.take_protocol_events());
            protocol_events.extend(command_context.take_post_response_events());
            scheduler_events.extend(self.with_fixture(|conn| conn.take_scheduler_events()));
        }

        if drain_after_command {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events))
                .await;
            if completed {
                if let Some(runtime_output_barrier) = runtime_output_barrier {
                    Box::pin(self.release_runtime_output_barrier_like_scheduler(
                        runtime_output_barrier,
                        true,
                    ))
                    .await;
                }
            } else {
                assert!(
                    runtime_output_barrier.is_none(),
                    "a pending Runtime command must transfer its output barrier to the pending reply"
                );
            }
            if !completed && !self.pending_runtime_deferred_replies.is_empty() {
                let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
            }
            Vec::new()
        } else {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, Vec::new())).await;
            if completed {
                if let Some(runtime_output_barrier) = runtime_output_barrier {
                    let mut release_scheduler_events =
                        Box::pin(self.release_runtime_output_barrier_like_scheduler(
                            runtime_output_barrier,
                            false,
                        ))
                        .await;
                    scheduler_events.append(&mut release_scheduler_events);
                }
            } else {
                assert!(
                    runtime_output_barrier.is_none(),
                    "a pending Runtime command must transfer its output barrier to the pending reply"
                );
            }
            if !completed && !self.pending_runtime_deferred_replies.is_empty() {
                let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
            }
            scheduler_events
        }
    }

    fn admit_runtime_output_barrier(
        &mut self,
        command: &ParsedCdpCommand,
    ) -> Option<RuntimeCommandOutputBarrierPermit> {
        command
            .runtime_command_executes_page_javascript()
            .then(|| {
                self.with_fixture(|conn| {
                    conn.admit_runtime_command_output_barrier(
                        command.request().id(),
                        command.command_output_session_id(),
                    )
                })
            })
            .flatten()
    }

    async fn complete_command_step_like_scheduler(
        &mut self,
        mut step: CdpCommandTaskStep,
        command_context: &mut CommandDispatchContext,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
        scheduler_events: &mut Vec<CdpSchedulerEvent>,
        runtime_output_barrier: &mut Option<RuntimeCommandOutputBarrierPermit>,
    ) -> bool {
        loop {
            match step {
                CdpCommandTaskStep::Complete(outcome) => {
                    let (
                        mut events,
                        mut post_renderer_output_events,
                        renderer_output_boundary,
                        mut post_response_events,
                        mut new_scheduler_events,
                        mut renderer_output_predecessor,
                    ) = outcome.into_renderer_owner_turn_parts();
                    if let Some(command_predecessor) =
                        command_context.take_renderer_output_predecessor()
                    {
                        command_predecessor
                            .merge_into_same_stream_tail(&mut renderer_output_predecessor);
                    }
                    if let Some(predecessor) = renderer_output_predecessor {
                        Box::pin(
                            self.route_renderer_output_predecessor_before_command_response(
                                predecessor,
                            ),
                        )
                        .await;
                    }
                    protocol_events.append(&mut events);
                    if let Some(renderer_output_boundary) = renderer_output_boundary {
                        // The production actor flushes the already-materialized
                        // prefix, admits the exact renderer publication, then
                        // continues with the suffix. Do the same here instead
                        // of letting the test harness's final batch flatten the
                        // independently transported commit in front of its
                        // Page.navigate/Fetch responses.
                        Box::pin(self.route_test_scheduler_causal_batch(
                            std::mem::take(protocol_events),
                            Vec::new(),
                        ))
                        .await;
                        Box::pin(
                            self.route_renderer_output_predecessor_before_command_response(
                                renderer_output_boundary,
                            ),
                        )
                        .await;
                    }
                    protocol_events.append(&mut post_renderer_output_events);
                    protocol_events.append(&mut post_response_events);
                    scheduler_events.append(&mut new_scheduler_events);
                    return true;
                }
                CdpCommandTaskStep::Pending(mut pending)
                    if pending.waits_for_scheduler_deferred_inspector_reply() =>
                {
                    let owner = pending
                        .runtime_command_owner_scope()
                        .expect("deferred Runtime commands have an exact owner")
                        .clone();
                    protocol_events
                        .extend(pending.take_scheduler_deferred_inspector_reply_events());
                    self.with_fixture(|conn| {
                        crate::domains::activity::project_protocol_local_command_outputs(
                            conn,
                            &owner,
                            command_context,
                        )
                    });
                    protocol_events.extend(command_context.take_protocol_events());
                    protocol_events.extend(command_context.take_post_response_events());
                    scheduler_events.extend(self.with_fixture(|conn| conn.take_scheduler_events()));
                    self.enqueue_pending_runtime_deferred_reply(
                        *pending,
                        std::mem::take(command_context),
                        runtime_output_barrier.take(),
                    );
                    return false;
                }
                CdpCommandTaskStep::Pending(pending) => {
                    // Keep the test scheduler's pending-command boundary shaped
                    // like production. Some domain completions carry a complete
                    // renderer Page build, so composing both futures inline can
                    // exceed Rust's default test-thread stack before the owner
                    // turn gets a chance to yield.
                    let completed = self.drive_owned_wait(pending.wait()).await;
                    step = self
                        .state
                        .with_state(|state| {
                            state.complete_command_dispatch_with_context(completed, command_context)
                        })
                        .expect("live test owner");
                }
            }
        }
    }

    /// Project the exact concrete renderer output owned by a command before
    /// exposing that command's response.
    ///
    /// Production performs the same fence in
    /// `flush_renderer_publication_predecessor`: the command carries a cursor,
    /// while the publication itself arrives over the renderer transport. The
    /// test scheduler must consume that real transport input rather than
    /// rescanning renderer state or draining unrelated work. Merely removing
    /// the publication from the ordered transport is insufficient: async
    /// owner actions in that batch must have returned as well.
    fn retain_renderer_insertion_output(
        &mut self,
        cursor: Option<moli_core::RendererOutputCursor>,
        events: Vec<BackgroundProtocolEvent>,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some(cursor) = cursor.filter(|cursor| {
            self.renderer_insertions
                .get(&cursor.stream())
                .is_some_and(|sequence| *sequence <= cursor.sequence())
        }) else {
            return events;
        };
        let (immediate, retained): (Vec<_>, Vec<_>) = events.into_iter().partition(|event| {
            event.is_network_protocol_observation()
                || event.command_response_payload_ref().is_some()
                || event.as_runtime_inspector_response_ready().is_some()
                || event.protocol_method().is_some_and(|method| {
                    // WebMCP can reject a cross-origin result at commit
                    // while a destination resource still blocks parsing.
                    method.starts_with("Fetch.")
                        || method.starts_with("IO.")
                        || method.starts_with("WebMCP.")
                })
        });
        if !retained.is_empty() {
            self.renderer_insertion_output.push_back((cursor, retained));
        }
        immediate
    }

    fn take_renderer_insertion_output(
        &mut self,
        cursor: moli_core::RendererOutputCursor,
    ) -> Vec<BackgroundProtocolEvent> {
        let release = self
            .renderer_insertions
            .get(&cursor.stream())
            .is_some_and(|sequence| *sequence <= cursor.sequence());
        if !release {
            return Vec::new();
        }
        self.renderer_insertions.remove(&cursor.stream());
        let mut retained = VecDeque::new();
        let mut events = Vec::new();
        while let Some((source, batch)) = self.renderer_insertion_output.pop_front() {
            if source.stream() == cursor.stream() {
                events.extend(batch);
            } else {
                retained.push_back((source, batch));
            }
        }
        self.renderer_insertion_output = retained;
        events
    }

    async fn release_renderer_insertion_output(&mut self, cursor: moli_core::RendererOutputCursor) {
        let events = self.take_renderer_insertion_output(cursor);
        Box::pin(self.route_test_scheduler_causal_batch(events, Vec::new())).await;
    }

    async fn route_renderer_output_predecessor_before_command_response(
        &mut self,
        predecessor: RendererOutputFence,
    ) {
        let cursor = predecessor.cursor();
        let projected = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            while !self.with_fixture(|conn| conn.renderer_output_cursor_is_projected(cursor)) {
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "renderer output predecessor lost its scheduler inputs: {predecessor:?}"
                );
            }
        })
        .await;
        assert!(
            projected.is_ok(),
            "timed out waiting for renderer output predecessor {predecessor:?}"
        );
        Box::pin(self.release_renderer_insertion_output(cursor)).await;
    }

    /// Admits one direct command's exact renderer fence through the production-
    /// shaped transport and ordered ingress.
    ///
    /// Protocol-neutral command tests do not have a parsed CDP response for
    /// `complete_command_step_like_scheduler()` to hold. They still must route
    /// the cursor rather than inspecting renderer state or manufacturing the
    /// owner action that the concrete publication contains.
    #[cfg(test)]
    pub(crate) async fn route_direct_command_renderer_predecessor_for_test(
        &mut self,
        predecessor: RendererOutputFence,
    ) {
        Box::pin(self.route_renderer_output_predecessor_before_command_response(predecessor)).await;
    }

    /// Completes one protocol-neutral command across the same concrete
    /// renderer-output boundary as the production actor.
    pub(crate) async fn execute_automation_command_through_renderer_fence_for_test(
        &mut self,
        command: AutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        let outcome = self
            .execute_owned_automation_command_for_test(command)
            .await;
        let (result, output) = outcome.into_result_and_owner_turn();
        let (messages, scheduler_events) =
            self.route_completed_command_outcome_for_test(output).await;
        self.sent.extend(messages);
        self.route_direct_command_output_for_test(Vec::new(), scheduler_events)
            .await;
        result
    }

    /// Snapshot immutable sidecars for assertions, while consuming the actual
    /// output through the original renderer insertion boundary exactly once.
    pub(crate) async fn execute_automation_with_captured_sidecars_for_test(
        &mut self,
        command: AutomationCommand,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<BackgroundProtocolEvent>,
    ) {
        let outcome = self
            .execute_owned_automation_command_for_test(command)
            .await;
        self.consume_automation_with_captured_sidecars_for_test(outcome)
            .await
    }

    pub(crate) async fn consume_automation_with_captured_sidecars_for_test(
        &mut self,
        outcome: crate::conn::AutomationCommandDispatchOutcome,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<BackgroundProtocolEvent>,
    ) {
        let (result, output) = outcome.into_result_and_owner_turn();
        let (before, after, boundary, post, scheduler, predecessor) =
            output.into_renderer_owner_turn_parts();
        let captured = before.iter().chain(&after).chain(&post).cloned().collect();
        let output = crate::conn::CdpTurnOutcome::new_with_protocol_and_post_response_events(
            before, post, scheduler,
        )
        .with_renderer_output_boundary(boundary, after)
        .with_renderer_output_predecessor(predecessor);
        let (messages, scheduler) = self.route_completed_command_outcome_for_test(output).await;
        self.sent.extend(messages);
        self.route_direct_command_output_for_test(Vec::new(), scheduler)
            .await;
        (result, captured)
    }

    /// Routes an explicitly completed command turn through the same ordered
    /// renderer boundary used by the production actor.
    ///
    /// A few ownership tests drive `PendingCdpCommandDispatch` directly so
    /// they can inspect its scheduler sidecars. They must still admit concrete
    /// renderer output at its exact position rather than flattening the
    /// outcome into a message-only vector.
    pub(crate) async fn route_completed_command_outcome_for_test(
        &mut self,
        outcome: crate::CdpRendererOwnerTurnOutcome,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        let sent_start = self.sent.len();
        let (
            before_renderer_output,
            post_renderer_output,
            renderer_output_boundary,
            post_response_events,
            scheduler_events,
            renderer_output_predecessor,
        ) = outcome.into_renderer_owner_turn_parts();
        if let Some(predecessor) = renderer_output_predecessor {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(predecessor))
                .await;
        }
        Box::pin(self.route_test_scheduler_causal_batch(before_renderer_output, Vec::new())).await;
        if let Some(boundary) = renderer_output_boundary {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(boundary))
                .await;
        } else {
            assert!(
                post_renderer_output.is_empty(),
                "post-renderer output requires an exact boundary"
            );
        }
        let mut suffix = post_renderer_output;
        suffix.extend(post_response_events);
        Box::pin(self.route_test_scheduler_causal_batch(suffix, Vec::new())).await;
        (self.sent.drain(sent_start..).collect(), scheduler_events)
    }

    async fn release_runtime_output_barrier_like_scheduler(
        &mut self,
        permit: RuntimeCommandOutputBarrierPermit,
        route_scheduler_events: bool,
    ) -> Vec<CdpSchedulerEvent> {
        let completion = self
            .state
            .with_state(|state| state.release_runtime_command_output_barrier_turn(permit))
            .expect("live test owner");
        let (protocol_events, scheduler_events) =
            completion.into_outcome().into_protocol_event_parts();
        if route_scheduler_events {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events))
                .await;
            Vec::new()
        } else {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, Vec::new())).await;
            scheduler_events
        }
    }

    fn enqueue_runtime_output_barrier_completion(
        &mut self,
        completion: RuntimeCommandOutputBarrierCompletion,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let (protocol_events, scheduler_events) =
            completion.into_outcome().into_protocol_event_parts();
        if !scheduler_events.is_empty() {
            work.push_front(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
        if !protocol_events.is_empty() {
            work.push_front(TestSchedulerWork::ProtocolEvents(protocol_events));
        }
    }

    async fn route_test_scheduler_causal_batch(
        &mut self,
        initial_events: Vec<BackgroundProtocolEvent>,
        initial_scheduler_events: Vec<CdpSchedulerEvent>,
    ) {
        // Route only output caused by the current scheduler input. Deferred
        // external inputs remain separate turns, matching the actor.
        let mut work = VecDeque::new();
        if !initial_events.is_empty() {
            work.push_back(TestSchedulerWork::ProtocolEvents(initial_events));
        }
        if !initial_scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(initial_scheduler_events));
        }
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
    }

    /// Routes one explicit renderer publication through the same capture,
    /// barrier, and concrete-residence path as the production adapter.
    ///
    /// This helper is for boundary tests that manufacture a typed publication
    /// instead of receiving it from a live renderer. It does not provide a
    /// direct output drain or a broad source scan.
    pub(crate) async fn route_renderer_publication_for_test(
        &mut self,
        publication: RendererOutputTransportMessage,
    ) -> Vec<Value> {
        let sent_start = self.sent.len();
        let cursor = match &publication {
            RendererOutputTransportMessage::Publication(publication) => Some(publication.cursor()),
            _ => None,
        };
        let mut work = VecDeque::from([TestSchedulerWork::RendererPublication(publication)]);
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
        if let Some(cursor) = cursor {
            tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
                while !self.with_fixture(|conn| conn.renderer_output_cursor_is_projected(cursor)) {
                    assert!(
                        matches!(
                            Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                            TestSchedulerTurnOutcome::Processed(_)
                        ),
                        "explicit renderer publication lost its owner work"
                    );
                }
            })
            .await
            .expect("explicit renderer projection should finish");
            Box::pin(self.release_renderer_insertion_output(cursor)).await;
        }
        self.sent.drain(sent_start..).collect()
    }

    fn enqueue_pending_runtime_deferred_reply(
        &mut self,
        mut pending: PendingCdpCommandDispatch,
        command_context: CommandDispatchContext,
        runtime_output_barrier: Option<RuntimeCommandOutputBarrierPermit>,
    ) {
        if let Some(command_id) = pending.command_id()
            && let Some(response_rx) = pending.take_scheduler_deferred_inspector_reply_receiver()
        {
            let session_id = pending.session_id().map(str::to_owned);
            let response_tx = self.with_fixture(|conn| {
                conn.runtime_inspector_response_ready_sender()
                    .expect("test scheduler should install its typed runtime-response channel")
            });
            tokio::spawn(async move {
                let response = response_rx.await.map_err(|error| {
                    anyhow::Error::new(error).context("RuntimeDeferredInspectorResponseCanceled")
                });
                let _ = response_tx.send(RuntimeInspectorResponseReady::new(
                    command_id,
                    session_id.as_deref(),
                    response,
                ));
            });
        }
        self.pending_runtime_deferred_replies
            .push_back(PendingTestRuntimeDeferredReply::new(
                pending,
                command_context,
                runtime_output_barrier,
            ));
    }

    async fn route_test_scheduler_work_queue(&mut self, work: &mut VecDeque<TestSchedulerWork>) {
        self.route_ready_protocol_scheduler_work_for_test_context(work)
            .await;
        while let Some(item) = work.pop_front() {
            match item {
                TestSchedulerWork::ProtocolWorkCompleted(completed) => {
                    let renderer_cursor = completed.renderer_projection_cursor();
                    let output = self
                        .state
                        .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                        .expect("live test owner");
                    let (events, scheduler_events) = if renderer_cursor.is_some() {
                        let (events, scheduler) = output.into_protocol_event_parts();
                        (
                            self.retain_renderer_insertion_output(renderer_cursor, events),
                            scheduler,
                        )
                    } else {
                        let (messages, scheduler) =
                            Box::pin(self.route_completed_command_outcome_for_test(
                                output.with_renderer_output_predecessor(None),
                            ))
                            .await;
                        self.sent.extend(messages);
                        (Vec::new(), scheduler)
                    };
                    if !events.is_empty() {
                        work.push_front(TestSchedulerWork::ProtocolEvents(events));
                    }
                    if !scheduler_events.is_empty() {
                        work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
                    }
                }
                TestSchedulerWork::ProtocolEvents(events) => {
                    Box::pin(self.route_protocol_events_like_scheduler(events, work)).await;
                }
                TestSchedulerWork::SchedulerEvents(scheduler_events) => {
                    Box::pin(self.route_scheduler_events_for_test_context(scheduler_events, work))
                        .await;
                }
                TestSchedulerWork::BackgroundEvent(event) => {
                    Box::pin(self.route_protocol_events_like_scheduler(vec![event], work)).await;
                }
                TestSchedulerWork::BackgroundNavigationCompletion(completion) => {
                    Box::pin(
                        self.route_background_navigation_completion_like_scheduler(
                            completion, work,
                        ),
                    )
                    .await;
                }
                TestSchedulerWork::RuntimeDeferredReplyReady(response) => {
                    Box::pin(self.complete_runtime_response_ready_like_scheduler(
                        response,
                        Vec::new(),
                        work,
                    ))
                    .await;
                }
                TestSchedulerWork::RendererPublication(publication) => {
                    Box::pin(self.ingest_renderer_publication_like_scheduler(publication, work))
                        .await;
                }
                TestSchedulerWork::ReleaseRuntimeOutputBarrier(permit) => {
                    let completion = self
                        .state
                        .with_state(|state| {
                            state.release_runtime_command_output_barrier_turn(permit)
                        })
                        .expect("live test owner");
                    self.enqueue_runtime_output_barrier_completion(completion, work);
                }
                TestSchedulerWork::CancelRuntimeOutputBarrier(permit) => {
                    let completion = self
                        .state
                        .with_state(|state| {
                            state.cancel_runtime_command_output_barrier_turn(permit)
                        })
                        .expect("live test owner");
                    self.enqueue_runtime_output_barrier_completion(completion, work);
                }
            }
            self.route_ready_protocol_scheduler_work_for_test_context(work)
                .await;
            if let Some(Some(completed)) = self.pending_owned_protocol_work.next().now_or_never() {
                work.push_back(TestSchedulerWork::ProtocolWorkCompleted(completed));
            }
        }
    }

    async fn route_background_navigation_completion_like_scheduler(
        &mut self,
        completion: crate::domains::page::BackgroundNavigationCompletion,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        // Match the production actor's three-part boundary:
        //
        //   already-produced navigation output
        //   -> exact renderer Page cursor
        //   -> navigation commit output
        //
        // The event and renderer transports are independent, so flattening
        // them after the fact would allow the commit cursor to move the new
        // realm in front of frameStartedNavigating/Page.navigate's response.
        let mut prefix = Vec::new();
        while let Ok(event) = self.background_event_rx.try_recv() {
            prefix.push(event);
        }
        if !prefix.is_empty() {
            Box::pin(self.route_protocol_events_like_scheduler(prefix, work)).await;
        }

        let outcome = self
            .state
            .with_state(|state| state.admit_background_navigation_completion(completion))
            .expect("live test owner");
        let (
            mut completion_prefix,
            mut completion_suffix,
            renderer_output_boundary,
            mut post_response_events,
            scheduler_events,
            renderer_output_predecessor,
        ) = outcome.into_renderer_owner_turn_parts();
        assert!(
            renderer_output_predecessor.is_none(),
            "background navigation completion must use an insertion boundary"
        );
        if !completion_prefix.is_empty() {
            Box::pin(self.route_protocol_events_like_scheduler(
                std::mem::take(&mut completion_prefix),
                work,
            ))
            .await;
        }
        if let Some(boundary) = renderer_output_boundary {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(boundary))
                .await;
        }
        completion_suffix.append(&mut post_response_events);
        while let Ok(event) = self.background_event_rx.try_recv() {
            completion_suffix.push(event);
        }
        if !completion_suffix.is_empty() {
            work.push_back(TestSchedulerWork::ProtocolEvents(completion_suffix));
        }
        if !scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
    }

    async fn run_one_ready_test_scheduler_turn(&mut self) -> TestSchedulerTurnOutcome {
        let mut work = VecDeque::new();
        let scheduler_events = self
            .state
            .with_state(|state| state.take_scheduler_events())
            .expect("live test owner");
        if !scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
            Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
            return TestSchedulerTurnOutcome::Processed(TestSchedulerInputKind::StateEvents);
        }
        if let Some(Some(completed)) = self.pending_owned_protocol_work.next().now_or_never() {
            work.push_back(TestSchedulerWork::ProtocolWorkCompleted(completed));
            Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
            return TestSchedulerTurnOutcome::Processed(TestSchedulerInputKind::ProtocolWork);
        }
        let input_kind = if self.background_navigation_scheduler_enabled
            && let Ok(completion) = self.background_navigation_completion_rx.try_recv()
        {
            work.push_back(TestSchedulerWork::BackgroundNavigationCompletion(
                completion,
            ));
            TestSchedulerInputKind::BackgroundNavigationCompletion
        } else if self.background_navigation_scheduler_enabled
            && let Ok(event) = self.background_event_rx.try_recv()
        {
            work.push_back(TestSchedulerWork::BackgroundEvent(event));
            TestSchedulerInputKind::BackgroundEvent
        } else if !self.pending_runtime_deferred_replies.is_empty() {
            match self.runtime_inspector_response_ready_rx.try_recv() {
                Ok(response) => {
                    work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
                    TestSchedulerInputKind::RuntimeDeferredReply
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if let Ok(publication) = self.renderer_publication_rx.try_recv() {
                        work.push_back(TestSchedulerWork::RendererPublication(publication));
                        TestSchedulerInputKind::RendererPublication
                    } else {
                        return TestSchedulerTurnOutcome::Idle;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return TestSchedulerTurnOutcome::Idle;
                }
            }
        } else if let Ok(publication) = self.renderer_publication_rx.try_recv() {
            work.push_back(TestSchedulerWork::RendererPublication(publication));
            TestSchedulerInputKind::RendererPublication
        } else if let Ok(response) = self.runtime_inspector_response_ready_rx.try_recv() {
            work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
            TestSchedulerInputKind::RuntimeDeferredReply
        } else {
            return TestSchedulerTurnOutcome::Idle;
        };
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
        TestSchedulerTurnOutcome::Processed(input_kind)
    }

    pub(crate) async fn wait_for_test_command_response(
        &mut self,
        command_id: u64,
        response_start: usize,
    ) {
        let response = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            loop {
                if self
                    .sent
                    .get(response_start..)
                    .unwrap_or_default()
                    .iter()
                    .any(|message| message.get("id").and_then(Value::as_u64) == Some(command_id))
                {
                    return;
                }
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "CDP command `{command_id}` lost all scheduler input before its response"
                );
            }
        })
        .await;
        if response.is_err() {
            panic!(
                "timed out waiting for CDP command `{command_id}` response; sent={:?}; diagnostics={}",
                self.sent,
                self.with_fixture(|conn| conn.moli_memory_diagnostics())
            );
        }
    }

    async fn route_ready_test_command_response(&mut self, command_id: u64, response_start: usize) {
        while !self
            .sent
            .get(response_start..)
            .unwrap_or_default()
            .iter()
            .any(|message| message.get("id").and_then(Value::as_u64) == Some(command_id))
            && matches!(
                Box::pin(self.run_one_ready_test_scheduler_turn()).await,
                TestSchedulerTurnOutcome::Processed(_)
            )
        {}
    }

    async fn wait_for_one_test_scheduler_turn(&mut self) -> TestSchedulerTurnOutcome {
        let ready = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
        if matches!(ready, TestSchedulerTurnOutcome::Processed(_)) {
            return ready;
        }

        let mut work = VecDeque::new();
        let state_ready = self.state.scheduler_events_ready();
        let background_navigation_scheduler_enabled = self.background_navigation_scheduler_enabled;
        let input_kind = if !self.pending_runtime_deferred_replies.is_empty() {
            tokio::select! {
                biased;
                Some(completed) = self.pending_owned_protocol_work.next(), if !self.pending_owned_protocol_work.is_empty() => {
                    work.push_back(TestSchedulerWork::ProtocolWorkCompleted(completed));
                    TestSchedulerInputKind::ProtocolWork
                }
                _ = state_ready.notified() => {
                    let events = self.state.with_state(|state| state.take_scheduler_events()).expect("live test owner");
                    work.push_back(TestSchedulerWork::SchedulerEvents(events));
                    TestSchedulerInputKind::StateEvents
                }
                maybe_response = self.runtime_inspector_response_ready_rx.recv() => {
                    let Some(response) = maybe_response else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
                    TestSchedulerInputKind::RuntimeDeferredReply
                }
                maybe_completion = self.background_navigation_completion_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(completion) = maybe_completion else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundNavigationCompletion(completion));
                    TestSchedulerInputKind::BackgroundNavigationCompletion
                }
                maybe_event = self.background_event_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(event) = maybe_event else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundEvent(event));
                    TestSchedulerInputKind::BackgroundEvent
                }
                maybe_publication = self.renderer_publication_rx.recv() => {
                    let Some(publication) = maybe_publication else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RendererPublication(publication));
                    TestSchedulerInputKind::RendererPublication
                }
            }
        } else {
            tokio::select! {
                biased;
                Some(completed) = self.pending_owned_protocol_work.next(), if !self.pending_owned_protocol_work.is_empty() => {
                    work.push_back(TestSchedulerWork::ProtocolWorkCompleted(completed));
                    TestSchedulerInputKind::ProtocolWork
                }
                _ = state_ready.notified() => {
                    let events = self.state.with_state(|state| state.take_scheduler_events()).expect("live test owner");
                    work.push_back(TestSchedulerWork::SchedulerEvents(events));
                    TestSchedulerInputKind::StateEvents
                }
                maybe_completion = self.background_navigation_completion_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(completion) = maybe_completion else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundNavigationCompletion(completion));
                    TestSchedulerInputKind::BackgroundNavigationCompletion
                }
                maybe_event = self.background_event_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(event) = maybe_event else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundEvent(event));
                    TestSchedulerInputKind::BackgroundEvent
                }
                maybe_publication = self.renderer_publication_rx.recv() => {
                    let Some(publication) = maybe_publication else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RendererPublication(publication));
                    TestSchedulerInputKind::RendererPublication
                }
                maybe_response = self.runtime_inspector_response_ready_rx.recv() => {
                    let Some(response) = maybe_response else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
                    TestSchedulerInputKind::RuntimeDeferredReply
                }
            }
        };
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
        TestSchedulerTurnOutcome::Processed(input_kind)
    }

    async fn route_protocol_events_like_scheduler(
        &mut self,
        mut events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        loop {
            if events.is_empty() {
                return;
            }
            if let Some(position) = events
                .iter()
                .position(|event| event.as_runtime_inspector_response_ready().is_some())
            {
                self.sent.extend(
                    events
                        .drain(..position)
                        .filter_map(BackgroundProtocolEvent::into_wire_message),
                );
                let response = events
                    .remove(0)
                    .take_runtime_inspector_response_ready()
                    .expect("runtime response event position should contain typed response");
                Box::pin(
                    self.complete_runtime_response_ready_like_scheduler(response, events, work),
                )
                .await;
                return;
            }
            let pending_ids = self.pending_runtime_deferred_reply_command_ids();
            if pending_ids.is_empty() {
                self.sent.extend(
                    events
                        .into_iter()
                        .filter_map(BackgroundProtocolEvent::into_wire_message),
                );
                return;
            }
            let Some(position) = events.iter().position(|event| {
                event
                    .protocol_message()
                    .and_then(|message| message.get("id"))
                    .and_then(Value::as_u64)
                    .is_some_and(|id| pending_ids.contains(&id))
            }) else {
                self.sent.extend(
                    events
                        .into_iter()
                        .filter_map(BackgroundProtocolEvent::into_wire_message),
                );
                return;
            };
            self.sent.extend(
                events
                    .drain(..position)
                    .filter_map(BackgroundProtocolEvent::into_wire_message),
            );
            let message = events.remove(0).into_protocol_message();
            let Some(command_id) = message.get("id").and_then(Value::as_u64) else {
                self.sent.push(message);
                continue;
            };
            let Some(index) = self
                .pending_runtime_deferred_replies
                .iter()
                .position(|pending| pending.command_id() == Some(command_id))
            else {
                self.sent.push(message);
                continue;
            };
            let mut pending = self
                .pending_runtime_deferred_replies
                .remove(index)
                .expect("pending runtime deferred reply index should exist");
            self.state
                .with_state(|state| {
                    state.forget_scheduler_deferred_inspector_reply(pending.pending)
                })
                .expect("live test owner");
            self.sent.push(json!({
                "id": command_id,
                "error": {
                    "code": -32000,
                    "message": "RuntimeDeferredReplyLooseProtocolResponse",
                },
            }));
            if let Some(runtime_output_barrier) = pending.runtime_output_barrier.take() {
                work.push_front(TestSchedulerWork::CancelRuntimeOutputBarrier(
                    runtime_output_barrier,
                ));
            }
            if !events.is_empty() {
                work.push_front(TestSchedulerWork::ProtocolEvents(events));
            }
            return;
        }
    }

    async fn route_scheduler_events_for_test_context(
        &mut self,
        scheduler_events: Vec<CdpSchedulerEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        for event in scheduler_events {
            match event {
                CdpSchedulerEvent::RendererOutputProjectionPrepared { pending } => {
                    self.pending_owned_protocol_work
                        .push(pending.wait().boxed_local());
                }
                CdpSchedulerEvent::ProtocolWorkPublished { work } => {
                    self.pending_protocol_scheduler_work.push_back(work);
                }
                CdpSchedulerEvent::PageScreencastStarted { .. } => {}
            }
        }
        let scheduler_events = self
            .state
            .with_state(|state| state.take_scheduler_events())
            .expect("live test owner");
        if !scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
    }

    async fn route_ready_protocol_scheduler_work_for_test_context(
        &mut self,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        loop {
            let selected_index = match self.pending_protocol_scheduler_work.front() {
                Some(front)
                    if front.is_ready()
                        && self
                            .state
                            .with_state(|state| state.protocol_work_predecessor_is_projected(front))
                            .expect("live test owner") =>
                {
                    0
                }
                Some(front)
                    if front.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction =>
                {
                    let Some(index) =
                        self.pending_protocol_scheduler_work
                            .iter()
                            .position(|candidate| {
                                candidate.is_ready()
                                    && self
                                        .state
                                        .with_state(|state| {
                                            state.protocol_work_predecessor_is_projected(candidate)
                                        })
                                        .expect("live test owner")
                                    && candidate.navigation_dependency()
                                        == crate::ProtocolNavigationDependency::ReplacesPendingLoad
                            })
                    else {
                        return;
                    };
                    // Production checks the pending load observer out of the
                    // FIFO while it waits for the renderer. An unconstrained
                    // location owner action may then run and replace that
                    // exact source Document, which completes the observer as
                    // Superseded. Keeping the pending observer at the front in
                    // this protocol-only harness would deadlock the action
                    // needed to make it terminal.
                    index
                }
                Some(_) | None => return,
            };
            if !self.background_navigation_scheduler_enabled
                && self
                    .pending_protocol_scheduler_work
                    .get(selected_index)
                    .is_some_and(ProtocolSchedulerWork::requires_background_navigation_scheduler)
            {
                // The default protocol fixture has no navigation task lane.
                // Keep independent popup work queued until a navigation test
                // opts into the background scheduler and drives its owned
                // completions, as the production actor does.
                return;
            }
            let protocol_work = self
                .pending_protocol_scheduler_work
                .remove(selected_index)
                .expect("ready protocol work must remain resident");
            let renderer_boundaries = protocol_work
                .renderer_output_predecessors()
                .iter()
                .map(moli_core::RendererOutputFence::cursor)
                .collect::<Vec<_>>();
            match self
                .state
                .with_state(|state| state.try_start_owned_protocol_scheduler_work(protocol_work))
                .expect("live test owner")
            {
                Ok(Some(pending)) => self
                    .pending_owned_protocol_work
                    .push(pending.wait().boxed_local()),
                Ok(None) => {}
                Err(work_item) => {
                    let outcome = self
                        .state
                        .with_state(|state| {
                            state.try_complete_protocol_scheduler_work_turn(*work_item)
                        })
                        .expect("live test owner")
                        .expect("finite protocol work");
                    let (events, scheduler_events) = outcome.into_protocol_event_parts();
                    let mut prefix = renderer_boundaries
                        .into_iter()
                        .flat_map(|cursor| self.take_renderer_insertion_output(cursor))
                        .collect::<Vec<_>>();
                    prefix.extend(events);
                    if !prefix.is_empty() {
                        work.push_back(TestSchedulerWork::ProtocolEvents(prefix));
                    }
                    if !scheduler_events.is_empty() {
                        work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
                    }
                }
            }
        }
    }

    async fn ingest_renderer_publication_like_scheduler(
        &mut self,
        publication: RendererOutputTransportMessage,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let scheduler_events = self.with_fixture(|conn| conn.take_scheduler_events());
        if !scheduler_events.is_empty() {
            work.push_front(TestSchedulerWork::RendererPublication(publication));
            work.push_front(TestSchedulerWork::SchedulerEvents(scheduler_events));
            return;
        }
        if let RendererOutputTransportMessage::Publication(publication) = &publication
            && publication.records().iter().any(|record| {
                matches!(
                    record.item(),
                    moli_core::RendererOutputItem::Observation(
                        moli_core::RendererProtocolObservation::MainDocumentCommit { .. }
                    )
                )
            })
        {
            let cursor = publication.cursor();
            self.renderer_insertions
                .entry(cursor.stream())
                .or_insert(cursor.sequence());
        }
        // Native projection keeps advancing. Only its materialized protocol
        // effects wait for the source command's exact renderer insertion.
        let outcome = self
            .state
            .with_state(|state| state.admit_renderer_output(publication))
            .expect("live test owner");
        let (protocol_events, scheduler_events) = outcome.into_protocol_event_parts();
        if !protocol_events.is_empty() {
            work.push_back(TestSchedulerWork::ProtocolEvents(protocol_events));
        }
        if !scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
    }

    fn pending_runtime_deferred_reply_command_ids(&self) -> Vec<u64> {
        self.pending_runtime_deferred_replies
            .iter()
            .filter_map(PendingTestRuntimeDeferredReply::command_id)
            .collect()
    }

    async fn complete_runtime_response_ready_like_scheduler(
        &mut self,
        response: RuntimeInspectorResponseReady,
        suffix_events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        // Production admits an exact renderer cursor before resolving the
        // response correlation. Keep the test scheduler on the same boundary:
        // a SharedWorker `Destroyed` publication must terminate the pending
        // call with `Target closed` before a later V8 context-destruction
        // response can become visible.
        if let Some(predecessor) = response.renderer_output_predecessor() {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(predecessor))
                .await;
        }
        let command_id = response.command_id();
        let Some(index) = self
            .pending_runtime_deferred_replies
            .iter()
            .position(|pending| pending.command_id() == Some(command_id))
        else {
            let mut response_events = Vec::new();
            let mut background_events = Vec::new();
            self.with_fixture(|conn| {
                conn.route_registered_runtime_inspector_response_into(
                    response,
                    &mut response_events,
                    &mut background_events,
                )
            });
            if !background_events.is_empty() {
                work.push_front(TestSchedulerWork::ProtocolEvents(background_events));
            }
            self.sent.extend(
                response_events
                    .into_iter()
                    .filter_map(BackgroundProtocolEvent::into_wire_message),
            );
            if !suffix_events.is_empty() {
                work.push_front(TestSchedulerWork::ProtocolEvents(suffix_events));
            }
            return;
        };
        let pending = self
            .pending_runtime_deferred_replies
            .remove(index)
            .expect("pending runtime deferred reply index should exist");
        Box::pin(
            self.complete_renderer_runtime_deferred_response_like_scheduler(
                pending,
                response,
                suffix_events,
                work,
            ),
        )
        .await;
    }

    async fn complete_renderer_runtime_deferred_response_like_scheduler(
        &mut self,
        mut pending: PendingTestRuntimeDeferredReply,
        response: RuntimeInspectorResponseReady,
        suffix_events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        if pending.pending.command_id().is_none() {
            self.pending_runtime_deferred_replies.push_back(pending);
            return;
        }
        self.state
            .with_state(|state| {
                state.route_scheduler_deferred_inspector_response(&mut pending.pending, response)
            })
            .expect("live test owner");
        Box::pin(self.complete_runtime_deferred_reply_like_scheduler(pending, suffix_events, work))
            .await;
    }

    async fn complete_runtime_deferred_reply_like_scheduler(
        &mut self,
        mut pending: PendingTestRuntimeDeferredReply,
        suffix_events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let completed = self
            .state
            .with_state(|state| state.complete_scheduler_deferred_inspector_reply(pending.pending))
            .expect("live test owner");
        let step = self
            .state
            .with_state(|state| {
                state
                    .complete_command_dispatch_with_context(completed, &mut pending.command_context)
            })
            .expect("live test owner");
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut pending.command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut pending.runtime_output_barrier,
        ))
        .await;
        if !suffix_events.is_empty() {
            work.push_front(TestSchedulerWork::ProtocolEvents(suffix_events));
        }
        if completed {
            if let Some(runtime_output_barrier) = pending.runtime_output_barrier {
                work.push_front(TestSchedulerWork::ReleaseRuntimeOutputBarrier(
                    runtime_output_barrier,
                ));
            }
        } else {
            assert!(
                pending.runtime_output_barrier.is_none(),
                "a still-pending Runtime command must transfer its output barrier"
            );
        }
        if !scheduler_events.is_empty() {
            work.push_front(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
        if !protocol_events.is_empty() {
            work.push_front(TestSchedulerWork::ProtocolEvents(protocol_events));
        }
    }

    // ── Assertion helpers ─────────────────────────────────────────────────────

    /// Assert that a `{id, result, sessionId?}` message exists in the sent
    /// queue and remove it.  `session_id = None` means "no sessionId field".
    pub fn expect_result(&mut self, id: u64, result: Value, session_id: Option<&str>) {
        let expected = build_result(id, &result, session_id);
        self.find_and_remove(&expected, "result");
    }

    /// Assert that a `{id, error: {code, message}}` message exists.
    pub fn expect_error(&mut self, id: u64, code: i32, message: &str) {
        let expected = json!({ "id": id, "error": { "code": code, "message": message } });
        self.find_and_remove(&expected, "error");
    }

    /// Assert that an event `{method, params, sessionId?}` message exists.
    /// When `params` is `None`, only the method name is checked.
    pub fn expect_event(&mut self, method: &str, params: Option<&Value>) {
        let pos = self.sent.iter().position(|v| {
            if v["method"].as_str() != Some(method) {
                return false;
            }
            if let Some(expected_params) = params {
                values_subset(expected_params, &v["params"])
            } else {
                true // any params
            }
        });
        match pos {
            Some(i) => {
                self.sent.remove(i);
            }
            None => {
                let queue: String = self.sent.iter().map(|v| format!("  {}\n", v)).collect();
                panic!("expected event '{method}' not found in sent queue:\n{queue}");
            }
        }
    }

    /// Take and return the next message; panics if the queue is empty.
    pub fn take_one(&mut self) -> Value {
        if self.sent.is_empty() {
            panic!("expected a message in the sent queue but it is empty");
        }
        self.sent.remove(0)
    }

    /// Take and return the first sent message matching the predicate.
    pub fn take_first_matching(
        &mut self,
        description: &str,
        matches: impl FnMut(&Value) -> bool,
    ) -> Value {
        let pos = self
            .sent
            .iter()
            .position(matches)
            .unwrap_or_else(|| panic!("expected {description} in sent queue: {:?}", self.sent));
        self.sent.remove(pos)
    }

    /// Take and return the response with the requested id.
    pub fn take_response_by_id(&mut self, id: u64) -> Value {
        let pos = self
            .sent
            .iter()
            .position(|message| message["id"] == json!(id))
            .unwrap_or_else(|| {
                panic!(
                    "expected a response with id {id} in sent queue: {:?}",
                    self.sent
                )
            });
        self.sent.remove(pos)
    }

    /// Drain all pending sent messages (useful to discard setup noise).
    pub fn take_all(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.sent)
    }

    /// Completes at most one scheduler input that is already ready.
    ///
    /// Unlike the removed broad test capture, this cannot snapshot an
    /// arbitrary session. It only consumes a concrete renderer publication, Runtime
    /// response, scheduler event, or ready `ProtocolSchedulerWork` already
    /// resident in the production-shaped harness.
    #[cfg(test)]
    pub(crate) async fn complete_one_ready_scheduler_input_for_test(&mut self) {
        let scheduler_events = self.with_fixture(|conn| conn.take_scheduler_events());
        Box::pin(self.route_test_scheduler_causal_batch(Vec::new(), scheduler_events)).await;
        let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn find_and_remove(&mut self, expected: &Value, kind: &str) {
        let pos = self.sent.iter().position(|v| values_subset(expected, v));
        match pos {
            Some(i) => {
                self.sent.remove(i);
            }
            None => {
                let queue: String = self.sent.iter().map(|v| format!("  {}\n", v)).collect();
                panic!("expected {kind} not found.\nExpected:\n  {expected}\nSent queue:\n{queue}");
            }
        }
    }
}

fn protocol_events_into_messages(events: Vec<BackgroundProtocolEvent>) -> Vec<Value> {
    events
        .into_iter()
        .filter_map(BackgroundProtocolEvent::into_wire_message)
        .collect()
}

#[cfg(test)]
pub(crate) fn protocol_events_into_internal_messages(
    events: Vec<BackgroundProtocolEvent>,
) -> Vec<Value> {
    events
        .into_iter()
        .filter(BackgroundProtocolEvent::has_protocol_wire_message)
        .map(|event| event.into_parts().0)
        .collect()
}

/// Collects explicitly ready work through the same owned admission and
/// completion used by the actor. Pending transport inputs belong in TestContext.
#[cfg(test)]
pub(crate) async fn collect_ready_scheduler_output(
    state: &crate::ProtocolStateHandle,
    out: &mut Vec<Value>,
    scheduler_events: Vec<CdpSchedulerEvent>,
    materialize: fn(BackgroundProtocolEvent) -> Value,
) {
    let mut queue = VecDeque::from(scheduler_events);
    while let Some(event) = queue.pop_front() {
        let outcome = match event {
            CdpSchedulerEvent::RendererOutputProjectionPrepared { pending } => {
                let completed = pending.wait().await;
                state
                    .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                    .expect("live test owner")
            }
            CdpSchedulerEvent::ProtocolWorkPublished { work } => {
                assert!(
                    work.is_ready()
                        && state
                            .with_state(|state| state.protocol_work_predecessor_is_projected(&work))
                            .expect("live test owner"),
                    "pending work requires the stateful test scheduler"
                );
                match state
                    .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                    .expect("live test owner")
                {
                    Ok(Some(pending)) => {
                        let completed = pending.wait().await;
                        state
                            .with_state(|state| {
                                state.complete_owned_protocol_scheduler_work(completed)
                            })
                            .expect("live test owner")
                    }
                    Ok(None) => continue,
                    Err(work) => state
                        .with_state(|state| state.try_complete_protocol_scheduler_work_turn(*work))
                        .expect("live test owner")
                        .expect("finite work"),
                }
            }
            CdpSchedulerEvent::PageScreencastStarted { .. } => continue,
        };
        let (events, scheduler_events) = outcome.into_protocol_event_parts();
        out.extend(
            events
                .into_iter()
                .filter(BackgroundProtocolEvent::has_protocol_wire_message)
                .map(materialize),
        );
        queue.extend(scheduler_events);
        queue.extend(
            state
                .with_state(|state| state.take_scheduler_events())
                .expect("live test owner"),
        );
    }
}

#[cfg(test)]
pub(crate) trait TestSessionId<'a> {}

#[cfg(test)]
impl<'a> TestSessionId<'a> for Option<&'a str> {}

#[cfg(test)]
impl<'a> TestSessionId<'a> for &'a str {}

#[cfg(test)]
pub async fn spawn_connection_drop_server() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let _ = stream.shutdown().await;
    });
    (addr, server)
}

#[cfg(test)]
pub(crate) async fn wait_until_message(
    ctx: &mut TestContext,
    session_id: impl TestSessionId<'_> + Copy,
    description: &str,
    predicate: impl Fn(&Value) -> bool,
) {
    wait_until_messages(ctx, session_id, description, |messages| {
        messages.iter().any(&predicate)
    })
    .await;
}

#[cfg(test)]
pub(crate) async fn wait_until_messages(
    ctx: &mut TestContext,
    _session_id: impl TestSessionId<'_> + Copy,
    description: &str,
    predicate: impl Fn(&[Value]) -> bool,
) {
    for _ in 0..256 {
        if predicate(&ctx.sent) {
            return;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        if predicate(&ctx.sent) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ctx.complete_one_ready_scheduler_input_for_test().await;
    if predicate(&ctx.sent) {
        return;
    }
    panic!("timed out waiting for {description}; sent={:?}", ctx.sent);
}

/// Wait for a protocol message produced by a real scheduler input without
/// synthesizing broad capture turns or moving messages already collected in
/// `ctx.sent`.
///
/// Resource and runtime completions publish typed wakes into `TestContext`.
/// Tests asserting those completion boundaries should block on that channel
/// instead of tying correctness to a polling iteration budget.
#[cfg(test)]
pub(crate) async fn wait_until_scheduler_message(
    ctx: &mut TestContext,
    description: &str,
    predicate: impl Fn(&Value) -> bool,
) {
    let waited = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
        loop {
            if ctx.sent.iter().any(&predicate) {
                return;
            }
            assert!(
                matches!(
                    Box::pin(ctx.wait_for_one_test_scheduler_turn()).await,
                    TestSchedulerTurnOutcome::Processed(_)
                ),
                "test scheduler lost all external input while waiting for {description}"
            );
        }
    })
    .await;

    if waited.is_err() {
        panic!(
            "timed out waiting for {description} from a real scheduler input; sent={:?}",
            ctx.sent
        );
    }
}

/// Wait for the terminal loading event of one concrete frame while preserving
/// the complete event sequence for assertions that follow.
///
/// `Page.navigate` only acknowledges that navigation was accepted; Chromium
/// likewise allows document replacement to continue after that response. DOM
/// tests must synchronize with the frame lifecycle before retaining frontend
/// node ids from the new document.
#[cfg(test)]
pub(crate) async fn wait_until_frame_stopped_loading(ctx: &mut TestContext, frame_id: &str) {
    let description = format!("Page.frameStoppedLoading for {frame_id}");
    wait_until_scheduler_message(ctx, &description, |message| {
        message["method"] == json!("Page.frameStoppedLoading")
            && message["params"]["frameId"] == json!(frame_id)
    })
    .await;
}

/// Wait for the renderer-owned load fact of one exact document generation.
///
/// Unlike `Page.frameStoppedLoading`, the authoritative binding carries the
/// loader id, so a test cannot accidentally accept a terminal event left by an
/// older document in the same frame.
#[cfg(test)]
pub(crate) async fn wait_until_renderer_document_load(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
) {
    let description = format!("renderer load for {frame_id}/{loader_id}");
    ctx.wait_until_scheduler_state(&description, |conn| {
        conn.renderer_document_lifecycle_authoritative_state_for_session_owner(session_id)
            .is_some_and(|(binding, snapshot)| {
                binding.frame_id == frame_id
                    && binding.loader_id == loader_id
                    && snapshot.load.is_some()
            })
    })
    .await;
}

/// Build a result message, omitting sessionId when it is None.
fn build_result(id: u64, result: &Value, session_id: Option<&str>) -> Value {
    let mut v = json!({ "id": id, "result": result });
    if let Some(sid) = session_id {
        v["sessionId"] = json!(sid);
    }
    v
}

/// Return true when every field of `expected` appears in `actual` with the
/// same value.  Arrays and nested objects are compared recursively.
fn values_subset(expected: &Value, actual: &Value) -> bool {
    match expected {
        Value::Object(exp_map) => {
            let Value::Object(act_map) = actual else {
                return false;
            };
            exp_map.iter().all(|(k, ev)| {
                act_map
                    .get(k)
                    .map(|av| values_subset(ev, av))
                    .unwrap_or(false)
            })
        }
        Value::Array(exp_arr) => {
            let Value::Array(act_arr) = actual else {
                return false;
            };
            if exp_arr.len() != act_arr.len() {
                return false;
            }
            exp_arr
                .iter()
                .zip(act_arr.iter())
                .all(|(e, a)| values_subset(e, a))
        }
        _ => expected == actual,
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{PageId, RendererRuntimeInspectorAsyncCompletion};

    use super::*;

    #[tokio::test]
    async fn test_context_drops_unmatched_typed_runtime_response_like_scheduler() {
        let mut ctx = TestContext::new();
        let response = RuntimeInspectorResponseReady::new(
            42,
            None,
            Ok(
                RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                    42,
                    json!({
                        "id": 42,
                        "result": {}
                    }),
                ),
            ),
        );

        ctx.complete_runtime_response_ready_like_scheduler(
            response,
            Vec::new(),
            &mut VecDeque::new(),
        )
        .await;

        assert!(
            ctx.sent.is_empty(),
            "unmatched typed runtime completion must stay internal in the test harness too"
        );
    }

    #[tokio::test]
    async fn test_context_consumes_selected_task_output_publications_one_per_turn() {
        let mut conn = CdpConnection::new();
        let (publication_tx, publication_rx) = moli_core::renderer_output_transport_channel();
        let (runtime_response_tx, runtime_response_rx) = tokio::sync::mpsc::unbounded_channel();
        let (background_event_tx, background_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (background_navigation_completion_tx, background_navigation_completion_rx) =
            tokio::sync::mpsc::unbounded_channel();
        conn.set_renderer_publication_sender(publication_tx.clone());
        conn.set_runtime_inspector_response_ready_sender(runtime_response_tx);
        let mut ctx = TestContext {
            state: crate::ProtocolStateOwner::new(conn),
            sent: Vec::new(),
            pending_runtime_deferred_replies: VecDeque::new(),
            pending_protocol_scheduler_work: VecDeque::new(),
            pending_owned_protocol_work: FuturesUnordered::new(),
            runtime_inspector_response_ready_rx: runtime_response_rx,
            renderer_publication_rx: publication_rx,
            renderer_publication_tx: publication_tx.clone(),
            background_event_tx,
            background_event_rx,
            background_navigation_completion_tx,
            background_navigation_completion_rx,
            background_navigation_scheduler_enabled: false,
            renderer_insertions: Default::default(),
            renderer_insertion_output: VecDeque::new(),
        };
        let opened = |page_id| {
            RendererOutputTransportMessage::from(RendererOutputStreamControl::Opened {
                stream: RendererOutputStreamIdentity::new_page_for_protocol_test(
                    PageId::new_for_testing(page_id),
                ),
            })
        };
        let first = opened(1);
        let second = opened(2);
        let second_stream = match &second {
            RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream },
            ) => *stream,
            RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Closed { .. },
            )
            | RendererOutputTransportMessage::PageReservationReleased { .. }
            | RendererOutputTransportMessage::CursorLeaseDeclared { .. }
            | RendererOutputTransportMessage::CursorLeaseReleased { .. }
            | RendererOutputTransportMessage::Publication(_) => {
                unreachable!("test input is an opened stream control")
            }
        };
        publication_tx.send(first).expect("first scheduler input");
        publication_tx.send(second).expect("second scheduler input");

        assert_eq!(
            ctx.run_one_ready_test_scheduler_turn().await,
            TestSchedulerTurnOutcome::Processed(TestSchedulerInputKind::RendererPublication)
        );
        let queued = ctx
            .renderer_publication_rx
            .try_recv()
            .expect("second input must remain queued for the next turn");
        assert!(matches!(
            queued,
            RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream }
            ) if stream == second_stream
        ));
    }
}
