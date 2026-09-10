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
    let mut bc = ctx
        .conn
        .new_browser_context_fixture_for_test("BID-service".to_owned());
    let mut target = service_worker_target();
    target.attach_session(session_id.to_owned());
    bc.insert_service_worker_target(target);
    ctx.conn.install_browser_context_fixture_for_test(bc);
}

fn load_unattached_service_worker_target(ctx: &mut TestContext) {
    let mut bc = ctx
        .conn
        .new_browser_context_fixture_for_test("BID-service".to_owned());
    bc.insert_service_worker_target(service_worker_target());
    ctx.conn.install_browser_context_fixture_for_test(bc);
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
    let target = ctx
        .conn
        .service_worker_target_for_session_mut(Some(session_id))
        .expect("service worker target session should exist");
    let event = RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
        context_id: Some(context_id),
        realm_id: Some(crate::devtools_runtime::DevToolsRealmId::from(format!(
            "{}:native-realm-{context_id}",
            target.target_id
        ))),
        frame_id: None,
        origin: Some("https://example.test".to_owned()),
        name: Some(String::new()),
        is_default: Some(true),
        context_type: Some("service-worker".to_owned()),
        grant_universal_access: None,
    };
    target.record_runtime_execution_context_created_event(&event);
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

fn service_worker_pause_on_start(ctx: &TestContext) -> bool {
    ctx.conn
        .browser_context
        .as_ref()
        .expect("browser context should exist")
        .service_worker_pause_on_start()
}

#[tokio::test]
async fn target_auto_attach_wait_for_debugger_toggles_service_worker_pause_on_start() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    assert!(!service_worker_pause_on_start(&ctx));

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
    assert!(service_worker_pause_on_start(&ctx));

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
    assert!(!service_worker_pause_on_start(&ctx));
}

#[tokio::test]
async fn target_auto_attach_wait_for_debugger_respects_service_worker_filter() {
    let mut ctx = TestContext::new();
    load_service_worker_target(&mut ctx, "SID-service-worker");
    assert!(!service_worker_pause_on_start(&ctx));

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
        !service_worker_pause_on_start(&ctx),
        "waitForDebuggerOnStart must not pause Service Workers when the target filter excludes them"
    );
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);
}

#[tokio::test]
async fn service_worker_pause_on_start_waits_for_last_owner() {
    let mut ctx = TestContext::new();
    load_unattached_service_worker_target(&mut ctx);
    assert!(!service_worker_pause_on_start(&ctx));
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);

    crate::domains::target::set_service_worker_pause_on_start_owner(&mut ctx.conn, None, true);
    assert!(service_worker_pause_on_start(&ctx));
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 1);

    crate::domains::target::set_service_worker_pause_on_start_owner(
        &mut ctx.conn,
        Some("SID-service-worker-owner"),
        true,
    );
    assert!(service_worker_pause_on_start(&ctx));
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 2);

    crate::domains::target::set_service_worker_pause_on_start_owner(&mut ctx.conn, None, false);
    assert!(
        service_worker_pause_on_start(&ctx),
        "one remaining DevTools owner must keep new Service Worker targets paused"
    );
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 1);

    crate::domains::target::set_service_worker_pause_on_start_owner(
        &mut ctx.conn,
        Some("SID-service-worker-owner"),
        false,
    );
    assert!(!service_worker_pause_on_start(&ctx));
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
        Some("TID-service-worker:native-realm-91007")
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
                    == Some("TID-service-worker:native-realm-91007")
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

