use super::*;

use crate::conn::ServiceWorkerTargetState;
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsErrorKind,
    DevToolsGetRealmsCommand, DevToolsGetServiceWorkerLogsCommand, DevToolsProtocol,
    DevToolsSessionId, DevToolsTargetId, RuntimeExecutionContextEvent,
};
use moli_core::page::{
    RendererServiceWorkerExceptionMessage, RendererServiceWorkerFetchDiagnostic,
    RendererServiceWorkerFetchDiagnosticResult, RendererServiceWorkerVersionStatus,
};

fn load_service_worker_target(ctx: &mut TestContext, session_id: &str) {
    let mut bc = BrowserContext::new("BID-service".to_owned());
    let mut target = service_worker_target();
    target.attach_session(session_id.to_owned());
    bc.insert_service_worker_target(target);
    ctx.conn.browser_context = Some(bc);
}

fn load_unattached_service_worker_target(ctx: &mut TestContext) {
    let mut bc = BrowserContext::new("BID-service".to_owned());
    bc.insert_service_worker_target(service_worker_target());
    ctx.conn.browser_context = Some(bc);
}

fn service_worker_target() -> ServiceWorkerTargetState {
    ServiceWorkerTargetState::new(
        3,
        7,
        "TID-service-worker".to_owned(),
        "https://example.test/service-worker.js".to_owned(),
        "https://example.test/".to_owned(),
        RendererServiceWorkerVersionStatus::Activated,
        None,
    )
}

fn record_service_worker_console(ctx: &mut TestContext, session_id: &str, message: &str) {
    ctx.conn
        .service_worker_target_for_session_mut(Some(session_id))
        .expect("service worker target session should exist")
        .record_console_message(message.to_owned(), Vec::new(), None);
}

fn record_service_worker_runtime_context(ctx: &mut TestContext, session_id: &str, context_id: i64) {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(context_id),
        realm_id: None,
        frame_id: None,
        origin: None,
        name: None,
        is_default: None,
        context_type: Some("service-worker".to_owned()),
        grant_universal_access: None,
    };
    ctx.conn
        .service_worker_target_for_session_mut(Some(session_id))
        .expect("service worker target session should exist")
        .record_runtime_execution_context_created_event(&event);
}

fn record_unattached_service_worker_console(ctx: &mut TestContext, message: &str) {
    ctx.conn
        .browser_context_by_id_mut("BID-service")
        .and_then(|browser_context| browser_context.service_worker_target_mut("TID-service-worker"))
        .expect("service worker target should exist")
        .record_console_message(message.to_owned(), Vec::new(), None);
}

fn record_service_worker_exception(ctx: &mut TestContext, session_id: &str, message: &str) {
    ctx.conn
        .service_worker_target_for_session_mut(Some(session_id))
        .expect("service worker target session should exist")
        .record_exception_message(RendererServiceWorkerExceptionMessage {
            message: message.to_owned(),
            filename: "https://example.test/service-worker.js".to_owned(),
            lineno: 5,
            colno: 13,
            event_kind: "error_event".to_owned(),
            phase: "runtime".to_owned(),
            source: "runtime".to_owned(),
        });
}

fn service_worker_fetch_diagnostic(internal_id: u64) -> RendererServiceWorkerFetchDiagnostic {
    RendererServiceWorkerFetchDiagnostic {
        internal_id,
        document_url: "https://example.test/app/".to_owned(),
        request_url: format!("https://example.test/api/{internal_id}"),
        method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        destination: "".to_owned(),
        result: RendererServiceWorkerFetchDiagnosticResult::Fallback,
    }
}

#[tokio::test]
async fn target_auto_attach_wait_for_debugger_toggles_service_worker_pause_on_start() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    let runtime = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist")
        .renderer_runtime();
    assert!(!runtime.service_worker_pause_on_start_for_devtools());

    ctx.process_async(json!({
        "id": 78,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true
        }
    }))
    .await;
    ctx.expect_result(78, json!({}), None);
    assert!(runtime.service_worker_pause_on_start_for_devtools());

    ctx.process_async(json!({
        "id": 79,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": true
        }
    }))
    .await;
    ctx.expect_result(79, json!({}), None);
    assert!(!runtime.service_worker_pause_on_start_for_devtools());
}

#[tokio::test]
async fn target_auto_attach_wait_for_debugger_respects_service_worker_filter() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    let runtime = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist")
        .renderer_runtime();
    assert!(!runtime.service_worker_pause_on_start_for_devtools());

    ctx.process_async(json!({
        "id": 80,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "filter": [
                { "type": "service_worker", "exclude": true },
                { "type": "page" }
            ]
        }
    }))
    .await;
    ctx.expect_result(80, json!({}), None);

    assert!(
        !runtime.service_worker_pause_on_start_for_devtools(),
        "waitForDebuggerOnStart must not pause Service Workers when the target filter excludes them"
    );
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);
}

