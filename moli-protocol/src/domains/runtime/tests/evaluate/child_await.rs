use super::*;

async fn child_context(ctx: &mut TestContext) -> i64 {
    with_loaded_document_async(
        ctx,
        r#"<!doctype html><title>parent</title><iframe srcdoc="<title>child</title><body>child realm</body>"></iframe>"#,
    )
    .await;
    ctx.process_async(json!({"id": 12_001, "method": "Page.getFrameTree"}))
        .await;
    let frame_id =
        take_response_by_id(ctx, 12_001)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .expect("the iframe must have its own browsing context")
            .to_owned();
    ctx.process_async(json!({
        "id": 12_002, "method": "Page.createIsolatedWorld",
        "params": {"frameId": frame_id, "worldName": "child-await-materialization"}
    }))
    .await;
    assert!(take_response_by_id(ctx, 12_002)["result"]["executionContextId"].is_i64());
    ctx.process_async(json!({"id": 12_003, "method": "Runtime.enable"}))
        .await;
    assert_eq!(take_response_by_id(ctx, 12_003)["result"], json!({}));
    crate::testing::wait_until_message(ctx, None, "child default execution context", |message| {
        message["method"] == "Runtime.executionContextCreated"
            && message["params"]["context"]["auxData"]["frameId"] == frame_id
            && message["params"]["context"]["auxData"]["isDefault"] == true
    })
    .await;
    ctx.sent
        .iter()
        .find(|message| {
            message["method"] == "Runtime.executionContextCreated"
                && message["params"]["context"]["auxData"]["frameId"] == frame_id
                && message["params"]["context"]["auxData"]["isDefault"] == true
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("Runtime.enable must report the actual child default context")
}

#[tokio::test]
async fn child_runtime_evaluate_timer_reply_follows_console_on_owner_turns() {
    let mut ctx = TestContext::new();
    let context_id = child_context(&mut ctx).await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 12_010, "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id, "returnByValue": true, "awaitPromise": true,
            "expression": "new Promise(resolve => setTimeout(() => { console.log('child-await-output'); resolve({value:42, title:document.title, child:window!==top}); }, 20))"
        }
    }))
    .await;
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "child timer-backed Inspector reply",
        |message| message["id"] == 12_010,
    )
    .await;
    let console = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == "Runtime.consoleAPICalled"
                && message["params"]["executionContextId"] == context_id
                && message["params"]["args"][0]["value"] == "child-await-output"
        })
        .expect("the real child timer must run without a follow-up evaluation");
    let terminal = ctx
        .sent
        .iter()
        .position(|message| message["id"] == 12_010)
        .unwrap();
    assert!(
        console < terminal,
        "concrete output must precede the response: {:?}",
        ctx.sent
    );
    let response = take_response_by_id(&mut ctx, 12_010);
    assert_eq!(
        response["result"]["result"]["value"],
        json!({"value":42, "title":"child", "child":true})
    );
    ctx.with_fixture(|conn| assert!(!conn.has_pending_inspector_awaits()));
}

