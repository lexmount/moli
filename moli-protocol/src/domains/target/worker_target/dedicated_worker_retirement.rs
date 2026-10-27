use anyhow::Result;
use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::automation::AutomationResult;
use crate::conn::{CommandWorkflowContext, ProtocolWorkflowDriver};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::target::session_disposal::{PreparedSessionDisposal, SessionDisposalIdentity};

#[derive(Debug, thiserror::Error)]
#[error("Target is closing")]
struct WorkerRetirementInProgress;

/// The renderer's original worker and its original owning Page, even after
/// that Page has been replaced. Retirement never resolves an ambient target.
#[derive(Clone)]
pub(crate) struct DedicatedWorkerRetirementIdentity {
    browser_context_id: String,
    target_id: String,
    instance_id: u64,
    owner_local_host_id: moli_core::RendererOwnerLocalHostId,
    owner_page: TargetPageResidenceIdentity,
}

struct WorkerSessionRetirement {
    identity: SessionDisposalIdentity,
    disposal: PreparedSessionDisposal,
    detachment: TargetSessionDetachCleanupPlan,
    info_events: Rc<RefCell<Vec<BackgroundProtocolEvent>>>,
}

pub(crate) struct PreparedDedicatedWorkerRetirement {
    identity: DedicatedWorkerRetirementIdentity,
    sessions: Vec<WorkerSessionRetirement>,
    destroyed_delta: Option<PreparedTargetHostDelta>,
    uncommitted: Vec<CapturedWorkerAttachment>,
    start_protocol_events: Vec<BackgroundProtocolEvent>,
}

/// Native capture may accept an auto-attach before its ordered protocol
/// projection commits the binding. Closing owns that exact attachment too;
/// the original captured fact may still be published, but it cannot leave a
/// late binding behind after target retirement.
#[derive(Clone)]
pub(crate) struct CapturedWorkerAttachment {
    identity: TargetSharedWorkerProtocolAttachmentIdentity,
    detachment: TargetSessionDetachCleanupPlan,
}

struct WorkerSessionFailureCleanup {
    identity: SessionDisposalIdentity,
    detachment: TargetSessionDetachCleanupPlan,
    join: Option<crate::conn::SessionDisposalWait>,
    info_events: Rc<RefCell<Vec<BackgroundProtocolEvent>>>,
}

struct WorkerRetirementFailureCleanup {
    identity: DedicatedWorkerRetirementIdentity,
    sessions: Vec<WorkerSessionFailureCleanup>,
    destroyed_delta: Option<PreparedTargetHostDelta>,
    uncommitted: Vec<CapturedWorkerAttachment>,
}

