use anyhow::Result;
use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::conn::{PreparedCommandWorkflow, ProtocolWorkflowDriver};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::target::session_disposal::PreparedRemovedWorkerSession;

/// The renderer target has already transferred its attachment scopes into
/// these ordered outputs. Waiting for its connection-owned handlers does not
/// retain either the target or a connection borrow.
pub(crate) struct PreparedSharedWorkerRetirement {
    sessions: Vec<RemovedSharedWorkerAttachment>,
    destroyed: Option<PreparedTargetHostDelta>,
    prefix: Vec<BackgroundProtocolEvent>,
    preparation_error: Option<anyhow::Error>,
}

#[derive(Clone)]
struct RemovedSharedWorkerAttachment {
    retirement: TargetSharedWorkerProtocolAttachmentRetirement,
    cleanup: Option<PreparedRemovedWorkerSession>,
}

#[derive(Clone)]
struct SharedWorkerRetirementCompletion {
    sessions: Vec<RemovedSharedWorkerAttachment>,
    destroyed: Rc<RefCell<Option<PreparedTargetHostDelta>>>,
}

impl PreparedSharedWorkerRetirement {
    pub(in crate::domains::target) fn prepare_close(
        conn: &mut CdpConnection,
        target_id: &str,
    ) -> Option<Self> {
        let CdpSessionRoute::SharedWorkerTarget {
            browser_context_id, ..
        } = conn.target_session_route_for_target_id(target_id)?
        else {
            return None;
        };
        let context = conn.browser_context_by_id(&browser_context_id)?;
        let target = context.shared_worker_target(target_id)?;
        let instance_id = target.renderer_instance_id;
        let renderer = context.renderer_runtime();
        // Settle the original protocol calls before native destruction can
        // race their terminal. Both belong to this finite close admission.
        let outputs = remove_shared_worker_target(conn, &browser_context_id, instance_id);
        renderer.close_shared_worker_for_target_close(instance_id);
        Some(Self::prepare_outputs(conn, outputs))
    }

    fn prepare_outputs(conn: &mut CdpConnection, outputs: TargetPreparedOutputs) -> Self {
        let mut prepared = Self {
            sessions: Vec::new(),
            destroyed: None,
            prefix: Vec::new(),
            preparation_error: None,
        };
        for output in outputs.worker_target_lifecycle_outputs {
            match output {
                WorkerTargetLifecycleOutput::SharedWorkerAttachmentEvents {
                    attachment,
                    events,
                } => {
                    if attachment.is_current() {
                        prepared.prefix.extend(events);
                    }
                }
                WorkerTargetLifecycleOutput::SharedWorkerDetached {
                    retirement,
                    cleanup_plan,
                } => {
                    if retirement.is_current() {
                        let cleanup =
                            match PreparedRemovedWorkerSession::prepare(conn, cleanup_plan) {
                                Ok(cleanup) => Some(cleanup),
                                Err(error) => {
                                    prepared.preparation_error.get_or_insert(error);
                                    None
                                }
                            };
                        prepared.sessions.push(RemovedSharedWorkerAttachment {
                            retirement,
                            cleanup,
                        });
                    }
                }
                WorkerTargetLifecycleOutput::SharedWorkerDestroyed { target_delta } => {
                    prepared.destroyed = Some(target_delta);
                }
                _ => unreachable!("SharedWorker retirement contains only terminal output"),
            }
        }
        prepared
    }

