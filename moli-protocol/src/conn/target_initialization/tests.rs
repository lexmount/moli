use super::*;
use crate::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsCreateTargetCommand,
    FrontendProtocol,
};
use crate::testing::{TestContext, wait_until_scheduler_message};
use serde_json::json;

#[tokio::test]
async fn retired_target_creation_preserves_its_no_such_target_error() {
    let mut ctx = TestContext::new();
    let step = ctx.with_fixture(|conn| {
        let step = conn.start_command_dispatch(
            r#"{"id":91,"method":"Target.createTarget","params":{"url":"about:blank"}}"#,
        );
        assert!(matches!(step, CdpCommandTaskStep::Pending(_)));
        conn.install_browser_context_fixture_for_test(BrowserContext::new(
            "BID-replacement".into(),
        ));
        step
    });
    let (messages, scheduler) = ctx.complete_command_task_step_for_test(step).await;
    let response = messages.iter().find(|message| message["id"] == 91).unwrap();
    assert_eq!(response["error"]["code"], -31998, "{response}");
    assert_eq!(response["error"]["message"], "UnknownTargetId");
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
}

fn attached_target() -> TestContext {
    let mut context = BrowserContext::new("BID-initialization".into());
    context.set_active_target_id("TID-initialization");
    context.attach_active_session("SID-initialization-A");
    assert!(
        context
            .assign_attached_session_to_target("TID-initialization", "SID-initialization-B".into())
    );
    let ctx = TestContext::new();
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(context));
    ctx
}

#[tokio::test]
async fn target_initialization_survives_initiator_waiter_and_session_disposal() {
    let mut ctx = attached_target();
    let first = ctx.with_fixture(|conn| {
        conn.ensure_target_initialization(&CommandOwnerScope::for_session("SID-initialization-A"))
            .unwrap()
            .unwrap()
    });
    let reservation = ctx.with_fixture(|conn| {
        conn.pending_target_page_residence_identity_for_owner(&CommandOwnerScope::for_session(
            "SID-initialization-B",
        ))
        .unwrap()
    });
    drop(first);
    ctx.process_async(json!({"id":51,"method":"Target.detachFromTarget","params":{"sessionId":"SID-initialization-A"}})).await;
    ctx.expect_result(51, json!({}), None);
    let joined = ctx.with_fixture(|conn| {
        let owner = CommandOwnerScope::for_session("SID-initialization-B");
        assert_eq!(
            conn.target_page_residence_identity_for_owner(&owner)
                .or_else(|| conn.pending_target_page_residence_identity_for_owner(&owner)),
            Some(reservation)
        );
        conn.ensure_target_initialization(&owner).unwrap()
    });
    if let Some(joined) = joined {
        joined.wait().await.unwrap();
    }
    ctx.with_fixture(|conn| {
        let owner = CommandOwnerScope::for_session("SID-initialization-B");
        assert!(
            conn.runtime_session_owner_slot_for_owner(&owner)
                .unwrap()
                .has_loaded_page()
        );
        assert!(conn.session_route(Some("SID-initialization-A")).is_none());
        assert!(conn.session_route(Some("SID-initialization-B")).is_some());
    });
    ctx.process_and_wait_for_response_async(json!({"id":52,"sessionId":"SID-initialization-B", "method":"Runtime.evaluate","params":{"expression":"6 * 7"}})).await;
    assert_eq!(ctx.take_response_by_id(52)["result"]["result"]["value"], 42);
}

#[tokio::test]
async fn default_target_initialization_remains_claimed_after_its_last_session_detaches() {
    let mut ctx = TestContext::new();
    let default = ctx.with_fixture(|conn| {
        conn.install_default_browser_target();
        let default = conn.default_target_id().to_owned();
        let owner = CommandOwnerScope::for_route(
            conn.target_session_route_for_target_id(&default).unwrap(),
        );
        conn.runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap()
            .start_initial_document_page_build();
        default
    });
    ctx.process_async(json!({"id":80,"method":"Target.attachToTarget","params":{"targetId":default,"flatten":true}})).await;
    let session = ctx.take_response_by_id(80)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.process_async(
        json!({"id":81,"method":"Target.detachFromTarget","params":{"sessionId":session}}),
    )
    .await;
    ctx.expect_result(81, json!({}), None);
    let create = ctx.with_fixture(|conn| conn.start_command_dispatch(
        r#"{"id":82,"method":"Target.createTarget","params":{"url":"about:blank","background":true}}"#
    ));
    ctx.with_fixture(|conn| {
        let route = conn
            .target_session_route_for_target_id(&default)
            .expect("pending initialization keeps the default Target's published identity");
        let owner = CommandOwnerScope::for_route(route);
        let slot = conn
            .runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap();
        assert!(slot.has_initial_document_page_build_in_progress());
        slot.fail_initial_document_page_build("test teardown".into());
    });
    let (messages, scheduler) = ctx.complete_command_task_step_for_test(create).await;
    let created = messages.iter().find(|message| message["id"] == 82).unwrap();
    let target = created["result"]["targetId"].as_str().unwrap();
    assert_ne!(target, default);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
}

