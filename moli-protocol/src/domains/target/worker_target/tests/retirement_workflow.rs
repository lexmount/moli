use super::*;
use crate::conn::{
    CdpCommandTaskStep, CommandDispatchContext, CompletedCdpCommandDispatch, ParsedCdpCommand,
    PendingCdpCommandDispatch,
};
use futures_util::FutureExt as _;

fn install_attached_worker(
    conn: &mut CdpConnection,
    owner_page: TargetPageResidenceIdentity,
    host: moli_core::RendererOwnerLocalHostId,
) {
    conn.browser_context_by_id_mut("BID-1")
        .unwrap()
        .insert_dedicated_worker_target(DedicatedWorkerTargetState::new(
            owner_page,
            host,
            101,
            "TID-retiring-worker".into(),
            String::new(),
            Vec::new(),
        ));
    let route = conn
        .prepare_auto_attached_dedicated_worker_session_binding(
            "TID-retiring-worker",
            "SID-retiring-worker".into(),
        )
        .unwrap();
    conn.register_session_route_for_test("SID-retiring-worker", route);
}

fn discover_worker(conn: &mut CdpConnection) {
    conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
    let snapshot = conn.devtools_target_info("TID-retiring-worker").unwrap();
    let events = conn
        .prepared_target_host_delta_event_plan(PreparedTargetHostDelta::created(
            "TID-retiring-worker",
            Some(snapshot),
        ))
        .into_background_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .clone()
            .into_protocol_message()
            .into_json()
            .expect("scalar protocol fixture")["method"],
        "Target.targetCreated"
    );
}

fn start(
    conn: &mut CdpConnection,
    id: u64,
    method: &str,
    params: Value,
    context: &mut CommandDispatchContext,
) -> CdpCommandTaskStep {
    let command = ParsedCdpCommand::parse_str(
        json!({"id": id, "method": method, "params": params}).to_string(),
    )
    .unwrap();
    conn.start_parsed_command_dispatch_with_context(&command, context)
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
        panic!("worker completion must be finite")
    };
    let (events, _, _) = outcome.into_protocol_event_parts();
    protocol_messages(&events)
}