async fn real_service_worker_session() -> (TestContext, String, String, tokio::task::JoinHandle<()>)
{
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

    (
        ctx,
        service_worker_session_id,
        format!("http://{addr}/"),
        server,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_on_real_service_worker_session_enters_worker_global() {
    let (mut ctx, service_worker_session_id, scope, server) = real_service_worker_session().await;
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
        json!(format!("service:true:{scope}"))
    );

    server.abort();
}

async fn restart_service_worker_session(ctx: &mut TestContext, session_id: &str, scope: &str) {
    let version_id = ctx
        .conn
        .service_worker_target_for_session(Some(session_id))
        .expect("attached worker")
        .renderer_version_id
        .to_string();
    ctx.process_async(json!({"id": 90, "method": "ServiceWorker.enable", "sessionId": "SID-page"}))
        .await;
    ctx.expect_result(90, json!({}), Some("SID-page"));
    ctx.process_async(
        json!({"id": 91, "method": "ServiceWorker.stopWorker", "sessionId": "SID-page",
        "params": {"versionId": version_id}}),
    )
    .await;
    ctx.expect_result(91, json!({}), Some("SID-page"));
    wait_until_message(ctx, Some("SID-page"), "the exact worker stops", |message| {
        message["method"] == "ServiceWorker.workerVersionUpdated"
            && message["params"]["versions"]
                .as_array()
                .is_some_and(|versions| {
                    versions.iter().any(|version| {
                        version["versionId"] == version_id && version["runningStatus"] == "stopped"
                    })
                })
    })
    .await;
    ctx.sent.clear();
    ctx.process_async(
        json!({"id": 92, "method": "ServiceWorker.startWorker", "sessionId": "SID-page",
        "params": {"scopeURL": scope}}),
    )
    .await;
    ctx.expect_result(92, json!({}), Some("SID-page"));
    wait_until_message(
        ctx,
        Some("SID-page"),
        "the same version starts a new run",
        |message| {
            message["method"] == "ServiceWorker.workerVersionUpdated"
                && message["params"]["versions"]
                    .as_array()
                    .is_some_and(|versions| {
                        versions.iter().any(|version| {
                            version["versionId"] == version_id
                                && version["runningStatus"] == "running"
                        })
                    })
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_service_worker_inspection_never_dispatches_to_a_restarted_run() {
    let (mut ctx, session_id, scope, server) = real_service_worker_session().await;
    let old_target = ctx
        .conn
        .service_worker_target_for_session(Some(&session_id))
        .expect("worker")
        .inspection_target()
        .expect("live run");
    let old_endpoint = ctx
        .conn
        .worker_inspection_endpoint_for_session(Some(&session_id))
        .expect("live inspection endpoint");
    let disposal = ctx
        .conn
        .session_disposal_plan(&session_id)
        .expect("disposal plan");
    let disposal_endpoint =
        crate::domains::runtime::worker_inspection_endpoint_for_disposal(&ctx.conn, &disposal)
            .expect("capture the worker before async session cleanup");
    let pending = ctx
        .conn
        .start_worker_runtime_protocol_message_for_session(
            Some(&session_id),
            json!({"id": 9001, "method": "Runtime.evaluate", "params": {
                "expression": "globalThis.__staleWorkerDispatch = true", "returnByValue": true
            }})
            .to_string(),
        )
        .expect("capture the original worker before polling dispatch");

    restart_service_worker_session(&mut ctx, &session_id, &scope).await;
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("fixture context")
            .worker_inspection_endpoint(old_target)
            .is_none(),
        "the old run identity must not bind the restarted version"
    );
    assert!(!old_endpoint.attach_session(Some(session_id.clone())));
    assert!(!old_endpoint.detach_session(Some(session_id.clone())));
    assert!(!disposal_endpoint.detach_session(Some(session_id.clone())));

    let stale_dispatch = pending.wait().await;
    ctx.process_async(json!({"id": 93, "method": "Runtime.evaluate", "sessionId": session_id,
        "params": {"expression": "globalThis.__staleWorkerDispatch === undefined", "returnByValue": true}})).await;
    let probe = take_response_by_id(&mut ctx, 93);
    server.abort();
    assert_eq!(
        probe["result"]["result"]["value"], true,
        "a captured inspection command must not mutate the replacement run: {probe}"
    );
    assert!(
        stale_dispatch.is_err(),
        "the retired endpoint must reject the captured command"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_service_worker_frontend_inspection_never_dispatches_to_a_restarted_run() {
    let (mut ctx, session_id, scope, server) = real_service_worker_session().await;
    let raw = json!({"id": 9002, "method": "Runtime.evaluate", "sessionId": session_id,
        "params": {"expression": "globalThis.__staleWorkerDispatch = true", "returnByValue": true}
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("capture a real frontend Worker command before polling");
    restart_service_worker_session(&mut ctx, &session_id, &scope).await;
    let completed = pending.wait().await;
    assert!(
        matches!(
            ctx.conn.complete_pending_command_dispatch(completed).await,
            CdpCommandTaskStep::Complete(_)
        ),
        "retired frontend inspection must complete without replay"
    );
    ctx.process_async(json!({"id": 94, "method": "Runtime.evaluate", "sessionId": session_id,
        "params": {"expression": "globalThis.__staleWorkerDispatch === undefined", "returnByValue": true}})).await;
    let probe = take_response_by_id(&mut ctx, 94);
    server.abort();
    assert_eq!(
        probe["result"]["result"]["value"], true,
        "the old frontend command must not mutate the replacement run: {probe}"
    );
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
