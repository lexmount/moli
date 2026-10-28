use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::*;

/// Typed projection of the original navigation. The command's existing owner
/// keeps its success value; Native/domain completions update the same value.
#[derive(Clone, Debug)]
pub(crate) struct AutomationNavigationResult(Rc<AutomationNavigationOutput>);

#[derive(Debug)]
struct AutomationNavigationOutput {
    host: crate::conn::ProtocolPageHostScope,
    wait: DevToolsNavigationWait,
    fallback_loader_id: Option<String>,
    source_response_transferred: Cell<bool>,
    result: RefCell<Result<DirectNavigationResult, DevToolsError>>,
}

impl AutomationNavigationResult {
    pub(crate) fn owner(&self) -> &CommandOwnerScope {
        self.0.host.owner()
    }

    pub(crate) fn host(&self) -> crate::conn::ProtocolPageHostScope {
        self.0.host.clone()
    }

    pub(crate) fn finish(self) -> Result<AutomationResult, DevToolsError> {
        if self.0.source_response_transferred.get() && self.0.wait != DevToolsNavigationWait::None {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                MissingAutomationNavigationResult,
            ));
        }
        let mut result = self.0.result.borrow_mut();
        if self.0.wait == DevToolsNavigationWait::None
            && let Ok(result) = result.as_mut()
        {
            result.ensure_navigation_id_from_loader(self.0.fallback_loader_id.clone());
        }
        result.clone().map(DirectNavigationResult::into_result)
    }

    fn update(
        &self,
        operation: impl FnOnce(&mut DirectNavigationResult) -> Result<(), DevToolsError>,
    ) {
        let mut result = self.0.result.borrow_mut();
        if let Ok(value) = result.as_mut()
            && let Err(error) = operation(value)
        {
            *result = Err(error);
        }
    }

    pub(in crate::domains::page) fn transfer_source_response(&self) {
        self.0.source_response_transferred.set(true);
    }

    pub(super) fn observe_identity(&self, frame_id: &str, loader_id: &str) {
        self.update(|result| {
            result.set_navigation_identity(frame_id, loader_id);
            if self.0.wait == DevToolsNavigationWait::None {
                result.ensure_navigation_id_from_loader(Some(loader_id.to_owned()));
            }
            Ok(())
        });
    }

    pub(super) fn observe_load(
        &self,
        conn: &CdpConnection,
        completed: &CompletedNavigateLoadCommand,
    ) {
        self.update(|result| direct_navigation_result_from_completed_load(conn, completed, result));
    }

    pub(super) fn observe_request(
        &self,
        completed: &CompletedContinueNavigationWithoutRequestPauseCommand,
    ) {
        self.update(|result| direct_navigation_result_from_fetch_continuation(completed, result));
    }

    pub(super) fn observe_same_document(&self, completed: &CompletedSameDocumentNavigateCommand) {
        self.update(|result| {
            direct_navigation_result_from_completed_same_document(completed, result)
        });
    }

    pub(super) fn observe_history(&self, completed: &CompletedSameDocumentHistoryTraversalCommand) {
        self.update(|result| {
            direct_navigation_result_from_completed_same_document_traversal(completed, result)
        });
    }

    pub(in crate::domains::page) fn observe_child(
        &self,
        completed: &CompletedChildFrameNavigateCommand,
    ) {
        self.update(|result| {
            direct_navigation_result_from_completed_child_frame(completed, result)
        });
    }
}

