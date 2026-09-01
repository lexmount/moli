use serde_json::json;

use crate::conn::BrowserContext;
use crate::testing::TestContext;

pub(super) async fn with_loaded_document_async(ctx: &mut TestContext, html: &str) {
    ctx.conn
        .insert_browser_context(BrowserContext::new("BID-1".into()));
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should be installed before loading test page")
        .set_active_target_id("TID-1".to_owned());
    let data_url = format!("data:text/html,{html}");
    ctx.install_navigation_fixture_for_session_owner(&data_url, None)
        .await;
}

pub(super) async fn with_loaded_document_for_active_target_async(
    ctx: &mut TestContext,
    html: &str,
    session_id: &str,
    target_id: &str,
) {
    ctx.conn
        .insert_browser_context(BrowserContext::new("BID-1".into()));
    {
        let bc = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be installed before loading test page");
        bc.set_active_target_id(target_id.to_owned());
        bc.attach_active_session(session_id.to_owned());
    }
    let data_url = format!("data:text/html,{html}");
    ctx.install_navigation_fixture_for_session_owner(&data_url, Some(session_id))
        .await;
}

pub(super) async fn with_loaded_http_document_async(
    ctx: &mut TestContext,
    url: &str,
    session_id: &str,
    target_id: &str,
) {
    ctx.conn
        .insert_browser_context(BrowserContext::new("BID-1".into()));
    {
        let bc = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be installed before loading test page");
        bc.set_active_target_id(target_id.to_owned());
        bc.attach_active_session(session_id.to_owned());
    }
    ctx.install_navigation_fixture_for_session_owner(url, Some(session_id))
        .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should be installed before loading test page");
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
}

pub(super) fn take_response_by_id(ctx: &mut TestContext, id: u64) -> serde_json::Value {
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .expect("expected a response with the requested id");
    ctx.sent.remove(pos)
}

/// Drive concrete scheduler inputs until a response with the given `id` appears in
/// `ctx.sent`, then take and return it. Used for V8 inspector requests whose
/// reply is delivered by an asynchronous agent callback, including
/// `awaitPromise=true` and `HeapProfiler.collectGarbage`.
pub(super) async fn wait_for_response_by_id_async<'a>(
    ctx: &mut crate::testing::TestContext,
    session_id: impl crate::testing::TestSessionId<'a> + Copy,
    id: u64,
) -> serde_json::Value {
    crate::testing::wait_until_message(
        ctx,
        session_id,
        &format!("response with id {id}"),
        |message| message["id"] == json!(id),
    )
    .await;
    take_response_by_id(ctx, id)
}

pub(super) async fn enable_runtime_and_take_execution_context_id_async(
    ctx: &mut TestContext,
    id: u64,
) -> i64 {
    ctx.process_async(json!({"id": id, "method": "Runtime.enable"}))
        .await;
    let response = take_response_by_id(ctx, id);
    assert_eq!(response["result"], json!({}));
    ctx.sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("Runtime.enable must emit executionContextCreated with a context id")
}

pub(super) async fn create_isolated_world_async(
    ctx: &mut TestContext,
    id: u64,
    world_name: &str,
) -> i64 {
    let target_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_target_id())
        .unwrap_or("TID-1")
        .to_owned();
    ctx.process_async(json!({
        "id": id,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": target_id,
            "worldName": world_name
        }
    }))
    .await;
    take_response_by_id(ctx, id)["result"]["executionContextId"]
        .as_i64()
        .expect("Page.createIsolatedWorld must return an executionContextId")
}
