use super::*;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const SCRIPT_BODY: &str = "window.resourceTreeLoaded = true;";
const STYLESHEET_BODY: &str = "body { color: rgb(1, 2, 3); }";
const SEARCH_SCRIPT_BODY: &str =
    "window.searchResourceLoaded = true;\nconst benchmarkNeedle = 'script value';";
const CHILD_SEARCH_SCRIPT_BODY: &str = "const childResourceNeedle = 'child script value';";
const CHILD_DOCUMENT_BODY: &str = "<!doctype html>\n\
         <html>\n\
         <p>Child Resource Fixture</p>\n\
         <script src=\"/child-search.js\"></script>\n\
         </html>";

#[derive(Default)]
struct StylesheetImportHits {
    root: AtomicUsize,
    child: AtomicUsize,
    leaf: AtomicUsize,
}

async fn resource_tree_document() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><html><head>\
         <link rel=\"stylesheet\" href=\"/app.css#theme\">\
         <script src=\"/app.js#revision\"></script>\
         </head><body>resource tree</body></html>",
    )
}

async fn resource_tree_script() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "application/javascript; charset=utf-8",
        )],
        SCRIPT_BODY,
    )
}

async fn resource_tree_stylesheet() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "text/css; charset=utf-8",
        )],
        STYLESHEET_BODY,
    )
}

async fn resource_tree_import_document() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><html><head>\
         <link rel=\"stylesheet\" href=\"/styles/root.css\">\
         <link rel=\"stylesheet\" href=\"/styles/root.css\">\
         </head><body>stylesheet imports</body></html>",
    )
}

async fn resource_tree_import_root(
    axum::extract::State(hits): axum::extract::State<Arc<StylesheetImportHits>>,
) -> impl axum::response::IntoResponse {
    hits.root.fetch_add(1, Ordering::SeqCst);
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
        "@import url('./nested/child.css'); body { color: rgb(1, 2, 3); }",
    )
}

async fn resource_tree_import_child(
    axum::extract::State(hits): axum::extract::State<Arc<StylesheetImportHits>>,
) -> impl axum::response::IntoResponse {
    hits.child.fetch_add(1, Ordering::SeqCst);
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
        "@import url('../leaf.css'); body { background-color: rgb(4, 5, 6); }",
    )
}

async fn resource_tree_import_leaf(
    axum::extract::State(hits): axum::extract::State<Arc<StylesheetImportHits>>,
) -> impl axum::response::IntoResponse {
    hits.leaf.fetch_add(1, Ordering::SeqCst);
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
        "body { border-top-color: rgb(7, 8, 9); }",
    )
}

async fn resource_tree_xml_document() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "application/xml")],
        "<?xml version=\"1.0\"?><semantic-root><value>xml</value></semantic-root>",
    )
}

async fn resource_search_document() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "text/html; charset=utf-8",
        )],
        "<!doctype html>\r\n\
         <html>\r\n\
         <head>\r\n\
         <meta charset=\"utf-8\">\r\n\
         <title>CDP Core Fixture</title>\r\n\
         <script src=\"/search.js#revision\"></script>\r\n\
         </head>\r\n\
         <body>\r\n\
         <h1 id=\"title\">CDP Core Fixture</h1>\r\n\
         <script>RegExp.prototype.exec = () => { throw new Error('page regex'); }; globalThis.RegExp = null; document.body.dataset.runtimeOnly = 'set';</script>\r\n\
         </body>\r\n\
         </html>",
    )
}

async fn resource_search_script() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "application/javascript; charset=utf-8",
        )],
        SEARCH_SCRIPT_BODY,
    )
}

async fn empty_resource_search_document() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "text/html; charset=utf-8",
        )],
        "",
    )
}

async fn child_resource_search_parent() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><iframe src=\"/child-document\"></iframe>",
    )
}

