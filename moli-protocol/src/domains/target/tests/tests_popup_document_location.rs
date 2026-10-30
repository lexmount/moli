use super::*;

async fn location_opener() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><title>opener</title><body></body>",
    )
}

async fn location_popup() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        r#"<!doctype html><script>
        const descriptor = Object.getOwnPropertyDescriptor(document, 'location');
        const borrowed = Object.getOwnPropertyDescriptor(opener.document, 'location').get;
        globalThis.__locationSnapshot = () => [
          document.defaultView === window,
          document.location === location,
          descriptor.get.call(document) === location,
          borrowed.call(document) === location,
          descriptor.get.call(opener.document) === opener.location,
          document.location?.href === document.URL,
          descriptor.enumerable,
          !descriptor.configurable,
          new Document().location === null,
          document.implementation.createHTMLDocument('').location === null
        ];
        opener.__retainedPopupDocument = document;
        opener.__retainedPopupLocationGetter = descriptor.get;
        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/record', false);
        xhr.send(JSON.stringify(__locationSnapshot()));
        </script>"#,
    )
}

async fn assert_popup_document_location(source: &str) {
    let snapshots = Arc::new(Mutex::new(Vec::<Value>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_snapshots = snapshots.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/opener", get(location_opener))
                .route("/popup", get(location_popup))
                .route(
                    "/record",
                    axum::routing::post(move |body: String| async move {
                        server_snapshots
                            .lock()
                            .push(serde_json::from_str(&body).unwrap());
                        StatusCode::OK
                    }),
                ),
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
                "id": 1, "method": "Target.createTarget", "params": {"url": opener_url}
            }))
            .await;
            let opener_id = take_created_target_id(&mut ctx, 1);
            let context_id = ctx.with_fixture(|conn| conn.browser_context.as_ref().unwrap().id.clone());
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
                format!("open({}, 'popup-document') !== null", json!(popup_url))
            } else {
                format!(
                    r#"(() => {{
                      const link = document.createElement({});
                      link.href = {};
                      link.target = 'popup-document';
                      link.rel = 'opener';
                      document.body.appendChild(link);
                      return link.click() === undefined;
                    }})()"#,
                    json!(source),
                    json!(popup_url)
                )
            };
            ctx.take_all();
            ctx.process_async(json!({
                "id": 3, "method": "Runtime.evaluate", "sessionId": opener_session,
                "params": {
                    "expression": format!("document.location === location && ({action})"),
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 3)["result"]["result"]["value"], true);
            let messages = ctx.take_all();
            let popup_id = messages
                .iter()
                .find(|message| {
                    message["method"] == "Target.targetCreated"
                        && message["params"]["targetInfo"]["url"] == popup_url
                })
                .expect("popup target is published")["params"]["targetInfo"]["targetId"]
                .as_str()
                .unwrap()
                .to_owned();
            ctx.wait_until_scheduler_state("popup parser snapshot recorded", |conn| {
                conn.browser_context_by_id(&context_id)
                    .and_then(|context| loaded_page_for_target(context, &popup_id))
                    .is_some_and(|page| page.final_url().as_str() == popup_url)
                    && !snapshots.lock().is_empty()
            })
            .await;
            let expected = json!(vec![true; 10]);
            assert_eq!(*snapshots.lock(), vec![expected.clone()], "{source}: parser Document.location");
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
                "params": {"expression": "JSON.stringify(__locationSnapshot())", "returnByValue": true}
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 5)["result"]["result"]["value"], expected.to_string());
            ctx.process_async(json!({
                "id": 6, "method": "Target.closeTarget", "params": {"targetId": popup_id}
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 6)["result"]["success"], true);
            ctx.process_async(json!({
                "id": 7, "method": "Runtime.evaluate", "sessionId": opener_session,
                "params": {
                    "expression": "__retainedPopupDocument.location === null && __retainedPopupLocationGetter.call(document) === location",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 7)["result"]["result"]["value"], true);
        })
        .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn window_open_documents_read_their_relevant_window_location() {
    assert_popup_document_location("window").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hyperlink_documents_read_their_relevant_window_location() {
    for source in ["a", "area"] {
        assert_popup_document_location(source).await;
    }
}
