use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    rc::Rc,
};

use anyhow::Result;
use moli_core::RendererBrowserContextRuntimeId;

use crate::automation::{DevToolsError, DevToolsErrorKind, DevToolsTargetKind};
use crate::conn::{
    BackgroundProtocolEvent, BrowserContext, CdpConnection, CdpSessionRoute,
    CommandDispatchContext, CommandOwnerScope, CommandWorkflowContext, PreparedCommandWorkflow,
    PreparedTargetHostClosure, ProtocolStateAccess, ProtocolWorkflowDriver,
};
use crate::domains::{
    command_output::CommandOutputPlan,
    page::{CompletedTargetTermination, PageTargetTerminationOwnerAction, TargetTermination},
};

use super::{events, worker_target};

const DISPOSE_REASON: &str = "Browser context disposed";
const INSPECTOR_DETACHED_REASON: &str = "Render process gone.";

struct PageTargetDisposal {
    action: PageTargetTerminationOwnerAction,
    closure: Option<PreparedTargetHostClosure>,
    termination: Option<TargetTermination>,
}

/// The original lifetime, rather than a reusable frontend context id.
#[derive(Clone)]
pub(crate) struct BrowserContextDisposalIdentity {
    id: String,
    runtime: RendererBrowserContextRuntimeId,
}

impl BrowserContextDisposalIdentity {
    fn is_current(&self, conn: &CdpConnection) -> bool {
        conn.browser_context_by_id(&self.id)
            .is_some_and(|context| context.routes_renderer_browser_context_runtime(self.runtime))
    }
}

struct BrowserContextDisposalState {
    pages: Vec<PageTargetDisposal>,
    fetch: VecDeque<crate::domains::fetch::PreparedFetchCleanup>,
    inspector_events: Option<Vec<BackgroundProtocolEvent>>,
    events: Vec<BackgroundProtocolEvent>,
}

/// Keeps admitted cleanup and its output with the command through waiter
/// cancellation and a discarded ready result. Native Page close reuses the
/// same receipts as Target.closeTarget, with one context-wide focus restore.
#[derive(Clone)]
pub(crate) struct PreparedBrowserContextDisposal {
    identity: BrowserContextDisposalIdentity,
    state: Rc<RefCell<BrowserContextDisposalState>>,
}