async fn child_resource_search_document() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "text/html; charset=utf-8",
        )],
        CHILD_DOCUMENT_BODY,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_returns_network_child_document_source() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(child_resource_search_parent))
                .route(
                    "/child-document",
                    axum::routing::get(child_resource_search_document),
                )
                .route(
                    "/child-search.js",
                    axum::routing::get(child_resource_search_script),
                ),
        )
        .await
        .unwrap();
    });
    let parent_url = format!("http://{addr}/parent");
    let child_url = format!("http://{addr}/child-document");
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-CHILD-RESPONSE-BODY",
        "TID-CHILD-RESPONSE-BODY",
        "SID-CHILD-RESPONSE-BODY",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 35,
        "method": "Page.enable",
        "sessionId": "SID-CHILD-RESPONSE-BODY",
    }))
    .await;
    take_response_by_id(&mut ctx, 35);
    ctx.process_async(json!({
        "id": 36,
        "method": "Network.enable",
        "sessionId": "SID-CHILD-RESPONSE-BODY",
    }))
    .await;
    take_response_by_id(&mut ctx, 36);
    ctx.process_async(json!({
        "id": 37,
        "method": "Page.navigate",
        "sessionId": "SID-CHILD-RESPONSE-BODY",
        "params": { "url": parent_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 37);

    wait_until_messages(
        &mut ctx,
        "SID-CHILD-RESPONSE-BODY",
        "child Document Network.loadingFinished",
        |messages| {
            let child_request_id = messages.iter().find_map(|message| {
                (message["method"] == json!("Network.responseReceived")
                    && message["params"]["response"]["url"] == json!(child_url))
                .then(|| message["params"]["requestId"].as_str())
                .flatten()
            });
            child_request_id.is_some_and(|request_id| {
                messages.iter().any(|message| {
                    message["method"] == json!("Network.loadingFinished")
                        && message["params"]["requestId"] == json!(request_id)
                })
            })
        },
    )
    .await;
    let child_request_id = ctx
        .sent
        .iter()
        .find_map(|message| {
            (message["method"] == json!("Network.responseReceived")
                && message["params"]["response"]["url"] == json!(child_url))
            .then(|| message["params"]["requestId"].as_str().map(str::to_owned))
            .flatten()
        })
        .expect("child Document response request id");

    ctx.process_async(json!({
        "id": 38,
        "method": "Network.getResponseBody",
        "sessionId": "SID-CHILD-RESPONSE-BODY",
        "params": { "requestId": child_request_id }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 38)["result"],
        json!({ "body": CHILD_DOCUMENT_BODY, "base64Encoded": false })
    );

    server.abort();
}

async fn child_resource_search_script() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "application/javascript; charset=utf-8",
        )],
        CHILD_SEARCH_SCRIPT_BODY,
    )
}

async fn empty_child_resource_search_parent() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><iframe src=\"/empty-child-document\"></iframe>",
    )
}

async fn empty_child_resource_search_document() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE.as_str(),
            "text/html; charset=utf-8",
        )],
        "",
    )
}

fn unique_resource_search_cache_dir() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-cdp-resource-search-{}-{nonce}",
        std::process::id()
    ))
}

async fn cacheable_empty_child_resource_search_parent(
    axum::extract::State(_): axum::extract::State<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
) -> impl axum::response::IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
            ("cache-control", "no-store"),
        ],
        "<!doctype html><iframe src=\"/cached-empty-child-document\"></iframe>",
    )
}

async fn cacheable_empty_child_resource_search_document(
    axum::extract::State(hits): axum::extract::State<
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    >,
) -> impl axum::response::IntoResponse {
    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    (
        [
            (
                axum::http::header::CONTENT_TYPE.as_str(),
                "text/html; charset=utf-8",
            ),
            ("cache-control", "public, max-age=31536000, immutable"),
        ],
        "",
    )
}

