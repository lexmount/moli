use super::*;
use parking_lot::Mutex;

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_module_completions_follow_the_committed_document_without_a_frontend() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let observed = observed.clone();
            async move {
                let (status, mime, body) = match uri.path() {
                    "/module.js" => (StatusCode::OK, "text/javascript", "export default 42;"),
                    "/graph.js" => (
                        StatusCode::OK,
                        "text/javascript",
                        "export { default } from './module.js';",
                    ),
                    "/module.json" => (StatusCode::OK, "application/json", "{\"answer\":42}"),
                    "/missing.js" => (StatusCode::NOT_FOUND, "text/javascript", "not found"),
                    "/parser.js" => (StatusCode::OK, "text/javascript", "globalThis.parserModuleAnswer = 42;"),
                    "/runtime.js" => (StatusCode::OK, "text/javascript", "globalThis.runtimeModuleAnswer = 42;"),
                    "/popup-navigate" => (
                        StatusCode::OK,
                        "text/html",
                        "<!doctype html><script type=module src=/parser.js></script><body>module owner</body>",
                    ),
                    _ => (
                        StatusCode::OK,
                        "text/html",
                        "<!doctype html><body>module owner</body>",
                    ),
                };
                if uri.path().contains('.') {
                    observed.lock().push(uri.path().to_owned());
                }
                (
                    status,
                    [("content-type", mime), ("access-control-allow-origin", "*")],
                    body,
                ).into_response()
            }
        })),
        "auxiliary-module-completion",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/parent")).await;

    // Never discover or attach a frontend to the auxiliary target. Its own
    // networking tasks must finish while only the opener is being observed.
    for mode in ["blank", "navigate"] {
        let source = format!(
            "{}\nstartAuxiliaryModuleProbe({}, {});true",
            include_str!("../../../../tests/fixtures/auxiliary-module-completion.js"),
            json!(base),
            json!(mode),
        );
        assert_eq!(
            evaluate_window_name_probe(&mut opener, 3, &source).await,
            true
        );
        timeout(Duration::from_secs(5), async {
            loop {
                let result =
                    evaluate_window_name_probe(&mut opener, 4, "globalThis.auxiliaryModuleResult")
                        .await;
                if result["done"] == true {
                    assert!(result["error"].is_null(), "{mode}: {result}");
                    assert_eq!(result["retainedWindow"], true, "{mode}: {result}");
                    assert_eq!(
                        result["checks"],
                        json!([
                            "parser",
                            "runtime",
                            "javascript",
                            "graph",
                            "json",
                            "fetch-error"
                        ]),
                        "{mode}: {result}"
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{mode}: auxiliary module completion did not reach its opener"));
    }
    let requests = requests.lock().clone();
    for path in [
        "/module.js",
        "/graph.js",
        "/module.json",
        "/missing.js",
        "/runtime.js",
    ] {
        assert_eq!(
            requests.iter().filter(|request| *request == path).count(),
            2,
            "{path}: {requests:?}"
        );
    }
    assert_eq!(
        requests
            .iter()
            .filter(|request| *request == "/parser.js")
            .count(),
        1
    );
    abort_test_cdp_server(server).await;
}