#[tokio::test]
async fn retired_initialization_cannot_install_or_clear_a_replacement_target_with_the_same_id() {
    let ctx = attached_target();
    let owner = CommandOwnerScope::for_session("SID-initialization-A");
    let retired =
        ctx.with_fixture(|conn| conn.ensure_target_initialization(&owner).unwrap().unwrap());
    let public = ctx.with_fixture(|conn| {
        conn.take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                CdpSchedulerEvent::ProtocolTaskPrepared { pending } => Some(pending),
                _ => None,
            })
            .expect("Target's supervised initialization")
    });
    ctx.with_fixture(|conn| {
        let mut replacement = BrowserContext::new("BID-initialization".into());
        replacement.set_active_target_id("TID-initialization");
        replacement.attach_active_session("SID-initialization-A");
        conn.install_browser_context_fixture_for_test(replacement);
    });
    let replacement =
        ctx.with_fixture(|conn| conn.ensure_target_initialization(&owner).unwrap().unwrap());
    let reservation = ctx.with_fixture(|conn| {
        conn.pending_target_page_residence_identity_for_owner(&owner)
            .unwrap()
    });
    assert!(retired.wait().await.is_err());
    let completed = public.wait().await;
    ctx.with_fixture(|conn| {
        let (events, scheduler) = conn
            .complete_owned_protocol_scheduler_work(completed)
            .into_protocol_event_parts();
        assert!(events.is_empty());
        conn.extend_scheduler_events(scheduler);
    });
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.pending_target_page_residence_identity_for_owner(&owner),
            Some(reservation.clone())
        );
        assert!(conn.target_initial_document_waiter(&owner).is_some());
    });
    replacement.wait().await.unwrap();
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.target_page_residence_identity_for_owner(&owner),
            Some(reservation.clone())
        );
        assert!(
            conn.runtime_session_owner_slot_for_owner(&owner)
                .unwrap()
                .has_loaded_page()
        );
    });
}

#[tokio::test]
async fn page_enable_finishes_while_the_targets_initial_document_is_held() {
    let ctx = attached_target();
    let held = ctx.with_fixture(|conn| {
        let owner = CommandOwnerScope::for_session("SID-initialization-A");
        let slot = conn
            .runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap();
        slot.start_initial_document_page_build();
        slot.initial_document_page_build_waiter().unwrap()
    });
    assert!(held.clone().wait().now_or_never().is_none());
    let outcome = ctx.with_fixture(|conn| {
        conn.start_command_dispatch(
            r#"{"id":61,"sessionId":"SID-initialization-A","method":"Page.enable"}"#,
        )
    });
    let CdpCommandTaskStep::Complete(outcome) = outcome else {
        panic!("Page.enable commits its Browser subscription without waiting for a Document");
    };
    assert!(
        outcome
            .into_parts()
            .0
            .iter()
            .any(|message| message["id"] == 61 && message.get("error").is_none())
    );
    assert!(held.clone().wait().now_or_never().is_none());
    ctx.with_fixture(|conn| {
        assert!(
            conn.target_page_session_state_for_session(Some("SID-initialization-A"))
                .unwrap()
                .page_domain_enabled
        );
        conn.runtime_session_owner_slot_mut_for_owner(&CommandOwnerScope::for_session(
            "SID-initialization-A",
        ))
        .unwrap()
        .complete_initial_document_page_build();
    });
    held.wait().await.unwrap();
}

