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
        conn.start_initial_document_page_ensure_for_owner(&CommandOwnerScope::for_session(
            "SID-initialization-A",
        ))
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
        conn.start_initial_document_page_ensure_for_owner(&owner)
            .unwrap()
    });
    if let Some(joined) = joined {
        joined
            .wait()
            .await
            .unwrap()
            .install_using(&mut ProtocolWorkflowDriver::new(ctx.handle()))
            .await
            .unwrap();
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
    let retired = ctx.with_fixture(|conn| {
        conn.start_initial_document_page_ensure_for_owner(&owner)
            .unwrap()
            .unwrap()
    });
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
    let replacement = ctx.with_fixture(|conn| {
        conn.start_initial_document_page_ensure_for_owner(&owner)
            .unwrap()
            .unwrap()
    });
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
    replacement
        .wait()
        .await
        .unwrap()
        .install_using(&mut ProtocolWorkflowDriver::new(ctx.handle()))
        .await
        .unwrap();
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
