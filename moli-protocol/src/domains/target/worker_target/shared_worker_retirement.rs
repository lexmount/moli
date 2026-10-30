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
pub(crate) struct RemovedSharedWorkerAttachment {
    retirement: TargetSharedWorkerProtocolAttachmentRetirement,
    cleanup: Rc<RefCell<Option<PreparedRemovedWorkerSession>>>,
    captured: Option<TargetSessionDetachCleanupPlan>,
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
                        let (cleanup, captured) = if conn
                            .session_route(Some(cleanup_plan.session_id()))
                            .is_none()
                        {
                            // The frozen attach may still be held by its original
                            // Runtime output barrier. Its native scope, rather
                            // than a later literal SID, identifies any late binding.
                            (None, Some(cleanup_plan))
                        } else {
                            match PreparedRemovedWorkerSession::prepare(conn, cleanup_plan) {
                                Ok(cleanup) => (Some(cleanup), None),
                                Err(error) => {
                                    prepared.preparation_error.get_or_insert(error);
                                    (None, None)
                                }
                            }
                        };
                        prepared.sessions.push(RemovedSharedWorkerAttachment {
                            retirement,
                            cleanup: Rc::new(RefCell::new(cleanup)),
                            captured,
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

    pub(in crate::domains::target) fn into_command_workflow(
        self,
        access: crate::conn::ProtocolStateHandle,
    ) -> PreparedCommandWorkflow {
        let completion = SharedWorkerRetirementCompletion {
            sessions: self.sessions,
            destroyed: Rc::new(RefCell::new(self.destroyed)),
        };
        let recovery = completion.clone();
        let recovery_access = access.clone();
        PreparedCommandWorkflow::new(move |mut context| async move {
            let result = completion
                .run(&mut ProtocolWorkflowDriver::new(access), &mut context)
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
                        &mut ProtocolWorkflowDriver::new(recovery_access),
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

impl RemovedSharedWorkerAttachment {
    pub(crate) fn prepare_captured(&self, conn: &mut CdpConnection) -> Result<()> {
        if self.cleanup.borrow().is_some() || !self.retirement.is_current() {
            return Ok(());
        }
        let Some(plan) = self.captured.as_ref() else {
            return Ok(());
        };
        let original = crate::conn::TargetWorkerProtocolAttachmentIdentity::SharedOrDedicated(
            self.retirement.identity().clone(),
        );
        if conn.committed_worker_protocol_attachment_identity(plan.session_id()) == Some(&original)
        {
            let cleanup = PreparedRemovedWorkerSession::prepare(conn, plan.clone())?;
            // Register the owned native wait in the original recovery receipt
            // before this finite operation can lose its reply.
            *self.cleanup.borrow_mut() = Some(cleanup);
        } else {
            // No original binding remains. Retire the native authority in the
            // same turn; a held attach cannot create a new phantom afterward.
            self.retirement.clone().retire();
        }
        Ok(())
    }
}

impl SharedWorkerRetirementCompletion {
    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        for session in self.sessions {
            if session.captured.is_some() {
                let preparation = session.clone();
                context.with_state(driver, move |state, _| {
                    state.prepare_removed_shared_worker_attachment(preparation)
                })??;
            }
            let cleanup = session.cleanup.borrow().clone();
            if let Some(cleanup) = &cleanup {
                cleanup.wait().await;
            }
            context.with_state(driver, move |state, context| {
                let events =
                    state.finish_removed_shared_worker_attachment(cleanup, session.retirement);
                // SharedWorker removal has always exposed its complete
                // retirement prefix before the close acknowledgement.
                context.context_mut().protocol_events_mut().extend(events);
            })?;
        }
        context.with_state(driver, move |state, context| {
            let delta = self.destroyed.borrow().clone();
            if let Some(delta) = delta {
                let events = state.finish_removed_worker_target(delta);
                context.context_mut().protocol_events_mut().extend(events);
                self.destroyed.borrow_mut().take();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, CompletedCdpCommandDispatch,
        OwnedProtocolWaitFailure, ParsedCdpCommand, PendingCdpCommandDispatch,
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

    fn capture_peer_attach(conn: &CdpConnection) -> PreparedTargetAttach {
        let route = CdpSessionRoute::SharedWorkerTarget {
            browser_context_id: "BID-shared-retirement".into(),
            target_id: TARGET.into(),
        };
        let attachment = conn
            .worker_protocol_attachment_identity_for_route(&route, PEER)
            .unwrap();
        PreparedTargetAttach::new(
            TARGET,
            conn.devtools_target_info(TARGET).unwrap(),
            [TargetAttachSessionCommit::auto_attached(
                PEER.to_owned(),
                None,
                route,
                false,
            )],
        )
        .with_worker_attachment(attachment)
    }

    async fn drive(
        ctx: &mut crate::testing::TestContext,
        pending: PendingCdpCommandDispatch,
    ) -> CompletedCdpCommandDispatch {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            ctx.drive_owned_wait(pending.wait()),
        )
        .await
        .expect("owned worker wait remains bounded")
    }

    fn finish(
        conn: &mut CdpConnection,
        completed: CompletedCdpCommandDispatch,
        context: &mut CommandDispatchContext,
    ) -> Vec<Value> {
        let CdpCommandTaskStep::Complete(outcome) =
            conn.complete_command_dispatch_with_context(completed, context)
        else {
            panic!("all admitted waits have completed");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        events
            .into_iter()
            .map(|event| {
                event
                    .into_protocol_message()
                    .into_json()
                    .expect("scalar protocol fixture")
            })
            .collect()
    }

    #[tokio::test]
    async fn shared_worker_close_keeps_original_cleanup_after_cancellation_and_ready_loss() {
        for phase in 0..4 {
            let conn = fixture();
            let identities = [SESSION, PEER].map(|session| {
                conn.shared_worker_target_for_session(Some(session))
                    .unwrap()
                    .protocol_attachment_identity("BID-shared-retirement", session)
                    .unwrap()
            });
            let mut ctx = crate::testing::TestContext::from_conn(conn);
            let mut context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) = ctx.with_fixture(|conn| {
                start(
                    conn,
                    501,
                    "Target.closeTarget",
                    json!({"targetId":TARGET}),
                    &mut context,
                )
            }) else {
                panic!("SharedWorker close uses the common owned workflow");
            };
            assert!(pending.continues_after_session_detachment());
            ctx.with_fixture(|conn| assert!(conn.devtools_target_info(TARGET).is_none()));
            ctx.with_fixture(|conn| {
                assert!(conn.session_is_closing(SESSION) && conn.session_is_closing(PEER))
            });
            let mut pending = Box::pin(pending.wait());
            if phase == 1 {
                drop(pending.as_mut().now_or_never());
            }
            for id in 510..518 {
                ctx.with_fixture(|conn| {
                    assert!(
                        matches!(
                            start(
                                conn,
                                id,
                                "Browser.getVersion",
                                json!({}),
                                &mut CommandDispatchContext::default()
                            ),
                            CdpCommandTaskStep::Complete(_)
                        ),
                        "independent clients advance before cleanup is driven"
                    )
                });
            }
            let completed = if phase >= 2 {
                let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    (&mut pending).await
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
                    let CdpCommandTaskStep::Pending(recovery) = ctx.with_fixture(|conn| {
                        conn.complete_failed_command_wait_with_context(
                            Some(501),
                            None,
                            OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut context,
                        )
                    }) else {
                        panic!("original in-flight context retains cleanup responsibility");
                    };
                    drive(&mut ctx, *recovery).await
                }
            };
            let messages = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
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
                ctx.with_fixture(|conn| assert!(conn.session_route(Some(session)).is_none()));
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
    async fn shared_worker_close_retires_an_original_attach_committed_after_admission() {
        let mut conn = fixture();
        let attachment = conn
            .shared_worker_target_for_session(Some(PEER))
            .unwrap()
            .protocol_attachment_identity("BID-shared-retirement", PEER)
            .unwrap();
        let info = conn.devtools_target_info(TARGET).unwrap();
        let prepared_attach = PreparedTargetAttach::new(
            TARGET,
            info,
            [TargetAttachSessionCommit::auto_attached(
                PEER.to_owned(),
                None,
                CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: "BID-shared-retirement".into(),
                    target_id: TARGET.into(),
                },
                false,
            )],
        );
        let prepared_attach = prepared_attach.with_worker_attachment(
            crate::conn::TargetWorkerProtocolAttachmentIdentity::SharedOrDedicated(
                attachment.clone(),
            ),
        );
        conn.rollback_attached_session_without_event(PEER);
        assert!(conn.session_route(Some(PEER)).is_none());
        assert!(
            attachment.is_current(),
            "capture still owns its original target attachment"
        );
        let mut ctx = crate::testing::TestContext::from_conn(conn);
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = ctx.with_fixture(|conn| {
            start(
                conn,
                1001,
                "Target.closeTarget",
                json!({"targetId":TARGET}),
                &mut context,
            )
        }) else {
            panic!("close owns its cleanup outside the actor");
        };
        // The real held attach can be published while native cleanup waits.
        // Its retirement scope is still current, even though the target has
        // already been removed from the live inventory.
        assert!(attachment.is_current());
        let attached = ctx.with_fixture(|conn| {
            conn.commit_prepared_attach_event_plan(prepared_attach)
                .into_background_events()
        });
        assert!(attached.iter().any(|event| {
            event
                .clone()
                .into_protocol_message()
                .into_json()
                .expect("scalar protocol fixture")["method"]
                == "Target.attachedToTarget"
        }));
        let original_binding =
            ctx.with_fixture(|conn| conn.session_binding_lifetime(PEER).unwrap());
        let completed = drive(&mut ctx, *pending).await;
        let mut step = ctx.with_fixture(|conn| {
            conn.complete_command_dispatch_with_context(completed, &mut context)
        });
        let messages = loop {
            match step {
                CdpCommandTaskStep::Pending(pending) => {
                    let completed = drive(&mut ctx, *pending).await;
                    step = ctx.with_fixture(|conn| {
                        conn.complete_command_dispatch_with_context(completed, &mut context)
                    });
                }
                CdpCommandTaskStep::Complete(outcome) => {
                    let (events, _, _) = outcome.into_protocol_event_parts();
                    break events
                        .into_iter()
                        .map(|event| {
                            event
                                .into_protocol_message()
                                .into_json()
                                .expect("scalar protocol fixture")
                        })
                        .collect::<Vec<_>>();
                }
            }
        };
        ctx.with_fixture(|conn| {
            assert!(
                conn.session_route(Some(PEER)).is_none(),
                "the original late attachment must not survive target retirement"
            )
        });
        assert!(original_binding.is_closed());
        assert!(!attachment.is_current());
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget"
                    && message["params"]["sessionId"] == PEER)
                .count(),
            1,
            "{messages:?}"
        );
        assert_eq!(
            messages
                .iter()
                .find(|message| message["id"] == 1001)
                .unwrap()["result"]["success"],
            true,
            "a captured attach is not an invalid-session close failure: {messages:?}"
        );
    }

    #[tokio::test]
    async fn shared_worker_close_retires_captured_authority_without_closing_its_successor() {
        for rebind in [false, true] {
            let mut conn = fixture();
            let prepared = capture_peer_attach(&conn);
            let original = conn
                .shared_worker_target_for_session(Some(PEER))
                .unwrap()
                .protocol_attachment_identity("BID-shared-retirement", PEER)
                .unwrap();
            conn.rollback_attached_session_without_event(PEER);
            let mut ctx = crate::testing::TestContext::from_conn(conn);
            let mut context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(close) = ctx.with_fixture(|conn| {
                start(
                    conn,
                    1101,
                    "Target.closeTarget",
                    json!({"targetId":TARGET}),
                    &mut context,
                )
            }) else {
                panic!("retirement owns the captured attachment");
            };
            let successor = if rebind {
                // Reuse all literal routing ids, but create a new native scope.
                let mut worker = SharedWorkerTargetState::new(
                    moli_core::RendererOwnerLocalHostId::new_for_testing(107),
                    SharedWorkerInstanceId::from_u64(307),
                    TARGET.into(),
                    Some("TID-owner".into()),
                    String::new(),
                    String::new(),
                );
                worker.attach_session(PEER.into());
                ctx.with_fixture(|conn| {
                    conn.browser_context_by_id_mut("BID-shared-retirement")
                        .unwrap()
                        .insert_shared_worker_target(worker)
                });
                let prepared = ctx.with_fixture(|conn| capture_peer_attach(conn));
                let _ = ctx.with_fixture(|conn| conn.commit_prepared_attach_event_plan(prepared));
                Some((
                    ctx.with_fixture(|conn| {
                        conn.committed_worker_protocol_attachment_identity(PEER)
                            .unwrap()
                            .clone()
                    }),
                    ctx.with_fixture(|conn| conn.session_binding_lifetime(PEER).unwrap()),
                ))
            } else {
                None
            };
            let completed = drive(&mut ctx, *close).await;
            let messages = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
            assert_eq!(
                messages
                    .iter()
                    .find(|message| message["id"] == 1101)
                    .unwrap()["result"]["success"],
                true,
                "{messages:?}"
            );
            assert!(!original.is_current());
            ctx.with_fixture(|conn| {
                assert!(
                    conn.commit_prepared_attach_event_plan(prepared)
                        .into_background_events()
                        .is_empty(),
                    "an expired captured attach cannot create a phantom or overwrite a successor"
                )
            });
            if let Some((attachment, lifetime)) = successor {
                assert!(attachment.is_current());
                ctx.with_fixture(|conn| {
                    assert!(!lifetime.is_closed() && !conn.session_is_closing(PEER))
                });
                ctx.with_fixture(|conn| {
                    assert_eq!(
                        conn.committed_worker_protocol_attachment_identity(PEER),
                        Some(&attachment)
                    )
                });
            } else {
                ctx.with_fixture(|conn| assert!(conn.session_route(Some(PEER)).is_none()));
            }
        }
    }

    #[tokio::test]
    async fn shared_worker_late_attach_cleanup_retains_its_result_after_ready_loss() {
        let mut conn = fixture();
        let prepared = capture_peer_attach(&conn);
        conn.rollback_attached_session_without_event(PEER);
        let mut ctx = crate::testing::TestContext::from_conn(conn);
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(close) = ctx.with_fixture(|conn| {
            start(
                conn,
                1201,
                "Target.closeTarget",
                json!({"targetId":TARGET}),
                &mut context,
            )
        }) else {
            panic!("close owns late attachment cleanup");
        };
        let _ = ctx.with_fixture(|conn| conn.commit_prepared_attach_event_plan(prepared));
        let binding = ctx.with_fixture(|conn| conn.session_binding_lifetime(PEER).unwrap());
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), close.wait())
            .await
            .unwrap();
        drop(completed);
        let CdpCommandTaskStep::Pending(recovery) = ctx.with_fixture(|conn| {
            conn.complete_failed_command_wait_with_context(
                Some(1201),
                None,
                OwnedProtocolWaitFailure::Cancelled.message(),
                &mut context,
            )
        }) else {
            panic!("the original context retains late-attachment cleanup and its accepted result");
        };
        let completed = drive(&mut ctx, *recovery).await;
        let messages = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
        assert!(binding.is_closed());
        ctx.with_fixture(|conn| assert!(conn.session_route(Some(PEER)).is_none()));
        assert_eq!(
            messages
                .iter()
                .find(|message| message["id"] == 1201)
                .unwrap()["result"]["success"],
            true
        );
        for session in [SESSION, PEER] {
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
                    .complete_command_dispatch_with_context(detached, &mut detach_context))
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), close)
            .await
            .unwrap();
        let CdpCommandTaskStep::Complete(closed) = handle
            .with_state(|state| {
                state.complete_command_dispatch_with_context(completed, &mut close_context)
            })
            .unwrap()
        else {
            panic!("the final close does not require a mailbox pump");
        };
        let (events, _, _) = closed.into_protocol_event_parts();
        assert_eq!(
            events
                .iter()
                .map(|event| event
                    .clone()
                    .into_protocol_message()
                    .into_json()
                    .expect("scalar protocol fixture"))
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
        conn.register_session_route_for_test(PEER, CdpSessionRoute::Browser);
        let successor = conn.session_binding_lifetime(PEER).unwrap();
        let mut ctx = crate::testing::TestContext::from_conn(conn);
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = ctx.with_fixture(|conn| {
            start(
                conn,
                801,
                "Target.closeTarget",
                json!({"targetId":TARGET}),
                &mut context,
            )
        }) else {
            panic!("preparation failure retains all other cleanup responsibilities");
        };
        let completed = drive(&mut ctx, *pending).await;
        let mut step = ctx.with_fixture(|conn| {
            conn.complete_command_dispatch_with_context(completed, &mut context)
        });
        let messages = loop {
            match step {
                CdpCommandTaskStep::Pending(pending) => {
                    let completed = drive(&mut ctx, *pending).await;
                    step = ctx.with_fixture(|conn| {
                        conn.complete_command_dispatch_with_context(completed, &mut context)
                    });
                }
                CdpCommandTaskStep::Complete(outcome) => {
                    let (events, _, _) = outcome.into_protocol_event_parts();
                    break events
                        .into_iter()
                        .map(|event| {
                            event
                                .into_protocol_message()
                                .into_json()
                                .expect("scalar protocol fixture")
                        })
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
            "UnknownTargetId"
        );
        ctx.with_fixture(|conn| assert!(conn.session_route(Some(SESSION)).is_none()));
        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.session_route(Some(PEER)),
                Some(CdpSessionRoute::Browser)
            )
        });
        ctx.with_fixture(|conn| assert!(!successor.is_closed() && !conn.session_is_closing(PEER)));
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
            let conn = fixture();
            let original = conn.session_binding_lifetime(SESSION).unwrap();
            let original_route = conn.session_route(Some(SESSION)).unwrap();
            let mut ctx = crate::testing::TestContext::from_conn(conn);
            let mut detach_context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(detach) = ctx.with_fixture(|conn| {
                start(
                    conn,
                    601,
                    "Target.detachFromTarget",
                    json!({"sessionId":SESSION}),
                    &mut detach_context,
                )
            }) else {
                panic!("original detachment owns its handlers");
            };
            let mut close_context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(close) = ctx.with_fixture(|conn| {
                start(
                    conn,
                    602,
                    "Target.closeTarget",
                    json!({"targetId":TARGET}),
                    &mut close_context,
                )
            }) else {
                panic!("close joins the admitted cleanup");
            };
            let mut close = Box::pin(close.wait());
            assert!(close.as_mut().now_or_never().is_none());
            let completed = drive(&mut ctx, *detach).await;
            let detached = ctx.with_fixture(|conn| finish(conn, completed, &mut detach_context));
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
                ctx.with_fixture(|conn| {
                    conn.register_session_route_for_test(SESSION, route.clone())
                });
                Some((
                    route,
                    ctx.with_fixture(|conn| conn.session_binding_lifetime(SESSION).unwrap()),
                ))
            } else {
                None
            };
            let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                (&mut close).await
            })
            .await
            .unwrap();
            let closed = ctx.with_fixture(|conn| finish(conn, completed, &mut close_context));
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
                ctx.with_fixture(|conn| assert_eq!(conn.session_route(Some(SESSION)), Some(route)));
                assert!(!lifetime.is_closed());
                ctx.with_fixture(|conn| assert!(!conn.session_is_closing(SESSION)));
            }
        }
    }
}