#[tokio::test]
async fn service_worker_pause_on_start_waits_for_last_owner() {
    let mut ctx = TestContext::new();
    load_unattached_service_worker_target(&mut ctx);
    let runtime = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist")
        .renderer_runtime();
    assert!(!runtime.service_worker_pause_on_start_for_devtools());
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);

    crate::domains::target::set_service_worker_pause_on_start_owner(&mut ctx.conn, None, true);
    assert!(runtime.service_worker_pause_on_start_for_devtools());
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 1);

    crate::domains::target::set_service_worker_pause_on_start_owner(
        &mut ctx.conn,
        Some("SID-service-worker-owner"),
        true,
    );
    assert!(runtime.service_worker_pause_on_start_for_devtools());
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 2);

    crate::domains::target::set_service_worker_pause_on_start_owner(&mut ctx.conn, None, false);
    assert!(
        runtime.service_worker_pause_on_start_for_devtools(),
        "one remaining DevTools owner must keep new Service Worker targets paused"
    );
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 1);

    crate::domains::target::set_service_worker_pause_on_start_owner(
        &mut ctx.conn,
        Some("SID-service-worker-owner"),
        false,
    );
    assert!(!runtime.service_worker_pause_on_start_for_devtools());
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);
}

#[tokio::test]
async fn runtime_enable_on_service_worker_session_without_renderer_context_delays_buffered_logs() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    record_service_worker_console(&mut ctx, "SID-service-worker", "warn: boot warning");
    record_service_worker_exception(
        &mut ctx,
        "SID-service-worker",
        "Uncaught Error: boot failure",
    );

    ctx.process_async(json!({
        "id": 81,
        "method": "Runtime.enable",
        "sessionId": "SID-service-worker"
    }))
    .await;

    ctx.expect_result(81, json!({}), Some("SID-service-worker"));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Runtime.executionContextCreated")),
        "Runtime.enable must not synthesize a worker execution context before renderer context creation: {:?}",
        ctx.sent
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
        "Runtime.enable must not emit Runtime.consoleAPICalled before a real renderer context id exists: {:?}",
        ctx.sent
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Runtime.exceptionThrown")),
        "Runtime.enable must not emit Runtime.exceptionThrown before a real renderer context id exists: {:?}",
        ctx.sent
    );
}

#[tokio::test]
async fn runtime_enable_on_service_worker_session_replays_real_worker_context() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    record_service_worker_console(&mut ctx, "SID-service-worker", "warn: boot warning");
    record_service_worker_exception(
        &mut ctx,
        "SID-service-worker",
        "Uncaught Error: boot failure",
    );
    record_service_worker_runtime_context(&mut ctx, "SID-service-worker", 91_007);

    ctx.process_async(json!({
        "id": 82,
        "method": "Runtime.enable",
        "sessionId": "SID-service-worker"
    }))
    .await;

    ctx.expect_result(82, json!({}), Some("SID-service-worker"));
    let context_event = ctx.take_first_matching("service worker execution context", |message| {
        message["method"] == json!("Runtime.executionContextCreated")
    });
    assert_eq!(context_event["sessionId"], "SID-service-worker");
    assert_eq!(context_event["params"]["context"]["id"], json!(91_007));
    assert_eq!(
        context_event["params"]["context"]["origin"],
        json!("https://example.test")
    );
    assert_eq!(context_event["params"]["context"]["name"], json!(""));
    assert_eq!(
        context_event["params"]["context"]["auxData"],
        json!({
            "isDefault": true,
            "type": "service-worker"
        })
    );

    let console_event = ctx.take_first_matching("service worker runtime console", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
    });
    assert_eq!(console_event["sessionId"], "SID-service-worker");
    assert_eq!(console_event["params"]["type"], "warning");
    assert_eq!(
        console_event["params"]["args"],
        json!([{
            "type": "string",
            "value": "boot warning"
        }])
    );
    assert_eq!(console_event["params"]["executionContextId"], json!(91_007));

    let exception_event = ctx.take_first_matching("service worker runtime exception", |message| {
        message["method"] == json!("Runtime.exceptionThrown")
    });
    assert_eq!(exception_event["sessionId"], "SID-service-worker");
    let details = &exception_event["params"]["exceptionDetails"];
    assert_eq!(details["text"], json!("Uncaught Error: boot failure"));
    assert_eq!(
        details["url"],
        json!("https://example.test/service-worker.js")
    );
    assert_eq!(details["executionContextId"], json!(91_007));
    assert_eq!(details["lineNumber"], json!(4));
    assert_eq!(details["columnNumber"], json!(12));
}

#[tokio::test]
async fn get_realms_on_service_worker_target_waits_for_renderer_context() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-service-worker")),
                browser_context_id: None,
            },
            realm_type: Some("service-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert!(
        result.realms.is_empty(),
        "getRealms must not synthesize a service worker realm before renderer context creation: {:?}",
        result.realms
    );
}

