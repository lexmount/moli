use super::test_support::{
    create_isolated_world_async, enable_runtime_and_take_execution_context_id_async,
    take_response_by_id, wait_for_response_by_id_async, with_loaded_document_async,
    with_loaded_document_for_active_target_async, with_loaded_http_document_async,
};
use crate::conn::{
    BrowserContext, CdpCommandTaskStep, CdpSchedulerEvent, NETWORK_ERROR_PAGE_URL,
    PendingCdpCommandDispatch,
};
use crate::domains::page::BackgroundNavigationCompletion;
use crate::testing::{
    TestContext, spawn_connection_drop_server, wait_until_message, wait_until_messages,
};
use axum::{Router, http::header::CONTENT_TYPE, response::IntoResponse, routing::get};
use moli_core::page::RendererSharedWorkerConsoleMessage;
use moli_core::{PageId, RendererOutputStreamControl, RendererOutputStreamIdentity};
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::json;
use tokio::net::TcpListener;

async fn complete_pending_command_task_for_test(
    ctx: &mut TestContext,
    pending: PendingCdpCommandDispatch,
) -> (Vec<serde_json::Value>, Vec<CdpSchedulerEvent>) {
    ctx.complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
        .await
}
async fn complete_command_task_step_for_test(
    ctx: &mut TestContext,
    step: CdpCommandTaskStep,
) -> (Vec<serde_json::Value>, Vec<CdpSchedulerEvent>) {
    ctx.complete_command_task_step_for_test(step).await
}
fn load_shared_worker_target(ctx: &mut TestContext, session_id: &str) {
    let mut bc = BrowserContext::new("BID-shared".to_owned());
    let mut target = crate::conn::SharedWorkerTargetState::new(
        moli_core::RendererOwnerLocalHostId::new_for_testing(1),
        SharedWorkerInstanceId::from_u64(81),
        "TID-shared-worker".to_owned(),
        None,
        "https://example.test/shared-worker.js".to_owned(),
        "worker".to_owned(),
    );
    target.attach_session(session_id.to_owned());
    bc.insert_shared_worker_target(target);
    ctx.conn.browser_context = Some(bc);
}
fn load_dedicated_worker_target(ctx: &mut TestContext, session_id: &str) {
    let browser_context_id = "BID-dedicated".to_owned();
    let mut bc = BrowserContext::new(browser_context_id.clone());
    let mut target = crate::conn::DedicatedWorkerTargetState::new(
        crate::conn::TargetPageResidenceIdentity::new_for_test(browser_context_id, None, 1),
        moli_core::RendererOwnerLocalHostId::new_for_testing(1),
        82,
        "TID-dedicated-worker".to_owned(),
        "worker".to_owned(),
        Vec::new(),
    );
    target.attach_session(session_id.to_owned());
    bc.insert_dedicated_worker_target(target);
    ctx.conn.browser_context = Some(bc);
}
fn record_shared_worker_console(ctx: &mut TestContext, session_id: &str, message: &str) {
    ctx.conn
        .shared_worker_target_for_session_mut(Some(session_id))
        .expect("shared worker target session should exist")
        .record_console_message(RendererSharedWorkerConsoleMessage {
            message: message.to_owned(),
            args: Vec::new(),
            stack: None,
        });
}
async fn start_attached_shared_worker_session(
    ctx: &mut TestContext,
    command_id_base: u64,
    worker_name: &str,
    worker_source: &str,
) -> String {
    ctx.process_async(json!({
        "id": command_id_base,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(command_id_base, json!({}), None);

    let source_literal =
        serde_json::to_string(worker_source).expect("worker source should serialize");
    let name_literal = serde_json::to_string(worker_name).expect("worker name should serialize");
    let expression = format!(
        r#"
(() => {{
  const source = {source_literal};
  const url = "data:text/javascript," + encodeURIComponent(source);
  const worker = new SharedWorker(url, {name_literal});
  globalThis.__sharedWorkerRuntimeBindingProbe = worker;
  worker.port.start();
  return "started";
}})()
"#
    );
    ctx.process_async(json!({
        "id": command_id_base + 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true
        }
    }))
    .await;
    let start_response = take_response_by_id(ctx, command_id_base + 1);
    assert_eq!(
        start_response["result"]["result"]["value"],
        json!("started")
    );

    wait_until_message(ctx, None, "shared worker target auto-attach", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("shared_worker")
            && message["params"]["targetInfo"]["title"] == json!(worker_name)
    })
    .await;
    let attached = ctx.take_first_matching("shared worker target auto-attach", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("shared_worker")
            && message["params"]["targetInfo"]["title"] == json!(worker_name)
    });
    attached["params"]["sessionId"]
        .as_str()
        .expect("shared worker session id")
        .to_owned()
}
async fn push_loaded_runtime_frontend_enabled_background_context_async(
    ctx: &mut TestContext,
    browser_context_id: &str,
    target_id: &str,
    session_id: &str,
    html: &str,
) {
    let page = ctx
        .conn
        .load_page_via_runtime_async(&format!("data:text/html,{html}"))
        .await
        .expect("test background page should load");
    let mut background_context = crate::conn::BrowserContext::new(browser_context_id.to_owned());
    let _ = background_context
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    background_context.set_active_target_id(target_id.to_owned());
    background_context.attach_active_session(session_id.to_owned());
    background_context
        .devtools_session_state
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.conn.inactive_browser_contexts.push(background_context);
}
async fn with_loaded_runtime_frontend_enabled_background_target_async(
    ctx: &mut TestContext,
    active_target_id: &str,
    active_session_id: &str,
    background_target_id: &str,
    background_session_id: &str,
    html: &str,
) {
    let background_target = crate::conn::BackgroundTarget::with_url(
        background_target_id.to_owned(),
        Some(background_session_id.to_owned()),
        format!("data:text/html,{html}"),
    );

    let mut browser_context = crate::conn::BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id(active_target_id.to_owned());
    browser_context.attach_active_session(active_session_id.to_owned());
    browser_context.background_targets.push(background_target);
    browser_context.mutate_parked_page_session_state(background_target_id, |state| {
        state
            .devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
    });
    ctx.conn.browser_context = Some(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        &format!("data:text/html,{html}"),
        Some(background_session_id),
    )
    .await;
}

mod bindings;
mod console;
mod evaluate;
mod inspector;
mod navigation;
mod objects;
mod service_worker;
mod shared_worker;
