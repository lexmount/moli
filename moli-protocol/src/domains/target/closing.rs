use futures_util::{FutureExt as _, future::LocalBoxFuture};
use serde::Deserialize;
use std::{cell::RefCell, rc::Rc};

use crate::automation::{DevToolsCloseTargetResult, DevToolsTargetId};

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloseTargetParams {
    pub(super) target_id: String,
}

pub(super) fn start_close_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: CloseTargetParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    if !conn.target_handler_may_close_target(cmd.session_id, &params.target_id) {
        return super::target_command_error(-32000, "Not allowed");
    }
    let command = build_cdp_close_target_command(cmd, params);
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::CloseTarget(command),
    )
}

pub(super) fn build_cdp_close_target_command(
    cmd: &Cmd<'_>,
    params: CloseTargetParams,
) -> DevToolsCloseTargetCommand {
    DevToolsCloseTargetCommand {
        context: cmd.automation_context(Some(params.target_id.as_str()), None::<&str>),
        target_id: DevToolsTargetId::from(params.target_id),
    }
}

pub(super) fn start_devtools_close_target_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCloseTargetCommand,
) -> TargetCommandTaskStep {
    let page_target_id = conn
        .primary_page_target_id_for_tab_target_id(command.target_id.as_str())
        .unwrap_or(command.target_id.as_str());
    if conn.page_target_is_closing(page_target_id) {
        return super::target_command_error(-32000, "Target is closing");
    }
    if let Some(closing) = PreparedPageTargetCloseCommand::prepare(conn, &command) {
        return TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
            command_id,
            session_id: command_session_id.map(str::to_owned),
            kind: Box::new(PendingTargetCommandKind::ClosePageTarget {
                closing,
                response: CommandOutputPlan::from_devtools_result(AutomationResult::CloseTarget(
                    DevToolsCloseTargetResult { success: true },
                )),
                access: conn.protocol_state_access(),
            }),
        });
    }
    pending_close_target_command(command_id, command_session_id, command)
}

#[derive(Clone)]
pub(crate) struct PreparedPageTargetCloseCommand {
    action: crate::domains::page::PageTargetTerminationOwnerAction,
    termination: Rc<RefCell<Option<crate::domains::page::TargetTermination>>>,
    fetch: Option<TargetCloseFetchCleanup>,
    output: Rc<RefCell<Option<PageTargetCloseCommandOutput>>>,
}

struct PageTargetCloseCommandOutput {
    closes_default_target: bool,
    protocol_events: Vec<BackgroundProtocolEvent>,
    background_events: Vec<BackgroundProtocolEvent>,
}

#[derive(Clone)]
struct TargetCloseFetchCleanup {
    wait: TargetCloseFetchWait,
    output: Rc<RefCell<Option<TargetCloseFetchOutput>>>,
}

#[derive(Clone)]
enum TargetCloseFetchWait {
    Owned(futures_util::future::Shared<LocalBoxFuture<'static, ()>>),
    Inline(Rc<RefCell<Option<crate::domains::fetch::PreparedFetchCleanup>>>),
}

struct TargetCloseFetchOutput {
    events: Vec<BackgroundProtocolEvent>,
    result: crate::conn::SessionPageUpdateResult,
}

impl TargetCloseFetchCleanup {
    fn new(
        cleanup: crate::domains::fetch::PreparedFetchCleanup,
        access: Option<crate::conn::ProtocolStateAccess>,
    ) -> Self {
        let output: Rc<RefCell<Option<TargetCloseFetchOutput>>> = Rc::default();
        let wait = match access {
            Some(access) => {
                let output = Rc::clone(&output);
                TargetCloseFetchWait::Owned(
                    async move {
                        let mut driver = crate::conn::ProtocolWorkflowDriver::owned(access);
                        let completed = Self::run(cleanup, &mut driver).await;
                        *output.borrow_mut() = Some(completed);
                    }
                    .boxed_local()
                    .shared(),
                )
            }
            None => TargetCloseFetchWait::Inline(Rc::new(RefCell::new(Some(cleanup)))),
        };
        Self { wait, output }
    }