#[tokio::test]
async fn get_realms_on_service_worker_target_returns_real_renderer_realm() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    record_service_worker_runtime_context(&mut ctx, "SID-service-worker", 91_007);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-service-worker")),
                browser_context_id: None,
            },
            realm_type: Some("service-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert_eq!(result.realms.len(), 1);
    let realm = &result.realms[0];
    assert_eq!(
        realm.target_id.as_ref().map(DevToolsTargetId::as_str),
        Some("TID-service-worker")
    );
    assert_eq!(realm.context_id, Some(91_007));
    assert_eq!(
        realm.realm_id.as_ref().map(|realm| realm.as_str()),
        Some("service-worker-TID-service-worker")
    );
    assert_eq!(realm.frame_id, None);
    assert_eq!(realm.origin.as_deref(), Some("https://example.test"));
    assert_eq!(realm.context_type.as_deref(), Some("service-worker"));
}

#[tokio::test]
async fn get_realms_global_enumeration_waits_for_service_worker_renderer_context() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: None,
                browser_context_id: None,
            },
            realm_type: Some("service-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert!(
        result.realms.iter().all(|realm| {
            realm.target_id.as_ref().map(DevToolsTargetId::as_str) != Some("TID-service-worker")
        }),
        "global getRealms must not synthesize service worker target realms before renderer context creation: {:?}",
        result.realms
    );
}

#[tokio::test]
async fn get_realms_global_enumeration_includes_service_worker_real_renderer_realm() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    record_service_worker_runtime_context(&mut ctx, "SID-service-worker", 91_007);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: None,
                browser_context_id: None,
            },
            realm_type: Some("service-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert!(
        result.realms.iter().any(|realm| {
            realm.target_id.as_ref().map(DevToolsTargetId::as_str) == Some("TID-service-worker")
                && realm.context_id == Some(91_007)
                && realm.context_type.as_deref() == Some("service-worker")
                && realm.realm_id.as_ref().map(|realm| realm.as_str())
                    == Some("service-worker-TID-service-worker")
        }),
        "global getRealms should include service worker target realms after renderer context creation: {:?}",
        result.realms
    );
}

#[tokio::test]
async fn get_service_worker_logs_drains_classic_cursor_without_attaching_target() {
    let mut ctx = TestContext::new();
    load_unattached_service_worker_target(&mut ctx);
    record_unattached_service_worker_console(&mut ctx, "log: classic buffered log");

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetServiceWorkerLogs(
            DevToolsGetServiceWorkerLogsCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverClassic,
                    session_id: Some(DevToolsSessionId::from("classic-session-1")),
                    target_id: None,
                    browser_context_id: None,
                },
                target_id: None,
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::ServiceWorkerLogs(result) =
        result.expect("getServiceWorkerLogs should succeed")
    else {
        panic!("expected ServiceWorkerLogs result");
    };

    assert_eq!(result.entries.len(), 1);
    let entry = &result.entries[0];
    assert_eq!(
        entry.target_id.as_ref().map(DevToolsTargetId::as_str),
        Some("TID-service-worker")
    );
    assert_eq!(entry.console_type, "log");
    assert_eq!(entry.text, "classic buffered log");
    assert_eq!(entry.execution_context_id, None);
    assert!(entry.timestamp.is_some());

    let target_info = ctx
        .conn
        .browser_context_by_id("BID-service")
        .expect("browser context")
        .target_infos()
        .into_iter()
        .find(|target| target["targetId"] == json!("TID-service-worker"))
        .expect("service worker target info");
    assert_eq!(target_info["attached"], json!(false));

    let (second_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetServiceWorkerLogs(
            DevToolsGetServiceWorkerLogsCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverClassic,
                    session_id: Some(DevToolsSessionId::from("classic-session-1")),
                    target_id: None,
                    browser_context_id: None,
                },
                target_id: None,
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::ServiceWorkerLogs(second_result) =
        second_result.expect("second getServiceWorkerLogs should succeed")
    else {
        panic!("expected second ServiceWorkerLogs result");
    };
    assert!(second_result.entries.is_empty());

    let missing = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetServiceWorkerLogs(
            DevToolsGetServiceWorkerLogsCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverClassic,
                    session_id: Some(DevToolsSessionId::from("classic-session-1")),
                    target_id: None,
                    browser_context_id: None,
                },
                target_id: Some(DevToolsTargetId::from("TID-missing")),
            },
        ))
        .await
        .into_parts()
        .0
        .expect_err("missing service worker target should fail");
    assert_eq!(missing.kind, DevToolsErrorKind::NoSuchTarget);
    assert_eq!(missing.message, "No such service worker target");
}