#[tokio::test]
async fn automation_initial_navigation_needs_neither_page_enable_nor_an_inspector_session() {
    let mut ctx = TestContext::new();
    let url = "data:text/html,<title>automation initialization</title>";
    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: None,
                    target_id: None,
                    browser_context_id: None,
                },
                url: url.into(),
                browser_context_id: None,
                activate: false,
            }),
        )
        .await
        .unwrap();
    let AutomationResult::CreateTarget(created) = result else {
        panic!("target creation result")
    };
    ctx.wait_until_scheduler_state("automation Target's initial URL", |conn| {
        let route = conn
            .target_session_route_for_target_id(created.target_id.as_str())
            .unwrap();
        let owner = CommandOwnerScope::for_route(route);
        conn.runtime_session_owner_slot_for_owner(&owner)
            .and_then(|slot| slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == url)
    })
    .await;
    ctx.with_fixture(|conn| {
        assert!(conn.browser_contexts().all(|bc| {
            bc.attached_session_ids_for_target(created.target_id.as_str())
                .is_empty()
        }));
    });
}

#[tokio::test]
async fn target_creation_continues_without_its_command_waiter_or_inspector_session() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/initial", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/initial",
                axum::routing::get(|| async {
                    axum::response::Html(
                        "<!doctype html><title>Target-owned initialization</title>",
                    )
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    ctx.process_async(
        json!({"id":70,"method":"Target.setDiscoverTargets","params":{"discover":true}}),
    )
    .await;
    ctx.expect_result(70, json!({}), None);
    let pending = ctx.with_fixture(|conn| {
        conn.start_command_dispatch(
            &json!({
                "id":71,"method":"Target.createTarget","params":{"url":url,"background":true}
            })
            .to_string(),
        )
    });
    let CdpCommandTaskStep::Pending(mut pending) = pending else {
        panic!("creation waits for its initial document");
    };
    // The owner admits public work before polling or cancelling the command.
    let scheduler = pending.take_scheduler_events();
    assert!(
        scheduler
            .iter()
            .any(|event| matches!(event, CdpSchedulerEvent::ProtocolTaskPrepared { .. }))
    );
    drop(pending);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
    wait_until_scheduler_message(&mut ctx, "Target-owned creation publication", |message| {
        message["method"] == "Target.targetCreated" && message["params"]["targetInfo"]["url"] == url
    })
    .await;
    let target = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["url"] == url
        })
        .unwrap()["params"]["targetInfo"]["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.wait_until_scheduler_state("initial Target URL committed without a Session", |conn| {
        let owner =
            CommandOwnerScope::for_route(conn.target_session_route_for_target_id(&target).unwrap());
        conn.runtime_session_owner_slot_for_owner(&owner)
            .and_then(|slot| slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == url)
    })
    .await;
    assert!(!ctx.sent.iter().any(|message| message["id"] == 71));
    server.abort();
}

