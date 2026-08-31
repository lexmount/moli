use std::sync::Arc;

use serde_json::{Value, json};
use url::Url;

use crate::{conn::BrowserContext, testing::TestContext};

fn take_response_by_id(ctx: &mut TestContext, id: u64) -> Value {
    let index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing response {id}; sent={:?}", ctx.sent));
    ctx.sent.remove(index)
}

fn take_result_by_id(ctx: &mut TestContext, id: u64) -> Value {
    let response = take_response_by_id(ctx, id);
    assert!(
        response.get("error").is_none(),
        "expected command {id} to succeed: {response}"
    );
    response["result"].clone()
}

async fn spawn_static_html_server() -> (String, tokio::task::JoinHandle<()>) {
    let body = Arc::new("<!doctype html><title>DOMStorage</title>".to_owned());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("DOMStorage test listener should bind");
    let addr = listener
        .local_addr()
        .expect("DOMStorage test listener should have an address");
    let app = axum::Router::new().route(
        "/page",
        axum::routing::get({
            let body = Arc::clone(&body);
            move || {
                let body = Arc::clone(&body);
                async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        body.as_str().to_owned(),
                    )
                }
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("DOMStorage test server should serve");
    });
    (format!("http://{addr}/page"), server)
}

async fn spawn_child_frame_server() -> (String, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("DOMStorage child frame listener should bind");
    let addr = listener
        .local_addr()
        .expect("DOMStorage child frame listener should have an address");
    let child_url = format!("http://localhost:{}/child", addr.port());
    let top_body = Arc::new(format!(
        r#"<script>
            window.childFrameLoaded = new Promise(resolve => {{
                window.resolveChildFrameLoaded = resolve;
            }});
        </script>
        <iframe src="{child_url}" onload="window.resolveChildFrameLoaded(true)"></iframe>"#
    ));
    let app = axum::Router::new()
        .route(
            "/page",
            axum::routing::get({
                let top_body = Arc::clone(&top_body);
                move || {
                    let top_body = Arc::clone(&top_body);
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            top_body.as_str().to_owned(),
                        )
                    }
                }
            }),
        )
        .route(
            "/child",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><title>child storage</title>",
                )
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("DOMStorage child frame server should serve");
    });
    (format!("http://{addr}/page"), child_url, server)
}

