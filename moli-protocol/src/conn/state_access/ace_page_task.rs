use std::{cell::RefCell, rc::Rc};

#[cfg(test)]
mod tests;

use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use moli_core::{
    RendererPageCommand, RendererPageReply,
    ace_page_task::{AcePageTask, AcePageTaskKind},
    page::CompletedPageCommand,
};

use super::{ProtocolWorkflowDriver, ProtocolWorkflowState};
use crate::conn::{
    CdpCommandTaskStep, CdpConnection, CdpRendererOwnerTurnOutcome, CdpSchedulerEvent,
    CdpSessionRoute, CommandDispatchContext, CommandOwnerScope, CompletedCommandWorkflow,
    PreparedCommandWorkflow, ProtocolPageHostScope, TargetPageResidenceIdentity,
};
use crate::domains::command_output::CommandOutputPlan;

/// Browser facts captured in the task's admission/completion turns. Policy and
/// cross-call observations belong to the embedder, not the protocol connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserSnapshot {
    pub source_target_id: Option<String>,
    pub targets: Vec<BrowserTargetSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserTargetSnapshot {
    pub id: String,
    pub url: String,
    pub title: String,
    pub opener_id: Option<String>,
    pub is_page: bool,
    pub has_page: bool,
    pub pending_dialog: bool,
    pub pending_navigation: bool,
}

#[derive(Clone, Debug)]
pub struct AcePageTaskResult {
    pub value: serde_json::Value,
    pub before: BrowserSnapshot,
    pub after: BrowserSnapshot,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct AcePageTaskError {
    pub code: u16,
    pub message: String,
}

impl AcePageTaskError {
    pub fn new(code: u16, message: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: format!("{message:#}"),
        }
    }
}

type TaskResult = Result<AcePageTaskResult, AcePageTaskError>;
type TaskResultSlot = Rc<RefCell<Option<TaskResult>>>;
type WorkflowReceipt = Rc<RefCell<Option<CompletedCommandWorkflow>>>;

/// Rejoining a lost waiter polls the same admitted workflow, never a new task.
#[derive(Clone)]
pub struct PendingAcePageTaskDispatch {
    pending: Shared<LocalBoxFuture<'static, WorkflowReceipt>>,
    result: TaskResultSlot,
}

pub struct CompletedAcePageTaskDispatch {
    workflow: WorkflowReceipt,
    result: TaskResultSlot,
}

impl PendingAcePageTaskDispatch {
    pub async fn wait(&self) -> CompletedAcePageTaskDispatch {
        CompletedAcePageTaskDispatch {
            workflow: self.pending.clone().await,
            result: self.result.clone(),
        }
    }
}

fn store_result(slot: &TaskResultSlot, result: TaskResult) -> CommandOutputPlan {
    assert!(
        slot.borrow_mut().replace(result).is_none(),
        "one result per page task"
    );
    CommandOutputPlan::default()
}