#[tokio::test]
async fn panicking_initialization_preserves_output_and_retires_without_resetting_an_installed_page()
{
    use crate::domains::command_output::CommandOutputPlan;
    use moli_core::{
        PageId, RendererOutputCursor, RendererOutputFence, RendererOutputStreamIdentity,
    };

    fn fail() {
        panic!("injected initialization failure");
    }
    for after_install in [false, true] {
        let ctx = attached_target();
        let owner = CommandOwnerScope::for_session("SID-initialization-A");
        let cursor = RendererOutputCursor::new_for_test(
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(9901)),
            1,
        );
        let waiter = ctx.with_fixture(|conn| {
            conn.reserve_target_initialization(&owner).unwrap().submit(conn, None, Box::new(move |mut task, driver, context| Box::pin(async move {
                let page = if after_install { task.install(driver).await? } else { LoadedPageCreationDiagnosticsParts::default() };
                let mut prefix = CommandOutputPlan::default();
                prefix.push_background_event(BackgroundProtocolEvent::inspector_message(json!({"method":"Tracing.dataCollected","params":{"value":["before panic"]}}), None));
                prefix.insert_renderer_output_boundary(RendererOutputFence::new_for_test(cursor));
                context.retain_output(prefix);
                context.context_mut().defer_command_events(vec![BackgroundProtocolEvent::inspector_message(json!({"method":"Tracing.tracingComplete","params":{}}), None)]);
                fail();
                Ok(page)
            }))).unwrap()
        });
        let public = ctx.with_fixture(|conn| {
            conn.take_scheduler_events()
                .into_iter()
                .find_map(|event| match event {
                    CdpSchedulerEvent::ProtocolTaskPrepared { pending } => Some(pending),
                    _ => None,
                })
                .unwrap()
        });
        let error = waiter.clone().wait().await.err().unwrap();
        assert_eq!(error.kind, DevToolsErrorKind::Internal);
        assert!(
            error
                .message
                .contains(OwnedProtocolWaitFailure::Panicked.message())
        );
        ctx.with_fixture(|conn| {
            let slot = conn.runtime_session_owner_slot_for_owner(&owner).unwrap();
            assert_eq!(slot.has_loaded_page(), after_install);
            assert!(slot.initial_document_page_build_waiter().is_none());
            let mut output = CommandDispatchContext::default();
            conn.append_initialization_output(
                &waiter,
                TargetInitializationStage::Complete,
                Some(&mut output),
            );
            let (events, boundary, after) = output.take_renderer_fenced_protocol_events();
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].clone().into_protocol_message()["params"]["value"],
                json!(["before panic"])
            );
            assert_eq!(boundary.unwrap().cursor(), cursor);
            assert_eq!(after.len(), 1);
            assert_eq!(
                after[0].clone().into_protocol_message()["method"],
                "Tracing.tracingComplete"
            );
            assert!(output.take_deferred_command_events().is_empty());
            conn.append_initialization_output(
                &waiter,
                TargetInitializationStage::Complete,
                Some(&mut output),
            );
            assert!(output.take_protocol_events().is_empty());
        });
        let completed = public.wait().await;
        ctx.with_fixture(|conn| {
            conn.complete_owned_protocol_scheduler_work(completed);
        });
        let retry = ctx.with_fixture(|conn| {
            assert!(conn.initial_document_work_mut(&owner).unwrap().is_none());
            conn.ensure_target_initialization(&owner).unwrap()
        });
        assert_eq!(retry.is_none(), after_install);
        if let Some(retry) = retry {
            assert!(retry.wait().await.is_ok());
        }
    }
}

#[tokio::test]
async fn initialization_reservation_rejects_conflicts_and_releases_an_abandoned_stage() {
    let ctx = attached_target();
    let owner = CommandOwnerScope::for_session("SID-initialization-A");
    let reservation = ctx.with_fixture(|conn| {
        let reservation = conn.reserve_target_initialization(&owner).unwrap();
        assert!(conn.reserve_target_initialization(&owner).is_err());
        assert!(conn.ensure_target_initialization(&owner).unwrap().is_none());
        reservation
    });
    drop(reservation);
    let waiter = ctx.with_fixture(|conn| {
        let reservation = conn.reserve_target_initialization(&owner).unwrap();
        let waiter = reservation
            .submit(
                conn,
                None,
                Box::new(|mut task, driver, _| Box::pin(async move { task.install(driver).await })),
            )
            .unwrap();
        assert!(conn.reserve_target_initialization(&owner).is_err());
        let joined = conn.ensure_target_initialization(&owner).unwrap().unwrap();
        assert!(Rc::ptr_eq(&joined.receipt, &waiter.receipt));
        waiter
    });
    assert!(waiter.wait().await.is_ok());
}

#[tokio::test]
async fn retired_reservation_cannot_start_a_build_for_a_replacement_host() {
    let ctx = attached_target();
    let owner = CommandOwnerScope::for_session("SID-initialization-A");
    let reservation = ctx.with_fixture(|conn| conn.reserve_target_initialization(&owner).unwrap());
    ctx.with_fixture(|conn| {
        let mut replacement = BrowserContext::new("BID-initialization".into());
        replacement.set_active_target_id("TID-initialization");
        replacement.attach_active_session("SID-initialization-A");
        conn.install_browser_context_fixture_for_test(replacement);
        let error = reservation
            .submit(
                conn,
                None,
                Box::new(|mut task, driver, _| Box::pin(async move { task.install(driver).await })),
            )
            .err()
            .unwrap();
        assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
        assert!(
            !conn
                .runtime_session_owner_slot_for_owner(&owner)
                .unwrap()
                .has_initial_document_page_build_in_progress()
        );
    });
}

