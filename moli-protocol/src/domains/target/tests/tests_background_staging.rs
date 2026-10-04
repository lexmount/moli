use super::*;
use axum::extract::State;
use axum::http::Uri;

fn is_runtime_context_event(message: &serde_json::Value) -> bool {
    matches!(
        message["method"].as_str(),
        Some("Runtime.executionContextsCleared") | Some("Runtime.executionContextCreated")
    )
}

fn take_staged_about_blank_runtime_context(
    ctx: &mut TestContext,
    session_id: &str,
    target_id: &str,
) {
    let created = ctx.take_first_matching(
        "staged background Runtime.executionContextCreated",
        |message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        },
    );
    assert_eq!(
        created["params"]["context"]["name"],
        json!("about:blank#second")
    );
    assert_eq!(
        created["params"]["context"]["auxData"]["frameId"],
        json!(target_id)
    );
    assert!(
        created["params"]["context"]["uniqueId"].as_str().is_some(),
        "staged about:blank context should come from V8 native Runtime.enable replay: {created:?}"
    );
}

async fn wait_for_session_main_document_loading_finished(
    ctx: &mut TestContext,
    session_id: &str,
    request_url: &str,
    description: &str,
) {
    let request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(session_id)
                && message["params"]["request"]["url"] == json!(request_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .unwrap_or_else(|| {
            panic!(
                "main-document request should precede its completion: {:?}",
                ctx.sent
            )
        })
        .to_owned();
    crate::testing::wait_until_messages(ctx, Some(session_id), description, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["sessionId"] == json!(session_id)
                && message["params"]["requestId"] == json!(request_id)
        })
    })
    .await;
}

async fn loaded_page_html_for_test(ctx: &mut TestContext) -> String {
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|bc| bc.active_page_target_mut().runtime_slot.loaded_page_mut())
        .expect("loaded page");
    page.serialize_html_async()
        .await
        .expect("loaded page should serialize HTML")
}

fn load_same_context_loaded_background_runtime_owner_async<'a>(
    ctx: &'a mut TestContext,
    browser_context_id: &'a str,
    active_target_id: &'a str,
    active_html: &'a str,
    background_url: &'a str,
    command_id_base: u64,
) -> impl std::future::Future<Output = LoadedBackgroundRuntimeOwner> + 'a {
    // Keep the full navigation setup state on the heap instead of embedding it
    // in every caller's future and its debug-build stack temporaries.
    Box::pin(async move {
        load_bc_with_titled_page_async(ctx, browser_context_id, active_target_id, active_html)
            .await;
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active");

        ctx.process_async(json!({
            "id": command_id_base,
            "method": "Target.setAutoAttach",
            "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
        }))
        .await;
        ctx.expect_result(command_id_base, json!({}), None);

        ctx.process_async(json!({
            "id": command_id_base + 1,
            "method": "Target.createTarget",
            "params": {
                "background": true,
                "browserContextId": browser_context_id,
                "url": "about:blank#second"
            }
        }))
        .await;
        let created = ctx.take_one();
        assert_eq!(created["method"], "Target.targetCreated");
        let second_target_id = created["params"]["targetInfo"]["targetId"]
            .as_str()
            .expect("second target id")
            .to_owned();
        let attached = ctx.take_one();
        assert_eq!(attached["method"], "Target.attachedToTarget");
        let second_session_id = attached["params"]["sessionId"]
            .as_str()
            .expect("second target session id")
            .to_owned();
        ctx.expect_result(
            command_id_base + 1,
            json!({ "targetId": second_target_id }),
            None,
        );

        ctx.process_async(json!({
            "id": command_id_base + 2,
            "method": "Target.activateTarget",
            "params": { "targetId": second_target_id }
        }))
        .await;
        ctx.expect_result(command_id_base + 2, json!({}), None);

        ctx.process_async(json!({
            "id": command_id_base + 3,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": { "url": background_url }
        }))
        .await;
        let _ = take_response_by_id(ctx, command_id_base + 3);
        ctx.take_all();

        ctx.process_async(json!({
            "id": command_id_base + 4,
            "method": "Target.activateTarget",
            "params": { "targetId": active_target_id }
        }))
        .await;
        ctx.expect_result(command_id_base + 4, json!({}), None);
        ctx.take_all();

        ctx.process_async(json!({
            "id": command_id_base + 5,
            "method": "Runtime.enable",
            "sessionId": second_session_id
        }))
        .await;
        let _ = take_response_by_id(ctx, command_id_base + 5);
        ctx.take_all();

        LoadedBackgroundRuntimeOwner {
            target_id: second_target_id,
            session_id: second_session_id,
        }
    })
}

#[path = "tests_background_staging/background_state.rs"]
mod background_state;
#[path = "tests_background_staging/environment.rs"]
mod environment;
#[path = "tests_background_staging/lifecycle.rs"]
mod lifecycle;
#[path = "tests_background_staging/protocol_domains.rs"]
mod protocol_domains;
#[path = "tests_background_staging/runtime.rs"]
mod runtime;
use self::runtime::LoadedBackgroundRuntimeOwner;