#[tokio::test]
async fn worker_close_joins_original_detachment_and_recovers_its_admitted_workflow() {
    for cancel_phase in 0..3 {
        let (mut conn, owner_page, renderer_page) = dedicated_worker_fixture();
        install_attached_worker(&mut conn, owner_page, renderer_page.owner_local_host_id());
        discover_worker(&mut conn);
        let mut ctx = crate::testing::TestContext::from_conn(conn);
        let mut detachment_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(detachment) = ctx.with_fixture(|conn| {
            start(
                conn,
                201,
                "Target.detachFromTarget",
                json!({"sessionId":"SID-retiring-worker"}),
                &mut detachment_context,
            )
        }) else {
            panic!("the original detachment owns the handlers")
        };
        let mut close_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(closing) = ctx.with_fixture(|conn| {
            start(
                conn,
                202,
                "Target.closeTarget",
                json!({"targetId":"TID-retiring-worker"}),
                &mut close_context,
            )
        }) else {
            panic!("worker close is an owned workflow")
        };
        assert!(closing.continues_after_session_detachment());
        let mut close_wait = Box::pin(closing.wait());
        if cancel_phase != 0 {
            assert!(close_wait.as_mut().now_or_never().is_none());
        }
        let peer = ctx.with_fixture(|conn| {
            start(
                conn,
                203,
                "Browser.getVersion",
                json!({}),
                &mut CommandDispatchContext::default(),
            )
        });
        assert!(
            matches!(peer, CdpCommandTaskStep::Complete(_)),
            "an independent command advances while the worker is closing"
        );
        ctx.with_fixture(|conn| {
            assert!(
                conn.prepare_auto_attached_dedicated_worker_session_binding(
                    "TID-retiring-worker",
                    "SID-must-not-attach".into()
                )
                .is_none()
            )
        });
        let duplicate = ctx.with_fixture(|conn| {
            start(
                conn,
                204,
                "Target.closeTarget",
                json!({"targetId":"TID-retiring-worker"}),
                &mut CommandDispatchContext::default(),
            )
        });
        let CdpCommandTaskStep::Complete(duplicate) = duplicate else {
            panic!("duplicate closing is rejected at admission")
        };
        let (events, _, _) = duplicate.into_protocol_event_parts();
        assert_eq!(
            events[0]
                .clone()
                .into_protocol_message()
                .into_json()
                .expect("scalar protocol fixture")["error"]["message"],
            "Target is closing"
        );
        let completed = drive(&mut ctx, *detachment).await;
        let detached = ctx.with_fixture(|conn| finish(conn, completed, &mut detachment_context));
        assert!(
            detached
                .iter()
                .any(|message| message["id"] == 201 && message.get("result").is_some()),
            "original detachment must finish: {detached:?}"
        );
        assert_eq!(
            detached
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget")
                .count(),
            1
        );
        if cancel_phase == 2 {
            // Receive the shared workflow's terminal and discard it before
            // actor completion. Recovery must keep its actual success/events.
            let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                (&mut close_wait).await
            })
            .await
            .unwrap();
            drop(completed);
        } else {
            drop(close_wait);
        }
        let CdpCommandTaskStep::Pending(recovery) = ctx.with_fixture(|conn| {
            conn.complete_failed_command_wait_with_context(
                Some(202),
                None,
                "controlled worker cancellation",
                &mut close_context,
            )
        }) else {
            panic!("the original in-flight entry resumes retirement")
        };
        let completed = drive(&mut ctx, *recovery).await;
        let closed = ctx.with_fixture(|conn| finish(conn, completed, &mut close_context));
        assert_eq!(
            closed.iter().filter(|message| message["id"] == 202).count(),
            1
        );
        assert_eq!(
            closed[0]["result"]["success"], true,
            "ready loss cannot replace completed retirement: {closed:?}"
        );
        assert_eq!(
            closed
                .iter()
                .filter(|message| message["method"] == "Target.targetDestroyed")
                .count(),
            1,
            "retirement must publish its frozen destruction: {closed:?}"
        );
        assert!(
            closed
                .iter()
                .all(|message| message["method"] != "Target.detachedFromTarget"),
            "the joined original detachment already published this event"
        );
        ctx.with_fixture(|conn| assert!(conn.session_route(Some("SID-retiring-worker")).is_none()));
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context_by_id("BID-1")
                    .unwrap()
                    .dedicated_worker_target("TID-retiring-worker")
                    .is_none()
            )
        });
    }
}