impl PreparedBrowserContextDisposal {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        browser_context_id: String,
    ) -> Result<Self, DevToolsError> {
        let context = conn
            .browser_context_by_id(&browser_context_id)
            .ok_or_else(|| browser_context_not_found(&browser_context_id))?;
        if context.closing {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "Browser context is closing",
            ));
        }
        let identity = BrowserContextDisposalIdentity {
            id: browser_context_id.clone(),
            runtime: context.renderer_runtime().id(),
        };
        let mut target_ids = context
            .background_targets()
            .rev()
            .map(|target| target.target_id().to_owned())
            .collect::<Vec<_>>();
        target_ids.extend(context.active_target_id().map(str::to_owned));
        let mut seen = HashSet::new();
        let inspector_sessions = context
            .devtools_target_infos()
            .into_iter()
            .filter(|target| {
                matches!(
                    target.kind,
                    DevToolsTargetKind::Page
                        | DevToolsTargetKind::SharedWorker
                        | DevToolsTargetKind::ServiceWorker
                )
            })
            .filter_map(|target| target.target_id)
            .flat_map(|target| target_session_ids(conn, context, target.as_str()))
            .filter(|session| seen.insert(session.clone()))
            .collect::<Vec<_>>();
        let active_owner = context.active_target_id().and_then(|id| {
            conn.target_session_route_for_target_id(id)
                .map(CommandOwnerScope::for_route)
        });
        let mut pending_owners = active_owner.into_iter().collect::<Vec<_>>();
        pending_owners.extend(
            inspector_sessions
                .iter()
                .map(|sid| CommandOwnerScope::for_session(sid)),
        );
        let mut events = Vec::new();
        let mut claimed = Vec::new();
        for owner in pending_owners {
            conn.fail_pending_inspector_awaits_for_owner_background_events_into(
                &mut events,
                &mut claimed,
                &owner,
                DISPOSE_REASON,
            );
        }
        conn.browser_context_by_id_mut(&browser_context_id)
            .expect("the validated context remains present during preparation")
            .closing = true;
        let mut pages = Vec::new();
        let mut fetch = VecDeque::new();
        for target_id in target_ids {
            let Some(route) = conn.target_session_route_for_target_id(&target_id) else {
                continue;
            };
            let owner = CommandOwnerScope::for_route(route);
            let action =
                PageTargetTerminationOwnerAction::new(conn, owner.clone(), target_id.clone());
            conn.mark_page_target_closing(&owner);
            if let Some(pending) = conn.take_pending_fetch_state_for_owner(&owner) {
                let mut cleanup =
                    crate::domains::fetch::prepare_browser_context_disposal(conn, &owner, pending);
                // Main-resource terminals precede detach. Subresource promises
                // disappear with their realm, without synthetic loadingFailed
                // or a Browser ACK fence joining unrelated Page streams.
                events.extend(cleanup.take_navigation_events_for_context_disposal());
                fetch.push_back(cleanup);
            }
            pages.push(PageTargetDisposal {
                action,
                closure: Some(conn.prepare_target_host_closure(&target_id)),
                termination: None,
            });
        }
        events.extend(claimed);
        Ok(Self {
            identity,
            state: Rc::new(RefCell::new(BrowserContextDisposalState {
                pages,
                fetch,
                inspector_events: Some(
                    inspector_sessions
                        .into_iter()
                        .map(|session| {
                            events::inspector_detached_event(&session, INSPECTOR_DETACHED_REASON)
                        })
                        .collect(),
                ),
                events,
            })),
        })
    }

    pub(crate) fn into_workflow(self, access: ProtocolStateAccess) -> PreparedCommandWorkflow {
        let recovery = self.clone();
        let recovery_access = access.clone();
        self.workflow(access, CommandOutputPlan::success())
            .with_failure_cleanup(move |response| recovery.workflow(recovery_access, response))
    }

    fn workflow(
        self,
        access: ProtocolStateAccess,
        response: CommandOutputPlan,
    ) -> PreparedCommandWorkflow {
        PreparedCommandWorkflow::new(move |mut context| async move {
            let result =
                Box::pin(self.run(&mut ProtocolWorkflowDriver::owned(access), &mut context)).await;
            match result {
                Ok(()) => context.complete_in_owner_turn(response, move |state, _| {
                    state.finish_browser_context_disposal(self)
                }),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
    }

    pub(crate) async fn run(
        &self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        loop {
            let Some(cleanup) = self.state.borrow_mut().fetch.pop_front() else {
                break;
            };
            let mut events = Vec::new();
            let result = cleanup
                .run_using(driver)
                .await
                .into_disposal_output(&mut events);
            self.state.borrow_mut().events.extend(events);
            if let Some(error) = result.first_error {
                tracing::warn!(%error, "failed to clear Fetch state while disposing browser context");
            }
        }
        {
            let mut state = self.state.borrow_mut();
            if let Some(events) = state.inspector_events.take() {
                state.events.extend(events);
            }
        }
        self.close_workers(driver, context).await?;
        let page_count = self.state.borrow().pages.len();
        for index in 0..page_count {
            let disposal = self.clone();
            let termination = driver
                .with_state(move |state| state.prepare_browser_context_page_close(disposal, index))
                .await?;
            if let Some(termination) = termination {
                let completed = termination.run(driver).await;
                let disposal = self.clone();
                driver
                    .with_state(move |state| {
                        state.finish_browser_context_page_close(disposal, completed);
                    })
                    .await?;
            }
        }
        // A script already running at admission may have created a worker
        // while the Pages were closing. Sweep once more after their close.
        self.close_workers(driver, context).await?;
        let identity = self.identity.clone();
        let removed = driver
            .with_state(move |state| state.remove_disposed_browser_context(identity))
            .await?;
        if let Some(mut removed) = removed {
            removed.close_all_pages_async().await;
        }
        let updates = driver
            .with_state(|state| state.start_restore_browser_focus_after_removal())
            .await?;
        let focus = match updates {
            Ok(updates) => super::activation::wait_for_surface_updates(driver, updates).await,
            Err(error) => Err(error),
        };
        if let Err(error) = focus {
            tracing::warn!(%error, "failed to restore focus after browser context disposal");
        }
        Ok(())
    }

    async fn close_workers(
        &self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let identity = self.identity.clone();
        let outputs = driver
            .with_state(move |state| state.prepare_browser_context_worker_disposal(identity))
            .await?;
        let Some(outputs) = outputs else {
            return Ok(());
        };
        context.materialize_projection_output();
        let start = context.context_mut().protocol_events_mut().len();
        let owner = CommandOwnerScope::for_route(CdpSessionRoute::BrowserContext {
            browser_context_id: self.identity.id.clone(),
        });
        let result = outputs.project_using(driver, owner, context).await;
        context.materialize_projection_output();
        self.state
            .borrow_mut()
            .events
            .extend(context.context_mut().protocol_events_mut().split_off(start));
        if let Err(error) = result {
            tracing::warn!(%error, "failed to close browser context worker targets");
        }
        Ok(())
    }

    pub(crate) fn prepare_page_close(
        &self,
        conn: &mut CdpConnection,
        index: usize,
    ) -> Option<TargetTermination> {
        if !self.identity.is_current(conn) {
            return None;
        }
        let mut state = self.state.borrow_mut();
        let page = &mut state.pages[index];
        if let Some(closure) = page.closure.take() {
            page.termination = TargetTermination::prepare_for_browser_context_disposal(
                conn,
                page.action.clone(),
                closure,
            );
        }
        page.termination.clone()
    }

    pub(crate) fn finish_page_close(
        &self,
        conn: &mut CdpConnection,
        completed: CompletedTargetTermination,
    ) {
        let (events, scheduler) = completed.finish(conn).into_protocol_event_parts();
        self.state.borrow_mut().events.extend(events);
        conn.extend_scheduler_events(scheduler);
    }

    pub(crate) fn finish(self, conn: &mut CdpConnection) -> CommandOutputPlan {
        conn.release_idle_navigation_engine_memory_after_target_close();
        let mut output = CommandOutputPlan::default();
        output.extend_background_events(std::mem::take(&mut self.state.borrow_mut().events));
        output
    }

    pub(crate) async fn complete_inline(
        self,
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
        mut response: CommandOutputPlan,
    ) -> CommandOutputPlan {
        let (mut context, _scope) = CommandWorkflowContext::take_from(command_context, None, None);
        let result =
            Box::pin(self.run(&mut ProtocolWorkflowDriver::inline(conn), &mut context)).await;
        context.finish_projection_inline(conn, command_context);
        if let Err(error) = result {
            tracing::warn!(%error, "browser context disposal cleanup failed");
        }
        response.extend(self.finish(conn));
        response
    }
}

impl BrowserContextDisposalIdentity {
    pub(crate) fn prepare_worker_disposal(
        &self,
        conn: &mut CdpConnection,
    ) -> Option<crate::domains::activity::PreparedProtocolOutputs> {
        if !self.is_current(conn) {
            return None;
        }
        Some(
            worker_target::prepare_browser_context_worker_targets_for_disposal(
                conn,
                &self.id,
                DISPOSE_REASON,
            ),
        )
    }

    pub(crate) fn remove(self, conn: &mut CdpConnection) -> Option<BrowserContext> {
        if !self.is_current(conn) {
            return None;
        }
        conn.download_behavior.clear_browser_context(&self.id);
        conn.permission_overrides
            .retain(|entry| entry.browser_context_id.as_deref() != Some(&self.id));
        // Keep any selection made by a peer while native cleanup was waiting.
        let restore = super::previously_active_browser_context_id(conn);
        conn.remove_browser_context_by_id_restoring_active(&self.id, restore.as_deref())
    }
}

fn target_session_ids(
    conn: &CdpConnection,
    context: &BrowserContext,
    target_id: &str,
) -> Vec<String> {
    let mut sessions = context.devtools_session_ids_for_target(target_id);
    sessions.extend(conn.attached_sessions_for_target(target_id));
    sessions.sort();
    sessions.dedup();
    sessions
}

fn browser_context_not_found(browser_context_id: &str) -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::Internal,
        format!("Failed to find context with id {browser_context_id}"),
    )
}