#[tokio::test]
async fn setup_wakes_all_waiters_and_hands_off_only_its_own_output_before_navigation_ends() {
    use crate::domains::command_output::CommandOutputPlan;
    for claim_after_completion in [false, true] {
        let ctx = attached_target();
        let owner = CommandOwnerScope::for_session("SID-initialization-A");
        let (begin, started) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let pending = ctx.with_fixture(|conn| conn.reserve_target_initialization(&owner).unwrap().submit_creation(conn, TargetInitializationStage::SetupFinished, Box::new(move |mut task, driver, context| Box::pin(async move {
            started.await.unwrap();
            let page = task.install(driver).await?;
            context.context_mut().push_protocol_event(BackgroundProtocolEvent::inspector_message(json!({"method":"Tracing.dataCollected","params":{"value":["setup"]}}), None));
            context.context_mut().defer_command_events(vec![BackgroundProtocolEvent::page_file_chooser_opened(None, "FRAME-setup", "selectSingle", 42, None)]);
            task.setup_finished(context, &page);
            held.await.unwrap();
            context.context_mut().push_protocol_event(BackgroundProtocolEvent::inspector_message(json!({"method":"Tracing.dataCollected","params":{"value":["navigation"]}}), None));
            Ok(page)
        }))).unwrap());
        let observer = pending.initialization.clone();
        let mut page_wait = Box::pin(observer.clone().wait_page());
        let mut setup_wait = Box::pin(
            observer
                .clone()
                .wait_for(TargetInitializationStage::SetupFinished),
        );
        let mut creation_wait = Box::pin(pending.wait());
        assert!(page_wait.as_mut().now_or_never().is_none());
        assert!(setup_wait.as_mut().now_or_never().is_none());
        assert!(creation_wait.as_mut().now_or_never().is_none());
        begin.send(()).unwrap();
        let mut completion = Box::pin(observer.clone().wait());
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::select! {
                ready = async { tokio::join!(page_wait, setup_wait, creation_wait) } => {
                    assert!(ready.0.is_ok());
                    assert!(ready.1.is_ok());
                    ready.2
                }
                _ = &mut completion => panic!("navigation is still held"),
            }
        })
        .await
        .expect("all stage waiters wake independently of navigation completion");
        assert!(completion.as_mut().now_or_never().is_none());
        let mut release = Some(release);
        if claim_after_completion {
            release.take().unwrap().send(()).unwrap();
            assert!(completion.as_mut().await.is_ok());
        }
        let mut output = CommandDispatchContext::default();
        let (mut context, _recovery) = CommandWorkflowContext::take_from(&mut output, None, None);
        assert!(
            completed
                .publish_into(&mut ProtocolWorkflowDriver::new(ctx.handle()), &mut context)
                .is_ok()
        );
        let completed = context.complete(CommandOutputPlan::default());
        ctx.with_fixture(|conn| {
            completed.finish(conn, &mut output);
        });
        let events = output.take_protocol_events();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].clone().into_protocol_message()["params"]["value"],
            json!(["setup"])
        );
        assert!(matches!(
            events[1].clone().into_parts().1,
            Some(crate::automation::AutomationEvent::PageFileChooserOpened(_))
        ));
        if let Some(release) = release {
            release.send(()).unwrap();
            assert!(completion.await.is_ok());
        }
        ctx.with_fixture(|conn| {
            let mut tail = CommandDispatchContext::default();
            conn.append_initialization_output(
                &observer,
                TargetInitializationStage::Complete,
                Some(&mut tail),
            );
            let events = tail.take_protocol_events();
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].clone().into_protocol_message()["params"]["value"],
                json!(["navigation"])
            );
            conn.finish_target_initialization(observer);
            assert!(conn.initial_document_work_mut(&owner).unwrap().is_none());
        });
    }
}

