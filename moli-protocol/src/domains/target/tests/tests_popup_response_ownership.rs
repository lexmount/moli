use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Default)]
struct PopupResponseRecords {
    requests: Arc<AtomicUsize>,
    scripts: Arc<Mutex<Vec<Value>>>,
}

async fn opener_document() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><title>opener</title><body>popup response owner</body>",
    )
}

async fn popup_document(
    axum::extract::State(records): axum::extract::State<PopupResponseRecords>,
) -> impl IntoResponse {
    records.requests.fetch_add(1, Ordering::SeqCst);
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        r#"<!doctype html><title>popup</title><script>
        globalThis.__before = sessionStorage.getItem('probe');
        globalThis.__runs = Number(sessionStorage.getItem('side-effects') || 0) + 1;
        sessionStorage.setItem('side-effects', String(__runs));
        sessionStorage.setItem('probe', 'child');
        const record = {before: __before, runs: __runs, hasOpener: opener !== null};
        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/record', false);
        xhr.send(JSON.stringify(record));
        </script>"#,
    )
}

async fn record_popup_script(
    axum::extract::State(records): axum::extract::State<PopupResponseRecords>,
    body: String,
) -> StatusCode {
    records
        .scripts
        .lock()
        .push(serde_json::from_str(&body).expect("popup script posts a JSON record"));
    StatusCode::OK
}

async fn assert_popup_response_has_one_script_owner(
    source: &str,
    features: &str,
    suppress_opener: bool,
) {
    let records = PopupResponseRecords::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_records = records.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/opener", get(opener_document))
                .route("/popup", get(popup_document))
                .route("/record", axum::routing::post(record_popup_script))
                .with_state(server_records),
        )
        .await
        .unwrap();
    });
    let opener_url = format!("http://{addr}/opener");
    let popup_url = format!("http://{addr}/popup");
    let mut ctx = TestContext::new();
    enable_root_target_discovery_for_test(&mut ctx);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_async(json!({
                "id": 1, "method": "Target.createTarget",
                "params": {"url": opener_url}
            }))
            .await;
            let opener_id = take_response_by_id(&mut ctx, 1)["result"]["targetId"]
                .as_str()
                .expect("created opener target")
                .to_owned();
            let context_id = ctx.conn.browser_context.as_ref().unwrap().id.clone();
            ctx.wait_until_scheduler_state("opener document committed", |conn| {
                conn.browser_context_by_id(&context_id)
                    .and_then(|context| loaded_page_for_target(context, &opener_id))
                    .is_some_and(|page| page.final_url().as_str() == opener_url)
            })
            .await;
            ctx.process_async(json!({
                "id": 2, "method": "Target.attachToTarget",
                "params": {"targetId": opener_id, "flatten": true}
            }))
            .await;
            let opener_session = take_response_by_id(&mut ctx, 2)["result"]["sessionId"]
                .as_str()
                .unwrap()
                .to_owned();
            let action = if source == "window" {
                format!(
                    "window.open({}, '_blank', {}) === null",
                    json!(popup_url),
                    json!(features)
                )
            } else {
                format!(
                    r#"(() => {{
                      const element = document.createElement({});
                      element.href = {};
                      element.target = '_blank';
                      element.rel = {};
                      const map = document.createElement('map');
                      map.name = 'popup-map';
                      document.body.appendChild(map);
                      map.appendChild(element);
                      return element.click() === undefined;
                    }})()"#,
                    json!(source),
                    json!(popup_url),
                    json!(features),
                )
            };
            ctx.take_all();
            ctx.process_async(json!({
                "id": 3, "method": "Runtime.evaluate", "sessionId": opener_session,
                "params": {
                    "expression": format!("sessionStorage.setItem('probe', 'parent');{action}"),
                    "returnByValue": true
                }
            }))
            .await;
            let opened = take_response_by_id(&mut ctx, 3);
            assert_eq!(
                opened["result"]["result"]["value"],
                if source == "window" { suppress_opener } else { true },
                "{source}, {features}: {opened:?}"
            );
            let messages = ctx.take_all();
            let created = messages
                .iter()
                .filter(|message| {
                    message["method"] == "Target.targetCreated"
                        && message["params"]["targetInfo"]["type"] == "page"
                        && message["params"]["targetInfo"]["url"] == popup_url
                })
                .collect::<Vec<_>>();
            assert_eq!(created.len(), 1, "{source}, {features}: {messages:?}");
            let popup_id = created[0]["params"]["targetInfo"]["targetId"]
                .as_str()
                .unwrap()
                .to_owned();
            ctx.wait_until_scheduler_state("popup response script completed", |conn| {
                conn.browser_context_by_id(&context_id)
                    .and_then(|context| loaded_page_for_target(context, &popup_id))
                    .is_some_and(|page| page.final_url().as_str() == popup_url)
                    && !records.scripts.lock().is_empty()
            })
            .await;
            ctx.process_async(json!({
                "id": 4, "method": "Target.attachToTarget",
                "params": {"targetId": popup_id, "flatten": true}
            }))
            .await;
            let popup_session = take_response_by_id(&mut ctx, 4)["result"]["sessionId"]
                .as_str()
                .unwrap()
                .to_owned();
            ctx.process_async(json!({
                "id": 5, "method": "Runtime.evaluate", "sessionId": popup_session,
                "params": {
                    "expression": "JSON.stringify([__before,__runs,sessionStorage.getItem('side-effects'),opener === null])",
                    "returnByValue": true
                }
            }))
            .await;
            let snapshot = take_response_by_id(&mut ctx, 5);
            let before = if suppress_opener {
                Value::Null
            } else {
                json!("parent")
            };
            assert_eq!(
                snapshot["result"]["result"]["value"],
                json!([before, 1, "1", suppress_opener]).to_string(),
                "{source}, {features}: {snapshot:?}"
            );
            assert_eq!(records.requests.load(Ordering::SeqCst), 1);
            assert_eq!(
                *records.scripts.lock(),
                vec![json!({"before": before, "runs": 1, "hasOpener": !suppress_opener})],
                "{source}, {features}: one HTTP response must execute once"
            );
            ctx.process_async(json!({
                "id": 6, "method": "Runtime.evaluate", "sessionId": opener_session,
                "params": {"expression": "sessionStorage.getItem('probe')", "returnByValue": true}
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 6)["result"]["result"]["value"],
                "parent"
            );
            ctx.process_async(json!({
                "id": 7, "method": "Target.closeTarget", "params": {"targetId": popup_id}
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 7)["result"]["success"], true);
        })
        .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_noopener_response_has_one_script_owner() {
    for (features, suppress_opener) in [
        ("", false),
        ("noopener", true),
        ("noreferrer", true),
        ("popup,noopener", true),
    ] {
        assert_popup_response_has_one_script_owner("window", features, suppress_opener).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_noopener_response_has_one_script_owner() {
    for source in ["a", "area"] {
        for (rel, suppress_opener) in [
            ("", true),
            ("opener", false),
            ("noopener", true),
            ("noreferrer", true),
        ] {
            assert_popup_response_has_one_script_owner(source, rel, suppress_opener).await;
        }
    }
}