#[tokio::test]
async fn runtime_evaluate_on_service_worker_session_does_not_fall_back_to_page_runtime() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");

    ctx.process_async(json!({
        "id": 82,
        "method": "Runtime.evaluate",
        "sessionId": "SID-service-worker",
        "params": { "expression": "self.registration.scope" }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 82);
    assert_eq!(response["sessionId"], "SID-service-worker");
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(
        response["error"]["message"],
        "ServiceWorkerRuntimeUnavailable"
    );
}

#[tokio::test]
async fn network_enable_on_service_worker_session_toggles_target_local_cursor() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    ctx.conn
        .service_worker_target_for_session_mut(Some("SID-service-worker"))
        .expect("service worker target session should exist")
        .record_fetch_diagnostic(service_worker_fetch_diagnostic(1));

    ctx.process_async(json!({
        "id": 83,
        "method": "Network.enable",
        "sessionId": "SID-service-worker"
    }))
    .await;

    ctx.expect_result(83, json!({}), Some("SID-service-worker"));
    let target = ctx
        .conn
        .service_worker_target_for_session(Some("SID-service-worker"))
        .expect("service worker target session should exist");
    assert!(target.network_enabled("SID-service-worker"));
    assert!(
        target
            .pending_fetch_diagnostics("SID-service-worker")
            .is_empty(),
        "Network.enable should start at the current service worker diagnostic tail"
    );

    ctx.conn
        .service_worker_target_for_session_mut(Some("SID-service-worker"))
        .expect("service worker target session should exist")
        .record_fetch_diagnostic(service_worker_fetch_diagnostic(2));
    assert_eq!(
        ctx.conn
            .service_worker_target_for_session(Some("SID-service-worker"))
            .expect("service worker target session should exist")
            .pending_fetch_diagnostics("SID-service-worker")
            .len(),
        1
    );

    ctx.process_async(json!({
        "id": 84,
        "method": "Network.disable",
        "sessionId": "SID-service-worker"
    }))
    .await;

    ctx.expect_result(84, json!({}), Some("SID-service-worker"));
    let target = ctx
        .conn
        .service_worker_target_for_session(Some("SID-service-worker"))
        .expect("service worker target session should exist");
    assert!(!target.network_enabled("SID-service-worker"));
    assert!(
        target
            .pending_fetch_diagnostics("SID-service-worker")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_on_real_service_worker_session_enters_worker_global() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>service worker runtime target</body>",
        )
    }

    async fn service_worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