    pub(in crate::domains::target) async fn run_inline(
        self,
        conn: &mut CdpConnection,
        command_context: &mut crate::conn::CommandDispatchContext,
    ) -> Result<()> {
        let completion = SharedWorkerRetirementCompletion {
            sessions: self.sessions,
            destroyed: Rc::new(RefCell::new(self.destroyed)),
        };
        let mut local_context = crate::conn::CommandDispatchContext::default();
        let (mut context, _scope) =
            crate::conn::CommandWorkflowContext::take_from(&mut local_context, None, None);
        context
            .context_mut()
            .protocol_events_mut()
            .extend(self.prefix);
        let result = completion
            .run(&mut ProtocolWorkflowDriver::inline(conn), &mut context)
            .await;
        let plan = context
            .complete(CommandOutputPlan::default())
            .finish(conn, &mut local_context);
        command_context
            .protocol_events_mut()
            .extend(local_context.take_protocol_events());
        command_context
            .protocol_events_mut()
            .extend(plan.into_background_events(None, None));
        match result.err().or(self.preparation_error) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(in crate::domains::target) fn into_command_workflow(
        self,
        access: crate::conn::ProtocolStateAccess,
    ) -> PreparedCommandWorkflow {
        let completion = SharedWorkerRetirementCompletion {
            sessions: self.sessions,
            destroyed: Rc::new(RefCell::new(self.destroyed)),
        };
        let recovery = completion.clone();
        let recovery_access = access.clone();
        PreparedCommandWorkflow::new(move |mut context| async move {
            let result = completion
                .run(&mut ProtocolWorkflowDriver::owned(access), &mut context)
                .await;
            match result.err().or(self.preparation_error) {
                None => context.complete(CommandOutputPlan::from_devtools_result(
                    crate::automation::AutomationResult::CloseTarget(
                        crate::automation::DevToolsCloseTargetResult { success: true },
                    ),
                )),
                Some(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
        .with_failure_cleanup(move |response| {
            PreparedCommandWorkflow::new(move |mut context| async move {
                if let Err(error) = recovery
                    .run(
                        &mut ProtocolWorkflowDriver::owned(recovery_access),
                        &mut context,
                    )
                    .await
                {
                    tracing::debug!(%error, "SharedWorker retirement lost its protocol owner");
                }
                context.complete(response)
            })
        })
        .with_start_protocol_events(self.prefix)
    }
}

impl SharedWorkerRetirementCompletion {
    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        for session in self.sessions {
            if let Some(cleanup) = &session.cleanup {
                cleanup.wait().await;
            }
            context
                .with_state(driver, move |state, context| {
                    let events = state.finish_removed_shared_worker_attachment(
                        session.cleanup,
                        session.retirement,
                    );
                    // SharedWorker removal has always exposed its complete
                    // retirement prefix before the close acknowledgement.
                    context.context_mut().protocol_events_mut().extend(events);
                })
                .await?;
        }
        context
            .with_state(driver, move |state, context| {
                let delta = self.destroyed.borrow().clone();
                if let Some(delta) = delta {
                    let events = state.finish_removed_worker_target(delta);
                    context.context_mut().protocol_events_mut().extend(events);
                    self.destroyed.borrow_mut().take();
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, CompletedCdpCommandDispatch,
        OwnedProtocolWaitFailure, ParsedCdpCommand, PendingCdpCommandDispatch,
        ProtocolStateRequestReceiver,
    };
    use futures_util::FutureExt as _;
    use serde_json::Value;

    const TARGET: &str = "TID-shared-retirement";
    const SESSION: &str = "SID-shared-retirement";
    const PEER: &str = "SID-shared-retirement-peer";

    fn fixture() -> CdpConnection {
        let mut conn = CdpConnection::new();
        let mut browser = BrowserContext::new("BID-shared-retirement".into());
        browser.set_active_target_id("TID-owner");
        let mut worker = SharedWorkerTargetState::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(107),
            SharedWorkerInstanceId::from_u64(307),
            TARGET.into(),
            Some("TID-owner".into()),
            String::new(),
            String::new(),
        );
        worker.attach_session(SESSION.into());
        worker.attach_session(PEER.into());
        browser.insert_shared_worker_target(worker);
        conn.install_browser_context_fixture_for_test(browser);
        conn.set_target_discovery_for_owner(
            None,
            crate::conn::CdpTargetFilter::default_target_discovery(),
        );
        let info = conn.devtools_target_info(TARGET).unwrap();
        let _ = conn.prepared_target_host_delta_event_plan(PreparedTargetHostDelta::created(
            TARGET,
            Some(info),
        ));
        conn
    }

    fn start(
        conn: &mut CdpConnection,
        id: u64,
        method: &str,
        params: Value,
        context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        let command = ParsedCdpCommand::parse_str(
            json!({"id":id,"method":method,"params":params}).to_string(),
        )
        .unwrap();
        conn.start_parsed_command_dispatch_with_context(&command, context)
    }

    async fn drive(
        conn: &mut CdpConnection,
        requests: &mut ProtocolStateRequestReceiver,
        pending: PendingCdpCommandDispatch,
    ) -> CompletedCdpCommandDispatch {
        let mut pending = Box::pin(pending.wait());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    completed = &mut pending => return completed,
                    request = requests.recv() => request.unwrap().apply(conn),
                }
            }
        })
        .await
        .expect("SharedWorker close releases its connection while cleanup waits")
    }

    fn finish(
        conn: &mut CdpConnection,
        completed: CompletedCdpCommandDispatch,
        context: &mut CommandDispatchContext,
    ) -> Vec<Value> {
        let CdpCommandTaskStep::Complete(outcome) = conn
            .try_complete_ready_command_dispatch_with_context(completed, context)
            .unwrap_or_else(|_| panic!("SharedWorker close completion is finite"))
        else {
            panic!("all admitted waits have completed");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect()
    }

    #[tokio::test]
    async fn shared_worker_close_keeps_original_cleanup_after_cancellation_and_ready_loss() {
        for phase in 0..4 {
            let mut conn = fixture();
            let identities = [SESSION, PEER].map(|session| {
                conn.shared_worker_target_for_session(Some(session))
                    .unwrap()
                    .protocol_attachment_identity("BID-shared-retirement", session)
                    .unwrap()
            });
            let mut requests = conn.take_protocol_state_requests().unwrap();
            let mut context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) = start(
                &mut conn,
                501,
                "Target.closeTarget",
                json!({"targetId":TARGET}),
                &mut context,
            ) else {
                panic!("SharedWorker close uses the common owned workflow");
            };
            assert!(pending.continues_after_session_detachment());
            assert!(conn.devtools_target_info(TARGET).is_none());
            assert!(conn.session_is_closing(SESSION) && conn.session_is_closing(PEER));
            let mut pending = Box::pin(pending.wait());
            if phase == 1 {
                assert!(pending.as_mut().now_or_never().is_none());
                requests.recv().await.unwrap().apply(&mut conn);
            }
            for id in 510..518 {
                assert!(
                    matches!(
                        start(
                            &mut conn,
                            id,
                            "Browser.getVersion",
                            json!({}),
                            &mut CommandDispatchContext::default()
                        ),
                        CdpCommandTaskStep::Complete(_)
                    ),
                    "independent clients advance before cleanup is driven"
                );
            }
            let completed = if phase >= 2 {
                let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        tokio::select! {
                            completed = &mut pending => break completed,
                            request = requests.recv() => request.unwrap().apply(&mut conn),
                        }
                    }
                })
                .await
                .unwrap();
                if phase == 3 {
                    Some(completed)
                } else {
                    drop(completed);
                    None
                }
            } else {
                drop(pending);
                None
            };
            let completed = match completed {
                Some(completed) => completed,
                None => {
                    let CdpCommandTaskStep::Pending(recovery) = conn
                        .complete_failed_command_wait_with_context(
                            Some(501),
                            None,
                            OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut context,
                        )
                    else {
                        panic!("original in-flight context retains cleanup responsibility");
                    };
                    drive(&mut conn, &mut requests, *recovery).await
                }
            };
            let messages = finish(&mut conn, completed, &mut context);
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == 501)
                    .count(),
                1,
                "{messages:?}"
            );
            assert_eq!(
                messages
                    .iter()
                    .find(|message| message["id"] == 501)
                    .unwrap()["result"]["success"],
                true,
                "{messages:?}"
            );
            let ack = messages
                .iter()
                .position(|message| message["id"] == 501)
                .unwrap();
            assert!(
                messages
                    .iter()
                    .position(|message| message["method"] == "Target.targetDestroyed")
                    .unwrap()
                    < ack,
                "SharedWorker destroy remains before close ACK: {messages:?}"
            );
            for session in [SESSION, PEER] {
                assert!(
                    messages
                        .iter()
                        .position(|message| message["method"] == "Target.detachedFromTarget"
                            && message["params"]["sessionId"] == session)
                        .unwrap()
                        < ack,
                    "SharedWorker detach remains before close ACK: {messages:?}"
                );
                assert!(conn.session_route(Some(session)).is_none());
                assert_eq!(
                    messages
                        .iter()
                        .filter(|message| message["method"] == "Target.detachedFromTarget"
                            && message["params"]["sessionId"] == session)
                        .count(),
                    1,
                    "{messages:?}"
                );
            }
            assert!(identities.iter().all(|identity| !identity.is_current()));
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Target.targetDestroyed"
                        && message["params"]["targetId"] == TARGET)
                    .count(),
                1,
                "{messages:?}"
            );
        }
    }

    #[tokio::test]
    async fn shared_worker_close_recovers_rejected_owner_work_without_losing_accepted_output() {
        let mut conn = fixture();
        let mut requests = conn.take_protocol_state_requests().unwrap();
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = start(
            &mut conn,
            901,
            "Target.closeTarget",
            json!({"targetId":TARGET}),
            &mut context,
        ) else {
            panic!("close admits owned cleanup");
        };
        let mut pending = Box::pin(pending.wait());
        assert!(pending.as_mut().now_or_never().is_none());
        requests.recv().await.unwrap().apply(&mut conn);
        assert!(conn.session_route(Some(SESSION)).is_none());
        assert!(pending.as_mut().now_or_never().is_none());
        drop(requests.recv().await.unwrap());
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), pending)
            .await
            .unwrap();
        let CdpCommandTaskStep::Pending(recovery) = conn
            .try_complete_ready_command_dispatch_with_context(completed, &mut context)
            .unwrap_or_else(|_| panic!("finite failed completion"))
        else {
            panic!("a rejected operation retains cleanup for the other attachment");
        };
        let completed = drive(&mut conn, &mut requests, *recovery).await;
        let messages = finish(&mut conn, completed, &mut context);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == 901)
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .find(|message| message["id"] == 901)
                .unwrap()["error"]["message"],
            "protocol owner closed: channel closed"
        );
        for session in [SESSION, PEER] {
            assert!(conn.session_route(Some(session)).is_none());
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Target.detachedFromTarget"
                        && message["params"]["sessionId"] == session)
                    .count(),
                1,
                "{messages:?}"
            );
        }
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.targetDestroyed")
                .count(),
            1,
            "{messages:?}"
        );
    }

    #[tokio::test]
    async fn shared_worker_close_uses_direct_with_state_while_original_cleanup_waits() {
        let owner = crate::conn::ProtocolStateOwner::new(fixture());
        let handle = owner.handle();
        let detach_command = ParsedCdpCommand::parse_str(
            json!({"id":701,"method":"Target.detachFromTarget","params":{"sessionId":SESSION}})
                .to_string(),
        )
        .unwrap();
        let mut detach_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(detach) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(
                    &detach_command,
                    &mut detach_context,
                )
            })
            .unwrap()
        else {
            panic!("direct detachment has owned waits");
        };
        let close_command = ParsedCdpCommand::parse_str(
            json!({"id":702,"method":"Target.closeTarget","params":{"targetId":TARGET}})
                .to_string(),
        )
        .unwrap();
        let mut close_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(close) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&close_command, &mut close_context)
            })
            .unwrap()
        else {
            panic!("direct SharedWorker close owns its wait");
        };
        let mut close = Box::pin(close.wait());
        assert!(close.as_mut().now_or_never().is_none());
        for id in 710..718 {
            let command = ParsedCdpCommand::parse_str(
                json!({"id":id,"method":"Browser.getVersion"}).to_string(),
            )
            .unwrap();
            let mut context = CommandDispatchContext::default();
            assert!(matches!(
                handle
                    .with_state(|state| state
                        .start_parsed_command_dispatch_with_context(&command, &mut context))
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
        }
        let detached = tokio::time::timeout(std::time::Duration::from_secs(10), detach.wait())
            .await
            .unwrap();
        assert!(matches!(
            handle
                .with_state(|state| state
                    .try_complete_ready_command_dispatch_with_context(detached, &mut detach_context)
                    .unwrap_or_else(|_| panic!("finite direct completion")))
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), close)
            .await
            .unwrap();
        let CdpCommandTaskStep::Complete(closed) = handle
            .with_state(|state| {
                state
                    .try_complete_ready_command_dispatch_with_context(completed, &mut close_context)
                    .unwrap_or_else(|_| panic!("finite direct completion"))
            })
            .unwrap()
        else {
            panic!("the final close does not require a mailbox pump");
        };
        let (events, _, _) = closed.into_protocol_event_parts();
        assert_eq!(
            events
                .iter()
                .map(|event| event.clone().into_protocol_message())
                .find(|message| message["id"] == 702)
                .unwrap()["result"]["success"],
            true
        );
    }

    #[tokio::test]
    async fn shared_worker_close_attempts_every_original_cleanup_after_preparation_failure() {
        let mut conn = fixture();
        let identities = [SESSION, PEER].map(|session| {
            conn.shared_worker_target_for_session(Some(session))
                .unwrap()
                .protocol_attachment_identity("BID-shared-retirement", session)
                .unwrap()
        });
        let _ = conn.rollback_attached_session_without_event(PEER);
        let mut requests = conn.take_protocol_state_requests().unwrap();
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = start(
            &mut conn,
            801,
            "Target.closeTarget",
            json!({"targetId":TARGET}),
            &mut context,
        ) else {
            panic!("preparation failure retains all other cleanup responsibilities");
        };
        let completed = drive(&mut conn, &mut requests, *pending).await;
        let mut step = conn
            .try_complete_ready_command_dispatch_with_context(completed, &mut context)
            .unwrap_or_else(|_| panic!("finite original completion"));
        let messages = loop {
            match step {
                CdpCommandTaskStep::Pending(pending) => {
                    let completed = drive(&mut conn, &mut requests, *pending).await;
                    step = conn
                        .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                        .unwrap_or_else(|_| panic!("finite recovery completion"));
                }
                CdpCommandTaskStep::Complete(outcome) => {
                    let (events, _, _) = outcome.into_protocol_event_parts();
                    break events
                        .into_iter()
                        .map(BackgroundProtocolEvent::into_protocol_message)
                        .collect::<Vec<_>>();
                }
            }
        };
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == 801)
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .find(|message| message["id"] == 801)
                .unwrap()["error"]["message"],
            "InvalidSessionId"
        );
        assert!(conn.session_route(Some(SESSION)).is_none());
        assert!(identities.iter().all(|identity| !identity.is_current()));
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget"
                    && message["params"]["sessionId"] == SESSION)
                .count(),
            1,
            "{messages:?}"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.targetDestroyed")
                .count(),
            1,
            "{messages:?}"
        );
    }

    #[tokio::test]
    async fn shared_worker_close_joins_original_detachment_without_retiring_a_rebound_session() {
        for rebind in [false, true] {
            let mut conn = fixture();
            let original = conn.session_binding_lifetime(SESSION).unwrap();
            let original_route = conn.session_route(Some(SESSION)).unwrap();
            let mut requests = conn.take_protocol_state_requests().unwrap();
            let mut detach_context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(detach) = start(
                &mut conn,
                601,
                "Target.detachFromTarget",
                json!({"sessionId":SESSION}),
                &mut detach_context,
            ) else {
                panic!("original detachment owns its handlers");
            };
            let mut close_context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(close) = start(
                &mut conn,
                602,
                "Target.closeTarget",
                json!({"targetId":TARGET}),
                &mut close_context,
            ) else {
                panic!("close joins the admitted cleanup");
            };
            let mut close = Box::pin(close.wait());
            assert!(close.as_mut().now_or_never().is_none());
            assert!(
                requests.try_recv().is_err(),
                "joining does not start another cleanup"
            );
            let completed = drive(&mut conn, &mut requests, *detach).await;
            let detached = finish(&mut conn, completed, &mut detach_context);
            assert_eq!(
                detached
                    .iter()
                    .filter(|message| message["id"] == 601)
                    .count(),
                1
            );
            assert!(original.is_closed());
            let successor = if rebind {
                let route = original_route;
                conn.register_session_route_for_test(SESSION, route.clone());
                Some((route, conn.session_binding_lifetime(SESSION).unwrap()))
            } else {
                None
            };
            let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        completed = &mut close => break completed,
                        request = requests.recv() => request.unwrap().apply(&mut conn),
                    }
                }
            })
            .await
            .unwrap();
            let closed = finish(&mut conn, completed, &mut close_context);
            assert_eq!(
                closed
                    .iter()
                    .filter(|message| message["method"] == "Target.detachedFromTarget"
                        && message["params"]["sessionId"] == SESSION)
                    .count(),
                0,
                "original detachment already published its event: {closed:?}"
            );
            assert_eq!(
                closed.iter().find(|message| message["id"] == 602).unwrap()["result"]["success"],
                true,
                "{closed:?}"
            );
            if let Some((route, lifetime)) = successor {
                assert_eq!(conn.session_route(Some(SESSION)), Some(route));
                assert!(!lifetime.is_closed());
                assert!(!conn.session_is_closing(SESSION));
            }
        }
    }
}