impl PreparedDedicatedWorkerRetirement {
    pub(in crate::domains::target) fn prepare_close(
        conn: &mut CdpConnection,
        target_id: &str,
    ) -> Option<Result<Self>> {
        let CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id, ..
        } = conn.target_session_route_for_target_id(target_id)?
        else {
            return None;
        };
        let context = conn.browser_context_by_id(&browser_context_id)?;
        let target = context.dedicated_worker_target(target_id)?;
        if target.is_retiring() {
            return Some(Err(WorkerRetirementInProgress.into()));
        }
        let identity = DedicatedWorkerRetirementIdentity {
            browser_context_id,
            target_id: target_id.to_owned(),
            instance_id: target.renderer_instance_id,
            owner_local_host_id: target.renderer_owner_local_host_id,
            owner_page: target.owner_page.clone(),
        };
        let renderer_runtime = context.renderer_runtime();
        let session_ids = target.session_ids();
        let mut start_protocol_events = Vec::new();
        let mut claimed = Vec::new();
        // Native destruction is allowed to progress while cleanup waits.
        // Settle the original calls at close admission first, preserving the
        // existing Target-detached terminal rather than racing a native
        // context-destroyed reply. Cleanup calls are admitted only afterwards.
        for session in session_ids {
            crate::domains::runtime::fail_pending_session_calls(
                conn,
                &mut start_protocol_events,
                &mut claimed,
                &session,
            );
        }
        start_protocol_events.extend(claimed);
        renderer_runtime.close_dedicated_worker_for_devtools(identity.instance_id);
        let outputs = prepare_dedicated_worker_target_retirement(
            conn,
            &identity.browser_context_id,
            identity.instance_id,
            DedicatedWorkerRetirementCause::OwnerRetired,
        );
        let mut sessions = Vec::new();
        let mut uncommitted = Vec::new();
        let mut destroyed_delta = None;
        for output in outputs.worker_target_lifecycle_outputs {
            match output {
                WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
                    target_delta,
                    cleanup_plan,
                } => {
                    // A concurrent detachment may already own this binding's
                    // handlers. Join its original watch rather than run them
                    // twice; the actor remains free to finish that detachment.
                    let Ok((plan, detachment)) =
                        super::super::session_disposal::prepare_session_disposal(
                            conn,
                            cleanup_plan.clone(),
                        )
                    else {
                        if conn
                            .session_route(Some(cleanup_plan.session_id()))
                            .is_none()
                            && let Some(attachment) = conn
                                .browser_context_by_id(&identity.browser_context_id)
                                .and_then(|context| {
                                    context.dedicated_worker_target(&identity.target_id)
                                })
                                .and_then(|target| {
                                    target.protocol_attachment_identity(
                                        &identity.browser_context_id,
                                        cleanup_plan.session_id(),
                                    )
                                })
                        {
                            uncommitted.push(CapturedWorkerAttachment {
                                identity: attachment,
                                detachment: cleanup_plan,
                            });
                        }
                        continue;
                    };
                    let disposal = PreparedSessionDisposal::prepare(conn, plan);
                    let info_events = target_delta
                        .map(|delta| {
                            conn.prepared_target_info_changed_event_plan_for_discovery_owners(delta)
                                .into_background_events()
                        })
                        .unwrap_or_default();
                    sessions.push(WorkerSessionRetirement {
                        identity: disposal.identity(),
                        disposal,
                        detachment,
                        info_events: Rc::new(RefCell::new(info_events)),
                    });
                }
                WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed { target_delta, .. } => {
                    destroyed_delta = target_delta;
                }
                _ => unreachable!("worker retirement contains only terminal lifecycle output"),
            }
        }
        Some(Ok(Self {
            identity,
            sessions,
            destroyed_delta,
            uncommitted,
            start_protocol_events,
        }))
    }

    pub(in crate::domains::target) fn into_command_workflow(
        mut self,
        access: crate::conn::ProtocolStateAccess,
    ) -> crate::conn::PreparedCommandWorkflow {
        let cleanup = self.failure_cleanup();
        let start_protocol_events = std::mem::take(&mut self.start_protocol_events);
        let failure_access = access.clone();
        crate::conn::PreparedCommandWorkflow::new(move |context| async move {
            self.run(&mut ProtocolWorkflowDriver::owned(access), context)
                .await
        })
        .with_failure_cleanup(move |response| {
            cleanup.into_command_workflow(failure_access, response)
        })
        .with_start_protocol_events(start_protocol_events)
    }

    fn failure_cleanup(&self) -> WorkerRetirementFailureCleanup {
        WorkerRetirementFailureCleanup {
            identity: self.identity.clone(),
            sessions: self
                .sessions
                .iter()
                .map(|session| WorkerSessionFailureCleanup {
                    identity: session.identity.clone(),
                    detachment: session.detachment.clone(),
                    join: match &session.disposal {
                        PreparedSessionDisposal::Join { wait, .. } => Some(wait.clone()),
                        PreparedSessionDisposal::Own(_) => None,
                    },
                    info_events: session.info_events.clone(),
                })
                .collect(),
            destroyed_delta: self.destroyed_delta.clone(),
            uncommitted: self.uncommitted.clone(),
        }
    }

    pub(in crate::domains::target) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        mut context: CommandWorkflowContext,
    ) -> crate::conn::CompletedCommandWorkflow {
        let mut first_error = None;
        for session in std::mem::take(&mut self.sessions) {
            let mut prefix = CommandOutputPlan::default();
            prefix.extend_background_events(std::mem::take(&mut *session.info_events.borrow_mut()));
            context.retain_output(prefix);
            let mut background = Vec::new();
            let mut protocol = Vec::new();
            let result = session
                .disposal
                .run(driver, &mut background, &mut protocol)
                .await;
            let mut output = CommandOutputPlan::default();
            output.extend_background_events(background);
            output.extend_background_events(protocol);
            context.retain_output(output);
            let failed = result.is_err();
            match result {
                Ok(predecessor) => assert!(
                    predecessor.is_none(),
                    "worker disposal cannot publish a Page renderer fence"
                ),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
            let worker = self.identity.clone();
            let completion = context
                .with_state(driver, move |state, context| {
                    let mut output = CommandOutputPlan::default();
                    let (events, error) = state.finish_dedicated_worker_session_retirement(
                        worker,
                        session.identity,
                        session.detachment,
                        failed,
                    );
                    output.extend_background_events(events);
                    context.retain_output(output);
                    error
                })
                .await;
            if let Some(error) = match completion {
                Ok(error) => error,
                Err(error) => Some(error),
            } {
                first_error.get_or_insert(error);
            }
        }
        context
            .finish_with_state(driver, move |state, context| {
                let mut output = CommandOutputPlan::default();
                output.extend_background_events(state.finish_dedicated_worker_retirement(
                    self.identity,
                    self.destroyed_delta,
                    self.uncommitted,
                ));
                context.retain_output(output);
                match first_error {
                    Some(error) => CommandOutputPlan::error(-32000, error),
                    None => CommandOutputPlan::from_devtools_result(AutomationResult::CloseTarget(
                        crate::automation::DevToolsCloseTargetResult { success: true },
                    )),
                }
            })
            .await
    }
}