    async fn run(
        cleanup: crate::domains::fetch::PreparedFetchCleanup,
        driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
    ) -> TargetCloseFetchOutput {
        let mut events = Vec::new();
        let result = cleanup
            .run_for_target_close(driver)
            .await
            .into_disposal_output(&mut events);
        TargetCloseFetchOutput { events, result }
    }

    async fn wait(&self, driver: &mut crate::conn::ProtocolWorkflowDriver<'_>) {
        match &self.wait {
            TargetCloseFetchWait::Owned(wait) => wait.clone().await,
            TargetCloseFetchWait::Inline(pending) => {
                let pending = pending.borrow_mut().take();
                if let Some(pending) = pending {
                    let completed = Self::run(pending, driver).await;
                    *self.output.borrow_mut() = Some(completed);
                }
            }
        }
    }

    fn has_renderer_output_predecessor(&self) -> bool {
        self.output
            .borrow()
            .as_ref()
            .is_some_and(|output| output.result.predecessor.is_some())
    }
}

impl PreparedPageTargetCloseCommand {
    fn prepare(conn: &mut CdpConnection, command: &DevToolsCloseTargetCommand) -> Option<Self> {
        let target_id = conn
            .primary_page_target_id_for_tab_target_id(command.target_id.as_str())
            .unwrap_or(command.target_id.as_str())
            .to_owned();
        let route = conn.target_session_route_for_target_id(&target_id)?;
        let owner = CommandOwnerScope::for_route(route.clone());
        let has_pending_fetch = !conn.target_fetch_pending_state_is_empty_for_owner(&owner)?;
        let context = conn.browser_context_by_id(route.browser_context_id()?)?;
        let active = context.is_active_target(&target_id);
        let primary_session = context
            .page_target(&target_id)?
            .session_id()
            .map(str::to_owned);
        let action = crate::domains::page::PageTargetTerminationOwnerAction::new(
            conn,
            owner.clone(),
            target_id,
        );
        let mut background_events = Vec::new();
        let mut protocol_events = Vec::new();
        if active {
            let primary = primary_session
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            conn.fail_pending_inspector_awaits_for_owner_background_events_into(
                &mut background_events,
                &mut protocol_events,
                &primary,
                "Target closed",
            );
        }
        conn.mark_page_target_closing(&owner);
        let (fetch, termination) = if has_pending_fetch {
            let cleanup = crate::domains::fetch::prepare_target_close(conn, &owner)
                .expect("a validated Page remains present during finite Fetch close preparation");
            (
                Some(TargetCloseFetchCleanup::new(
                    cleanup,
                    conn.protocol_state_access(),
                )),
                None,
            )
        } else {
            let termination =
                crate::domains::page::TargetTermination::prepare(conn, action.clone())
                    .expect("a validated Page remains present during finite close preparation");
            (None, Some(termination))
        };
        Some(Self {
            action,
            termination: Rc::new(RefCell::new(termination)),
            fetch,
            output: Rc::new(RefCell::new(Some(PageTargetCloseCommandOutput {
                closes_default_target: matches!(
                    command.target_id.as_str(),
                    crate::DEFAULT_CDP_PAGE_TARGET_ID | crate::DEFAULT_CDP_TAB_TARGET_ID
                ),
                protocol_events,
                background_events,
            }))),
        })
    }

    pub(super) fn take_protocol_events(&self) -> Vec<BackgroundProtocolEvent> {
        self.output
            .borrow_mut()
            .as_mut()
            .map(|output| std::mem::take(&mut output.protocol_events))
            .unwrap_or_default()
    }