#[tokio::test]
async fn child_runtime_evaluate_preserves_native_exception_details() {
    let mut ctx = TestContext::new();
    let context_id = child_context(&mut ctx).await;
    for (id, expression, await_promise) in [
        (
            12_020,
            "throw new TypeError('child-await-rejection')",
            false,
        ),
        (
            12_021,
            "new Promise((resolve,reject) => setTimeout(() => reject(new TypeError('child-await-rejection')), 20))",
            true,
        ),
    ] {
        ctx.process_async(json!({
            "id": id, "method": "Runtime.evaluate",
            "params": {"contextId":context_id, "expression":expression,
                       "returnByValue":true, "awaitPromise":await_promise}
        }))
        .await;
        let response = wait_for_response_by_id_async(&mut ctx, None, id).await;
        assert!(response.get("error").is_none(), "{response:?}");
        let details = &response["result"]["exceptionDetails"];
        assert_eq!(
            details["exception"]["className"], "TypeError",
            "{response:?}"
        );
        assert!(
            details["exception"]["description"]
                .as_str()
                .unwrap()
                .contains("child-await-rejection")
        );
        assert!(details["exceptionId"].is_number(), "{response:?}");
        assert!(
            !details["stackTrace"]["callFrames"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{response:?}"
        );
        ctx.with_fixture(|conn| assert!(!conn.has_pending_inspector_awaits()));
    }
}

#[tokio::test]
async fn child_runtime_evaluate_pending_promise_releases_page_dispatch() {
    let mut ctx = TestContext::new();
    let context_id = child_context(&mut ctx).await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 12_030, "method": "Runtime.evaluate",
        "params": {
            "contextId":context_id, "returnByValue":true, "awaitPromise":true,
            "expression":"new Promise(resolve => { globalThis.resolveChildAwait = resolve; console.log('child-await-pending'); })"
        }
    }))
    .await;
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "unresolved child expression started",
        |message| {
            message["method"] == "Runtime.consoleAPICalled"
                && message["params"]["args"][0]["value"] == "child-await-pending"
        },
    )
    .await;
    assert!(!ctx.sent.iter().any(|message| message["id"] == 12_030));
    ctx.with_fixture(|conn| {
        assert!(conn.has_unclaimed_pending_inspector_awaits_for_session_owner(None))
    });
    ctx.with_fixture(|conn| {
        assert!(!conn.has_claimed_pending_inspector_awaits_for_session_owner(None))
    });
    ctx.process_async(json!({
        "id":12_031, "method":"Runtime.evaluate",
        "params":{"expression":"document.title", "returnByValue":true}
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 12_031)["result"]["result"]["value"],
        "parent"
    );
    assert!(!ctx.sent.iter().any(|message| message["id"] == 12_030));
    ctx.process_async(json!({
        "id":12_032, "method":"Runtime.evaluate",
        "params":{"contextId":context_id, "expression":"resolveChildAwait(42)", "returnByValue":true}
    }))
    .await;
    let response = wait_for_response_by_id_async(&mut ctx, None, 12_030).await;
    assert_eq!(response["result"]["result"]["value"], 42);
    ctx.with_fixture(|conn| assert!(!conn.has_pending_inspector_awaits()));
}

#[tokio::test]
async fn child_runtime_evaluate_frame_retirement_rejects_pending_and_stale_requests() {
    let mut ctx = TestContext::new();
    let context_id = child_context(&mut ctx).await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id":12_040, "method":"Runtime.evaluate",
        "params":{"contextId":context_id, "returnByValue":true, "awaitPromise":true,
                  "expression":"new Promise(() => { console.log('child-await-retire'); })"}
    }))
    .await;
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "child expression before retirement",
        |message| {
            message["method"] == "Runtime.consoleAPICalled"
                && message["params"]["args"][0]["value"] == "child-await-retire"
        },
    )
    .await;
    assert!(!ctx.sent.iter().any(|message| message["id"] == 12_040));
    ctx.process_async(json!({
        "id":12_041, "method":"Runtime.evaluate",
        "params":{"expression":"document.querySelector('iframe').remove()", "returnByValue":true}
    }))
    .await;
    let response = wait_for_response_by_id_async(&mut ctx, None, 12_040).await;
    assert_eq!(response["error"]["code"], -32000, "{response:?}");
    assert!(
        !response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("outside an owner continuation")
    );
    ctx.with_fixture(|conn| assert!(!conn.has_pending_inspector_awaits()));
    ctx.process_async(json!({
        "id":12_042, "method":"Runtime.evaluate",
        "params":{"contextId":context_id, "expression":"42", "returnByValue":true}
    }))
    .await;
    let stale = take_response_by_id(&mut ctx, 12_042);
    assert_eq!(stale["error"]["code"], -32000);
    assert!(
        !ctx.sent.iter().any(|message| message["id"] == 12_040),
        "retirement must have a single terminal"
    );
}