impl WorkerRetirementFailureCleanup {
    fn into_command_workflow(
        self,
        access: crate::conn::ProtocolStateAccess,
        response: CommandOutputPlan,
    ) -> crate::conn::PreparedCommandWorkflow {
        crate::conn::PreparedCommandWorkflow::new(move |context| async move {
            self.run(
                &mut ProtocolWorkflowDriver::owned(access),
                context,
                response,
            )
            .await
        })
    }

    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        mut context: CommandWorkflowContext,
        response: CommandOutputPlan,
    ) -> crate::conn::CompletedCommandWorkflow {
        for session in self.sessions {
            if let Some(wait) = session.join.as_ref() {
                wait.clone().wait().await;
            }
            let mut prefix = CommandOutputPlan::default();
            prefix.extend_background_events(std::mem::take(&mut *session.info_events.borrow_mut()));
            context.retain_output(prefix);
            let worker = self.identity.clone();
            let result = context
                .with_state(driver, move |state, context| {
                    let (events, error) = state.recover_dedicated_worker_session_retirement(
                        worker,
                        session.identity,
                        session.detachment,
                        session.join.is_none(),
                    );
                    let mut output = CommandOutputPlan::default();
                    output.extend_background_events(events);
                    context.retain_output(output);
                    error
                })
                .await;
            match result {
                Ok(None) => {}
                Ok(Some(error)) | Err(error) => {
                    tracing::warn!(%error, "failed to retire original worker session after workflow failure")
                }
            }
        }
        context
            .finish_with_state(driver, move |state, context| {
                let mut output = CommandOutputPlan::default();
                output.extend_background_events(state.finish_dedicated_worker_retirement(
                    self.identity,
                    self.destroyed_delta,
                    self.uncommitted,
                ));
                context.retain_output(output);
                response
            })
            .await
    }
}