    pub(super) async fn wait(
        &self,
        driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
    ) -> Option<Box<crate::domains::page::CompletedTargetTermination>> {
        if let Some(fetch) = &self.fetch {
            fetch.wait(driver).await;
            if fetch.has_renderer_output_predecessor() {
                // Ordered ingress must consume the final renderer record
                // before the background owner action may take this host.
                return None;
            }
        }
        let termination = self.termination.borrow().clone();
        let termination = match termination {
            Some(termination) => Some(termination),
            None => {
                let retained = self.termination.clone();
                let action = self.action.clone();
                match driver
                    .with_state(move |state| {
                        let mut retained = retained.borrow_mut();
                        if retained.is_none() {
                            *retained = state.prepare_target_termination(action);
                        }
                        retained.clone()
                    })
                    .await
                {
                    Ok(termination) => termination,
                    Err(error) => {
                        tracing::warn!(%error, "protocol owner closed during target termination");
                        None
                    }
                }
            }
        };
        match termination {
            Some(termination) => Some(Box::new(termination.run(driver).await)),
            None => None,
        }
    }

    pub(super) fn finish(
        self,
        conn: &mut CdpConnection,
        context: &mut crate::conn::CommandDispatchContext,
        completed: Option<Box<crate::domains::page::CompletedTargetTermination>>,
        mut response: CommandOutputPlan,
    ) -> CommandOutputPlan {
        let Some(output) = self.output.borrow_mut().take() else {
            return CommandOutputPlan::default();
        };
        context.protocol_events_mut().extend(output.protocol_events);
        response.extend_background_events(output.background_events);
        let fetch = self
            .fetch
            .as_ref()
            .and_then(|fetch| fetch.output.borrow_mut().take());
        let predecessor = fetch.and_then(|fetch| {
            if let Some(error) = fetch.result.first_error {
                tracing::warn!(%error, "failed to clear Fetch state while closing target");
            }
            response.extend_background_events(fetch.events);
            fetch.result.predecessor
        });
        let has_predecessor = predecessor.is_some();
        let deferred_close =
            completed.is_none() && predecessor.is_some() && self.action.is_current_host(conn);
        if let Some(predecessor) = predecessor {
            context.set_renderer_output_predecessor(predecessor);
        }
        if let Some(completed) = completed {
            let (events, scheduler_events) = completed.finish(conn).into_protocol_event_parts();
            response.extend_background_events(events);
            conn.extend_scheduler_events(scheduler_events);
        } else if has_predecessor {
            conn.publish_page_target_termination_owner_action(self.action);
        }
        if output.closes_default_target
            && (deferred_close
                || conn
                    .target_session_route_for_target_id(crate::DEFAULT_CDP_PAGE_TARGET_ID)
                    .is_none())
        {
            conn.mark_default_browser_target_closed();
        }
        response
    }
}

pub(super) async fn complete_close_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCloseTargetCommand,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> CommandOutputPlan {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    match execute_devtools_close_target_command_async(
        conn,
        command,
        &mut side_effects,
        command_context,
    )
    .await
    {
        Ok(result) => {
            let mut plan =
                CommandOutputPlan::from_devtools_result(AutomationResult::CloseTarget(result));
            plan.extend(side_effects.into_plan());
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) async fn execute_devtools_close_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCloseTargetCommand,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> Result<DevToolsCloseTargetResult, DevToolsError> {
    let closes_default_target = matches!(
        command.target_id.as_str(),
        crate::DEFAULT_CDP_PAGE_TARGET_ID | crate::DEFAULT_CDP_TAB_TARGET_ID
    );
    let target_id = command.target_id.into_string();
    if let Some(plan) = conn.close_default_target_placeholder(&target_id).await {
        out.extend_background_events(plan.into_background_events());
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    let result = close_target_inner_async(conn, out, command_context, target_id).await;
    if result.is_ok() && closes_default_target {
        conn.mark_default_browser_target_closed();
    }
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    result
}

async fn close_target_inner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
    target_id: String,
) -> Result<DevToolsCloseTargetResult, DevToolsError> {
    let target_id = conn
        .primary_page_target_id_for_tab_target_id(&target_id)
        .map(str::to_owned)
        .unwrap_or(target_id);
    if let Err(message) = select_browser_context_for_target(conn, &target_id) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            format!("{message:#}"),
        ));
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_shared_worker_target(&target_id))
    {
        if !worker_target::close_shared_worker_target_for_target_close_async(
            conn,
            &target_id,
            command_context,
        )
        .await
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_dedicated_worker_target(&target_id))
    {
        if !worker_target::close_dedicated_worker_target_for_target_close_async(
            conn,
            &target_id,
            command_context,
        )
        .await
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_service_worker_target(&target_id))
    {
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn.browser_context.as_ref().is_some_and(|bc| {
        !matches!(
            bc.active_target_identity(),
            Some((ref active_target_id, _)) if active_target_id == &target_id
        )
    }) {
        let target_route = conn
            .target_session_route_for_target_id(&target_id)
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId")
            })?;
        let session_id = conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.background_target(&target_id))
            .and_then(|target| target.session_id().map(str::to_owned));
        let owner_scope = session_id
            .as_deref()
            .map(crate::conn::CommandOwnerScope::for_session)
            .unwrap_or_else(|| crate::conn::CommandOwnerScope::for_route(target_route));
        let renderer_output_predecessor =
            events::fail_pending_fetch_state_for_target_background_events_async(
                conn,
                out.background_events_mut(),
                session_id.as_deref(),
                "Target closed",
            )
            .await;
        settle_target_close_after_pending_fetches_async(
            conn,
            out,
            command_context,
            renderer_output_predecessor,
            owner_scope,
            target_id,
        )
        .await;
        return Ok(DevToolsCloseTargetResult { success: true });
    }

    let session_id = match conn.browser_context.as_ref() {
        Some(bc) => {
            if bc.active_target_id().is_none() && bc.has_no_background_targets() {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "TargetNotLoaded",
                ));
            }
            match bc.active_target_identity() {
                Some((_active_target_id, session_id)) => session_id,
                None => {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        "TargetNotLoaded",
                    ));
                }
            }
        }
        None => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "BrowserContextNotLoaded",
            ));
        }
    };

    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        out.background_events_mut(),
        command_context.protocol_events_mut(),
        session_id.as_deref(),
        "Target closed",
    );
    let renderer_output_predecessor =
        events::fail_pending_fetch_state_for_target_background_events_async(
            conn,
            out.background_events_mut(),
            session_id.as_deref(),
            "Target closed",
        )
        .await;
    let owner_scope = crate::conn::CommandOwnerScope::capture(conn, session_id.as_deref());
    settle_target_close_after_pending_fetches_async(
        conn,
        out,
        command_context,
        renderer_output_predecessor,
        owner_scope,
        target_id,
    )
    .await;
    Ok(DevToolsCloseTargetResult { success: true })
}

/// Preserves the final renderer publication without changing the ordinary
/// `Target.closeTarget` transaction boundary.
///
/// Most target closes produce no renderer output. Those closes still complete
/// synchronously and return their detach/destroy side effects with the command,
/// matching Chromium's target-domain behavior. A paused request can first
/// produce a terminal renderer record, however. Only that case must defer
/// target retirement until the command's exact cursor has crossed ordered
/// ingress; otherwise retiring the route would discard that final record.
async fn settle_target_close_after_pending_fetches_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    owner_scope: crate::conn::CommandOwnerScope,
    target_id: String,
) {
    let action =
        crate::domains::page::PageTargetTerminationOwnerAction::new(conn, owner_scope, target_id);
    if let Some(predecessor) = renderer_output_predecessor {
        command_context.set_renderer_output_predecessor(predecessor);
        conn.publish_page_target_termination_owner_action(action);
        return;
    }

    let outcome =
        crate::domains::page::complete_page_target_termination_owner_action_async(conn, action)
            .await;
    let (events, scheduler_events) = outcome.into_protocol_event_parts();
    out.extend_background_events(events);
    conn.extend_scheduler_events(scheduler_events);
}