globalThis.__serviceWorkerRuntimeProbe = "service:" + (
  typeof ServiceWorkerGlobalScope !== "undefined" &&
  self instanceof ServiceWorkerGlobalScope
) + ":" + self.registration.scope;
self.addEventListener("install", event => {
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});
"#,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/service-worker.js", get(service_worker)),
        )
        .await
        .unwrap();
    });
    let page_url = format!("http://{addr}/page");

    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-page", "TID-page").await;
    ctx.process_async(json!({
        "id": 84,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(84, json!({}), None);

    ctx.process_async(json!({
        "id": 85,
        "method": "Runtime.evaluate",
        "sessionId": "SID-page",
        "params": {
            "expression": r#"
(async () => {
  const registration = await navigator.serviceWorker.register("/service-worker.js");
  await navigator.serviceWorker.ready;
  return registration.active && registration.active.scriptURL;
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let register_response = wait_for_response_by_id_async(&mut ctx, "SID-page", 85).await;
    assert_eq!(
        register_response["result"]["result"]["value"],
        json!(format!("http://{addr}/service-worker.js"))
    );

    wait_until_message(
        &mut ctx,
        None,
        "service worker target auto-attach",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["type"] == json!("service_worker")
        },
    )
    .await;
    let attached = ctx.take_first_matching("service worker target auto-attach", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("service_worker")
    });
    let service_worker_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("service worker session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 86,
        "method": "Runtime.evaluate",
        "sessionId": service_worker_session_id,
        "params": {
            "expression": "globalThis.__serviceWorkerRuntimeProbe",
            "returnByValue": true
        }
    }))
    .await;
    let probe_response = take_response_by_id(&mut ctx, 86);
    assert_eq!(
        probe_response["result"]["result"]["value"],
        json!(format!("service:true:http://{addr}/"))
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_auxiliary_producers_use_fresh_pages_and_navigation_terminals() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>clients.openWindow owner</body>",
        )
    }

    async fn service_worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
self.addEventListener("install", event => {
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});
self.addEventListener("notificationclick", event => {
  event.waitUntil((async () => {
    const before = await clients.matchAll({
      includeUncontrolled: true,
      type: "window"
    });
    const source = before.find(client => client.url.endsWith("/page"));
    if (!source) {
      throw new Error("missing source WindowClient");
    }
    try {
      const destination = event.notification.title === "no-content"
        ? "/no-content"
        : "/redirect-opened";
      const opened = await clients.openWindow(destination);
      source.postMessage({
        request: event.notification.title,
        isNull: opened === null,
        url: opened && opened.url,
        type: opened && opened.type,
        frameType: opened && opened.frameType
      });
    } catch (error) {
      source.postMessage({
        errorName: error && error.name,
        errorMessage: error && error.message
      });
    }
  })());
});
"#,
        )
    }

    let (referer_tx, referer_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let opened_referer_tx = referer_tx.clone();
    let no_content_referer_tx = referer_tx.clone();
    let notification_referer_tx = referer_tx;
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/service-worker.js", get(service_worker))
                .route(
                    "/redirect-opened",
                    get(|| async { axum::response::Redirect::temporary("/opened") }),
                )
                .route(
                    "/opened",
                    get(move |headers: axum::http::HeaderMap| {
                        let opened_referer_tx = opened_referer_tx.clone();
                        async move {
                            let referer = headers
                                .get(axum::http::header::REFERER)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned();
                            let _ = opened_referer_tx.send(("opened".to_owned(), referer));
                            (
                                [(CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><title>opened by service worker</title>",
                            )
                        }
                    }),
                )
                .route(
                    "/no-content",
                    get(move |headers: axum::http::HeaderMap| {
                        let no_content_referer_tx = no_content_referer_tx.clone();
                        async move {
                            let referer = headers
                                .get(axum::http::header::REFERER)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned();
                            let _ = no_content_referer_tx.send(("no-content".to_owned(), referer));
                            axum::http::StatusCode::NO_CONTENT
                        }
                    }),
                )
                .route(
                    "/notification-opened",
                    get(move |headers: axum::http::HeaderMap| {
                        let notification_referer_tx = notification_referer_tx.clone();
                        async move {
                            let referer = headers
                                .get(axum::http::header::REFERER)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned();
                            let _ = notification_referer_tx
                                .send(("notification-opened".to_owned(), referer));
                            (
                                [(CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><title>notification action target</title>",
                            )
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let page_url = format!("http://{addr}/page");
    let worker_url = format!("http://{addr}/service-worker.js");
    let redirect_url = format!("http://{addr}/redirect-opened");
    let opened_url = format!("http://{addr}/opened");
    let no_content_url = format!("http://{addr}/no-content");
    let notification_opened_url = format!("http://{addr}/notification-opened");
    let scope_url = url::Url::parse(&format!("http://{addr}/")).unwrap();

    tokio::task::LocalSet::new()
        .run_until(async move {
            let mut referer_rx = referer_rx;
            let mut ctx = TestContext::new();
            ctx.enable_background_navigation_scheduler_for_test();
            with_loaded_http_document_async(&mut ctx, &page_url, "SID-page", "TID-page").await;
            let source_page_id = ctx
                .conn
                .browser_context_by_id("BID-1")
                .and_then(|browser_context| {
                    browser_context.active_target.runtime_slot.loaded_page()
                })
                .expect("source Page should remain loaded")
                .renderer_page_id();

            ctx.process_async(json!({
                "id": 87,
                "method": "Browser.setPermission",
                "params": {
                    "permission": { "name": "notifications" },
                    "setting": "granted",
                    "origin": format!("http://{addr}"),
                    "browserContextId": "BID-1"
                }
            }))
            .await;
            ctx.expect_result(87, json!({}), None);
            ctx.process_async(json!({
                "id": 88,
                "method": "Target.setDiscoverTargets",
                "params": { "discover": true }
            }))
            .await;
            ctx.expect_result(88, json!({}), None);

            ctx.process_async(json!({
                "id": 89,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page",
                "params": {
                    "expression": r#"
(async () => {
  globalThis.__armClientsOpenWindowResult = () => {
    globalThis.__clientsOpenWindowResult = new Promise(resolve => {
      navigator.serviceWorker.addEventListener("message", event => resolve(event.data), {
        once: true
      });
    });
  };
  globalThis.__armClientsOpenWindowResult();
  const registration = await navigator.serviceWorker.register("/service-worker.js");
  await navigator.serviceWorker.ready;
  await registration.showNotification("open-window");
  await registration.showNotification("no-content");
  await registration.showNotification("action-navigation", {
    actions: [{
      action: "navigate",
      title: "Navigate",
      navigate: "/notification-opened"
    }]
  });
  return registration.active && registration.active.state;
})()
"#,
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            let setup = wait_for_response_by_id_async(&mut ctx, "SID-page", 89).await;
            assert_eq!(setup["result"]["result"]["value"], json!("activated"));

            let renderer_runtime = ctx
                .conn
                .browser_context_by_id("BID-1")
                .expect("browser context should remain live")
                .renderer_runtime();
            assert!(renderer_runtime.dispatch_service_worker_notification_click(
                &scope_url,
                "open-window",
                ""
            ));

            wait_until_message(
                &mut ctx,
                None,
                "redirecting clients.openWindow target creation",
                |message| {
                    message["method"] == json!("Target.targetCreated")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["url"] == json!(redirect_url)
                },
            )
            .await;
            let target_created = ctx.take_first_matching(
                "redirecting clients.openWindow target creation",
                |message| {
                    message["method"] == json!("Target.targetCreated")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["url"] == json!(redirect_url)
                },
            );
            let popup_target_id = target_created["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("popup target id")
                .to_owned();
            assert!(
                target_created["params"]["targetInfo"]
                    .get("openerId")
                    .is_none(),
                "browser-context openWindow must not forge a Window opener"
            );
            assert_eq!(
                target_created["params"]["targetInfo"]["canAccessOpener"],
                json!(false)
            );

            ctx.wait_until_scheduler_state("clients.openWindow committed Page", |conn| {
                conn.browser_context_by_id("BID-1")
                    .and_then(|browser_context| {
                        browser_context.loaded_page_for_target(&popup_target_id)
                    })
                    .is_some_and(|page| page.final_url().as_str() == opened_url)
            })
            .await;

            let popup_client_id = ctx
                .conn
                .browser_context_by_id("BID-1")
                .and_then(|browser_context| browser_context.loaded_page_for_target(&popup_target_id))
                .expect("committed clients.openWindow Page")
                .service_worker_client_id();
            let controlled_client_ids = renderer_runtime
                .controlled_service_worker_window_client_ids_for_devtools(1, 1);
            assert!(
                controlled_client_ids.contains(&popup_client_id),
                "committed popup client {popup_client_id} should be controlled; ids={controlled_client_ids:?}; urls={:?}",
                renderer_runtime.controlled_service_worker_window_client_urls_for_devtools(1, 1)
            );

            ctx.process_async(json!({
                "id": 90,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page",
                "params": {
                    "expression": "globalThis.__clientsOpenWindowResult",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            let completion = wait_for_response_by_id_async(&mut ctx, "SID-page", 90).await;
            assert_eq!(
                completion["result"]["result"]["value"],
                json!({
                    "request": "open-window",
                    "isNull": false,
                    "url": opened_url,
                    "type": "window",
                    "frameType": "top-level"
                })
            );

            let popup_page = ctx
                .conn
                .browser_context_by_id("BID-1")
                .and_then(|browser_context| browser_context.loaded_page_for_target(&popup_target_id))
                .expect("clients.openWindow target must own a loaded renderer Page");
            assert_eq!(popup_page.final_url().as_str(), opened_url);
            assert_ne!(popup_page.renderer_page_id(), source_page_id);
            let redirected_page_id = popup_page.renderer_page_id();

            ctx.process_async(json!({
                "id": 91,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page",
                "params": {
                    "expression": "globalThis.__armClientsOpenWindowResult(); true",
                    "returnByValue": true
                }
            }))
            .await;
            let armed = wait_for_response_by_id_async(&mut ctx, "SID-page", 91).await;
            assert_eq!(armed["result"]["result"]["value"], json!(true));
            assert!(renderer_runtime.dispatch_service_worker_notification_click(
                &scope_url,
                "no-content",
                ""
            ));
            wait_until_message(
                &mut ctx,
                None,
                "no-content clients.openWindow target creation",
                |message| {
                    message["method"] == json!("Target.targetCreated")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["url"] == json!(no_content_url)
                },
            )
            .await;
            let no_content_target_created = ctx.take_first_matching(
                "no-content clients.openWindow target creation",
                |message| {
                    message["method"] == json!("Target.targetCreated")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["url"] == json!(no_content_url)
                },
            );
            let no_content_target_id = no_content_target_created["params"]["targetInfo"]
                ["targetId"]
                .as_str()
                .expect("no-content target id")
                .to_owned();
            ctx.process_async(json!({
                "id": 92,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page",
                "params": {
                    "expression": "globalThis.__clientsOpenWindowResult",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            let no_content_completion =
                wait_for_response_by_id_async(&mut ctx, "SID-page", 92).await;
            assert_eq!(
                no_content_completion["result"]["result"]["value"],
                json!({
                    "request": "no-content",
                    "isNull": true,
                    "url": null,
                    "type": null,
                    "frameType": null
                })
            );
            let no_content_page = ctx
                .conn
                .browser_context_by_id("BID-1")
                .and_then(|browser_context| {
                    browser_context.loaded_page_for_target(&no_content_target_id)
                })
                .expect("204 target should preserve its real initial Page");
            assert_eq!(no_content_page.final_url().as_str(), "about:blank");
            assert_ne!(no_content_page.renderer_page_id(), source_page_id);
            assert_ne!(no_content_page.renderer_page_id(), redirected_page_id);
            let no_content_page_id = no_content_page.renderer_page_id();

            assert!(renderer_runtime.dispatch_service_worker_notification_click(
                &scope_url,
                "action-navigation",
                "navigate"
            ));
            wait_until_message(
                &mut ctx,
                None,
                "notification action target creation",
                |message| {
                    message["method"] == json!("Target.targetCreated")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["url"]
                            == json!(notification_opened_url)
                },
            )
            .await;
            let notification_target_created = ctx.take_first_matching(
                "notification action target creation",
                |message| {
                    message["method"] == json!("Target.targetCreated")
                        && message["params"]["targetInfo"]["type"] == json!("page")
                        && message["params"]["targetInfo"]["url"]
                            == json!(notification_opened_url)
                },
            );
            assert!(
                notification_target_created["params"]["targetInfo"]
                    .get("openerId")
                    .is_none()
            );
            assert_eq!(
                notification_target_created["params"]["targetInfo"]["canAccessOpener"],
                json!(false)
            );
            let notification_target_id = notification_target_created["params"]["targetInfo"]
                ["targetId"]
                .as_str()
                .expect("notification action target id")
                .to_owned();
            ctx.wait_until_scheduler_state("notification action committed Page", |conn| {
                conn.browser_context_by_id("BID-1")
                    .and_then(|browser_context| {
                        browser_context.loaded_page_for_target(&notification_target_id)
                    })
                    .is_some_and(|page| page.final_url().as_str() == notification_opened_url)
            })
            .await;
            let notification_page = ctx
                .conn
                .browser_context_by_id("BID-1")
                .and_then(|browser_context| {
                    browser_context.loaded_page_for_target(&notification_target_id)
                })
                .expect("notification action target must own a real Page");
            assert_ne!(notification_page.renderer_page_id(), source_page_id);
            assert_ne!(notification_page.renderer_page_id(), redirected_page_id);
            assert_ne!(
                notification_page.renderer_page_id(),
                no_content_page_id
            );

            for expected_path in ["opened", "no-content", "notification-opened"] {
                let (path, referer) =
                    tokio::time::timeout(std::time::Duration::from_secs(2), referer_rx.recv())
                        .await
                        .unwrap_or_else(|_| panic!("{expected_path} request should reach server"))
                        .expect("request referer channel should remain live");
                assert_eq!(path, expected_path);
                assert_eq!(
                    referer, worker_url,
                    "{expected_path} navigation must retain the worker script referrer source"
                );
            }
        })
        .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn clients_open_window_continuation_survives_fetch_fulfill() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>clients.openWindow Fetch owner</body>",
        )
    }

    async fn service_worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
self.addEventListener("install", event => event.waitUntil(self.skipWaiting()));
self.addEventListener("activate", event => event.waitUntil(clients.claim()));
self.addEventListener("notificationclick", event => {
  event.waitUntil((async () => {
    const source = (await clients.matchAll({
      includeUncontrolled: true,
      type: "window"
    })).find(client => client.url.endsWith("/page"));
    const opened = await clients.openWindow("/fetch-target");
    source.postMessage({
      isNull: opened === null,
      url: opened && opened.url,
      type: opened && opened.type
    });
  })());
});
"#,
        )
    }

    async fn unexpected_fetch_target() -> axum::http::StatusCode {
        panic!("Fetch.fulfillRequest must replace transport")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/service-worker.js", get(service_worker))
                .route("/fetch-target", get(unexpected_fetch_target)),
        )
        .await
        .unwrap();
    });
    let page_url = format!("http://{addr}/page");
    let worker_url = format!("http://{addr}/service-worker.js");
    let target_url = format!("http://{addr}/fetch-target");
    let scope_url = url::Url::parse(&format!("http://{addr}/")).unwrap();

    tokio::task::LocalSet::new()
        .run_until(async move {
            let mut ctx = TestContext::new();
            ctx.enable_background_navigation_scheduler_for_test();
            with_loaded_http_document_async(&mut ctx, &page_url, "SID-page", "TID-page").await;
            ctx.process_async(json!({
                "id": 930,
                "method": "Browser.setPermission",
                "params": {
                    "permission": { "name": "notifications" },
                    "setting": "granted",
                    "origin": format!("http://{addr}"),
                    "browserContextId": "BID-1"
                }
            }))
            .await;
            ctx.expect_result(930, json!({}), None);
            ctx.process_async(json!({
                "id": 931,
                "method": "Target.setDiscoverTargets",
                "params": { "discover": true }
            }))
            .await;
            ctx.expect_result(931, json!({}), None);
            ctx.process_async(json!({
                "id": 933,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page",
                "params": {
                    "expression": r#"
(async () => {
  globalThis.__clientsOpenWindowFetchResult = new Promise(resolve => {
    navigator.serviceWorker.addEventListener("message", event => resolve(event.data), {
      once: true
    });
  });
  const registration = await navigator.serviceWorker.register("/service-worker.js");
  await navigator.serviceWorker.ready;
  await registration.showNotification("fetch-open-window");
  return registration.active && registration.active.state;
})()
"#,
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            let setup = wait_for_response_by_id_async(&mut ctx, "SID-page", 933).await;
            assert_eq!(setup["result"]["result"]["value"], json!("activated"));
            ctx.process_async(json!({
                "id": 932,
                "method": "Target.setAutoAttach",
                "params": {
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }
            }))
            .await;
            ctx.expect_result(932, json!({}), None);

            let renderer_runtime = ctx
                .conn
                .browser_context_by_id("BID-1")
                .expect("browser context should remain live")
                .renderer_runtime();
            assert!(renderer_runtime.dispatch_service_worker_notification_click(
                &scope_url,
                "fetch-open-window",
                ""
            ));
            wait_until_message(&mut ctx, None, "Fetch popup attachment", |message| {
                message["method"] == json!("Target.attachedToTarget")
                    && message["params"]["targetInfo"]["url"] == json!(target_url)
            })
            .await;
            let attached = ctx.take_first_matching("Fetch popup attachment", |message| {
                message["method"] == json!("Target.attachedToTarget")
                    && message["params"]["targetInfo"]["url"] == json!(target_url)
            });
            let popup_target_id = attached["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("Fetch popup target id")
                .to_owned();
            let popup_session_id = attached["params"]["sessionId"]
                .as_str()
                .expect("Fetch popup session id")
                .to_owned();
            assert!(
                attached["params"]["targetInfo"].get("openerId").is_none(),
                "ServiceWorker auxiliary target must not forge an opener"
            );

            ctx.process_async(json!({
                "id": 934,
                "method": "Fetch.enable",
                "sessionId": popup_session_id,
                "params": {
                    "patterns": [{
                        "urlPattern": "*",
                        "resourceType": "Document",
                        "requestStage": "Request"
                    }]
                }
            }))
            .await;
            ctx.expect_result(934, json!({}), Some(&popup_session_id));
            ctx.process_async(json!({
                "id": 935,
                "method": "Runtime.runIfWaitingForDebugger",
                "sessionId": popup_session_id
            }))
            .await;
            let _ = wait_for_response_by_id_async(&mut ctx, popup_session_id.as_str(), 935).await;
            wait_until_message(
                &mut ctx,
                Some(popup_session_id.as_str()),
                "clients.openWindow Fetch pause",
                |message| {
                    message["method"] == json!("Fetch.requestPaused")
                        && message["params"]["request"]["url"] == json!(target_url)
                },
            )
            .await;
            let paused = ctx.take_first_matching("clients.openWindow Fetch pause", |message| {
                message["method"] == json!("Fetch.requestPaused")
                    && message["sessionId"] == json!(popup_session_id)
                    && message["params"]["request"]["url"] == json!(target_url)
            });
            assert_eq!(
                paused["params"]["request"]["headers"]["Referer"],
                json!(worker_url)
            );
            let paused_request_id = paused["params"]["requestId"]
                .as_str()
                .expect("Fetch pause request id")
                .to_owned();
            ctx.process_async(json!({
                "id": 936,
                "method": "Fetch.fulfillRequest",
                "sessionId": popup_session_id,
                "params": {
                    "requestId": paused_request_id,
                    "responseCode": 200,
                    "responseHeaders": [{
                        "name": "content-type",
                        "value": "text/html; charset=utf-8"
                    }],
                    "body": "PCFkb2N0eXBlIGh0bWw+PG1haW4+ZmV0Y2gtZnVsZmlsbGVkPC9tYWluPg=="
                }
            }))
            .await;
            ctx.expect_result(936, json!({}), Some(&popup_session_id));

            ctx.process_async(json!({
                "id": 937,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page",
                "params": {
                    "expression": "globalThis.__clientsOpenWindowFetchResult",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            let completion = wait_for_response_by_id_async(&mut ctx, "SID-page", 937).await;
            assert_eq!(
                completion["result"]["result"]["value"],
                json!({
                    "isNull": false,
                    "url": target_url,
                    "type": "window"
                })
            );
            let popup_page = ctx
                .conn
                .browser_context_by_id("BID-1")
                .and_then(|browser_context| {
                    browser_context.loaded_page_for_target(&popup_target_id)
                })
                .expect("Fetch-fulfilled clients.openWindow Page");
            assert_eq!(popup_page.final_url().as_str(), target_url);
            ctx.process_async(json!({
                "id": 938,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "document.querySelector('main').textContent",
                    "returnByValue": true
                }
            }))
            .await;
            let popup_content =
                wait_for_response_by_id_async(&mut ctx, popup_session_id.as_str(), 938).await;
            assert_eq!(
                popup_content["result"]["result"]["value"],
                json!("fetch-fulfilled")
            );
        })
        .await;
    server.abort();
}

#[tokio::test]
async fn console_enable_on_service_worker_session_starts_at_current_target_cursor() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    record_service_worker_console(&mut ctx, "SID-service-worker", "log: before enable");

    ctx.process_async(json!({
        "id": 83,
        "method": "Console.enable",
        "sessionId": "SID-service-worker"
    }))
    .await;
    ctx.expect_result(83, json!({}), Some("SID-service-worker"));
    let target = ctx
        .conn
        .service_worker_target_for_session_mut(Some("SID-service-worker"))
        .expect("service worker target session should exist");
    assert!(
        target
            .pending_console_domain_messages("SID-service-worker")
            .is_empty()
    );

    target.record_console_message("error: after enable".to_owned(), Vec::new(), None);
    let pending = target.pending_console_domain_messages("SID-service-worker");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].message, "error: after enable");
}