impl CapturedWorkerAttachment {
    fn retire(
        self,
        conn: &mut CdpConnection,
        worker: &DedicatedWorkerRetirementIdentity,
    ) -> Vec<BackgroundProtocolEvent> {
        let original = self.identity.is_current()
            && conn
                .browser_context_by_id(&worker.browser_context_id)
                .and_then(|context| context.dedicated_worker_target(&worker.target_id))
                .and_then(|target| {
                    target.protocol_attachment_identity(
                        &worker.browser_context_id,
                        self.identity.session_id(),
                    )
                })
                .as_ref()
                == Some(&self.identity);
        let route = CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id: worker.browser_context_id.clone(),
            target_id: worker.target_id.clone(),
        };
        if !original
            || conn
                .session_route(Some(self.identity.session_id()))
                .as_ref()
                != Some(&route)
        {
            return Vec::new();
        }
        let plan =
            crate::conn::SessionDisposalPlan::for_session_route(self.identity.session_id(), &route)
                .expect("a dedicated-worker binding has a disposal plan");
        conn.mark_session_closing(&plan);
        // Before this first protocol binding existed, no frontend could enable
        // its domains. After binding, the target's existing Closing admission
        // rejects new work. Native resources are already owned by terminate;
        // only connection/control-plane handlers survive this attachment.
        crate::domains::session::dispose_closed_handlers_sync(conn, self.identity.session_id());
        let mut events = Vec::new();
        let mut protocol = Vec::new();
        crate::domains::runtime::fail_pending_session_calls(
            conn,
            &mut events,
            &mut protocol,
            self.identity.session_id(),
        );
        events.extend(protocol);
        if let Err(error) = conn.commit_session_disposal(&plan) {
            tracing::warn!(%error, "failed to retire captured worker attachment metadata");
        }
        events.extend(
            conn.commit_target_session_detachment_after_prepared_state_delta_event_plan(
                self.detachment,
            ),
        );
        events
    }
}

impl DedicatedWorkerRetirementIdentity {
    fn is_current(&self, conn: &CdpConnection) -> bool {
        conn.browser_context_by_id(&self.browser_context_id)
            .and_then(|context| context.dedicated_worker_targets.get(&self.instance_id))
            .is_some_and(|target| {
                target.target_id == self.target_id
                    && target.renderer_owner_local_host_id == self.owner_local_host_id
                    && target.owner_page == self.owner_page
                    && target.is_retiring()
            })
    }

    pub(crate) fn finish(
        self,
        conn: &mut CdpConnection,
        destroyed_delta: Option<PreparedTargetHostDelta>,
        uncommitted: Vec<CapturedWorkerAttachment>,
    ) -> Vec<BackgroundProtocolEvent> {
        if !self.is_current(conn) {
            return Vec::new();
        }
        let mut events = Vec::new();
        for attachment in uncommitted {
            events.extend(attachment.retire(conn, &self));
        }
        conn.browser_context_by_id_mut(&self.browser_context_id)
            .expect("a validated worker retains its browser context")
            .remove_dedicated_worker_target_by_renderer_instance(self.instance_id);
        if let Some(delta) = destroyed_delta {
            events.extend(conn.prepared_target_host_delta_event_plan(delta));
        }
        events
    }

    pub(crate) fn recover_session(
        self,
        conn: &mut CdpConnection,
        identity: SessionDisposalIdentity,
        detachment: TargetSessionDetachCleanupPlan,
        owns_cleanup: bool,
    ) -> (Vec<BackgroundProtocolEvent>, Option<anyhow::Error>) {
        if !owns_cleanup || !self.is_current(conn) || !identity.is_current(conn) {
            return (Vec::new(), None);
        }
        let mut background = Vec::new();
        let mut protocol = Vec::new();
        crate::domains::runtime::fail_pending_session_calls(
            conn,
            &mut background,
            &mut protocol,
            identity.plan().session_id(),
        );
        let detach_error = identity.detach_inspector(conn).err();
        background.extend(protocol);
        let (events, error) = self.finish_session(conn, identity, detachment, true);
        background.extend(events);
        (background, detach_error.or(error))
    }

