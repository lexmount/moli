use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn close_opener() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><title>close opener</title><body></body>",
    )
}

async fn close_popup(
    axum::extract::State(requests): axum::extract::State<Arc<AtomicUsize>>,
) -> impl IntoResponse {
    requests.fetch_add(1, Ordering::SeqCst);
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        r#"<!doctype html><script>
        onload = () => setTimeout(() => {
          const remaining = Number(new URLSearchParams(location.search).get('navs'));
          if (remaining) {
            location.href = '/popup?navs=0#1';
            return;
          }
          let calls = 0;
          onbeforeunload = () => {
            const snapshot = {
              calls: ++calls, closed: window.closed,
              document: document.location === location,
              history: null, navigation: navigation.currentEntry?.url ?? null,
              opener: opener !== null, error: null
            };
            try { snapshot.history = history.length; }
            catch (error) { snapshot.error = error.name; }
            window.close();
            localStorage.setItem('close-snapshot', JSON.stringify(snapshot));
          };
          addEventListener('beforeunload', () => {
            localStorage.setItem('close-listeners',
              String(Number(localStorage.getItem('close-listeners') || 0) + 1));
          });
          localStorage.setItem('close-before', String(history.length));
          window.close();
          localStorage.setItem('close-after', JSON.stringify({
            closed: window.closed, history: history.length,
            navigation: navigation.currentEntry?.url ?? null
          }));
        }, 0);
        </script>"#,
    )
}

async fn assert_close_keeps_history_until_beforeunload_finishes(features: &str, navigations: u32) {
    let requests = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_requests = requests.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/opener", get(close_opener))
                .route("/popup", get(close_popup))
                .with_state(server_requests),
        )
        .await
        .unwrap();
    });
    let opener_url = format!("http://{addr}/opener");
    let popup_url = format!("http://{addr}/popup?navs={navigations}");
    let mut ctx = TestContext::new();
    enable_root_target_discovery_for_test(&mut ctx);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_async(json!({
                "id": 1, "method": "Target.createTarget", "params": {"url": opener_url}
            }))
            .await;
            let opener_id = take_created_target_id(&mut ctx, 1);
            let context_id = ctx.conn.browser_context.as_ref().unwrap().id.clone();
            ctx.wait_until_scheduler_state("close opener committed", |conn| {
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
            let session = take_response_by_id(&mut ctx, 2)["result"]["sessionId"]
                .as_str()
                .unwrap()
                .to_owned();
            ctx.take_all();
            ctx.process_async(json!({
                "id": 3, "method": "Runtime.evaluate", "sessionId": session,
                "params": {
                    "expression": format!("open({}, '_blank', {}) === null", json!(popup_url), json!(features)),
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 3)["result"]["result"]["value"], !features.is_empty());
            let mut events = ctx.take_all();
            let popup_id = events
                .iter()
                .find(|message| {
                    message["method"] == "Target.targetCreated"
                        && message["params"]["targetInfo"]["url"] == popup_url
                })
                .expect("closing popup is published")["params"]["targetInfo"]["targetId"]
                .as_str()
                .unwrap()
                .to_owned();
            ctx.wait_until_scheduler_state("scripted popup close retired its target", |conn| {
                requests.load(Ordering::SeqCst) == navigations as usize + 1
                    && conn.browser_context_by_id(&context_id)
                        .is_some_and(|context| context.page_target(&popup_id).is_none())
            })
            .await;
            events.extend(ctx.take_all());
            assert_eq!(events.iter().filter(|message| {
                message["method"] == "Target.targetDestroyed"
                    && message["params"]["targetId"] == popup_id
            }).count(), 1, "reentrant close must retire the target once");
            ctx.process_async(json!({
                "id": 4, "method": "Runtime.evaluate", "sessionId": session,
                "params": {
                    "expression": "JSON.stringify([JSON.parse(localStorage.getItem('close-snapshot')),localStorage.getItem('close-listeners'),localStorage.getItem('close-before'),JSON.parse(localStorage.getItem('close-after'))])",
                    "returnByValue": true
                }
            }))
            .await;
            let final_url = if navigations == 0 {
                popup_url.clone()
            } else {
                format!("http://{addr}/popup?navs=0#1")
            };
            let expected = json!([{
                "calls": 1, "closed": false, "document": true,
                "history": navigations + 1, "navigation": final_url,
                "opener": features.is_empty(), "error": null
            }, "1", (navigations + 1).to_string(), {
                "closed": true, "history": navigations + 1, "navigation": final_url
            }]);
            let reply = take_response_by_id(&mut ctx, 4);
            let actual: Value = serde_json::from_str(
                reply["result"]["result"]["value"].as_str().expect("popup close observations"),
            ).unwrap();
            assert_eq!(actual, expected, "{features}, navs={navigations}: {reply}");
        })
        .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_close_keeps_history_until_beforeunload_finishes() {
    for navigations in [0, 1] {
        assert_close_keeps_history_until_beforeunload_finishes("", navigations).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn noopener_close_keeps_history_until_beforeunload_finishes() {
    for features in ["noopener", "noreferrer"] {
        for navigations in [0, 1] {
            assert_close_keeps_history_until_beforeunload_finishes(features, navigations).await;
        }
    }
}
