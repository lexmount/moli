use super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::*;
use crate::testing::{wait_until_message, wait_until_messages};
use serde_json::Map;

async fn attached_smoke_session(ctx: &mut TestContext, base: u64) -> AttachedPageSession {
    create_attached_page_session_async(ctx, base, base + 1, base + 2, base + 3, base + 4).await
}

async fn navigate_and_take_response(
    ctx: &mut TestContext,
    session_id: &str,
    id: u64,
    url: String,
) -> Value {
    ctx.process_async(json!({
        "id": id,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    take_response_by_id(ctx, id)
}

fn find_dom_node<'a>(node: &'a Value, predicate: &impl Fn(&Value) -> bool) -> Option<&'a Value> {
    if predicate(node) {
        return Some(node);
    }
    node.get("children")
        .and_then(Value::as_array)
        .and_then(|children| {
            children
                .iter()
                .find_map(|child| find_dom_node(child, predicate))
        })
}

fn find_dom_node_in_array<'a>(
    nodes: &'a Value,
    predicate: &impl Fn(&Value) -> bool,
) -> Option<&'a Value> {
    nodes
        .as_array()
        .and_then(|nodes| nodes.iter().find_map(|node| find_dom_node(node, predicate)))
}

fn attributes_to_map(attributes: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(attributes) = attributes.as_array() else {
        return out;
    };
    for pair in attributes.chunks(2) {
        if let [name, value] = pair
            && let Some(name) = name.as_str()
        {
            out.insert(name.to_owned(), value.clone());
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_page_lifecycle_order() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 71_000).await;

    ctx.process_async(json!({
        "id": 71_005,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(71_005, json!({}), Some(&attached.session_id));

    let navigation = navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        71_006,
        fixture.url("/chromium-cdp-lifecycle-page"),
    )
    .await;
    assert_eq!(navigation["sessionId"], attached.session_id);
    wait_until_messages(
        &mut ctx,
        Some(attached.session_id.as_str()),
        "Chromium lifecycle DCL, load, and stopped-loading sequence",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.domContentEventFired")
                    && message["sessionId"] == json!(attached.session_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Page.loadEventFired")
                    && message["sessionId"] == json!(attached.session_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Page.frameStoppedLoading")
                    && message["sessionId"] == json!(attached.session_id)
            })
        },
    )
    .await;

    let methods = ctx
        .sent
        .iter()
        .filter(|message| message["sessionId"] == json!(attached.session_id))
        .filter_map(|message| message["method"].as_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let dom = methods
        .iter()
        .position(|method| method == "Page.domContentEventFired")
        .expect("DOMContentLoaded event");
    let load = methods
        .iter()
        .position(|method| method == "Page.loadEventFired")
        .expect("load event");
    assert!(dom < load, "event order: {methods:?}");

    let started = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.frameStartedLoading")
        })
        .expect("frameStartedLoading");
    let stopped = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.frameStoppedLoading")
        })
        .expect("frameStoppedLoading");
    assert_eq!(started["params"]["frameId"], attached.target_id);
    assert_eq!(stopped["params"]["frameId"], attached.target_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_page_frame_tree_includes_parser_iframe() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 72_000).await;
    ctx.process_async(json!({
        "id": 72_005,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(72_005, json!({}), Some(&attached.session_id));
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        72_006,
        fixture.url("/iframe"),
    )
    .await;
    wait_until_message(
        &mut ctx,
        attached.session_id.as_str(),
        "parser child frame navigation",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/child"))
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 72_007,
        "method": "Page.getFrameTree",
        "sessionId": attached.session_id
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 72_007);
    let root = &frame_tree["result"]["frameTree"];
    assert_eq!(root["frame"]["id"], attached.target_id);
    assert!(
        root["frame"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("/iframe"))
    );
    let child_urls = root["childFrames"]
        .as_array()
        .expect("child frames")
        .iter()
        .filter_map(|child| child["frame"]["url"].as_str())
        .collect::<Vec<_>>();
    assert!(
        child_urls.iter().any(|url| url.ends_with("/child")),
        "{child_urls:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_page_fragment_navigation_keeps_frame() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 73_000).await;
    let first_url = fixture.url("/plain");
    let first = navigate_and_take_response(&mut ctx, &attached.session_id, 73_005, first_url).await;
    let first_frame = first["result"]["frameId"].clone();
    assert!(first_frame.is_string());

    let fragment_url = fixture.url("/plain#fragment");
    let second =
        navigate_and_take_response(&mut ctx, &attached.session_id, 73_006, fragment_url.clone())
            .await;
    assert!(second["result"]["errorText"].is_null());
    if second["result"]["frameId"].is_string() {
        assert_eq!(second["result"]["frameId"], first_frame);
    }

    ctx.process_async(json!({
        "id": 73_007,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": { "expression": "location.href", "returnByValue": true }
    }))
    .await;
    let location = take_response_by_id(&mut ctx, 73_007);
    assert_eq!(location["result"]["result"]["value"], fragment_url);
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_page_layout_metrics_are_populated() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 74_000).await;
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        74_005,
        fixture.url("/chromium-cdp-layout-page"),
    )
    .await;

    ctx.capture_fixture_layout(Some(&attached.session_id)).await;
    ctx.process_async(json!({
        "id": 74_006,
        "method": "Page.getLayoutMetrics",
        "sessionId": attached.session_id
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 74_006);
    let content_width = metrics["result"]["cssContentSize"]["width"]
        .as_f64()
        .expect("content width");
    let content_height = metrics["result"]["cssContentSize"]["height"]
        .as_f64()
        .expect("content height");
    let layout_width = metrics["result"]["cssLayoutViewport"]["clientWidth"]
        .as_f64()
        .expect("layout viewport width");
    let layout_height = metrics["result"]["cssLayoutViewport"]["clientHeight"]
        .as_f64()
        .expect("layout viewport height");
    assert!(content_width >= 10_000.0, "metrics: {metrics}");
    assert!(content_height >= 10_000.0, "metrics: {metrics}");
    assert!(content_width > layout_width, "metrics: {metrics}");
    assert!(content_height > layout_height, "metrics: {metrics}");
    assert!(layout_width > 0.0, "metrics: {metrics}");
    assert!(layout_height > 0.0, "metrics: {metrics}");
    assert!(
        metrics["result"]["cssVisualViewport"]["clientHeight"]
            .as_f64()
            .unwrap_or(0.0)
            > 0.0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_shared_worker_target_and_runtime_smoke() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    ctx.conn.set_target_discovery_for_owner(
        None,
        crate::conn::CdpTargetFilter::default_target_discovery(),
    );
    let attached = attached_smoke_session(&mut ctx, 75_000).await;

    ctx.process_async(json!({
        "id": 75_005,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(75_005, json!({}), None);
    ctx.take_all();

    let page_url = fixture.url("/shared-worker-smoke");
    let worker_url = fixture.url("/shared-worker-smoke.js");
    let navigation =
        navigate_and_take_response(&mut ctx, &attached.session_id, 75_006, page_url).await;
    assert_eq!(navigation["result"]["frameId"], attached.target_id);

    wait_until_message(&mut ctx, None, "shared worker targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["type"] == json!("shared_worker")
            && message["params"]["targetInfo"]["url"] == json!(worker_url)
    })
    .await;
    let created = ctx.take_first_matching("shared worker targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["type"] == json!("shared_worker")
            && message["params"]["targetInfo"]["url"] == json!(worker_url)
    });
    let shared_worker_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("shared worker target id")
        .to_owned();
    assert_eq!(
        created["params"]["targetInfo"]["title"],
        json!("shared-worker-smoke")
    );
    assert_eq!(created["params"]["targetInfo"]["attached"], json!(false));

    wait_until_message(
        &mut ctx,
        None,
        "shared worker attachedToTarget",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(shared_worker_target_id)
        },
    )
    .await;
    let attached_worker = ctx.take_first_matching("shared worker attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(shared_worker_target_id)
    });
    let shared_worker_session_id = attached_worker["params"]["sessionId"]
        .as_str()
        .expect("shared worker session id")
        .to_owned();
    assert_eq!(
        attached_worker["params"]["targetInfo"]["attached"],
        json!(true)
    );

    ctx.process_async(json!({
        "id": 75_007,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 75_007);
    assert_eq!(enable_response["sessionId"], shared_worker_session_id);
    assert_eq!(enable_response["result"], json!({}));

    let context_event = ctx.take_first_matching("shared worker execution context", |message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["sessionId"] == json!(shared_worker_session_id)
    });
    assert!(
        context_event["params"]["context"]["id"].as_i64().is_some(),
        "shared worker Runtime.enable should replay an execution context: {context_event:?}"
    );
    if let Some(aux_type) = context_event["params"]["context"]["auxData"]["type"].as_str() {
        assert_eq!(aux_type, "worker");
    }

    ctx.process_async(json!({
        "id": 75_008,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": r#"({
                name,
                pathname: self.location.pathname,
                selfEqualsGlobal: self === globalThis,
                isSharedWorker:
                    typeof SharedWorkerGlobalScope !== "undefined" &&
                    self instanceof SharedWorkerGlobalScope,
                boot: globalThis.__sharedWorkerSmoke,
            })"#,
            "returnByValue": true
        }
    }))
    .await;
    let probe_response = take_response_by_id(&mut ctx, 75_008);
    let probe = &probe_response["result"]["result"]["value"];
    assert_eq!(probe["name"], json!("shared-worker-smoke"));
    assert_eq!(probe["pathname"], json!("/shared-worker-smoke.js"));
    assert_eq!(probe["selfEqualsGlobal"], json!(true));
    assert_eq!(probe["isSharedWorker"], json!(true));
    assert_eq!(probe["boot"]["name"], json!("shared-worker-smoke"));
    assert_eq!(probe["boot"]["isSharedWorker"], json!(true));
    assert_eq!(probe["boot"]["connectCount"], json!(1));

    ctx.process_async(json!({
        "id": 75_009,
        "method": "Target.getTargets"
    }))
    .await;
    let initial_targets = take_response_by_id(&mut ctx, 75_009);
    let shared_worker_targets = initial_targets["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos")
        .iter()
        .filter(|target| target["type"] == json!("shared_worker"))
        .collect::<Vec<_>>();
    assert_eq!(
        shared_worker_targets.len(),
        1,
        "same-name shared worker should expose one CDP target: {initial_targets:?}"
    );
    assert_eq!(
        shared_worker_targets[0]["targetId"],
        json!(&shared_worker_target_id)
    );
    assert_eq!(shared_worker_targets[0]["attached"], json!(true));

    ctx.take_all();
    ctx.process_and_wait_for_response_async(json!({
        "id": 75_010,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": r#"
                new Promise((resolve, reject) => {
                    const timer = setTimeout(
                        () => reject(new Error("shared worker second connection timeout")),
                        1000
                    );
                    const worker = new SharedWorker(
                        "/shared-worker-smoke.js",
                        "shared-worker-smoke"
                    );
                    globalThis.__sharedWorkerSmokeSecondWorker = worker;
                    worker.port.onmessage = event => {
                        if (event.data && event.data.kind === "probe-result") {
                            clearTimeout(timer);
                            resolve(event.data);
                        }
                    };
                    worker.port.start();
                    worker.port.postMessage({
                        kind: "probe",
                        value: "second-page-probe"
                    });
                })
            "#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let second_probe_response = take_response_by_id(&mut ctx, 75_010);
    let second_probe = &second_probe_response["result"]["result"]["value"];
    assert_eq!(second_probe["echoed"], json!("second-page-probe"));
    assert_eq!(second_probe["name"], json!("shared-worker-smoke"));
    assert_eq!(second_probe["connectCount"], json!(2));
    assert_eq!(second_probe["isSharedWorker"], json!(true));

    ctx.process_async(json!({
        "id": 75_011,
        "method": "Target.getTargets"
    }))
    .await;
    let repeated_targets = take_response_by_id(&mut ctx, 75_011);
    let repeated_shared_worker_targets = repeated_targets["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos")
        .iter()
        .filter(|target| target["type"] == json!("shared_worker"))
        .collect::<Vec<_>>();
    assert_eq!(
        repeated_shared_worker_targets.len(),
        1,
        "reconnecting to the same shared worker must not create another CDP target: {repeated_targets:?}"
    );
    assert_eq!(
        repeated_shared_worker_targets[0]["targetId"],
        json!(&shared_worker_target_id)
    );
    let unexpected_created = ctx
        .take_all()
        .into_iter()
        .filter(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["type"] == json!("shared_worker")
        })
        .collect::<Vec<_>>();
    assert!(
        unexpected_created.is_empty(),
        "same-name SharedWorker reconnect should not emit a second targetCreated: {unexpected_created:?}"
    );

    ctx.process_async(json!({
        "id": 75_012,
        "method": "Target.closeTarget",
        "params": { "targetId": &shared_worker_target_id }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 75_012)["result"],
        json!({ "success": true })
    );
    ctx.take_first_matching("shared worker detached after closeTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(shared_worker_target_id)
            && message["params"]["sessionId"] == json!(shared_worker_session_id)
    });
    ctx.take_first_matching("shared worker destroyed after closeTarget", |message| {
        message["method"] == json!("Target.targetDestroyed")
            && message["params"]["targetId"] == json!(shared_worker_target_id)
    });

    ctx.process_async(json!({
        "id": 75_013,
        "method": "Target.getTargets"
    }))
    .await;
    let closed_targets = take_response_by_id(&mut ctx, 75_013);
    assert!(
        closed_targets["result"]["targetInfos"]
            .as_array()
            .expect("targetInfos")
            .iter()
            .all(|target| target["type"] != json!("shared_worker")),
        "Target.closeTarget should remove the shared worker target: {closed_targets:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_dedicated_worker_target_runtime_and_lifecycle_smoke() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    ctx.conn.set_target_discovery_for_owner(
        None,
        crate::conn::CdpTargetFilter::default_target_discovery(),
    );
    let attached = attached_smoke_session(&mut ctx, 75_100).await;
    let page_url = fixture.url("/chromium-cdp-lifecycle-page");
    let worker_url = fixture.url("/dedicated-worker-smoke.js");
    navigate_and_take_response(&mut ctx, &attached.session_id, 75_105, page_url).await;

    // Chromium's browser/root auto-attach does not claim page-owned dedicated
    // workers. The owning page session receives the relational worker target.
    ctx.process_async(json!({
        "id": 75_106,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [{ "type": "worker", "exclude": false }]
        }
    }))
    .await;
    ctx.expect_result(75_106, json!({}), None);
    ctx.process_async(json!({
        "id": 75_107,
        "method": "Target.setAutoAttach",
        "sessionId": attached.session_id,
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [{ "type": "worker", "exclude": false }]
        }
    }))
    .await;
    ctx.expect_result(75_107, json!({}), Some(&attached.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 75_108,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "globalThis.__dedicatedWorkerSmokeWorker = new Worker('/dedicated-worker-smoke.js', { name: 'parser' }); true",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 75_108)["result"]["result"]["value"],
        json!(true)
    );

    wait_until_message(
        &mut ctx,
        None,
        "dedicated worker attachedToTarget",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["type"] == json!("worker")
                && message["params"]["targetInfo"]["url"] == json!(worker_url)
        },
    )
    .await;
    let worker_messages = ctx.take_all();
    let created = worker_messages
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["type"] == json!("worker")
                && message["params"]["targetInfo"]["url"] == json!("")
        })
        .expect("DedicatedWorker discovery event");
    let worker_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("DedicatedWorker target id")
        .to_owned();
    assert_eq!(created["params"]["targetInfo"]["title"], json!(""));
    assert_eq!(created["params"]["targetInfo"]["attached"], json!(false));
    let changed = worker_messages
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(worker_target_id)
        })
        .expect("DedicatedWorker targetInfoChanged event");
    assert_eq!(changed["params"]["targetInfo"]["title"], json!("parser"));
    assert_eq!(changed["params"]["targetInfo"]["url"], json!(worker_url));
    assert_eq!(changed["params"]["targetInfo"]["attached"], json!(true));
    let attached_worker_messages = worker_messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(worker_target_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attached_worker_messages.len(),
        1,
        "only the owning page session may auto-attach a DedicatedWorker: {worker_messages:?}"
    );
    let attached_worker = attached_worker_messages[0];
    assert_eq!(attached_worker["sessionId"], json!(attached.session_id));
    assert_eq!(attached_worker["params"]["waitingForDebugger"], json!(true));
    let worker_session_id = attached_worker["params"]["sessionId"]
        .as_str()
        .expect("DedicatedWorker session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 75_109,
        "method": "Runtime.enable",
        "sessionId": worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 75_109);
    assert_eq!(enable_response["sessionId"], json!(worker_session_id));
    assert_eq!(enable_response["result"], json!({}));
    let context_event = ctx.take_first_matching(
        "DedicatedWorker execution context before script bootstrap",
        |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!(worker_session_id)
        },
    );
    assert_eq!(
        context_event["params"]["context"]["auxData"]["type"],
        json!("worker")
    );

    ctx.process_async(json!({
        "id": 75_110,
        "method": "Runtime.evaluate",
        "sessionId": worker_session_id,
        "params": {
            "expression": "typeof globalThis.__dedicatedWorkerSmoke",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 75_110)["result"]["result"]["value"],
        json!("undefined"),
        "worker top-level script must remain paused"
    );

    ctx.process_async(json!({
        "id": 75_111,
        "method": "Runtime.runIfWaitingForDebugger",
        "sessionId": worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 75_111)["result"], json!({}));
    wait_until_message(
        &mut ctx,
        None,
        "DedicatedWorker workerScriptLoaded",
        |message| {
            message["method"] == json!("Inspector.workerScriptLoaded")
                && message["sessionId"] == json!(worker_session_id)
        },
    )
    .await;
    let bootstrap_messages = ctx.take_all();
    let console_index = bootstrap_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["sessionId"] == json!(worker_session_id)
                && message["params"]["args"][0]["value"]
                    == json!("dedicated worker smoke boot:parser")
        })
        .expect("DedicatedWorker console event");
    let script_loaded_index = bootstrap_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Inspector.workerScriptLoaded")
                && message["sessionId"] == json!(worker_session_id)
                && message["params"] == json!({})
        })
        .expect("DedicatedWorker workerScriptLoaded event");
    assert!(
        console_index < script_loaded_index,
        "Chromium flushes top-level script output before workerScriptLoaded: {bootstrap_messages:?}"
    );
    ctx.process_async(json!({
        "id": 75_112,
        "method": "Runtime.evaluate",
        "sessionId": worker_session_id,
        "params": {
            "expression": "globalThis.__dedicatedWorkerSmoke",
            "returnByValue": true
        }
    }))
    .await;
    let probe_response = take_response_by_id(&mut ctx, 75_112);
    let probe = &probe_response["result"]["result"]["value"];
    assert_eq!(probe["name"], json!("parser"));
    assert_eq!(probe["pathname"], json!("/dedicated-worker-smoke.js"));
    assert_eq!(probe["selfEqualsGlobal"], json!(true));
    assert_eq!(probe["isDedicatedWorker"], json!(true));

    ctx.process_async(json!({
        "id": 75_113,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "globalThis.__dedicatedWorkerSmokeWorker.terminate(); true",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 75_113)["result"]["result"]["value"],
        json!(true)
    );
    wait_until_message(
        &mut ctx,
        None,
        "DedicatedWorker targetDestroyed",
        |message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(worker_target_id)
        },
    )
    .await;
    let terminal_messages = ctx.take_all();
    let detached_index = terminal_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["sessionId"] == json!(worker_session_id)
                && message["params"]["targetId"] == json!(worker_target_id)
        })
        .expect("DedicatedWorker detachedFromTarget");
    let destroyed_index = terminal_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(worker_target_id)
        })
        .expect("DedicatedWorker targetDestroyed");
    assert!(detached_index < destroyed_index);

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        75_114,
        fixture.url("/chromium-cdp-lifecycle-page?replacement=1"),
    )
    .await;
    ctx.take_all();
    ctx.process_async(json!({
        "id": 75_115,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "globalThis.__replacementWorker = new Worker('/dedicated-worker-smoke.js', { name: 'replacement' }); true",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 75_115)["result"]["result"]["value"],
        json!(true)
    );
    wait_until_message(
        &mut ctx,
        None,
        "replacement DedicatedWorker attach",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["type"] == json!("worker")
                && message["params"]["targetInfo"]["title"] == json!("replacement")
        },
    )
    .await;
    let replacement_attached =
        ctx.take_first_matching("replacement DedicatedWorker attach", |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["type"] == json!("worker")
                && message["params"]["targetInfo"]["title"] == json!("replacement")
        });
    assert_eq!(
        replacement_attached["sessionId"],
        json!(attached.session_id)
    );
    assert_eq!(
        replacement_attached["params"]["waitingForDebugger"],
        json!(true),
        "page-owned DedicatedWorker pause policy must survive renderer replacement"
    );
    let replacement_target_id = replacement_attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("replacement DedicatedWorker target id")
        .to_owned();
    let replacement_session_id = replacement_attached["params"]["sessionId"]
        .as_str()
        .expect("replacement DedicatedWorker session id")
        .to_owned();
    ctx.take_all();

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        75_116,
        fixture.url("/chromium-cdp-lifecycle-page?replacement=2"),
    )
    .await;
    wait_until_message(
        &mut ctx,
        None,
        "replacement DedicatedWorker destroyed by navigation",
        |message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(replacement_target_id)
        },
    )
    .await;
    let navigation_terminal_messages = ctx.take_all();
    let navigation_detached_index = navigation_terminal_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["sessionId"] == json!(replacement_session_id)
                && message["params"]["targetId"] == json!(replacement_target_id)
        })
        .expect("navigation must detach the old-document DedicatedWorker session");
    let navigation_destroyed_index = navigation_terminal_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(replacement_target_id)
        })
        .expect("navigation must destroy the old-document DedicatedWorker target");
    assert!(navigation_detached_index < navigation_destroyed_index);
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_runtime_evaluate_and_exception_contract() {
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 76_000).await;

    ctx.process_async(json!({
        "id": 76_005,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "({ answer: 42, nested: { ok: true }, list: [1, 2] })",
            "returnByValue": true
        }
    }))
    .await;
    let value = take_response_by_id(&mut ctx, 76_005);
    assert_eq!(
        value["result"]["result"]["value"],
        json!({"answer": 42, "nested": {"ok": true}, "list": [1, 2]})
    );

    ctx.process_async(json!({
        "id": 76_006,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": { "expression": "(() => { throw new Error('chromium sample throw'); })()" }
    }))
    .await;
    let thrown = take_response_by_id(&mut ctx, 76_006);
    assert!(
        thrown["result"]["exceptionDetails"]
            .to_string()
            .contains("chromium sample throw")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_performance_enable_and_metrics_contract() {
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 77_000).await;

    ctx.process_async(json!({
        "id": 77_004,
        "method": "Performance.getMetrics",
        "sessionId": attached.session_id
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 77_004)["result"]["metrics"],
        json!([])
    );

    for id in [77_005, 77_006] {
        ctx.process_async(json!({
            "id": id,
            "method": "Performance.enable",
            "sessionId": attached.session_id,
            "params": { "timeDomain": "threadTicks" }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(&attached.session_id));
    }

    ctx.process_async(json!({
        "id": 77_007,
        "method": "Performance.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_error(
        77_007,
        -32000,
        "Cannot change time domain while performance metrics collection is enabled.",
    );
    ctx.process_async(json!({
        "id": 77_008,
        "method": "Performance.setTimeDomain",
        "sessionId": attached.session_id,
        "params": { "timeDomain": "timeTicks" }
    }))
    .await;
    ctx.expect_error(
        77_008,
        -32000,
        "Cannot set time domain while performance metrics collection is enabled.",
    );

    ctx.process_async(json!({
        "id": 77_009,
        "method": "Performance.disable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(77_009, json!({}), Some(&attached.session_id));
    for (id, method, value) in [
        (77_010, "Performance.enable", "bogusTicks"),
        (77_011, "Performance.enable", "TimeTicks"),
        (77_012, "Performance.setTimeDomain", "bogusTicks"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": attached.session_id,
            "params": { "timeDomain": value }
        }))
        .await;
        ctx.expect_error(id, -32000, "Invalid time domain specification.");
    }

    ctx.process_async(json!({
        "id": 77_013,
        "method": "Performance.setTimeDomain",
        "sessionId": attached.session_id,
        "params": { "timeDomain": "threadTicks" }
    }))
    .await;
    ctx.expect_result(77_013, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 77_014,
        "method": "Performance.enable",
        "sessionId": attached.session_id,
        "params": { "timeDomain": null }
    }))
    .await;
    ctx.expect_result(77_014, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 77_020,
        "method": "Performance.getMetrics",
        "sessionId": attached.session_id
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 77_020);
    let names = metrics["result"]["metrics"]
        .as_array()
        .expect("metrics array")
        .iter()
        .filter_map(|metric| metric["name"].as_str())
        .collect::<Vec<_>>();
    for required in [
        "Timestamp",
        "Documents",
        "Frames",
        "Nodes",
        "LayoutCount",
        "RecalcStyleCount",
        "LayoutDuration",
        "RecalcStyleDuration",
        "ScriptDuration",
        "TaskDuration",
        "JSHeapUsedSize",
        "JSHeapTotalSize",
    ] {
        assert!(names.contains(&required), "missing {required}: {names:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_dom_get_attributes_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 78_000).await;
    let navigation = navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        78_005,
        fixture.url("/chromium-cdp-dom-page"),
    )
    .await;
    let loader_id = navigation["result"]["loaderId"]
        .as_str()
        .expect("navigation loader id");
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some(&attached.session_id),
        &attached.target_id,
        loader_id,
    )
    .await;

    ctx.process_async(json!({
        "id": 78_006,
        "method": "DOM.getDocument",
        "sessionId": attached.session_id,
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 78_006);
    let root = &document["result"]["root"];
    let paragraph = find_dom_node(root, &|node| node["nodeName"] == "P").expect("paragraph");

    ctx.process_async(json!({
        "id": 78_007,
        "method": "DOM.getAttributes",
        "sessionId": attached.session_id,
        "params": { "nodeId": paragraph["nodeId"] }
    }))
    .await;
    let attributes = take_response_by_id(&mut ctx, 78_007);
    let attributes = attributes_to_map(&attributes["result"]["attributes"]);
    assert_eq!(attributes.get("class"), Some(&json!("class1")));
    assert_eq!(attributes.get("attr1"), Some(&json!("attr1")));

    ctx.process_async(json!({
        "id": 78_008,
        "method": "DOM.getAttributes",
        "sessionId": attached.session_id,
        "params": { "nodeId": root["nodeId"] }
    }))
    .await;
    let error = take_response_by_id(&mut ctx, 78_008);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["sessionId"], json!(attached.session_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_dom_query_selector_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 79_000).await;
    let navigation = navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        79_005,
        fixture.url("/chromium-cdp-dom-query-page"),
    )
    .await;
    let loader_id = navigation["result"]["loaderId"]
        .as_str()
        .expect("navigation loader id");
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some(&attached.session_id),
        &attached.target_id,
        loader_id,
    )
    .await;

    ctx.process_async(json!({
        "id": 79_006,
        "method": "DOM.getDocument",
        "sessionId": attached.session_id
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 79_006);
    let root = &document["result"]["root"];
    let body = find_dom_node(root, &|node| node["nodeName"] == "BODY").expect("body");
    let body_node_id = body["nodeId"].clone();
    assert!(body.get("children").is_none());
    ctx.take_all();

    ctx.process_async(json!({
        "id": 79_007,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": body_node_id, "selector": "div" }
    }))
    .await;
    let first_messages = ctx.take_all();
    let first_event_index = first_messages
        .iter()
        .position(|message| {
            message["method"] == "DOM.setChildNodes"
                && message["params"]["parentId"] == body_node_id
        })
        .expect("querySelector should publish the body children");
    let first_response_index = first_messages
        .iter()
        .position(|message| message["id"] == 79_007)
        .expect("querySelector response");
    assert!(first_event_index < first_response_index);
    let first_event = &first_messages[first_event_index];
    let first_response = &first_messages[first_response_index];
    let first_node = find_dom_node_in_array(&first_event["params"]["nodes"], &|node| {
        node["nodeId"] == first_response["result"]["nodeId"]
    })
    .expect("query result should be published before the response");
    assert_eq!(
        attributes_to_map(&first_node["attributes"]).get("id"),
        Some(&json!("firstDiv"))
    );
    let second_node = find_dom_node_in_array(&first_event["params"]["nodes"], &|node| {
        attributes_to_map(&node["attributes"]).get("id") == Some(&json!("secondDiv"))
    })
    .expect("body expansion should publish every direct child");
    let second_node_id = second_node["nodeId"].clone();
    let depth_1 = find_dom_node_in_array(&first_event["params"]["nodes"], &|node| {
        attributes_to_map(&node["attributes"]).get("id") == Some(&json!("depth-1"))
    })
    .expect("body expansion should publish depth-1");
    let depth_1_id = depth_1["nodeId"].clone();

    ctx.process_async(json!({
        "id": 79_008,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": body_node_id, "selector": "div#secondDiv" }
    }))
    .await;
    let second_messages = ctx.take_all();
    assert!(
        !second_messages
            .iter()
            .any(|message| message["method"] == "DOM.setChildNodes")
    );
    let second_response = second_messages
        .iter()
        .find(|message| message["id"] == 79_008)
        .expect("second querySelector response");
    assert_eq!(second_response["result"]["nodeId"], second_node_id);

    ctx.process_async(json!({
        "id": 79_009,
        "method": "DOM.querySelectorAll",
        "sessionId": attached.session_id,
        "params": { "nodeId": body_node_id, "selector": "div.testClass" }
    }))
    .await;
    let all_messages = ctx.take_all();
    assert!(
        !all_messages
            .iter()
            .any(|message| message["method"] == "DOM.setChildNodes")
    );
    let all = all_messages
        .iter()
        .find(|message| message["id"] == 79_009)
        .expect("querySelectorAll response");
    assert_eq!(all["result"]["nodeIds"].as_array().unwrap().len(), 5);

    ctx.process_async(json!({
        "id": 79_010,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": body_node_id, "selector": "div#targetDiv" }
    }))
    .await;
    let deep_messages = ctx.take_all();
    let deep_relevant = deep_messages
        .iter()
        .filter(|message| message["method"] == "DOM.setChildNodes" || message["id"] == 79_010)
        .collect::<Vec<_>>();
    assert_eq!(deep_relevant.len(), 3);
    assert_eq!(deep_relevant[0]["params"]["parentId"], depth_1_id);
    let depth_2 = find_dom_node_in_array(&deep_relevant[0]["params"]["nodes"], &|node| {
        attributes_to_map(&node["attributes"]).get("id") == Some(&json!("depth-2"))
    })
    .expect("first path event should publish depth-2");
    assert_eq!(deep_relevant[1]["params"]["parentId"], depth_2["nodeId"]);
    assert_eq!(deep_relevant[2]["id"], 79_010);
    let deep_node = find_dom_node_in_array(&deep_relevant[1]["params"]["nodes"], &|node| {
        node["nodeId"] == deep_relevant[2]["result"]["nodeId"]
    })
    .expect("second path event should publish the query result");
    assert_eq!(
        attributes_to_map(&deep_node["attributes"]).get("id"),
        Some(&json!("targetDiv"))
    );

    ctx.process_async(json!({
        "id": 79_011,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": body_node_id, "selector": "div#targetDiv" }
    }))
    .await;
    let repeated_messages = ctx.take_all();
    assert!(
        !repeated_messages
            .iter()
            .any(|message| message["method"] == "DOM.setChildNodes")
    );
    assert!(
        repeated_messages
            .iter()
            .any(|message| message["id"] == 79_011)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_dom_get_node_for_location_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 80_000).await;
    let navigation = navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        80_005,
        fixture.url("/chromium-cdp-hit-test-page"),
    )
    .await;
    let loader_id = navigation["result"]["loaderId"]
        .as_str()
        .expect("hit-test navigation loader id")
        .to_owned();
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some(attached.session_id.as_str()),
        &attached.target_id,
        &loader_id,
    )
    .await;

    ctx.process_async(json!({
        "id": 80_006,
        "method": "DOM.getDocument",
        "sessionId": attached.session_id
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 80_006);
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document root node id");
    ctx.process_async(json!({
        "id": 80_007,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": root_id, "selector": "#hit-target" }
    }))
    .await;
    let target = take_response_by_id(&mut ctx, 80_007);
    let target_node_id = target["result"]["nodeId"]
        .as_u64()
        .expect("hit target frontend node id");
    ctx.process_async(json!({
        "id": 80_008,
        "method": "DOM.describeNode",
        "sessionId": attached.session_id,
        "params": { "nodeId": target_node_id }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 80_008);
    let expected_backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("hit target backend node id");

    ctx.capture_fixture_layout(Some(&attached.session_id)).await;
    ctx.process_async(json!({
        "id": 80_009,
        "method": "DOM.getNodeForLocation",
        "sessionId": attached.session_id,
        "params": { "x": 10, "y": 10 }
    }))
    .await;
    let hit = take_response_by_id(&mut ctx, 80_009);
    assert_eq!(hit["sessionId"], attached.session_id);
    assert_eq!(hit["result"]["backendNodeId"], expected_backend_node_id);
    assert!(hit["result"]["nodeId"].as_u64().is_some_and(|id| id > 0));
}