    pub(crate) fn finish_session(
        self,
        conn: &mut CdpConnection,
        identity: SessionDisposalIdentity,
        detachment: TargetSessionDetachCleanupPlan,
        failed: bool,
    ) -> (Vec<BackgroundProtocolEvent>, Option<anyhow::Error>) {
        if !self.is_current(conn) || !identity.is_current(conn) {
            return (Vec::new(), None);
        }
        if failed {
            crate::domains::session::dispose_closed_handlers_sync(
                conn,
                identity.plan().session_id(),
            );
        }
        let error = conn.commit_session_disposal(identity.plan()).err();
        let events =
            conn.commit_target_session_detachment_after_prepared_state_delta_event_plan(detachment);
        (events.into_iter().collect(), error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, OwnedProtocolWaitFailure,
        PendingCdpCommandDispatch, ProtocolStateRequestReceiver,
    };

    fn install_worker(conn: &mut CdpConnection, owner: TargetPageResidenceIdentity, host: u64) {
        conn.browser_context_by_id_mut("BID-worker-panic")
            .unwrap()
            .insert_dedicated_worker_target(crate::conn::DedicatedWorkerTargetState::new(
                owner,
                moli_core::RendererOwnerLocalHostId::new_for_testing(host),
                701,
                "TID-worker-panic".into(),
                String::new(),
                Vec::new(),
            ));
        for session in ["SID-worker-panic", "SID-worker-panic-peer"] {
            let route = conn
                .prepare_auto_attached_dedicated_worker_session_binding(
                    "TID-worker-panic",
                    session.into(),
                )
                .unwrap();
            conn.register_session_route_for_test(session, route);
        }
    }

    async fn drive(
        conn: &mut CdpConnection,
        requests: &mut ProtocolStateRequestReceiver,
        pending: PendingCdpCommandDispatch,
    ) -> crate::conn::CompletedCdpCommandDispatch {
        let mut pending = Box::pin(pending.wait());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    completed = &mut pending => break completed,
                    request = requests.recv() => request.unwrap().apply(conn),
                }
            }
        })
        .await
        .expect("panic cleanup must release its original in-flight responsibility")
    }

    #[tokio::test]
    async fn panicked_worker_cleanup_finishes_originals_and_preserves_rebound_successors() {
        fn fail() {
            panic!("controlled worker cleanup failure");
        }
        for rebound in [false, true] {
            let mut conn = CdpConnection::new();
            let mut browser = BrowserContext::new("BID-worker-panic".into());
            browser.set_active_target_id("TID-worker-owner");
            let attachment = browser
                .active_page_target_mut()
                .runtime_slot
                .set_page_attachment_id_for_test(1);
            let owner = TargetPageResidenceIdentity::new(
                "BID-worker-panic".into(),
                Some("TID-worker-owner".into()),
                attachment,
            );
            conn.install_browser_context_fixture_for_test(browser);
            install_worker(&mut conn, owner.clone(), 17);
            conn.set_target_discovery_for_owner(
                None,
                crate::conn::CdpTargetFilter::default_target_discovery(),
            );
            let info = conn.devtools_target_info("TID-worker-panic").unwrap();
            let _ = conn.prepared_target_host_delta_event_plan(PreparedTargetHostDelta::created(
                "TID-worker-panic",
                Some(info),
            ));
            let mut requests = conn.take_protocol_state_requests().unwrap();
            let access = conn.protocol_state_access().unwrap();
            let retirement =
                PreparedDedicatedWorkerRetirement::prepare_close(&mut conn, "TID-worker-panic")
                    .unwrap()
                    .unwrap();
            let cleanup = retirement.failure_cleanup();
            let workflow =
                crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
                    let mut prefix = CommandOutputPlan::default();
                    prefix.extend_background_events(std::mem::take(
                        &mut *retirement.sessions[0].info_events.borrow_mut(),
                    ));
                    prefix.push_background_event(
                        BackgroundProtocolEvent::page_file_chooser_opened(
                            None,
                            "FRAME-worker-panic",
                            "selectSingle",
                            77,
                            Some("NODE-worker-panic".into()),
                        ),
                    );
                    context.retain_output(prefix);
                    // Unwind after original Native handler preparation and an
                    // output prefix. No fault-injection state is added to production.
                    drop(retirement);
                    fail();
                    context.complete(CommandOutputPlan::success())
                })
                .with_failure_cleanup(move |response| {
                    cleanup.into_command_workflow(access, response)
                });
            let mut context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) =
                conn.start_command_workflow(workflow, Some(702), None, &mut context)
            else {
                panic!("the existing command entry supervises this workflow")
            };
            let completed = drive(&mut conn, &mut requests, *pending).await;
            let CdpCommandTaskStep::Pending(cleanup) = conn
                .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                .unwrap_or_else(|_| panic!("panic handoff is finite"))
            else {
                panic!("panic must run resource cleanup before returning its terminal")
            };
            if rebound {
                conn.browser_context_by_id_mut("BID-worker-panic")
                    .unwrap()
                    .remove_dedicated_worker_target_by_renderer_instance(701)
                    .unwrap();
                install_worker(&mut conn, owner, 119);
            }
            let bindings = ["SID-worker-panic", "SID-worker-panic-peer"]
                .map(|session| conn.session_binding_lifetime(session));
            let completed = drive(&mut conn, &mut requests, *cleanup).await;
            let CdpCommandTaskStep::Complete(outcome) = conn
                .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                .unwrap_or_else(|_| panic!("cleanup terminal is finite"))
            else {
                panic!("resource recovery is complete")
            };
            let (events, _, _) = outcome.into_protocol_event_parts();
            let messages = events
                .iter()
                .cloned()
                .map(BackgroundProtocolEvent::into_protocol_message)
                .collect::<Vec<_>>();
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == 702)
                    .count(),
                1
            );
            assert_eq!(
                messages[0]["error"]["message"],
                OwnedProtocolWaitFailure::Panicked.message()
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Target.targetInfoChanged")
                    .count(),
                1,
                "the frozen prefix is claimed once across panic: {messages:?}"
            );
            assert!(events.iter().any(|event| matches!(
                event.clone().into_parts().1,
                Some(crate::automation::AutomationEvent::PageFileChooserOpened(_))
            )));
            let destroyed = messages
                .iter()
                .filter(|message| message["method"] == "Target.targetDestroyed")
                .count();
            let detached = messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget")
                .count();
            if rebound {
                assert_eq!((destroyed, detached), (0, 0));
                let worker = conn
                    .browser_context_by_id("BID-worker-panic")
                    .unwrap()
                    .dedicated_worker_target("TID-worker-panic")
                    .unwrap();
                assert!(!worker.is_retiring());
                for (index, session) in ["SID-worker-panic", "SID-worker-panic-peer"]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(conn.session_binding_lifetime(session), bindings[index]);
                    assert!(!conn.session_is_closing(session));
                    assert!(worker.is_session(session));
                }
            } else {
                assert_eq!(
                    (destroyed, detached),
                    (1, 2),
                    "panic must finish all original bindings: {messages:?}"
                );
                assert!(
                    conn.browser_context_by_id("BID-worker-panic")
                        .unwrap()
                        .dedicated_worker_target("TID-worker-panic")
                        .is_none()
                );
                for session in ["SID-worker-panic", "SID-worker-panic-peer"] {
                    assert!(conn.session_route(Some(session)).is_none());
                }
            }
        }
    }
}