async fn loaded_dom_storage_context() -> (TestContext, String, String, tokio::task::JoinHandle<()>)
{
    let (page_url, server) = spawn_static_html_server().await;
    let origin = Url::parse(&page_url)
        .expect("page URL should parse")
        .origin()
        .ascii_serialization();
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-DOM-STORAGE".to_owned());
    browser_context.set_active_target_id("TID-DOM-STORAGE");
    browser_context.set_target_url(page_url.clone());
    browser_context.set_target_security_origin(origin.clone());
    browser_context.set_target_secure_context_type("Secure".to_owned());
    ctx.conn.browser_context = Some(browser_context);

    let page = ctx
        .conn
        .load_page_via_runtime_async(&page_url)
        .await
        .expect("DOMStorage test page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .active_page_state_mut()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);
    (ctx, page_url, origin, server)
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_storage_commands_round_trip_and_runtime_mutations_emit_ordered_events() {
    let (mut ctx, _page_url, origin, server) = loaded_dom_storage_context().await;

    ctx.process_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "localStorage.clear(); localStorage.setItem('before', 'enabled')"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 1);

    ctx.process_async(json!({
        "id": 2,
        "method": "DOMStorage.enable"
    }))
    .await;
    assert_eq!(take_result_by_id(&mut ctx, 2), json!({}));

    ctx.process_async(json!({
        "id": 3,
        "method": "DOMStorage.getDOMStorageItems",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "isLocalStorage": true
            }
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 3),
        json!({ "entries": [["before", "enabled"]] })
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOMStorage.setDOMStorageItem",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "isLocalStorage": true
            },
            "key": "server",
            "value": "written"
        }
    }))
    .await;
    assert_eq!(take_result_by_id(&mut ctx, 4), json!({}));
    let added = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("DOMStorage.domStorageItemAdded")
                && message["params"]["key"] == json!("server")
        })
        .expect("protocol write should emit DOMStorage.domStorageItemAdded");
    assert_eq!(added["params"]["newValue"], json!("written"));
    assert_eq!(
        added["params"]["storageId"]["securityOrigin"],
        json!(origin)
    );
    assert_eq!(added["params"]["storageId"]["isLocalStorage"], json!(true));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "localStorage.setItem('local-order', '1'); sessionStorage.setItem('session-order', '2')"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 5);
    let ordered_events = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("DOMStorage.domStorageItemAdded"))
        .map(|message| {
            (
                message["params"]["key"]
                    .as_str()
                    .expect("event key")
                    .to_owned(),
                message["params"]["storageId"]["isLocalStorage"]
                    .as_bool()
                    .expect("storage kind"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ordered_events,
        vec![
            ("local-order".to_owned(), true),
            ("session-order".to_owned(), false),
        ]
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "localStorage.setItem('local-order', '1')"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 6);
    assert!(
        ctx.sent.iter().all(|message| !message["method"]
            .as_str()
            .is_some_and(|method| method.starts_with("DOMStorage."))),
        "same-value writes must not emit DOMStorage events: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "localStorage.getItem('server')",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 7)["result"]["value"],
        json!("written")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 8,
        "method": "DOMStorage.setDOMStorageItem",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "isLocalStorage": true
            },
            "key": "server",
            "value": "updated"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 8);
    let updated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("DOMStorage.domStorageItemUpdated")
                && message["params"]["key"] == json!("server")
        })
        .expect("changed protocol write should emit an update");
    assert_eq!(updated["params"]["oldValue"], json!("written"));
    assert_eq!(updated["params"]["newValue"], json!("updated"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9,
        "method": "DOMStorage.removeDOMStorageItem",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "isLocalStorage": true
            },
            "key": "server"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 9);
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("DOMStorage.domStorageItemRemoved")
            && message["params"]["key"] == json!("server")
    }));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10,
        "method": "DOMStorage.clear",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "isLocalStorage": true
            }
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 10);
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("DOMStorage.domStorageItemsCleared")
            && message["params"]["storageId"]["isLocalStorage"] == json!(true)
    }));

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_storage_validates_storage_id_like_chromium_and_disable_drops_listener() {
    let (mut ctx, _page_url, origin, server) = loaded_dom_storage_context().await;
    ctx.process_async(json!({
        "id": 10,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-DOM-STORAGE" }
    }))
    .await;
    let storage_key = take_result_by_id(&mut ctx, 10)["storageKey"]
        .as_str()
        .expect("top frame storage key")
        .to_owned();

    ctx.process_async(json!({
        "id": 11,
        "method": "DOMStorage.getDOMStorageItems",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "storageKey": "storage-key:v1;origin=https://wrong.test;top-level-site=https://wrong.test",
                "isLocalStorage": true
            }
        }
    }))
    .await;
    let error = take_response_by_id(&mut ctx, 11);
    assert_eq!(error["error"]["code"], json!(-32000));
    assert_eq!(
        error["error"]["message"],
        json!("Frame not found for the given storage id")
    );

    ctx.process_async(json!({
        "id": 12,
        "method": "DOMStorage.getDOMStorageItems",
        "params": {
            "storageId": {
                "securityOrigin": "https://wrong.test",
                "storageKey": storage_key,
                "isLocalStorage": true
            }
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 12),
        json!({ "entries": [] }),
        "storageKey must take precedence over securityOrigin"
    );

    ctx.process_async(json!({
        "id": 121,
        "method": "DOMStorage.getDOMStorageItems",
        "params": {
            "storageId": {
                "securityOrigin": origin,
                "storageKey": "",
                "isLocalStorage": true
            }
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 121),
        json!({ "entries": [] }),
        "Chromium treats an empty storageKey as absent and falls back to securityOrigin"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "DOMStorage.enable"
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 13);
    ctx.process_async(json!({
        "id": 14,
        "method": "DOMStorage.disable"
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 14);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 15,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "localStorage.setItem('after-disable', 'silent')"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 15);
    assert!(
        ctx.sent.iter().all(|message| !message["method"]
            .as_str()
            .is_some_and(|method| method.starts_with("DOMStorage."))),
        "disabled DOMStorage session must not receive mutation events"
    );

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_storage_resolves_child_frame_storage_ids_without_collapsing_to_top_frame() {
    let (page_url, child_url, server) = spawn_child_frame_server().await;
    let top_origin = Url::parse(&page_url)
        .expect("top URL should parse")
        .origin()
        .ascii_serialization();
    let child_origin = Url::parse(&child_url)
        .expect("child URL should parse")
        .origin()
        .ascii_serialization();
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-DOM-STORAGE-CHILD".to_owned());
    browser_context.set_active_target_id("TID-DOM-STORAGE-CHILD");
    browser_context.set_target_url(page_url.clone());
    browser_context.set_target_security_origin(top_origin.clone());
    browser_context.set_target_secure_context_type("Secure".to_owned());
    ctx.conn.browser_context = Some(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async(&page_url)
        .await
        .expect("child-frame DOMStorage test page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_state_mut()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let child_frame_loaded = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        ctx.process_and_wait_for_response_async(json!({
            "id": 19,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "window.childFrameLoaded",
                "awaitPromise": true,
                "returnByValue": true
            }
        })),
    )
    .await;
    assert!(
        child_frame_loaded.is_ok(),
        "timed out waiting for child-frame load; sent={:?}",
        ctx.sent
    );
    assert_eq!(
        take_result_by_id(&mut ctx, 19)["result"]["value"],
        json!(true),
        "child frame load must complete before resolving its DOMStorage id"
    );

    ctx.process_async(json!({
        "id": 20,
        "method": "DOMStorage.setDOMStorageItem",
        "params": {
            "storageId": {
                "securityOrigin": child_origin,
                "isLocalStorage": true
            },
            "key": "child-only",
            "value": "child-value"
        }
    }))
    .await;
    assert_eq!(take_result_by_id(&mut ctx, 20), json!({}));

    ctx.process_async(json!({
        "id": 21,
        "method": "DOMStorage.getDOMStorageItems",
        "params": {
            "storageId": {
                "securityOrigin": child_origin,
                "isLocalStorage": true
            }
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 21),
        json!({ "entries": [["child-only", "child-value"]] })
    );

    ctx.process_async(json!({
        "id": 22,
        "method": "DOMStorage.getDOMStorageItems",
        "params": {
            "storageId": {
                "securityOrigin": top_origin,
                "isLocalStorage": true
            }
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 22),
        json!({ "entries": [] }),
        "child storage must stay isolated from the top-frame area"
    );

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_storage_mutations_fan_out_to_enabled_primary_and_auxiliary_sessions() {
    let (mut ctx, _page_url, _origin, server) = loaded_dom_storage_context().await;

    ctx.process_async(json!({
        "id": 30,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-DOM-STORAGE" }
    }))
    .await;
    let primary_session_id = take_result_by_id(&mut ctx, 30)["sessionId"]
        .as_str()
        .expect("primary target session id")
        .to_owned();
    ctx.process_async(json!({
        "id": 31,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-DOM-STORAGE" }
    }))
    .await;
    let auxiliary_session_id = take_result_by_id(&mut ctx, 31)["sessionId"]
        .as_str()
        .expect("auxiliary target session id")
        .to_owned();
    assert_ne!(primary_session_id, auxiliary_session_id);
    ctx.sent.clear();

    for (id, session_id) in [
        (32, primary_session_id.as_str()),
        (33, auxiliary_session_id.as_str()),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "DOMStorage.enable",
            "sessionId": session_id
        }))
        .await;
        assert_eq!(take_result_by_id(&mut ctx, id), json!({}));
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 34,
        "method": "Runtime.evaluate",
        "sessionId": primary_session_id.clone(),
        "params": {
            "expression": "localStorage.setItem('fanout', 'yes')"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 34);
    let mut event_session_ids = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("DOMStorage.domStorageItemAdded")
                && message["params"]["key"] == json!("fanout")
        })
        .filter_map(|message| message["sessionId"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    event_session_ids.sort();
    let mut expected_session_ids = vec![primary_session_id, auxiliary_session_id];
    expected_session_ids.sort();
    assert_eq!(event_session_ids, expected_session_ids);

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn local_storage_mutations_fan_out_across_targets_without_leaking_session_storage() {
    let (mut ctx, page_url, _origin, server) = loaded_dom_storage_context().await;

    ctx.process_async(json!({
        "id": 40,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": "BID-DOM-STORAGE",
            "url": page_url
        }
    }))
    .await;
    let background_target_id = take_result_by_id(&mut ctx, 40)["targetId"]
        .as_str()
        .expect("background target id")
        .to_owned();
    ctx.sent.clear();

    let mut session_ids = Vec::new();
    for (id, target_id) in [(41, "TID-DOM-STORAGE"), (42, background_target_id.as_str())] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.attachToTarget",
            "params": { "targetId": target_id }
        }))
        .await;
        session_ids.push(
            take_result_by_id(&mut ctx, id)["sessionId"]
                .as_str()
                .expect("target session id")
                .to_owned(),
        );
    }
    ctx.sent.clear();

    for (id, session_id) in [(43, &session_ids[0]), (44, &session_ids[1])] {
        ctx.process_async(json!({
            "id": id,
            "method": "DOMStorage.enable",
            "sessionId": session_id
        }))
        .await;
        let _ = take_result_by_id(&mut ctx, id);
    }
    ctx.sent.clear();
    let active_session_id = session_ids[0].clone();

    ctx.process_async(json!({
        "id": 45,
        "method": "Runtime.evaluate",
        "sessionId": active_session_id,
        "params": {
            "expression": "localStorage.setItem('cross-target-local', 'yes'); sessionStorage.setItem('target-session-only', 'yes')"
        }
    }))
    .await;
    let _ = take_result_by_id(&mut ctx, 45);

    let mut local_event_session_ids = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("DOMStorage.domStorageItemAdded")
                && message["params"]["key"] == json!("cross-target-local")
        })
        .filter_map(|message| message["sessionId"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    local_event_session_ids.sort();
    let mut expected_local_event_session_ids = session_ids;
    expected_local_event_session_ids.sort();
    assert_eq!(
        local_event_session_ids, expected_local_event_session_ids,
        "Chromium's local namespace notifies enabled inspector agents across targets"
    );

    let session_event_session_ids = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("DOMStorage.domStorageItemAdded")
                && message["params"]["key"] == json!("target-session-only")
        })
        .filter_map(|message| message["sessionId"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        session_event_session_ids,
        vec![active_session_id.as_str()],
        "sessionStorage notifications must stay in the top-level target namespace"
    );

    server.abort();
    let _ = server.await;
}