#[tokio::test]
async fn abandoned_setup_output_is_published_while_navigation_is_still_held() {
    let ctx = attached_target();
    let owner = CommandOwnerScope::for_session("SID-initialization-A");
    let (release, held) = tokio::sync::oneshot::channel();
    let pending = ctx.with_fixture(|conn| conn.reserve_target_initialization(&owner).unwrap().submit_creation(conn, TargetInitializationStage::SetupFinished, Box::new(move |mut task, driver, context| Box::pin(async move {
        let page = task.install(driver).await?;
        context.context_mut().push_protocol_event(BackgroundProtocolEvent::inspector_message(json!({"method":"Tracing.dataCollected","params":{"value":["setup"]}}), None));
        task.setup_finished(context, &page);
        held.await.unwrap();
        Ok(page)
    }))).unwrap());
    let observer = pending.initialization.clone();
    let public = ctx.with_fixture(|conn| {
        conn.take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                CdpSchedulerEvent::ProtocolTaskPrepared { pending } => Some(pending),
                _ => None,
            })
            .unwrap()
    });
    // Lose an already-ready receipt. Its output remains with the Target.
    drop(pending.wait().await);
    assert!(observer.clone().wait().now_or_never().is_none());
    let mut publication = Box::pin(public.wait());
    assert!(publication.as_mut().now_or_never().is_none());
    assert!(
        observer.receipt.state.borrow().setup_output.is_none(),
        "supervisor publishes the abandoned setup before navigation finishes"
    );
    release.send(()).unwrap();
    let completed = publication.await;
    ctx.with_fixture(|conn| {
        conn.complete_owned_protocol_scheduler_work(completed);
        assert!(conn.initial_document_work_mut(&owner).unwrap().is_none());
    });
}

#[tokio::test]
async fn automation_creation_returns_after_setup_while_its_initial_response_is_held() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/initial", listener.local_addr().unwrap());
    let (requested, observed) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    let gate = std::sync::Arc::new(tokio::sync::Mutex::new(Some((requested, held))));
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/initial",
                axum::routing::get(move || {
                    let gate = gate.clone();
                    async move {
                        let (requested, held) = gate.lock().await.take().unwrap();
                        let _ = requested.send(());
                        held.await.unwrap();
                        axum::response::Html(
                            "<!doctype html><title>Target workflow continued</title>",
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    let created = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        ctx.execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: None,
                    target_id: None,
                    browser_context_id: None,
                },
                url: url.clone(),
                browser_context_id: None,
                activate: false,
            }),
        ),
    )
    .await
    .expect("creation waits for setup, not the initial HTTP response")
    .unwrap();
    let AutomationResult::CreateTarget(created) = created else {
        panic!("created Target");
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), observed)
        .await
        .unwrap()
        .unwrap();
    ctx.with_fixture(|conn| {
        let owner = CommandOwnerScope::for_route(
            conn.target_session_route_for_target_id(created.target_id.as_str())
                .unwrap(),
        );
        assert!(
            conn.runtime_session_owner_slot_for_owner(&owner)
                .unwrap()
                .has_loaded_page()
        );
        assert!(
            conn.target_initial_document_waiter(&owner).is_some(),
            "the Target still owns navigation after the command returned"
        );
    });
    release.send(()).unwrap();
    ctx.wait_until_scheduler_state("initial URL after automation setup", |conn| {
        let owner = CommandOwnerScope::for_route(
            conn.target_session_route_for_target_id(created.target_id.as_str())
                .unwrap(),
        );
        conn.runtime_session_owner_slot_for_owner(&owner)
            .and_then(|slot| slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == url)
    })
    .await;
    server.abort();
}

#[tokio::test]
async fn final_owner_completion_preserves_its_failure_and_output_dependencies() {
    use crate::domains::command_output::CommandOutputPlan;
    use moli_core::{
        PageId, RendererOutputCursor, RendererOutputFence, RendererOutputStreamIdentity,
    };
    fn fail() {
        panic!("injected final owner failure");
    }
    for panic_in_completion in [false, true] {
        let ctx = attached_target();
        let owner = CommandOwnerScope::for_session("SID-initialization-A");
        let cursor = RendererOutputCursor::new_for_test(
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(9902)),
            1,
        );
        let waiter = ctx.with_fixture(|conn| {
            conn.reserve_target_initialization(&owner)
                .unwrap()
                .submit(
                    conn,
                    None,
                    Box::new(move |mut task, driver, context| {
                        Box::pin(async move {
                            let page = task.install(driver).await?;
                            context.defer_owner_turn_completion(move |_, context| {
                                context.push_protocol_event(
                                    BackgroundProtocolEvent::inspector_message(
                                        json!({"method":"Tracing.tracingComplete","params":{}}),
                                        None,
                                    ),
                                );
                                context.set_renderer_output_predecessor(
                                    RendererOutputFence::new_for_test(cursor),
                                );
                                if panic_in_completion {
                                    fail();
                                }
                                CommandOutputPlan::default()
                            });
                            Ok(page)
                        })
                    }),
                )
                .unwrap()
        });
        let result = waiter.clone().wait().await;
        if panic_in_completion {
            let error = result
                .err()
                .expect("recovery failure overrides the earlier business result");
            assert_eq!(error.kind, DevToolsErrorKind::Internal);
            assert!(
                error
                    .message
                    .contains(OwnedProtocolWaitFailure::Panicked.message())
            );
        } else {
            let diagnostics = result.expect("completed initialization");
            assert_eq!(
                diagnostics
                    .output_dependencies
                    .clone()
                    .into_single_stream()
                    .unwrap()
                    .cursor(),
                cursor
            );
        }
        ctx.with_fixture(|conn| {
            assert!(
                conn.runtime_session_owner_slot_for_owner(&owner)
                    .unwrap()
                    .has_loaded_page()
            );
            let mut output = CommandDispatchContext::default();
            conn.append_initialization_output(
                &waiter,
                TargetInitializationStage::Complete,
                Some(&mut output),
            );
            let events = output.take_protocol_events();
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].clone().into_protocol_message()["method"],
                "Tracing.tracingComplete"
            );
            conn.finish_target_initialization(waiter);
            assert!(conn.initial_document_work_mut(&owner).unwrap().is_none());
        });
    }
}