#[tokio::test]
async fn old_worker_retirement_cannot_remove_reused_target_instance_and_binding() {
    let (mut conn, owner_page, renderer_page) = dedicated_worker_fixture();
    install_attached_worker(
        &mut conn,
        owner_page.clone(),
        renderer_page.owner_local_host_id(),
    );
    discover_worker(&mut conn);
    let route = conn.session_route(Some("SID-retiring-worker")).unwrap();
    let plan =
        crate::conn::SessionDisposalPlan::for_session_route("SID-retiring-worker", &route).unwrap();
    conn.mark_session_closing(&plan);
    let ctx = crate::testing::TestContext::from_conn(conn);
    let mut context = CommandDispatchContext::default();
    let CdpCommandTaskStep::Pending(pending) = ctx.with_fixture(|conn| {
        start(
            conn,
            205,
            "Target.closeTarget",
            json!({"targetId":"TID-retiring-worker"}),
            &mut context,
        )
    }) else {
        panic!("worker close joins the original binding")
    };
    let mut waiting = Box::pin(pending.wait());
    assert!(waiting.as_mut().now_or_never().is_none());
    ctx.with_fixture(|conn| {
        conn.browser_context_by_id_mut("BID-1")
            .unwrap()
            .remove_dedicated_worker_target_by_renderer_instance(101)
            .unwrap()
    });
    ctx.with_fixture(|conn| {
        install_attached_worker(
            conn,
            owner_page,
            moli_core::RendererOwnerLocalHostId::new_for_testing(119),
        )
    });
    let successor = ctx.with_fixture(|conn| {
        conn.session_binding_lifetime("SID-retiring-worker")
            .unwrap()
    });
    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (&mut waiting).await
    })
    .await
    .unwrap();
    let output = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
    assert_eq!(output[0]["result"]["success"], true);
    assert!(
        output
            .iter()
            .all(|message| message["method"] != "Target.targetDestroyed"
                && message["method"] != "Target.detachedFromTarget"),
        "old cleanup must not publish retirement of the successor: {output:?}"
    );
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.session_binding_lifetime("SID-retiring-worker"),
            Some(successor)
        )
    });
    ctx.with_fixture(|conn| assert!(!conn.session_is_closing("SID-retiring-worker")));
    ctx.with_fixture(|conn| {
        let worker = {
            conn.browser_context_by_id("BID-1")
                .unwrap()
                .dedicated_worker_target("TID-retiring-worker")
                .unwrap()
        };
        assert!(!worker.is_retiring());
        assert!(worker.is_session("SID-retiring-worker"));
        assert_eq!(
            worker.renderer_owner_local_host_id,
            moli_core::RendererOwnerLocalHostId::new_for_testing(119)
        );
    });
}

#[tokio::test]
async fn worker_close_finishes_all_original_sessions_after_completed_workflow_is_discarded() {
    let (mut conn, owner_page, renderer_page) = dedicated_worker_fixture();
    install_attached_worker(&mut conn, owner_page, renderer_page.owner_local_host_id());
    let route = conn
        .prepare_auto_attached_dedicated_worker_session_binding(
            "TID-retiring-worker",
            "SID-retiring-peer".into(),
        )
        .unwrap();
    conn.register_session_route_for_test("SID-retiring-peer", route);
    discover_worker(&mut conn);
    let mut ctx = crate::testing::TestContext::from_conn(conn);
    let mut context = CommandDispatchContext::default();
    let CdpCommandTaskStep::Pending(pending) = ctx.with_fixture(|conn| {
        start(
            conn,
            206,
            "Target.closeTarget",
            json!({"targetId":"TID-retiring-worker"}),
            &mut context,
        )
    }) else {
        panic!("worker close owns its cleanup")
    };
    for session in ["SID-retiring-worker", "SID-retiring-peer"] {
        ctx.with_fixture(|conn| assert!(conn.session_is_closing(session)));
    }
    let mut waiting = Box::pin(pending.wait());
    // Lose the real completed workflow before its original context adopts it.
    drop(
        tokio::time::timeout(std::time::Duration::from_secs(10), &mut waiting)
            .await
            .unwrap(),
    );
    drop(waiting);
    ctx.with_fixture(|conn| {
        assert!(matches!(
            start(
                conn,
                207,
                "Browser.getVersion",
                json!({}),
                &mut CommandDispatchContext::default()
            ),
            CdpCommandTaskStep::Complete(_)
        ))
    });
    let CdpCommandTaskStep::Pending(pending) = ctx.with_fixture(|conn| {
        conn.complete_failed_command_wait_with_context(
            Some(206),
            None,
            "controlled ready state cancellation",
            &mut context,
        )
    }) else {
        panic!("cleanup recovery remains owned")
    };
    let completed = drive(&mut ctx, *pending).await;
    let messages = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
    assert_eq!(messages[0]["id"], 206);
    assert_eq!(
        messages[0]["result"]["success"], true,
        "cleanup must finish: {messages:?}"
    );
    assert_eq!(
        messages
            .iter()
            .filter_map(|message| message["method"].as_str())
            .collect::<Vec<_>>(),
        [
            "Target.targetInfoChanged",
            "Target.detachedFromTarget",
            "Target.detachedFromTarget",
            "Target.targetDestroyed",
        ]
    );
    for session in ["SID-retiring-worker", "SID-retiring-peer"] {
        ctx.with_fixture(|conn| assert!(conn.session_route(Some(session)).is_none()));
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget"
                    && message["params"]["sessionId"] == session)
                .count(),
            1
        );
    }
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context_by_id("BID-1")
                .unwrap()
                .dedicated_worker_target("TID-retiring-worker")
                .is_none()
        )
    });
}