pub(crate) fn prepare_automation_navigation(
    conn: &mut CdpConnection,
    mut command: AutomationCommand,
    background_id: Option<u64>,
) -> Result<(PageCommandTaskStep, AutomationNavigationResult), DevToolsError> {
    let route = devtools_navigation_target_route(conn, &command)?;
    let owner = CommandOwnerScope::for_route(route);
    let host = crate::conn::ProtocolPageHostScope::capture_for_operation(conn, &owner)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, error))?;
    let protocol = command.context().protocol;
    let wait = devtools_navigation_wait(&command);
    let (direct, payload, noop) = match &mut command {
        AutomationCommand::Navigate(command) => {
            Url::parse(&command.url).map_err(|_| {
                DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    AutomationNavigationError::InvalidUrl,
                )
            })?;
            let frame = command
                .context
                .target_id
                .as_ref()
                .map(|target| target.as_str());
            let loader = child_frame_navigation_target_id(conn, &owner, command)
                .is_none()
                .then_some(LOADER_ID);
            (
                DirectNavigationResult::navigate(protocol, frame, loader, command.url.clone()),
                protocol_neutral_navigate_result_payload(protocol, frame, loader, &command.url),
                false,
            )
        }
        AutomationCommand::Reload(command) => {
            let url = conn
                .runtime_session_owner_target_url_for_owner(&owner)
                .ok_or_else(|| {
                    DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        AutomationNavigationError::TargetNotLoaded,
                    )
                })?;
            command.context.session_id = None;
            (
                DirectNavigationResult::navigate(protocol, None, Some(LOADER_ID), url.clone()),
                protocol_neutral_navigate_result_payload(protocol, None, Some(LOADER_ID), &url),
                false,
            )
        }
        AutomationCommand::TraverseHistory(command) => {
            command.context.session_id = None;
            match resolve_devtools_history_traversal_destination(
                conn,
                &owner,
                &command.destination,
            )? {
                ResolvedDevToolsHistoryTraversal::Noop => {
                    (DirectNavigationResult::empty(), json!({}), true)
                }
                ResolvedDevToolsHistoryTraversal::Entry {
                    entry_id,
                    url,
                    same_document_delta,
                } => {
                    let mut result = DirectNavigationResult::traverse_history(
                        protocol,
                        same_document_delta.is_some(),
                    );
                    result.set_url(url.clone());
                    command.destination =
                        DevToolsHistoryTraversalDestination::Entry { entry_id, url };
                    (result, json!({}), false)
                }
            }
        }
        _ => unreachable!("only navigation commands use this admission"),
    };
    let result = AutomationNavigationResult(Rc::new(AutomationNavigationOutput {
        host,
        wait,
        fallback_loader_id: conn.current_document_loader_id_for_owner(&owner),
        source_response_transferred: Cell::new(false),
        result: RefCell::new(Ok(direct)),
    }));
    if noop {
        return Ok((
            PageCommandTaskStep::Complete(CommandOutputPlan::success()),
            result,
        ));
    }
    // An auth/Fetch handoff retains its actual source response id. A plain
    // wait=None background navigation has no second command response owner.
    let source_id = background_id;
    let reloaded_after_crash_session_ids = reloaded_after_crash_session_ids(conn, &owner);
    let step = start_devtools_page_command(
        conn,
        source_id,
        command,
        DevToolsNavigationStartOptions {
            owner,
            result_projection: NavigationResultProjection::Automation {
                protocol,
                payload,
                result: result.clone(),
            },
            reloaded_after_crash_session_ids,
            allow_background_navigation: wait == DevToolsNavigationWait::None,
        },
    );
    Ok((step, result))
}

/// The exclusive compatibility entry uses the same prepared domain command
/// and Page completion body. It does not submit a task to its own mailbox.
pub(crate) async fn execute_automation_navigation_inline(
    conn: &mut CdpConnection,
    command: AutomationCommand,
    background_id: Option<u64>,
) -> (
    Result<AutomationResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    let (step, result) = match prepare_automation_navigation(conn, command, background_id) {
        Ok(prepared) => prepared,
        Err(error) => return (Err(error), Vec::new(), None),
    };
    let mut dispatch = CommandDispatchContext::default();
    let (mut context, _recovery) =
        crate::conn::CommandWorkflowContext::take_from(&mut dispatch, None, None);
    let plan = match step {
        PageCommandTaskStep::Complete(plan) => Ok(plan),
        PageCommandTaskStep::Pending(pending) => {
            Box::pin(super::super::completion::complete_to_output_using(
                &mut crate::conn::ProtocolWorkflowDriver::inline(conn),
                &mut context,
                pending.wait().await,
            ))
            .await
        }
    };
    let completed = match plan {
        Ok(plan) => context.complete(plan),
        Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
    };
    let plan = completed.finish(conn, &mut dispatch);
    let (status, events) = plan.into_command_status_and_background_events();
    let result = match status {
        Some(Err(error)) => Err(error),
        _ => result.finish(),
    };
    let mut events = dispatch.take_protocol_events_before_events(events);
    events.extend(dispatch.take_post_response_events());
    (result, events, dispatch.take_renderer_output_predecessor())
}

#[derive(Debug, thiserror::Error)]
enum AutomationNavigationError {
    #[error("Invalid navigation URL")]
    InvalidUrl,
    #[error("TargetNotLoaded")]
    TargetNotLoaded,
}

#[derive(Debug, thiserror::Error)]
#[error("MissingDevToolsCommandResult")]
struct MissingAutomationNavigationResult;