async fn wait_for_resource_search_child_document_commit(
    ctx: &mut TestContext,
    session_id: &str,
    child_frame_id: &str,
    child_url: &str,
) {
    let child_frame_id = child_frame_id.to_owned();
    let child_url = child_url.to_owned();
    wait_until_message(
        ctx,
        session_id,
        "resource-search child document commit",
        move |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"] == json!(child_url)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn get_resource_tree_reports_observed_frame_subresources() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/document", axum::routing::get(resource_tree_document))
                .route("/app.js", axum::routing::get(resource_tree_script))
                .route("/app.css", axum::routing::get(resource_tree_stylesheet)),
        )
        .await
        .unwrap();
    });
    let document_url = format!("http://{addr}/document");

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-RESOURCE-TREE",
        "TID-RESOURCE-TREE",
        "SID-RESOURCE-TREE",
        &document_url,
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(&document_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_state_mut()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 1,
        "method": "Page.getResourceTree",
        "sessionId": "SID-RESOURCE-TREE",
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 1);
    assert_eq!(response["sessionId"], json!("SID-RESOURCE-TREE"));
    assert_eq!(
        response["result"]["frameTree"]["frame"]["id"],
        json!("TID-RESOURCE-TREE")
    );
    let resources = response["result"]["frameTree"]["resources"]
        .as_array()
        .expect("resource tree should always expose a resources array");
    assert_eq!(resources.len(), 2, "unexpected response: {response}");
    let stylesheet = resources
        .iter()
        .find(|resource| resource["type"] == json!("Stylesheet"))
        .expect("stylesheet resource");
    assert_eq!(
        stylesheet,
        &json!({
            "url": format!("http://{addr}/app.css"),
            "type": "Stylesheet",
            "mimeType": "text/css",
            "contentSize": STYLESHEET_BODY.len(),
        })
    );
    let script = resources
        .iter()
        .find(|resource| resource["type"] == json!("Script"))
        .expect("script resource");
    assert_eq!(
        script,
        &json!({
            "url": format!("http://{addr}/app.js"),
            "type": "Script",
            "mimeType": "application/javascript",
            "contentSize": SCRIPT_BODY.len(),
        })
    );

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.getFrameTree",
        "sessionId": "SID-RESOURCE-TREE",
    }))
    .await;
    let frame_tree_response = take_response_by_id(&mut ctx, 2);
    assert!(
        frame_tree_response["result"]["frameTree"]
            .get("resources")
            .is_none(),
        "Page.getFrameTree must retain its existing response shape: {frame_tree_response}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn external_stylesheet_import_graph_is_fetched_once_and_reported_as_resources() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(StylesheetImportHits::default());
    let server_hits = Arc::clone(&hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/document",
                    axum::routing::get(resource_tree_import_document),
                )
                .route(
                    "/styles/root.css",
                    axum::routing::get(resource_tree_import_root),
                )
                .route(
                    "/styles/nested/child.css",
                    axum::routing::get(resource_tree_import_child),
                )
                .route(
                    "/styles/leaf.css",
                    axum::routing::get(resource_tree_import_leaf),
                )
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });
    let document_url = format!("http://{addr}/document");

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-RESOURCE-IMPORTS",
        "TID-RESOURCE-IMPORTS",
        "SID-RESOURCE-IMPORTS",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 19,
        "method": "Page.enable",
        "sessionId": "SID-RESOURCE-IMPORTS",
    }))
    .await;
    take_response_by_id(&mut ctx, 19);
    ctx.process_async(json!({
        "id": 20,
        "method": "Page.navigate",
        "sessionId": "SID-RESOURCE-IMPORTS",
        "params": { "url": document_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 20);
    wait_until_message(
        &mut ctx,
        "SID-RESOURCE-IMPORTS",
        "stylesheet import document stopped loading",
        |message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!("TID-RESOURCE-IMPORTS")
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.getResourceTree",
        "sessionId": "SID-RESOURCE-IMPORTS",
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 21);
    let resources = response["result"]["frameTree"]["resources"]
        .as_array()
        .expect("resource tree resources");
    let stylesheet_urls = resources
        .iter()
        .filter(|resource| resource["type"] == json!("Stylesheet"))
        .map(|resource| resource["url"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        stylesheet_urls,
        vec![
            format!("http://{addr}/styles/root.css"),
            format!("http://{addr}/styles/nested/child.css"),
            format!("http://{addr}/styles/leaf.css"),
        ],
        "nested imports must retain fetch order and stylesheet-relative URL resolution: {response}"
    );
    assert_eq!(hits.root.load(Ordering::SeqCst), 1);
    assert_eq!(hits.child.load(Ordering::SeqCst), 1);
    assert_eq!(hits.leaf.load(Ordering::SeqCst), 1);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_and_resource_trees_report_main_document_response_mime() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/document.xml",
                axum::routing::get(resource_tree_xml_document),
            ),
        )
        .await
        .unwrap();
    });
    let document_url = format!("http://{addr}/document.xml");

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-XML-RESOURCE-TREE",
        "TID-XML-RESOURCE-TREE",
        "SID-XML-RESOURCE-TREE",
        &document_url,
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(&document_url)
        .await
        .expect("XML page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_state_mut()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    for (id, method) in [(11, "Page.getResourceTree"), (12, "Page.getFrameTree")] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-XML-RESOURCE-TREE",
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(
            response["result"]["frameTree"]["frame"]["mimeType"], "application/xml",
            "{method} should expose the committed response MIME: {response}"
        );
        ctx.sent.clear();
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn search_in_resource_uses_original_document_and_subresource_sources() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/document", axum::routing::get(resource_search_document))
                .route("/search.js", axum::routing::get(resource_search_script))
                .route(
                    "/empty-document",
                    axum::routing::get(empty_resource_search_document),
                ),
        )
        .await
        .unwrap();
    });
    let document_url = format!("http://{addr}/document");
    let script_url = format!("http://{addr}/search.js");

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-RESOURCE-SEARCH",
        "TID-RESOURCE-SEARCH",
        "SID-RESOURCE-SEARCH",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 2,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": document_url,
            "query": "CDP Core Fixture"
        }
    }))
    .await;
    let disabled = take_response_by_id(&mut ctx, 2);
    assert_eq!(disabled["error"]["code"], json!(-32000));
    assert_eq!(disabled["error"]["message"], json!("Agent is not enabled."));

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.enable",
        "sessionId": "SID-RESOURCE-SEARCH",
    }))
    .await;
    take_response_by_id(&mut ctx, 3);

    ctx.process_async(json!({
        "id": 1,
        "method": "Page.navigate",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": { "url": document_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 1);
    wait_until_frame_stopped_loading(&mut ctx, "TID-RESOURCE-SEARCH").await;

    ctx.process_async(json!({
        "id": 4,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": format!("{document_url}#ignored"),
            "query": "CDP Core Fixture"
        }
    }))
    .await;
    let document_search = take_response_by_id(&mut ctx, 4);
    assert_eq!(
        document_search["result"]["result"],
        json!([
            {
                "lineNumber": 4,
                "lineContent": "<title>CDP Core Fixture</title>",
            },
            {
                "lineNumber": 8,
                "lineContent": "<h1 id=\"title\">CDP Core Fixture</h1>",
            },
        ])
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": format!("{script_url}#ignored"),
            "query": "BENCHMARKNEEDLE",
            "caseSensitive": false
        }
    }))
    .await;
    let script_search = take_response_by_id(&mut ctx, 5);
    assert_eq!(
        script_search["result"]["result"],
        json!([{
            "lineNumber": 1,
            "lineContent": "const benchmarkNeedle = 'script value';",
        }])
    );

    ctx.process_async(json!({
        "id": 6,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": document_url,
            "query": "data-runtime-only"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 6)["result"]["result"],
        json!([]),
        "search must use the response source, not a serialized live DOM"
    );

    ctx.process_async(json!({
        "id": 7,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": document_url,
            "query": "[",
            "isRegex": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 7)["result"]["result"],
        json!([]),
        "Chromium treats an invalid search regex as no matches"
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "MISSING-FRAME",
            "url": document_url,
            "query": "x"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 8)["error"]["message"],
        json!("No frame for given id found")
    );

    ctx.process_async(json!({
        "id": 9,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": format!("http://{addr}/missing.js"),
            "query": "x"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 9)["error"]["message"],
        json!("No resource with given URL found")
    );

    let empty_document_url = format!("http://{addr}/empty-document");
    ctx.process_async(json!({
        "id": 10,
        "method": "Page.navigate",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": { "url": empty_document_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 10);
    ctx.process_async(json!({
        "id": 11,
        "method": "Page.searchInResource",
        "sessionId": "SID-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-RESOURCE-SEARCH",
            "url": empty_document_url,
            "query": "x"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 11)["error"]["message"],
        json!("Content unavailable. Resource was not cached")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn search_in_resource_routes_child_document_and_child_subresource_sources() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(child_resource_search_parent))
                .route(
                    "/child-document",
                    axum::routing::get(child_resource_search_document),
                )
                .route(
                    "/child-search.js",
                    axum::routing::get(child_resource_search_script),
                )
                .route(
                    "/empty-child-parent",
                    axum::routing::get(empty_child_resource_search_parent),
                )
                .route(
                    "/empty-child-document",
                    axum::routing::get(empty_child_resource_search_document),
                ),
        )
        .await
        .unwrap();
    });
    let parent_url = format!("http://{addr}/parent");
    let child_url = format!("http://{addr}/child-document");
    let child_script_url = format!("http://{addr}/child-search.js");
    let empty_child_parent_url = format!("http://{addr}/empty-child-parent");
    let empty_child_url = format!("http://{addr}/empty-child-document");

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-CHILD-RESOURCE-SEARCH",
        "TID-CHILD-RESOURCE-SEARCH",
        "SID-CHILD-RESOURCE-SEARCH",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 21,
        "method": "Page.enable",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
    }))
    .await;
    take_response_by_id(&mut ctx, 21);
    ctx.process_async(json!({
        "id": 20,
        "method": "Page.navigate",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
        "params": { "url": parent_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 20);

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.getFrameTree",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 22);
    let child_frame_id = frame_tree["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();
    wait_for_resource_search_child_document_commit(
        &mut ctx,
        "SID-CHILD-RESOURCE-SEARCH",
        &child_frame_id,
        &child_url,
    )
    .await;

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.searchInResource",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
        "params": {
            "frameId": child_frame_id,
            "url": format!("{child_url}#ignored"),
            "query": "Child Resource Fixture"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 23)["result"]["result"],
        json!([{
            "lineNumber": 2,
            "lineContent": "<p>Child Resource Fixture</p>",
        }])
    );

    ctx.process_async(json!({
        "id": 24,
        "method": "Page.searchInResource",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
        "params": {
            "frameId": child_frame_id,
            "url": child_script_url,
            "query": "childResourceNeedle"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 24)["result"]["result"],
        json!([{
            "lineNumber": 0,
            "lineContent": CHILD_SEARCH_SCRIPT_BODY,
        }])
    );

    ctx.process_async(json!({
        "id": 25,
        "method": "Page.navigate",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
        "params": { "url": empty_child_parent_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 25);
    ctx.process_async(json!({
        "id": 26,
        "method": "Page.getFrameTree",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
    }))
    .await;
    let empty_child_frame_id =
        take_response_by_id(&mut ctx, 26)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .expect("empty child frame id")
            .to_owned();
    wait_for_resource_search_child_document_commit(
        &mut ctx,
        "SID-CHILD-RESOURCE-SEARCH",
        &empty_child_frame_id,
        &empty_child_url,
    )
    .await;
    ctx.process_async(json!({
        "id": 27,
        "method": "Page.searchInResource",
        "sessionId": "SID-CHILD-RESOURCE-SEARCH",
        "params": {
            "frameId": empty_child_frame_id,
            "url": empty_child_url,
            "query": "x"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 27)["error"]["message"],
        json!("Content unavailable. Resource was not cached")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn search_in_resource_accepts_an_empty_cached_child_document() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let child_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_hits = std::sync::Arc::clone(&child_hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/parent",
                    axum::routing::get(cacheable_empty_child_resource_search_parent),
                )
                .route(
                    "/cached-empty-child-document",
                    axum::routing::get(cacheable_empty_child_resource_search_document),
                )
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });
    let parent_url = format!("http://{addr}/parent");
    let child_url = format!("http://{addr}/cached-empty-child-document");
    let cache_dir = unique_resource_search_cache_dir();
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let mut ctx = TestContext::from_conn(crate::conn::CdpConnection::new_with_fetch_config(
        fetch_config,
    ));
    load_bc_with_session(
        &mut ctx,
        "BID-CACHED-CHILD-RESOURCE-SEARCH",
        "TID-CACHED-CHILD-RESOURCE-SEARCH",
        "SID-CACHED-CHILD-RESOURCE-SEARCH",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 28,
        "method": "Page.enable",
        "sessionId": "SID-CACHED-CHILD-RESOURCE-SEARCH",
    }))
    .await;
    take_response_by_id(&mut ctx, 28);
    ctx.process_async(json!({
        "id": 29,
        "method": "Page.navigate",
        "sessionId": "SID-CACHED-CHILD-RESOURCE-SEARCH",
        "params": { "url": parent_url }
    }))
    .await;
    let first_loader_id = take_response_by_id(&mut ctx, 29)["result"]["loaderId"]
        .as_str()
        .expect("first parent navigation loader id")
        .to_owned();
    wait_until_renderer_document_load(
        &mut ctx,
        Some("SID-CACHED-CHILD-RESOURCE-SEARCH"),
        "TID-CACHED-CHILD-RESOURCE-SEARCH",
        &first_loader_id,
    )
    .await;
    let first_child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 30).await;
    wait_for_resource_search_child_document_commit(
        &mut ctx,
        "SID-CACHED-CHILD-RESOURCE-SEARCH",
        &first_child_frame_id,
        &child_url,
    )
    .await;
    ctx.process_async(json!({
        "id": 31,
        "method": "Page.searchInResource",
        "sessionId": "SID-CACHED-CHILD-RESOURCE-SEARCH",
        "params": {
            "frameId": first_child_frame_id,
            "url": child_url,
            "query": "x"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 31)["error"]["message"],
        json!("Content unavailable. Resource was not cached")
    );
    // `Page.navigate` acknowledges a replacement before its renderer
    // Document commits. Do not let the second child wait match retained
    // `frameNavigated` output from the first Document generation.
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 32,
        "method": "Page.navigate",
        "sessionId": "SID-CACHED-CHILD-RESOURCE-SEARCH",
        "params": { "url": parent_url }
    }))
    .await;
    let cached_loader_id = take_response_by_id(&mut ctx, 32)["result"]["loaderId"]
        .as_str()
        .expect("cached parent navigation loader id")
        .to_owned();
    wait_until_renderer_document_load(
        &mut ctx,
        Some("SID-CACHED-CHILD-RESOURCE-SEARCH"),
        "TID-CACHED-CHILD-RESOURCE-SEARCH",
        &cached_loader_id,
    )
    .await;
    let cached_child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 33).await;
    wait_for_resource_search_child_document_commit(
        &mut ctx,
        "SID-CACHED-CHILD-RESOURCE-SEARCH",
        &cached_child_frame_id,
        &child_url,
    )
    .await;
    assert_eq!(
        child_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the second child navigation must use the cached empty response"
    );
    ctx.process_async(json!({
        "id": 34,
        "method": "Page.searchInResource",
        "sessionId": "SID-CACHED-CHILD-RESOURCE-SEARCH",
        "params": {
            "frameId": cached_child_frame_id,
            "url": child_url,
            "query": "x"
        }
    }))
    .await;
    let search_response = take_response_by_id(&mut ctx, 34);
    assert_eq!(
        search_response["result"]["result"],
        json!([]),
        "cached empty child document should remain searchable: {search_response}"
    );

    server.abort();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_in_resource_does_not_invent_an_initial_about_blank_resource() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-EMPTY-RESOURCE-SEARCH",
        "TID-EMPTY-RESOURCE-SEARCH",
        "SID-EMPTY-RESOURCE-SEARCH",
        "about:blank",
    );
    ensure_initial_document_for_session(&mut ctx, Some("SID-EMPTY-RESOURCE-SEARCH")).await;
    ctx.process_async(json!({
        "id": 30,
        "method": "Page.enable",
        "sessionId": "SID-EMPTY-RESOURCE-SEARCH",
    }))
    .await;
    take_response_by_id(&mut ctx, 30);

    ctx.process_async(json!({
        "id": 31,
        "method": "Page.searchInResource",
        "sessionId": "SID-EMPTY-RESOURCE-SEARCH",
        "params": {
            "frameId": "TID-EMPTY-RESOURCE-SEARCH",
            "url": "about:blank",
            "query": "html"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 31)["error"]["message"],
        json!("No resource with given URL found")
    );
}
