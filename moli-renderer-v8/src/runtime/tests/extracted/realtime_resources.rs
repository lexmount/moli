use super::*;

fn resource_fixture(scope: &str) -> String {
    include_str!("../../../../tests/fixtures/realtime-resource-observers.html")
        .replace("__SCOPE__", scope)
        .replace(
            "__CHILD__",
            if scope == "root" {
                "<iframe src=\"/child\" style=\"width:400px;height:200px;border:0\"></iframe>"
            } else {
                ""
            },
        )
}

#[tokio::test(flavor = "multi_thread")]
async fn realtime_root_and_child_resources_advance_resize_observers_without_queries() {
    assert_resource_observer_completion(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn realtime_linked_child_stylesheet_resources_advance_observers_without_queries() {
    assert_resource_observer_completion(true).await;
}

async fn assert_resource_observer_completion(child_stylesheet_linked: bool) {
    use base64::Engine;

    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(
        crate::protocol_types::OptionalResourceFetchMask::FONT
            | crate::protocol_types::OptionalResourceFetchMask::IMAGE,
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed_requests = Arc::clone(&requests);
    let (completed_tx, mut completed_rx) = tokio::sync::mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let gates = std::collections::HashMap::from([
            ("root", tokio::sync::watch::channel(false).0),
            ("child", tokio::sync::watch::channel(false).0),
        ]);
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let gates = gates.clone();
            let observed_requests = Arc::clone(&observed_requests);
            let completed_tx = completed_tx.clone();
            connections.spawn(async move {
                let request = read_owner_wake_http_request_head(&mut stream).await;
                let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                observed_requests.lock().push(path.to_owned());
                let (body, content_type) = if path == "/child" {
                    let mut html = resource_fixture("child");
                    if child_stylesheet_linked {
                        let (before, rest) = html.split_once("<style>").unwrap();
                        let (_, after) = rest.split_once("</style>").unwrap();
                        html = format!("{before}<link rel=\"stylesheet\" href=\"/child.css\">{after}");
                    }
                    (html.into_bytes(), "text/html")
                } else if path == "/child.css" {
                    let html = resource_fixture("child");
                    let (_, rest) = html.split_once("<style>").unwrap();
                    let (css, _) = rest.split_once("</style>").unwrap();
                    (css.as_bytes().to_vec(), "text/css")
                } else if let Some(scope) = path.strip_prefix("/initial/") {
                    gates[scope].send_replace(true);
                    (b"ok".to_vec(), "text/plain")
                } else if let Some(scope) = path.strip_prefix("/complete/") {
                    completed_tx.send(scope.to_owned()).unwrap();
                    (b"ok".to_vec(), "text/plain")
                } else {
                    let scope = path.rsplit('/').next().unwrap().split('.').next().unwrap();
                    let mut gate = gates[scope].subscribe();
                    while !*gate.borrow_and_update() {
                        gate.changed().await.unwrap();
                    }
                    if path.starts_with("/font/") {
                        (
                            include_bytes!("../../../../../moli-layout/tests/fixtures/moli-ahem.woff2").to_vec(),
                            "font/woff2",
                        )
                    } else {
                        assert!(path.starts_with("/image/"), "unexpected request: {path}");
                        (
                            base64::engine::general_purpose::STANDARD
                                .decode("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7")
                                .unwrap(),
                            "image/gif",
                        )
                    }
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            });
        }
    });
    let mut page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        url::Url::parse(&format!("http://{address}/")).unwrap(),
        &resource_fixture("root"),
    )
    .await;
    // The first RO samples release the HTTP bodies. No command, getter or
    // capture can help the resource completions drive the final RO entries.
    let completed = tokio::time::timeout(Duration::from_secs(3), async {
        let mut scopes = std::collections::BTreeSet::new();
        while scopes.len() != 2 {
            scopes.insert(completed_rx.recv().await.expect("completion source"));
        }
        scopes
    })
    .await;
    let (reply, _) = page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "JSON.stringify({root:resourceProbe,child:document.querySelector('iframe').contentWindow.resourceProbe})".to_owned(),
        await_promise: false,
    }).await.unwrap();
    let state = mouse_input_json_witness(&reply);
    page.close_async().await.unwrap();
    server.abort();
    let _ = server.await;
    assert!(
        completed.is_ok(),
        "state={state}, requests={:?}",
        requests.lock()
    );
    for scope in ["root", "child"] {
        let state = &state[scope];
        assert_ne!(state["initial"]["text"][0], 48, "{scope}: {state}");
        assert_eq!(state["initial"]["asset"], serde_json::json!([0, 0]));
        assert_eq!(state["complete"], true, "{scope}: {state}");
        assert_eq!(state["fontsReady"], true, "{scope}: {state}");
        assert_eq!(state["errors"], serde_json::json!([]));
        for path in [
            format!("/font/{scope}.woff2"),
            format!("/image/{scope}.gif"),
        ] {
            assert_eq!(
                requests
                    .lock()
                    .iter()
                    .filter(|request| **request == path)
                    .count(),
                1,
                "{path}"
            );
        }
    }
}