#[tokio::test]
async fn worker_close_settles_original_calls_at_admission_before_cleanup_is_polled() {
    for cancel_before_poll in [false, true] {
        let (mut conn, owner_page, renderer_page) = dedicated_worker_fixture();
        install_attached_worker(&mut conn, owner_page, renderer_page.owner_local_host_id());
        let target = conn
            .shared_worker_target_for_session_mut(Some("SID-retiring-worker"))
            .unwrap();
        target.register_pending_inspector_await(
            "SID-retiring-worker",
            301,
            Some("SID-retiring-worker"),
            None,
        );
        let descriptor = crate::conn::RendererCommandDescriptor::from_synthesized_payload(
            json!({"id":302,"method":"Console.clearMessages","params":{}}).to_string(),
        )
        .unwrap();
        let registered = target
            .try_register_renderer_call(
                "SID-retiring-worker",
                302,
                Some(moli_page_types::RendererAgentAttachmentId::allocate()),
                descriptor,
            )
            .unwrap()
            .unwrap();
        let (correlation, native, response) = registered.into_parts();
        let response = response.unwrap();
        let mut ctx = crate::testing::TestContext::from_conn(conn);
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(mut pending) = ctx.with_fixture(|conn| {
            start(
                conn,
                303,
                "Target.closeTarget",
                json!({"targetId":"TID-retiring-worker"}),
                &mut context,
            )
        }) else {
            panic!("worker close remains outside state")
        };
        ctx.with_fixture(|conn| {
            assert!(
                !conn.has_pending_inspector_awaits_for_session_owner(Some("SID-retiring-worker"))
            )
        });
        assert!(
            native
                .send(json!({"id":correlation.renderer_call_id().get(),"result":{}}))
                .is_err(),
            "native destruction cannot replace the already settled close terminal"
        );
        let terminal = response
            .now_or_never()
            .expect("call retirement is synchronous at admission")
            .unwrap();
        assert_eq!(
            terminal
                .output
                .protocol_response(terminal.call_id)
                .unwrap()
                .as_json()
                .expect("scalar Inspector fixture")["error"]["message"],
            "Target detached"
        );
        if cancel_before_poll {
            drop(pending);
            let CdpCommandTaskStep::Pending(recovery) = ctx.with_fixture(|conn| {
                conn.complete_failed_command_wait_with_context(
                    Some(303),
                    None,
                    "controlled pre-poll cancellation",
                    &mut context,
                )
            }) else {
                panic!("the original close retains its output owner")
            };
            pending = recovery;
        }
        let completed = drive(&mut ctx, *pending).await;
        let messages = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
        let ack = messages
            .iter()
            .position(|message| message["id"] == 303)
            .unwrap();
        assert_eq!(messages[ack]["result"]["success"], true);
        for id in [301, 302] {
            let responses = messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message["id"] == id)
                .collect::<Vec<_>>();
            assert_eq!(
                responses.len(),
                1,
                "original errors survive cancellation exactly once: {messages:?}"
            );
            let (position, response) = responses[0];
            assert!(
                position < ack,
                "pending-call terminals remain before the close acknowledgement: {messages:?}"
            );
            assert_eq!(
                response["error"],
                json!({"code":-32000,"message":"Target detached"})
            );
        }
    }
}