#[tokio::test]
async fn failed_setup_returns_its_accepted_output_to_the_creation_command_once() {
    use crate::domains::command_output::CommandOutputPlan;
    use moli_core::{
        PageId, RendererOutputCursor, RendererOutputFence, RendererOutputStreamIdentity,
    };
    let ctx = attached_target();
    let owner = CommandOwnerScope::for_session("SID-initialization-A");
    let cursor = RendererOutputCursor::new_for_test(
        RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(9903)),
        1,
    );
    let pending = ctx.with_fixture(|conn| conn.reserve_target_initialization(&owner).unwrap().submit_creation(
        conn, TargetInitializationStage::SetupFinished,
        Box::new(move |_, _, context| Box::pin(async move {
            let mut prefix = CommandOutputPlan::default();
            prefix.push_background_event(BackgroundProtocolEvent::inspector_message(json!({"method":"Tracing.dataCollected","params":{"value":["accepted before setup failed"]}}), None));
            prefix.insert_renderer_output_boundary(RendererOutputFence::new_for_test(cursor));
            context.retain_output(prefix);
            context.context_mut().set_renderer_output_predecessor(RendererOutputFence::new_for_test(cursor));
            Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "setup target retired"))
        })),
    ).unwrap());
    let public = ctx.with_fixture(|conn| {
        conn.take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                CdpSchedulerEvent::ProtocolTaskPrepared { pending } => Some(pending),
                _ => None,
            })
            .unwrap()
    });
    let completed = pending.wait().await;
    let mut output = CommandDispatchContext::default();
    let (mut context, _recovery) = CommandWorkflowContext::take_from(&mut output, None, None);
    let error = completed
        .publish_into(&mut ProtocolWorkflowDriver::new(ctx.handle()), &mut context)
        .err()
        .unwrap();
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
    assert_eq!(error.message, "setup target retired");
    let completed = context.complete(CommandOutputPlan::default());
    ctx.with_fixture(|conn| {
        completed.finish(conn, &mut output);
    });
    let (events, boundary, after) = output.take_renderer_fenced_protocol_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].clone().into_protocol_message()["params"]["value"],
        json!(["accepted before setup failed"])
    );
    assert_eq!(boundary.unwrap().cursor(), cursor);
    assert!(after.is_empty());
    assert_eq!(
        output
            .take_renderer_output_dependencies()
            .into_single_stream()
            .unwrap()
            .cursor(),
        cursor
    );
    ctx.with_fixture(|conn| {
        conn.take_scheduler_events();
    });
    let completed = public.wait().await;
    ctx.with_fixture(|conn| {
        let (events, scheduler) = conn
            .complete_owned_protocol_scheduler_work(completed)
            .into_protocol_event_parts();
        assert!(events.is_empty());
        assert!(
            !scheduler
                .iter()
                .any(|event| matches!(event, CdpSchedulerEvent::ProtocolTaskPrepared { .. })),
            "claimed output must not be republished by the supervisor"
        );
        assert!(conn.initial_document_work_mut(&owner).unwrap().is_none());
    });
}