impl ProtocolWorkflowState<'_> {
    /// Admit a typed embedder task to the same supervised owner workflow as
    /// protocol commands. No wire command or Inspector attachment is created.
    pub fn start_ace_page_task(
        self,
        target_id: &str,
        task: Box<dyn AcePageTask>,
        context: &mut CommandDispatchContext,
    ) -> (PendingAcePageTaskDispatch, Vec<CdpSchedulerEvent>) {
        let conn = self.connection;
        let result: TaskResultSlot = Default::default();
        let workflow_result = result.clone();
        let prepared = (|| {
            conn.ensure_default_target_live(target_id);
            let route = conn
                .target_session_route_for_target_id(target_id)
                .filter(|route| matches!(route, CdpSessionRoute::PageTarget { .. }))
                .ok_or_else(|| AcePageTaskError::new(404, "Page target is unavailable"))?;
            let owner = CommandOwnerScope::for_route(route);
            let host = Rc::new(
                ProtocolPageHostScope::capture_for_ace_page_task(conn, &owner)
                    .map_err(|error| AcePageTaskError::new(409, error))?,
            );
            let initial = conn
                .start_initial_document_page_ensure_for_owner(&owner)
                .map_err(|error| AcePageTaskError::new(409, error))?;
            let kind = task.kind();
            let access = conn.protocol_state_access();
            Ok(PreparedCommandWorkflow::new(
                move |mut context| async move {
                    let mut driver = ProtocolWorkflowDriver::new(access);
                    if let Some(initial) = initial {
                        let diagnostics = match initial.wait().await {
                            Ok(completed) => completed.install_using(&mut driver).await,
                            Err(failed) => driver
                                .with_state(|state| state.reset_failed_initial_document(failed))
                                .and_then(|message| Err(anyhow::anyhow!(message))),
                        };
                        match diagnostics {
                            Ok(diagnostics) => {
                                if let Some(predecessor) = diagnostics.renderer_output_predecessor {
                                    context
                                        .context_mut()
                                        .set_renderer_output_predecessor(predecessor);
                                }
                            }
                            Err(error) => {
                                return context.complete(store_result(
                                    &workflow_result,
                                    Err(AcePageTaskError::new(409, error)),
                                ));
                            }
                        }
                    }
                    let prepared = context.with_state(&mut driver, |state, _| {
                        let conn = state.connection;
                        if !host.can_start_operation(conn) {
                            return Err(AcePageTaskError::new(404, "Page target is unavailable"));
                        }
                        let page_owner = conn
                            .target_page_residence_identity_for_owner(&owner)
                            .ok_or_else(|| AcePageTaskError::new(409, "NoDocumentLoaded"))?;
                        let before = conn.ace_page_task_browser_snapshot(
                            page_owner.browser_context_id(),
                            page_owner.target_id().map(str::to_owned),
                        );
                        let document = conn
                            .target_root_document_lifecycle_identity_for_owner(&owner)
                            .ok_or_else(|| AcePageTaskError::new(409, "NoDocumentLoaded"))?;
                        let page = conn
                            .loaded_page_mut_for_protocol_access_for_owner(&owner)
                            .map_err(|error| AcePageTaskError::new(409, error))?;
                        let pending = page
                            .document_command_target(document)
                            .and_then(|target| target.start(RendererPageCommand::AcePageTask(task)))
                            .map_err(|error| AcePageTaskError::new(409, error))?;
                        Ok((page_owner, before, pending))
                    });
                    let (page_owner, before, pending) = match prepared {
                        Ok(Ok(prepared)) => prepared,
                        Ok(Err(error)) => {
                            return context.complete(store_result(&workflow_result, Err(error)));
                        }
                        Err(error) => {
                            return context.complete(store_result(
                                &workflow_result,
                                Err(AcePageTaskError::new(409, error)),
                            ));
                        }
                    };
                    let completed = pending.wait().await;
                    let result = context.with_state(&mut driver, |state, context| {
                        state.finish_ace_page_task(
                            &host,
                            &page_owner,
                            kind,
                            completed,
                            context.context_mut(),
                        )
                    });
                    let output = match result {
                        Ok(Ok(output)) => output,
                        Ok(Err(error)) => {
                            return context.complete(store_result(&workflow_result, Err(error)));
                        }
                        Err(error) => {
                            return context.complete(store_result(
                                &workflow_result,
                                Err(AcePageTaskError::new(409, error)),
                            ));
                        }
                    };
                    // Settle every accepted input even if the task or an earlier
                    // effect failed. A task's business result cannot discard effects.
                    let mut effect_error = None;
                    for outcome in output.input_outcomes {
                        match crate::domains::input::handle_ace_page_task_input_dispatch_outcome(
                            &mut driver,
                            None,
                            &page_owner,
                            &host,
                            outcome,
                            context.context_mut(),
                        )
                        .await
                        {
                            Ok(events) => context.context_mut().defer_command_events(events),
                            Err(error) => {
                                if effect_error.is_none() {
                                    effect_error = Some(AcePageTaskError::new(500, error));
                                }
                            }
                        }
                    }
                    let task_result = output
                        .result
                        .map_err(|error| AcePageTaskError::new(409, error));
                    context.complete_in_owner_turn(CommandOutputPlan::default(), move |state, _| {
                        let result = (|| {
                            if !host.can_start_operation(state.connection) {
                                return Err(AcePageTaskError::new(
                                    404,
                                    "Page target closed during task",
                                ));
                            }
                            if kind == AcePageTaskKind::ReadOnly
                                && state
                                    .connection
                                    .target_page_residence_identity_for_owner(host.owner())
                                    .as_ref()
                                    != Some(&page_owner)
                            {
                                return Err(AcePageTaskError::new(
                                    409,
                                    "Document changed during task",
                                ));
                            }
                            let after = state.connection.ace_page_task_browser_snapshot(
                                page_owner.browser_context_id(),
                                page_owner.target_id().map(str::to_owned),
                            );
                            let value = task_result?.value;
                            if let Some(error) = effect_error {
                                return Err(error);
                            }
                            Ok(AcePageTaskResult {
                                value,
                                before,
                                after,
                            })
                        })();
                        store_result(&workflow_result, result)
                    })
                },
            ))
        })();
        let workflow = match prepared {
            Ok(workflow) => workflow,
            Err(error) => {
                let result = result.clone();
                PreparedCommandWorkflow::new(move |context| async move {
                    context.complete(store_result(&result, Err(error)))
                })
            }
        };
        let pending = workflow.start(context, None, None);
        (
            PendingAcePageTaskDispatch {
                pending: async move { Rc::new(RefCell::new(Some(pending.wait().await))) }
                    .boxed_local()
                    .shared(),
                result,
            },
            conn.take_scheduler_events(),
        )
    }

    /// Finish the owned task with a typed result and independently ordered events.
    pub fn complete_ace_page_task(
        self,
        completed: CompletedAcePageTaskDispatch,
        context: &mut CommandDispatchContext,
    ) -> (TaskResult, CdpRendererOwnerTurnOutcome) {
        let workflow = completed
            .workflow
            .borrow_mut()
            .take()
            .expect("page task completes once");
        let mut plan = workflow.finish(self.connection, context);
        // Recovery can retain an internal workflow failure. Consume its typed
        // status before projecting events; it must never become a CDP response.
        let failure = plan.take_command_status().and_then(Result::err);
        let result =
            match failure {
                Some(error) => Err(AcePageTaskError::new(500, error.message)),
                None => completed.result.borrow_mut().take().unwrap_or_else(|| {
                    Err(AcePageTaskError::new(500, "Page task produced no result"))
                }),
            };
        let CdpCommandTaskStep::Complete(output) = self
            .connection
            .complete_with_output_plan(context, plan, None, None)
        else {
            unreachable!("page task completion only publishes retained events");
        };
        (result, output)
    }

    fn finish_ace_page_task(
        self,
        host: &ProtocolPageHostScope,
        page: &TargetPageResidenceIdentity,
        kind: AcePageTaskKind,
        completed: anyhow::Result<CompletedPageCommand>,
        context: &mut CommandDispatchContext,
    ) -> Result<moli_core::ace_page_task::AcePageTaskExecution, AcePageTaskError> {
        let current = host.is_current(self.connection);
        let completed = completed
            .map_err(|error| AcePageTaskError::new(if current { 409 } else { 404 }, error))?;
        let output = if current {
            self.connection
                .settle_page_command_turn_for_owner(page, completed)
        } else {
            completed.into_output()
        };
        let completion = context.consume_renderer_command_turn_output(output);
        if !current {
            return Err(AcePageTaskError::new(404, "Page target closed during task"));
        }
        if kind == AcePageTaskKind::ReadOnly
            && self
                .connection
                .target_page_residence_identity_for_owner(host.owner())
                .as_ref()
                != Some(page)
        {
            return Err(AcePageTaskError::new(409, "Document changed during task"));
        }
        match completion.reply() {
            RendererPageReply::AcePageTask(output) => Ok(output.as_ref().clone()),
            _ => Err(AcePageTaskError::new(500, "Unexpected page task reply")),
        }
    }
}

impl CdpConnection {
    pub(crate) fn ace_page_task_browser_snapshot(
        &self,
        browser_context_id: &str,
        source_target_id: Option<String>,
    ) -> BrowserSnapshot {
        let mut snapshot = BrowserSnapshot {
            source_target_id,
            targets: Vec::new(),
        };
        let Some(context) = self.browser_context_by_id(browser_context_id) else {
            return snapshot;
        };
        snapshot.targets = context
            .devtools_target_infos()
            .into_iter()
            .filter_map(|info| {
                let id = info.target_id?;
                let page = context.page_target(id.as_str());
                Some(BrowserTargetSnapshot {
                    has_page: page.is_some(),
                    pending_dialog: page.is_some_and(|page| page.has_pending_javascript_dialog()),
                    pending_navigation: context
                        .has_pending_document_navigation_for_target(Some(id.as_str())),
                    id: id.into_string(),
                    url: info.url,
                    title: info.title,
                    opener_id: info.opener_id.map(|id| id.into_string()),
                    is_page: info.kind == crate::DevToolsTargetKind::Page,
                })
            })
            .collect();
        snapshot
    }
}