#[tokio::test]
async fn captured_worker_attach_remains_ordered_and_cannot_leave_a_late_binding_after_close() {
    for cancel_before_poll in [false, true] {
        let (conn, owner_page, renderer_page) = dedicated_worker_fixture();
        let mut ctx = crate::testing::TestContext::from_conn(conn);
        ctx.with_fixture(|conn| {
            conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery())
        });
        ctx.with_fixture(|conn| enable_dedicated_worker_auto_attach_for_owner_page(conn, false));
        let created = ctx.with_fixture(|conn| {
            register_dedicated_worker_target(
                conn,
                "BID-1",
                owner_page,
                renderer_page,
                vec![None],
                dedicated_worker_info(501, renderer_page, "https://example.test/held-worker.js"),
            )
        });
        let target_id = dedicated_created_target_id(&created);
        let _ = drain_target_lifecycle_events_for_test(&mut ctx, created).await;
        let held =
            ctx.with_fixture(|conn| {
                record_dedicated_worker_main_script(
                    conn,
                    "BID-1",
                    501,
                    "https://example.test/held-worker.js".into(),
                    crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(Box::new(
                        worker_response("https://example.test/held-worker.js", 200, false),
                    )),
                )
            });
        let session_id = dedicated_attached_session_id(&held);
        ctx.with_fixture(|conn| {
            assert!(
                conn.session_route(Some(&session_id)).is_none(),
                "capture owns the original Native attachment before binding publication"
            )
        });

        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(mut pending) = ctx.with_fixture(|conn| {
            start(
                conn,
                304,
                "Target.closeTarget",
                json!({"targetId":target_id}),
                &mut context,
            )
        }) else {
            panic!("retirement retains its original Native attachment snapshot")
        };
        let messages =
            protocol_messages(&drain_target_lifecycle_events_for_test(&mut ctx, held).await);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.attachedToTarget")
                .count(),
            1,
            "Closing must preserve the earlier accepted attachment fact: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == "Target.targetInfoChanged")
        );
        ctx.with_fixture(|conn| assert!(conn.session_route(Some(&session_id)).is_some()));
        ctx.with_fixture(|conn| {
            assert!(
                conn.session_is_closing(&session_id),
                "the existing target admission also closes a late-published session"
            )
        });
        let command = ParsedCdpCommand::parse_str(
            json!({"id":305,"sessionId":session_id,"method":"Runtime.enable"}).to_string(),
        )
        .unwrap();
        let CdpCommandTaskStep::Complete(rejected) = ctx.with_fixture(|conn| {
            conn.start_parsed_command_dispatch_with_context(
                &command,
                &mut CommandDispatchContext::default(),
            )
        }) else {
            panic!("a closing attachment cannot admit new Native work")
        };
        let (events, _, _) = rejected.into_protocol_event_parts();
        assert_eq!(
            events[0]
                .clone()
                .into_protocol_message()
                .into_json()
                .expect("scalar protocol fixture")["error"],
            json!({"code": -32001, "message": "Session is closing"})
        );
        ctx.with_fixture(|conn| {
            assert!(
                !conn
                    .shared_worker_target_for_session(Some(&session_id))
                    .unwrap()
                    .runtime_frontend_enabled(&session_id)
            )
        });
        if cancel_before_poll {
            drop(pending);
            let CdpCommandTaskStep::Pending(recovery) = ctx.with_fixture(|conn| {
                conn.complete_failed_command_wait_with_context(
                    Some(304),
                    None,
                    "controlled cancellation after captured attach",
                    &mut context,
                )
            }) else {
                panic!("the original close keeps cleanup authority")
            };
            pending = recovery;
        }
        let completed = drive(&mut ctx, *pending).await;
        let messages = ctx.with_fixture(|conn| finish(conn, completed, &mut context));
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == 304 && message["result"]["success"] == true)
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget"
                    && message["params"]["sessionId"] == session_id)
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.targetDestroyed")
                .count(),
            1
        );
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context_by_id("BID-1")
                    .unwrap()
                    .dedicated_worker_target(&target_id)
                    .is_none()
            )
        });
        ctx.with_fixture(|conn| {
            assert!(
                conn.session_route(Some(&session_id)).is_none(),
                "an earlier captured attachment cannot outlive the target close"
            )
        });
    }
}
