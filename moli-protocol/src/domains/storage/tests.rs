use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

use crate::{
    conn::{BrowserContext, CdpCommandTaskStep, PageTargetHost},
    domains::page::LOADER_ID,
    testing::{TestContext, wait_until_message, wait_until_renderer_document_load},
};
use moli_cookie_jar::test_support::{
    BrowserCookieStore, NetworkSameSiteContext, NetworkSameSiteContextDowngradeType,
    NetworkSiteContext,
};
use moli_cookie_jar::{
    NetworkCookieRequestContext, NetworkSiteContextMetadata, StoredCookieExclusionReason,
    StoredCookieQueryReport,
};

struct HttpCacheTestRoot {
    path: PathBuf,
}

impl HttpCacheTestRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-cdp-http-cache-{label}-{}-{nonce}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self { path }
    }
}

impl Drop for HttpCacheTestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn take_response_by_id(ctx: &mut TestContext, id: u64) -> Value {
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .expect("expected response with matching id");
    ctx.sent.remove(pos)
}

fn take_result_by_id(ctx: &mut TestContext, id: u64) -> Value {
    let response = take_response_by_id(ctx, id);
    assert!(
        response.get("error").is_none(),
        "expected result response, got {response:?}"
    );
    response
        .get("result")
        .cloned()
        .expect("response should include result")
}

fn json_number_as_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|value| value as u64))
        .expect("expected JSON number")
}

fn first_party_storage_key_for_origin(origin: &str) -> String {
    let url = Url::parse(origin).expect("origin should parse as URL");
    moli_storage_key::MoliStorageKey::first_party_from_url(&url, None).serialized_storage_key()
}

async fn wait_for_child_frame_navigated_url(
    ctx: &mut TestContext,
    child_frame_id: &str,
    expected_url: &str,
) {
    let child_frame_id = child_frame_id.to_owned();
    let expected_url = expected_url.to_owned();
    wait_until_message(
        ctx,
        None::<&str>,
        "child frame Page.frameNavigated for storage key fixture",
        move |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"] == json!(expected_url)
        },
    )
    .await;
}

fn usage_breakdown_value(result: &Value, storage_type: &str) -> u64 {
    let entries = result["usageBreakdown"]
        .as_array()
        .expect("usageBreakdown should be an array");
    let entry = entries
        .iter()
        .find(|entry| entry["storageType"].as_str() == Some(storage_type))
        .expect("expected usageBreakdown entry");
    json_number_as_u64(&entry["usage"])
}

const STORAGE_TEST_COMPLETION_BINDING: &str = "__moliStorageTestComplete";

async fn install_storage_test_completion_binding(ctx: &mut TestContext) {
    ctx.process_async(json!({
        "id": 91_001,
        "method": "Runtime.enable"
    }))
    .await;
    assert_eq!(take_result_by_id(ctx, 91_001), json!({}));

    ctx.process_async(json!({
        "id": 91_002,
        "method": "Runtime.addBinding",
        "params": { "name": STORAGE_TEST_COMPLETION_BINDING }
    }))
    .await;
    assert_eq!(take_result_by_id(ctx, 91_002), json!({}));
}

async fn wait_for_storage_test_completion(ctx: &mut TestContext) -> String {
    let event = ctx
        .wait_for_scheduler_message("IndexedDB transaction completion binding", |message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!(STORAGE_TEST_COMPLETION_BINDING)
        })
        .await;
    event["params"]["payload"]
        .as_str()
        .expect("storage completion binding payload should be a string")
        .to_owned()
}

fn seed_indexed_db_usage(manager: &moli_core::storage::SharedIndexedDbManager, origin: &str) {
    let mut manager = manager.lock();
    let opened = manager
        .open(moli_core::storage::IndexedDbOpenOptions {
            origin: origin.to_owned(),
            name: "site-data".to_owned(),
            version: Some(1),
        })
        .expect("IndexedDB seed open should succeed");
    let upgrade = opened
        .upgrade_transaction
        .expect("IndexedDB seed should create an upgrade transaction");
    manager
        .create_object_store(
            upgrade,
            "items",
            moli_core::storage::IndexedDbObjectStoreOptions::default(),
        )
        .expect("IndexedDB seed object store should be created");
    manager
        .commit_transaction(upgrade)
        .expect("IndexedDB seed upgrade should commit");
    let tx = manager
        .begin_transaction(
            opened.database,
            &[String::from("items")],
            moli_core::storage::IndexedDbTransactionMode::ReadWrite,
        )
        .expect("IndexedDB seed readwrite transaction should start");
    manager
        .put(
            tx,
            "items",
            Some(moli_core::storage::IndexedDbKey::from("key")),
            b"value".to_vec(),
        )
        .expect("IndexedDB seed put should succeed");
    manager
        .commit_transaction(tx)
        .expect("IndexedDB seed write should commit");
    manager
        .close_database(opened.database)
        .expect("IndexedDB seed database should close");
}

fn bucket_indexed_db_storage_key_for_test(
    context: &BrowserContext,
    storage_key: &str,
    name: &str,
) -> String {
    context
        .storage_bucket_store_for_test()
        .lock()
        .bucket_identity(storage_key, name)
        .unwrap_or_else(|| panic!("storage bucket `{name}` should have a persistent identity"))
        .indexed_db_storage_key()
}

fn seed_storage_bucket_opfs_usage_for_test(
    context: &BrowserContext,
    identity: &moli_core::storage::StorageBucketIdentity,
    file_name: &str,
) -> u64 {
    let storage_service = context
        .storage_bucket_store_for_test()
        .lock()
        .storage_service();
    let locator = identity.locator();
    let bucket_key = moli_storage_service::StorageService::opfs_bucket_key(&locator)
        .expect("bucket OPFS key should derive");
    let root = storage_service
        .ensure_opfs_root(&locator)
        .expect("bucket OPFS root should open");
    let file = storage_service
        .with_opfs(|opfs| opfs.get_file(&bucket_key, &root, file_name, true))
        .expect("bucket OPFS file should open");
    storage_service
        .with_opfs(|opfs| opfs.write_file(&bucket_key, &file, b"bucket opfs bytes", None))
        .expect("bucket OPFS file should write");
    storage_service
        .opfs_usage(&locator)
        .expect("bucket OPFS usage should load")
}

fn encoded_storage_bucket_cache_component(value: &str) -> String {
    percent_encoding::percent_encode(value.as_bytes(), percent_encoding::NON_ALPHANUMERIC)
        .to_string()
}

fn write_storage_bucket_cache_file(
    cache_storage_root: &std::path::Path,
    origin: &str,
    bucket_id: moli_storage_service::StorageBucketId,
    cache_name: &str,
    request_key: &str,
    body: &[u8],
    usage_bytes: u64,
) {
    let path = cache_storage_root
        .join(encoded_storage_bucket_cache_component(origin))
        .join(format!("bucket-{}", bucket_id.get()))
        .join(format!(
            "{}.json",
            encoded_storage_bucket_cache_component(cache_name)
        ));
    std::fs::create_dir_all(path.parent().expect("cache file should have a parent"))
        .expect("StorageBucket CacheStorage seed dir should be created");
    let mut entries = serde_json::Map::new();
    entries.insert(
        request_key.to_owned(),
        json!({
            "usageBytes": usage_bytes,
            "status": 200,
            "statusText": "OK",
            "headers": [["x-seed", "cache"]],
            "bodyBase64": BASE64_STANDARD.encode(body)
        }),
    );
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "version": 1,
            "entries": entries
        }))
        .expect("StorageBucket CacheStorage seed should serialize"),
    )
    .expect("StorageBucket CacheStorage seed file should be written");
}

async fn spawn_cacheable_text_server(
    body: &'static str,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let route_hits = Arc::clone(&hits);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("cache test listener should bind");
    let addr = listener
        .local_addr()
        .expect("cache test listener should have addr");
    let app = axum::Router::new().route(
        "/resource",
        axum::routing::get(move || {
            let route_hits = Arc::clone(&route_hits);
            async move {
                route_hits.fetch_add(1, Ordering::SeqCst);
                (
                    [
                        (axum::http::header::CACHE_CONTROL.as_str(), "max-age=60"),
                        (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                    ],
                    body,
                )
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("cache test server should serve");
    });
    (format!("http://{addr}/resource"), hits, server)
}

async fn spawn_static_html_server(
    body: impl Into<String>,
) -> (String, tokio::task::JoinHandle<()>) {
    let body = Arc::new(body.into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("static html test listener should bind");
    let addr = listener
        .local_addr()
        .expect("static html test listener should have addr");
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
            .expect("static html test server should serve");
    });
    (format!("http://{addr}/page"), server)
}

async fn spawn_partitioned_child_frame_server() -> (String, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("partitioned child frame test listener should bind");
    let addr = listener
        .local_addr()
        .expect("partitioned child frame test listener should have addr");
    let child_url = format!("http://localhost:{}/child", addr.port());
    let top_body = Arc::new(format!(r#"<iframe src="{child_url}"></iframe>"#));
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
                    "<p>child</p>",
                )
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("partitioned child frame test server should serve");
    });
    (format!("http://{addr}/page"), child_url, server)
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_frame_returns_top_frame_origin() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-SK-TOP".into());
    bc.set_active_target_id("TID-SK-TOP");
    bc.set_target_url("https://top.example/app".into());
    bc.set_target_security_origin("https://top.example".into());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 1,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-SK-TOP" }
    }))
    .await;

    ctx.expect_result(
        1,
        json!({ "storageKey": first_party_storage_key_for_origin("https://top.example") }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_top_frame_uses_loaded_page_storage_key() {
    let mut ctx = TestContext::new();
    let (page_url, server) = spawn_static_html_server(r#"<p>top</p>"#).await;
    let page_origin = Url::parse(&page_url)
        .expect("page url should parse")
        .origin()
        .ascii_serialization();
    let mut bc = BrowserContext::new("BID-SK-TOP-LIVE".into());
    bc.set_active_target_id("TID-SK-TOP-LIVE");
    bc.set_target_url("https://stale-target-url.example/app".into());
    bc.set_target_security_origin("https://stale-target-url.example".into());
    bc.set_target_secure_context_type("Secure".into());
    ctx.conn.browser_context = Some(bc);

    ctx.install_navigation_fixture_for_session_owner(&page_url, None)
        .await;

    ctx.process_async(json!({
        "id": 11,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-SK-TOP-LIVE" }
    }))
    .await;

    ctx.expect_result(
        11,
        json!({ "storageKey": first_party_storage_key_for_origin(&page_origin) }),
        None,
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_frame_returns_child_frame_inherited_origin() {
    let mut ctx = TestContext::new();
    let (page_url, server) =
        spawn_static_html_server(r#"<iframe name="srcdoc-child" srcdoc="<p>child</p>"></iframe>"#)
            .await;
    let top_origin = Url::parse(&page_url)
        .expect("page url should parse")
        .origin();
    let top_origin = top_origin.ascii_serialization();
    let mut bc = BrowserContext::new("BID-SK-CHILD".into());
    bc.set_active_target_id("TID-SK-CHILD");
    bc.set_target_url(page_url.clone());
    bc.set_target_security_origin(top_origin.clone());
    bc.set_target_secure_context_type("Secure".into());
    ctx.conn.browser_context = Some(bc);

    ctx.install_navigation_fixture_for_session_owner(&page_url, None)
        .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.getFrameTree"
    }))
    .await;
    let child_frame_id =
        ctx.take_response_by_id(2)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .expect("child frame id")
            .to_owned();

    ctx.process_async(json!({
        "id": 3,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": child_frame_id }
    }))
    .await;

    ctx.expect_result(
        3,
        json!({ "storageKey": first_party_storage_key_for_origin(&top_origin) }),
        None,
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_credentialless_child_uses_page_nonce() {
    let mut ctx = TestContext::new();
    let (page_url, server) = spawn_static_html_server(
        r#"<iframe id="child" name="credentialless-child" credentialless></iframe>"#,
    )
    .await;
    let top_origin = Url::parse(&page_url)
        .expect("page url should parse")
        .origin()
        .ascii_serialization();
    let mut bc = BrowserContext::new("BID-SK-CREDENTIALLESS".into());
    bc.set_active_target_id("TID-SK-CREDENTIALLESS");
    bc.set_target_url(page_url.clone());
    bc.set_target_security_origin(top_origin.clone());
    bc.set_target_secure_context_type("Secure".into());
    ctx.conn.browser_context = Some(bc);

    ctx.install_navigation_fixture_for_session_owner(&page_url, None)
        .await;

    ctx.process_async(json!({
        "id": 12,
        "method": "Page.getFrameTree"
    }))
    .await;
    let tree = take_response_by_id(&mut ctx, 12);
    let child_frame_id = tree["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    ctx.process_async(json!({
        "id": 13,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let child_storage_key = take_result_by_id(&mut ctx, 13)["storageKey"]
        .as_str()
        .expect("child storage key")
        .to_owned();
    assert!(
        child_storage_key.starts_with(&format!("storage-key:v1;origin={top_origin};")),
        "credentialless child keeps its inherited origin in the storage key: {child_storage_key}"
    );
    assert!(
        child_storage_key.contains(";opaque-nonce="),
        "credentialless child storage key should include the page credentialless nonce: {child_storage_key}"
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
(() => {
  const child = document.getElementById("child");
  const before = child.contentWindow.credentialless;
  child.credentialless = false;
  const ownerAfterRemoval = child.credentialless;
  const documentAfterRemoval = child.contentWindow.credentialless;
  const grandchild = child.contentDocument.createElement("iframe");
  grandchild.name = "credentialless-grandchild";
  child.contentDocument.body.appendChild(grandchild);
  return [
    before,
    ownerAfterRemoval,
    documentAfterRemoval,
    grandchild.contentWindow.credentialless
  ].join("|");
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 14)["result"]["value"],
        json!("true|false|true|true"),
        "current credentialless document should not downgrade when the owner attribute changes, and child documents inherit it"
    );

    ctx.process_async(json!({
        "id": 15,
        "method": "Page.getFrameTree"
    }))
    .await;
    let tree = take_response_by_id(&mut ctx, 15);
    let grandchild_frame_id =
        tree["result"]["frameTree"]["childFrames"][0]["childFrames"][0]["frame"]["id"]
            .as_str()
            .expect("grandchild frame id")
            .to_owned();

    ctx.process_async(json!({
        "id": 16,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": grandchild_frame_id }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 16)["storageKey"],
        json!(child_storage_key),
        "credentialless documents in the same page should share the same page nonce"
    );

    ctx.process_async(json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
(() => {
  const child = document.getElementById("child");
  child.contentDocument.open();
  child.contentDocument.write("<!doctype html><body>reset</body>");
  child.contentDocument.close();
  return String(child.contentWindow.credentialless);
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_result_by_id(&mut ctx, 17)["result"]["value"],
        json!("true"),
        "document.open() must preserve the current LocalWindow credentialless policy after the owner attribute changes"
    );

    ctx.process_async(json!({
        "id": 18,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let reset_child_storage_key = take_result_by_id(&mut ctx, 18)["storageKey"]
        .as_str()
        .expect("reset child storage key")
        .to_owned();
    assert_eq!(
        reset_child_storage_key, child_storage_key,
        "same-LocalWindow document.open() must preserve the credentialless page nonce"
    );

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_frame_returns_child_frame_partition_key() {
    let mut ctx = TestContext::new();
    let (page_url, child_url, server) = spawn_partitioned_child_frame_server().await;
    let top_origin = Url::parse(&page_url)
        .expect("page url should parse")
        .origin();
    let top_origin = top_origin.ascii_serialization();
    let mut bc = BrowserContext::new("BID-SK-PARTITIONED".into());
    bc.set_active_target_id("TID-SK-PARTITIONED");
    bc.set_target_url(page_url.clone());
    bc.set_target_security_origin(top_origin);
    bc.set_target_secure_context_type("Secure".into());
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(None);

    ctx.install_navigation_fixture_for_session_owner(&page_url, None)
        .await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.getFrameTree"
    }))
    .await;
    let child_frame_id =
        ctx.take_response_by_id(21)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .expect("child frame id")
            .to_owned();
    wait_for_child_frame_navigated_url(&mut ctx, &child_frame_id, &child_url).await;

    ctx.process_async(json!({
        "id": 22,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": child_frame_id }
    }))
    .await;

    ctx.expect_result(
        22,
        json!({
            "storageKey": format!(
                "storage-key:v1;origin={};top-level-site=http://127.0.0.1",
                Url::parse(&child_url)
                    .expect("child url should parse")
                    .origin()
                    .ascii_serialization()
            )
        }),
        None,
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_frame_rejects_opaque_child_frame() {
    let mut ctx = TestContext::new();
    let opaque_child_url = "data:text/html,<p>opaque</p>";
    let (page_url, server) =
        spawn_static_html_server(&format!(r#"<iframe src="{opaque_child_url}"></iframe>"#)).await;
    let top_origin = Url::parse(&page_url)
        .expect("page url should parse")
        .origin();
    let top_origin = top_origin.ascii_serialization();
    let mut bc = BrowserContext::new("BID-SK-OPAQUE".into());
    bc.set_active_target_id("TID-SK-OPAQUE");
    bc.set_target_url(page_url.clone());
    bc.set_target_security_origin(top_origin);
    bc.set_target_secure_context_type("Secure".into());
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(None);

    ctx.install_navigation_fixture_for_session_owner(&page_url, None)
        .await;
    wait_until_renderer_document_load(&mut ctx, None, "TID-SK-OPAQUE", LOADER_ID).await;

    ctx.process_async(json!({
        "id": 31,
        "method": "Page.getFrameTree"
    }))
    .await;
    let frame_tree = ctx.take_response_by_id(31);
    let child_frame = &frame_tree["result"]["frameTree"]["childFrames"][0]["frame"];
    let child_frame_id = child_frame["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();
    assert_eq!(
        child_frame["url"],
        json!(opaque_child_url),
        "root load must not become observable before the data: child navigation commits"
    );

    ctx.process_async(json!({
        "id": 32,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": child_frame_id }
    }))
    .await;

    ctx.expect_error(
        32,
        -32000,
        "Frame corresponds to an opaque origin and its storage key cannot be serialized",
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_storage_key_for_frame_rejects_unknown_frame() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-SK-MISSING".into());
    bc.set_active_target_id("TID-SK-MISSING");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 4,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-NOPE" }
    }))
    .await;

    ctx.expect_error(4, -32000, "NoFrameForGivenId");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_key_targets_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let (page_url, server) =
        spawn_static_html_server(r#"<iframe srcdoc="<p>child</p>"></iframe>"#).await;
    let top_origin = Url::parse(&page_url)
        .expect("page url should parse")
        .origin()
        .ascii_serialization();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-SK-BG".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 101,
        "sessionId": "SID-background",
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-background" }
    }))
    .await;
    ctx.expect_result(
        101,
        json!({ "storageKey": first_party_storage_key_for_origin(&top_origin) }),
        Some("SID-background"),
    );

    ctx.process_async(json!({
        "id": 102,
        "sessionId": "SID-background",
        "method": "Page.getFrameTree"
    }))
    .await;
    let child_frame_id =
        ctx.take_response_by_id(102)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .expect("child frame id")
            .to_owned();
    ctx.process_async(json!({
        "id": 103,
        "sessionId": "SID-background",
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    ctx.expect_result(
        103,
        json!({ "storageKey": first_party_storage_key_for_origin(&top_origin) }),
        Some("SID-background"),
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some("TID-active")
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_storage_key_keeps_background_owner_route_across_completion() {
    let mut ctx = TestContext::new();
    let (active_url, active_server) = spawn_static_html_server(r#"<p>active storage</p>"#).await;
    let (background_url, background_server) =
        spawn_static_html_server(r#"<p>background storage</p>"#).await;
    let background_origin = Url::parse(&background_url)
        .expect("background url should parse")
        .origin()
        .ascii_serialization();

    let active_page = ctx
        .conn
        .load_page_via_runtime_async(&active_url)
        .await
        .expect("active page should load");
    let background_page = ctx
        .conn
        .load_page_via_runtime_async(&background_url)
        .await
        .expect("background page should load");

    let mut background = PageTargetHost::with_url(
        "TID-storage-background".to_owned(),
        None,
        background_page.final_url().as_str().to_owned(),
    );
    background.replace_loaded_page(Some(background_page));

    let mut bc = BrowserContext::new("BID-storage-owner-route".to_owned());
    bc.set_active_target_id("TID-storage-active".to_owned());
    bc.set_target_url(active_page.final_url().as_str().to_owned());
    bc.active_target
        .runtime_slot
        .set_loaded_page_for_test(active_page);
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);

    let background_route = ctx
        .conn
        .target_session_route_for_target_id("TID-storage-background")
        .expect("background target route");
    let raw = serde_json::to_string(&json!({
        "id": 104,
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-storage-background" }
    }))
    .unwrap();
    let pending = {
        let previous_route = ctx
            .conn
            .replace_none_session_owner_route_override(Some(background_route.clone()));
        let step = ctx.conn.start_command_dispatch(&raw);
        ctx.conn
            .replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(outcome) => {
                panic!(
                    "background Storage.getStorageKeyForFrame should snapshot the live background page: {:?}",
                    outcome.into_parts().0
                )
            }
        }
    };

    let active_route = ctx
        .conn
        .target_session_route_for_target_id("TID-storage-active")
        .expect("active target route");
    let previous_route = ctx
        .conn
        .replace_none_session_owner_route_override(Some(active_route));
    let (messages, scheduler_events) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(pending))
        .await;
    ctx.conn
        .replace_none_session_owner_route_override(previous_route);

    assert!(scheduler_events.is_empty());
    assert_eq!(
        messages,
        vec![json!({
            "id": 104,
            "result": {
                "storageKey": first_party_storage_key_for_origin(&background_origin)
            }
        })],
        "Storage.getStorageKeyForFrame completion must use the captured background owner"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some("TID-storage-active")
    );

    active_server.abort();
    let _ = active_server.await;
    background_server.abort();
    let _ = background_server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_key_targets_inactive_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut active = BrowserContext::new("BID-active".to_owned());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active".to_owned());
    ctx.conn.browser_context = Some(active);

    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.set_target_url("https://inactive.example/app".to_owned());
    inactive.set_target_security_origin("https://inactive.example".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 111,
        "sessionId": "SID-inactive",
        "method": "Storage.getStorageKeyForFrame",
        "params": { "frameId": "TID-inactive" }
    }))
    .await;
    ctx.expect_result(
        111,
        json!({ "storageKey": first_party_storage_key_for_origin("https://inactive.example") }),
        Some("SID-inactive"),
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_cookies_accepts_auxiliary_page_session_route() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-aux-storage".into());
    browser_context.set_active_target_id("TID-aux-storage".to_owned());
    assert!(
        browser_context
            .assign_auxiliary_session_to_target("TID-aux-storage", "SID-aux-storage".to_owned())
    );
    ctx.conn.browser_context = Some(browser_context);

    ctx.process_async(json!({
        "id": 8,
        "method": "Storage.getCookies",
        "sessionId": "SID-aux-storage"
    }))
    .await;

    ctx.expect_result(8, json!({ "cookies": [] }), Some("SID-aux-storage"));
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_cookies_accepts_background_page_session_route() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-background-storage".into());
    browser_context.set_active_target_id("TID-active-storage".to_owned());
    browser_context.stage_background_target(
        "TID-background-storage".to_owned(),
        Some("SID-background-storage".to_owned()),
        "about:blank".to_owned(),
        None,
        None,
    );
    ctx.conn.browser_context = Some(browser_context);

    ctx.process_async(json!({
        "id": 9,
        "method": "Storage.getCookies",
        "sessionId": "SID-background-storage"
    }))
    .await;

    ctx.expect_result(9, json!({ "cookies": [] }), Some("SID-background-storage"));
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_cookie_metadata_round_trip_includes_priority_and_source() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-M".into()));

    ctx.process_async(json!({
        "id": 10,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-M",
            "cookies": [{
                "name": "meta",
                "value": "1",
                "url": "https://example.com:8443/app",
                "priority": "High",
                "sourceScheme": "Secure",
                "sourcePort": 8443
            }]
        }
    }))
    .await;
    ctx.expect_result(10, json!({}), None);

    ctx.process_async(json!({
        "id": 11,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-M" }
    }))
    .await;
    ctx.expect_result(
        11,
        json!({
            "cookies": [{
                "name": "meta",
                "value": "1",
                "priority": "High",
                "sourceScheme": "Secure",
                "sourcePort": 8443
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_cookies_omits_same_site_for_unspecified_cookie() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-U".into()));

    ctx.process_async(json!({
        "id": 111,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-U",
            "cookies": [{
                "name": "plain",
                "value": "1",
                "url": "https://example.com/app"
            }]
        }
    }))
    .await;
    ctx.expect_result(111, json!({}), None);

    ctx.process_async(json!({
        "id": 112,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-U" }
    }))
    .await;
    let result = ctx.take_one();
    let cookies = result["result"]["cookies"]
        .as_array()
        .expect("storage cookies array");
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].get("sameSite").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_returns_cookie_reports_for_accepted_cookie() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-R".into()));

    ctx.process_async(json!({
        "id": 15,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-R",
            "cookies": [{
                "name": "strict",
                "value": "1",
                "url": "https://example.com/app",
                "secure": true,
                "sameSite": "Strict"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        15,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "Strict",
                "warningReasons": []
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_reports_secure_access_warning_for_localhost_http() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-RW".into()));

    ctx.process_async(json!({
        "id": 17,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-RW",
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "http://localhost/app",
                "secure": true
            }]
        }
    }))
    .await;
    ctx.expect_result(
        17,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": ["SecureAccessGrantedNonCryptographic"]
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_returns_cookie_reports_for_rejected_cookie() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-RJ".into()));

    ctx.process_async(json!({
        "id": 16,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-RJ",
            "cookies": [{
                "name": "cross",
                "value": "1",
                "url": "https://example.com/app",
                "secure": false,
                "sameSite": "None"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        16,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "SameSiteNoneRequiresSecure"
                },
                "rejectionReasons": ["SameSiteNoneRequiresSecure"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_reports_multiple_rejection_reasons() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-RM".into()));

    ctx.process_async(json!({
        "id": 18,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-RM",
            "cookies": [{
                "name": "__Host-cross",
                "value": "1",
                "url": "https://example.com/app",
                "secure": false,
                "sameSite": "None"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        18,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "SameSiteNoneRequiresSecure"
                },
                "rejectionReasons": [
                    "SameSiteNoneRequiresSecure",
                    "PrefixViolation"
                ],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_reports_public_suffix_rejection() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PSL".into()));

    ctx.process_async(json!({
        "id": 19,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-PSL",
            "cookies": [{
                "name": "wide",
                "value": "1",
                "url": "https://foo.co.uk/app",
                "domain": "co.uk",
                "secure": true
            }]
        }
    }))
    .await;
    ctx.expect_result(
        19,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "PublicSuffix"
                },
                "rejectionReasons": ["PublicSuffix"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}

#[test]
fn cookie_query_report_json_projects_specific_same_site_warning_taxonomy() {
    let mut store = BrowserCookieStore::default();
    let url = Url::parse("https://example.com/foo/bar").unwrap();

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/foo; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &url,
        NetworkCookieRequestContext::subresource("GET")
            .with_site_context(NetworkSiteContext::new(
                NetworkSameSiteContext::SameSiteStrict,
                NetworkSameSiteContext::CrossSite,
            ))
            .with_site_context_metadata(NetworkSiteContextMetadata::schemeful_only(
                false,
                Some(NetworkSameSiteContextDowngradeType::StrictToCross),
            )),
    );
    let json_report = super::cookie_query_report_to_json(&report);

    assert_eq!(json_report["facadeExclusionReasons"], json!([]));
    assert_eq!(
        json_report["facadeStatus"],
        json!({
            "cookieAccessEnabled": true,
            "storeAvailable": true,
            "blockedReasons": [],
        })
    );
    assert_eq!(
        json_report["excludedCookies"][0]["exclusionReasons"],
        json!(["SameSiteLax"])
    );
    assert_eq!(
        json_report["excludedCookies"][0]["warningReasons"],
        json!(["StrictCrossDowngradeLaxSameSite"])
    );
    assert_eq!(
        json_report["excludedCookies"][0]["siteForCookiesSource"],
        json!("Unset")
    );
    assert_eq!(
        json_report["excludedCookies"][0]["topFrameOriginSource"],
        json!("Unset")
    );
    assert_eq!(
        json_report["excludedCookies"][0]["storageAccessStatusSource"],
        json!("RequestContext")
    );
}

#[test]
fn associated_cookies_json_projects_path_mismatch_to_cdp_not_on_path() {
    let mut store = BrowserCookieStore::default();
    let response_url = Url::parse("https://example.com/private/index.html").unwrap();
    let request_url = Url::parse("https://example.com/public/index.html").unwrap();

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/private; Secure".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    assert_eq!(report.excluded_cookies.len(), 1);
    assert_eq!(
        super::cookie_query_report_to_json(&report)["excludedCookies"][0]["exclusionReasons"],
        json!(["PathMismatch"]),
        "private diagnostics retain the cookie-jar reason name"
    );
    assert_eq!(
        super::associated_cookies_to_json(&report)[0]["blockedReasons"],
        json!(["NotOnPath"]),
        "public CDP output uses Network.CookieBlockedReason"
    );
}

#[test]
fn cookie_query_report_json_projects_facade_exclusion_reasons() {
    let json_report = super::cookie_query_report_to_json(&StoredCookieQueryReport {
        facade_status: moli_cookie_jar::StoredCookieFacadeStatus {
            cookie_access_enabled: false,
            store_available: false,
            blocked_reasons: vec![
                StoredCookieExclusionReason::StoreUnavailable,
                StoredCookieExclusionReason::CookiesDisabled,
            ],
        },
        facade_exclusion_reasons: vec![
            StoredCookieExclusionReason::StoreUnavailable,
            StoredCookieExclusionReason::CookiesDisabled,
        ],
        ..StoredCookieQueryReport::default()
    });

    assert_eq!(
        json_report["facadeExclusionReasons"],
        json!(["StoreUnavailable", "CookiesDisabled"])
    );
    assert_eq!(
        json_report["facadeStatus"],
        json!({
            "cookieAccessEnabled": false,
            "storeAvailable": false,
            "blockedReasons": ["StoreUnavailable", "CookiesDisabled"],
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_round_trips_partition_key() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-P".into()));

    ctx.process_async(json!({
        "id": 12,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-P",
            "cookies": [{
                "name": "__Host-chip",
                "value": "1",
                "url": "https://example.com/",
                "secure": true,
                "sameSite": "None",
                "partitionKey": { "topLevelSite": "https://example.com", "hasCrossSiteAncestor": false }
            }]
        }
    }))
        .await;
    ctx.expect_result(
        12,
        json!({
            "success": true,
            "cookieReports": [{
                "status": {
                    "kind": "Accepted",
                    "storeAction": "Inserted"
                },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-P" }
    }))
    .await;
    ctx.expect_result(
        13,
        json!({
            "cookies": [{
                "name": "__Host-chip",
                "value": "1",
                "partitionKey": {
                    "topLevelSite": "https://example.com",
                    "hasCrossSiteAncestor": false
                }
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_delete_cookies_matches_exact_partition_key() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PD".into()));

    ctx.process_async(json!({
        "id": 14,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-PD",
            "cookies": [
                {
                    "name": "chip",
                    "value": "one",
                    "url": "https://widget.example/",
                    "secure": true,
                    "sameSite": "None",
                    "partitionKey": {
                        "topLevelSite": "https://first.example",
                        "hasCrossSiteAncestor": true
                    }
                },
                {
                    "name": "chip",
                    "value": "two",
                    "url": "https://widget.example/",
                    "secure": true,
                    "sameSite": "None",
                    "partitionKey": {
                        "topLevelSite": "https://second.example",
                        "hasCrossSiteAncestor": true
                    }
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(14, json!({ "success": true }), None);

    ctx.process_async(json!({
        "id": 15,
        "method": "Storage.deleteCookies",
        "params": {
            "browserContextId": "BID-PD",
            "name": "chip",
            "domain": "widget.example",
            "path": "/",
            "partitionKey": {
                "topLevelSite": "https://first.example",
                "hasCrossSiteAncestor": true
            }
        }
    }))
    .await;
    ctx.expect_result(15, json!({}), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-PD" }
    }))
    .await;
    ctx.expect_result(
        16,
        json!({
            "cookies": [{
                "name": "chip",
                "value": "two",
                "partitionKey": {
                    "topLevelSite": "https://second.example",
                    "hasCrossSiteAncestor": true
                }
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_delete_cookies_rejects_malformed_partition_key() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PD-BAD".into()));

    ctx.process_async(json!({
        "id": 17,
        "method": "Storage.deleteCookies",
        "params": {
            "browserContextId": "BID-PD-BAD",
            "name": "chip",
            "partitionKey": { "topLevelSite": "not a site" }
        }
    }))
    .await;
    ctx.expect_error(17, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_keeps_cookie_store_available_after_lock_holder_panic() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-store-panic".into()));
    let cookie_store = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .cookie_store_for_test()
        .clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = cookie_store.lock();
        panic!("panic while holding cookie store lock");
    }));

    ctx.process_async(json!({
        "id": 120,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-store-panic",
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "https://example.com/"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        120,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 121,
        "method": "Storage.getCookies",
        "params": {
            "browserContextId": "BID-store-panic"
        }
    }))
    .await;
    ctx.expect_result(
        121,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "1",
                "domain": "example.com",
                "path": "/",
                "size": 4,
                "secure": true
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_accepts_leading_dot_domain() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-DOT".into()));

    ctx.process_async(json!({
        "id": 121,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-DOT",
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "https://example.com/app",
                "domain": ".example.com"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        121,
        json!({
            "success": true,
            "cookieReports": [{
                "status": {
                    "kind": "Accepted",
                    "storeAction": "Inserted"
                },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_reports_invalid_path_and_url_as_facade_rejections() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PATH".into()));

    ctx.process_async(json!({
        "id": 122,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-PATH",
            "cookies": [
                {
                    "name": "bad-path",
                    "value": "1",
                    "url": "https://example.com/app",
                    "path": "app"
                },
                {
                    "name": "bad-url",
                    "value": "1",
                    "url": "https://example.com:bad/app"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(
        122,
        json!({
            "success": false,
            "cookieReports": [
                {
                    "status": {
                        "kind": "Rejected",
                        "reason": "PathMustStartWithSlash"
                    },
                    "rejectionReasons": ["PathMustStartWithSlash"],
                    "effectiveSameSite": "NoRestriction",
                    "warningReasons": []
                },
                {
                    "status": {
                        "kind": "Rejected",
                        "reason": "InvalidUrl"
                    },
                    "rejectionReasons": ["InvalidUrl", "UnspecifiedDomain"],
                    "effectiveSameSite": "NoRestriction",
                    "warningReasons": []
                }
            ]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_reports_structured_name_value_facade_rejections() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-NV".into()));

    ctx.process_async(json!({
        "id": 123,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-NV",
            "cookies": [
                {
                    "name": "",
                    "value": ""
                },
                {
                    "name": "",
                    "value": "a=b",
                    "url": "https://example.com/app"
                },
                {
                    "name": "a=b",
                    "value": "1",
                    "url": "https://example.com/app"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(
        123,
        json!({
            "success": false,
            "cookieReports": [
                {
                    "status": {
                        "kind": "Rejected",
                        "reason": "EmptyNameAndValue"
                    },
                    "rejectionReasons": ["EmptyNameAndValue", "MissingCookieUrl"],
                    "effectiveSameSite": "NoRestriction",
                    "warningReasons": []
                },
                {
                    "status": {
                        "kind": "Rejected",
                        "reason": "EmptyNameValueContainsEquals"
                    },
                    "rejectionReasons": ["EmptyNameValueContainsEquals"],
                    "effectiveSameSite": "NoRestriction",
                    "warningReasons": []
                },
                {
                    "status": {
                        "kind": "Rejected",
                        "reason": "NameContainsEquals"
                    },
                    "rejectionReasons": ["NameContainsEquals"],
                    "effectiveSameSite": "NoRestriction",
                    "warningReasons": []
                }
            ]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_uses_browser_context_default_cookie_url_when_missing() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-DEF".into());
    bc.set_target_url("https://example.com/app".into());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 124,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-DEF",
            "cookies": [{
                "name": "sid",
                "value": "1"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        124,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 125,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-DEF" }
    }))
    .await;
    ctx.expect_result(
        125,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "1",
                "domain": "example.com",
                "path": "/",
                "size": 4,
                "secure": true
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_cookie_methods_accept_inactive_browser_context_ids() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-ACTIVE".into()));
    let mut inactive = BrowserContext::new("BID-INACTIVE".into());
    inactive.set_target_url("https://inactive.example/app".into());
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 1_251,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-INACTIVE",
            "cookies": [{
                "name": "sid",
                "value": "inactive",
                "url": "https://inactive.example/app"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        1_251,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 1_252,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-INACTIVE" }
    }))
    .await;
    ctx.expect_result(
        1_252,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "inactive",
                "domain": "inactive.example",
                "path": "/",
                "size": 11,
                "secure": true
            }]
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 1_253,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-ACTIVE" }
    }))
    .await;
    ctx.expect_result(1_253, json!({ "cookies": [] }), None);

    ctx.process_async(json!({
        "id": 1_254,
        "method": "Storage.deleteCookies",
        "params": {
            "browserContextId": "BID-INACTIVE",
            "name": "sid",
            "domain": "inactive.example"
        }
    }))
    .await;
    ctx.expect_result(1_254, json!({}), None);

    ctx.process_async(json!({
        "id": 1_255,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-INACTIVE" }
    }))
    .await;
    ctx.expect_result(1_255, json!({ "cookies": [] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_cookies_visible_to_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-clear".into()));

    ctx.process_async(json!({
        "id": 12_510,
        "method": "Storage.setCookies",
        "params": {
            "cookies": [
                { "name": "host", "value": "1", "url": "https://app.example.com/app" },
                { "name": "shared", "value": "1", "domain": "example.com", "path": "/" },
                { "name": "sibling", "value": "1", "url": "https://cdn.example.com/app" },
                { "name": "other", "value": "1", "url": "https://foo.co.uk/app" }
            ]
        }
    }))
    .await;
    ctx.expect_result(12_510, json!({}), None);

    ctx.process_async(json!({
        "id": 12_511,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://app.example.com",
            "storageTypes": "cookies,local_storage"
        }
    }))
    .await;
    ctx.expect_result(12_511, json!({}), None);

    ctx.process_async(json!({
        "id": 12_512,
        "method": "Storage.getCookies"
    }))
    .await;
    let response = ctx.take_response_by_id(12_512);
    let names = response["result"]["cookies"]
        .as_array()
        .expect("storage cookies")
        .iter()
        .map(|cookie| cookie["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["sibling", "other"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_non_cookie_types_do_not_clear_cookies() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-noncookie".into()));

    ctx.process_async(json!({
        "id": 12_520,
        "method": "Storage.setCookies",
        "params": {
            "cookies": [{ "name": "sid", "value": "1", "url": "https://app.example.com/app" }]
        }
    }))
    .await;
    ctx.expect_result(12_520, json!({}), None);

    ctx.process_async(json!({
        "id": 12_521,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://app.example.com",
            "storageTypes": "local_storage"
        }
    }))
    .await;
    ctx.expect_result(12_521, json!({}), None);

    ctx.process_async(json!({
        "id": 12_522,
        "method": "Storage.getCookies"
    }))
    .await;
    ctx.expect_result(
        12_522,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "1",
                "domain": "app.example.com",
                "path": "/",
                "size": 4,
                "secure": true
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_local_storage_area() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-local-storage".into()));

    let origin = Url::parse("https://app.example.com/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let sibling_origin = Url::parse("https://cdn.example.com/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let first_party_key = first_party_storage_key_for_origin(&origin);
    let sibling_first_party_key = first_party_storage_key_for_origin(&sibling_origin);
    let partitioned_origin_top_a =
        moli_core::network::web_storage_partitioned_area_key(&origin, "https://top-a.example");
    let partitioned_origin_top_b =
        moli_core::network::web_storage_partitioned_area_key(&origin, "https://top-b.example");
    let partitioned_sibling = moli_core::network::web_storage_partitioned_area_key(
        &sibling_origin,
        "https://top-a.example",
    );
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.web_storage_store_for_test().lock();
        assert!(store.set_item(&first_party_key, "local", "1"));
        assert!(store.set_item(&partitioned_origin_top_a, "local", "1a"));
        assert!(store.set_item(&partitioned_origin_top_b, "local", "1b"));
        assert!(store.set_item(&sibling_first_party_key, "local", "2"));
        assert!(store.set_item(&partitioned_sibling, "local", "2a"));
    }

    ctx.process_async(json!({
        "id": 12_525,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://app.example.com",
            "storageTypes": "local_storage"
        }
    }))
    .await;
    ctx.expect_result(12_525, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let mut store = bc.web_storage_store_for_test().lock();
    assert_eq!(store.len(&first_party_key), 0);
    assert_eq!(store.get_item(&first_party_key, "local"), None);
    assert_eq!(store.get_item(&partitioned_origin_top_a, "local"), None);
    assert_eq!(store.get_item(&partitioned_origin_top_b, "local"), None);
    assert_eq!(
        store.get_item(&sibling_first_party_key, "local"),
        Some("2".to_owned())
    );
    assert_eq!(
        store.get_item(&partitioned_sibling, "local"),
        Some("2a".to_owned())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_storage_key_clears_site_data_for_matching_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-storage-key-clear".into()));

    let origin = Url::parse("https://storage-key-clear.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let sibling_origin = Url::parse("https://storage-key-sibling.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);
    let sibling_storage_key = first_party_storage_key_for_origin(&sibling_origin);
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "1"));
        assert!(store.set_item(&sibling_storage_key, "local", "2"));
    }
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.storage_bucket_store_for_test().lock();
        store
            .open_bucket(&storage_key, "bucket-a")
            .expect("origin bucket should open");
        store
            .open_bucket(&sibling_storage_key, "bucket-b")
            .expect("sibling bucket should open");
    }
    let (bucket_a_key, sibling_bucket_key) = {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        (
            bucket_indexed_db_storage_key_for_test(bc, &storage_key, "bucket-a"),
            bucket_indexed_db_storage_key_for_test(bc, &sibling_storage_key, "bucket-b"),
        )
    };
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &bucket_a_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &sibling_bucket_key);
        assert!(
            moli_core::storage::indexed_db_origin_usage_bytes(
                bc.indexed_db_manager_for_test(),
                &bucket_a_key,
            )
            .expect("bucket-a usage should be readable")
                > 0
        );
    }

    ctx.process_async(json!({
        "id": 12_526,
        "method": "Storage.clearDataForStorageKey",
        "params": {
            "storageKey": storage_key,
            "storageTypes": "local_storage,storage_buckets"
        }
    }))
    .await;
    ctx.expect_result(12_526, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    {
        let mut store = bc.web_storage_store_for_test().lock();
        assert_eq!(store.get_item(&storage_key, "local"), None);
        assert_eq!(
            store.get_item(&sibling_storage_key, "local"),
            Some("2".to_owned())
        );
    }
    let store = bc.storage_bucket_store_for_test().lock();
    assert_eq!(store.keys(&storage_key), Vec::<String>::new());
    assert_eq!(store.keys(&sibling_storage_key), vec!["bucket-b"]);
    drop(store);
    assert_eq!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &bucket_a_key
        )
        .expect("bucket-a usage should be readable"),
        0
    );
    assert!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &sibling_bucket_key,
        )
        .expect("sibling bucket usage should be readable")
            > 0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_partitioned_storage_key_clears_exact_partition_only() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new(
        "BID-storage-key-partitioned-clear".into(),
    ));

    let origin = "https://partitioned-storage-key.example";
    let first_party_key = first_party_storage_key_for_origin(origin);
    let partitioned_key =
        moli_storage_key::partitioned_storage_key(origin, "https://top-level-storage-key.example");
    let other_partitioned_key = moli_storage_key::partitioned_storage_key(
        origin,
        "https://other-top-level-storage-key.example",
    );
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.web_storage_store_for_test().lock();
        assert!(store.set_item(&first_party_key, "local", "must-stay"));
        assert!(store.set_item(&partitioned_key, "local", "must-clear"));
        assert!(store.set_item(&other_partitioned_key, "local", "other-must-stay"));
    }
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.storage_bucket_store_for_test().lock();
        store
            .open_bucket(&partitioned_key, "bucket")
            .expect("partitioned bucket should open");
        store
            .open_bucket(&other_partitioned_key, "other-bucket")
            .expect("other partitioned bucket should open");
    }
    let (bucket_key, other_bucket_key) = {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        (
            bucket_indexed_db_storage_key_for_test(bc, &partitioned_key, "bucket"),
            bucket_indexed_db_storage_key_for_test(bc, &other_partitioned_key, "other-bucket"),
        )
    };
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &partitioned_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &other_partitioned_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &bucket_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &other_bucket_key);
    }

    ctx.process_async(json!({
        "id": 12_527,
        "method": "Storage.clearDataForStorageKey",
        "params": {
            "storageKey": partitioned_key,
            "storageTypes": "local_storage,indexeddb,storage_buckets"
        }
    }))
    .await;
    ctx.expect_result(12_527, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    {
        let mut store = bc.web_storage_store_for_test().lock();
        assert_eq!(
            store.get_item(&first_party_key, "local"),
            Some("must-stay".to_owned())
        );
        assert_eq!(store.get_item(&partitioned_key, "local"), None);
        assert_eq!(
            store.get_item(&other_partitioned_key, "local"),
            Some("other-must-stay".to_owned())
        );
    }
    {
        let store = bc.storage_bucket_store_for_test().lock();
        assert_eq!(store.keys(&partitioned_key), Vec::<String>::new());
        assert_eq!(store.keys(&other_partitioned_key), vec!["other-bucket"]);
    }
    assert_eq!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &partitioned_key
        )
        .expect("partitioned IndexedDB usage should be readable"),
        0
    );
    assert!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &other_partitioned_key
        )
        .expect("other partitioned IndexedDB usage should be readable")
            > 0
    );
    assert_eq!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &bucket_key
        )
        .expect("partitioned bucket IndexedDB usage should be readable"),
        0
    );
    assert!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &other_bucket_key
        )
        .expect("other partitioned bucket IndexedDB usage should be readable")
            > 0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_usage_and_quota_reports_local_storage_usage_for_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-usage-local".into()));

    let origin = Url::parse("https://usage.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let first_party_key = first_party_storage_key_for_origin(&origin);
    let partitioned_origin =
        moli_core::network::web_storage_partitioned_area_key(&origin, "https://top.example");
    let partitioned_sibling = moli_core::network::web_storage_partitioned_area_key(
        "https://usage-sibling.example",
        "https://top.example",
    );
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.web_storage_store_for_test().lock();
        assert!(store.set_item(&first_party_key, "local", "abc"));
        assert!(store.set_item(&partitioned_origin, "partitioned", "de"));
        assert!(store.set_item(&partitioned_sibling, "sibling", "ignored"));
        let mut session_store = bc.session_storage_store_for_test().lock();
        assert!(session_store.set_item(&first_party_key, "session", "session-only"));
        assert!(session_store.set_item(&partitioned_origin, "session", "also-ignored"));
    }

    ctx.process_async(json!({
        "id": 12_528,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://usage.example"
        }
    }))
    .await;

    let result = take_result_by_id(&mut ctx, 12_528);
    assert_eq!(json_number_as_u64(&result["usage"]), 5);
    assert_eq!(
        json_number_as_u64(&result["quota"]),
        moli_core::storage::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES
    );
    assert_eq!(result["overrideActive"], json!(false));
    assert_eq!(usage_breakdown_value(&result, "local_storage"), 5);
    assert_eq!(usage_breakdown_value(&result, "indexeddb"), 0);
    assert_eq!(usage_breakdown_value(&result, "storage_buckets"), 0);
    assert!(
        result["usageBreakdown"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["storageType"].as_str() != Some("session_storage")),
        "CDP StorageType has no session_storage token"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_usage_and_quota_reports_indexed_db_usage_for_origin() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-usage-indexeddb".into());
    browser_context.set_active_target_id("TID-usage-indexeddb");
    ctx.conn.browser_context = Some(browser_context);
    let url = Url::parse("https://idb-usage.example/app").unwrap();
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url.clone(),
        "<!doctype html><html><body>idb usage</body></html>".into(),
        None,
    )
    .await;

    install_storage_test_completion_binding(&mut ctx).await;

    {
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .active_target
            .runtime_slot
            .loaded_page_mut()
            .unwrap();
        let scheduled = page
            .evaluate_runtime_expression_async(
                r#"
(() => {
  const open = indexedDB.open("usage", 1);
  open.onerror = () => {
    globalThis.__moliStorageTestComplete(
      `open-error:${open.error && open.error.name}`
    );
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const put = tx.objectStore("kv").put("value", "key");
    put.onerror = () => {
      globalThis.__moliStorageTestComplete(
        `put-error:${put.error && put.error.name}`
      );
    };
    tx.oncomplete = () => {
      db.close();
      globalThis.__moliStorageTestComplete("stored");
    };
  };
  return "scheduled";
})()
"#,
            )
            .await
            .expect("indexeddb setup should evaluate");
        assert_eq!(scheduled["value"], json!("scheduled"));
    }
    assert_eq!(wait_for_storage_test_completion(&mut ctx).await, "stored");

    ctx.process_async(json!({
        "id": 12_529,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://idb-usage.example"
        }
    }))
    .await;

    let result = take_result_by_id(&mut ctx, 12_529);
    let indexed_db_usage = usage_breakdown_value(&result, "indexeddb");
    assert!(
        indexed_db_usage > 0,
        "IndexedDB metadata and record bytes should be reported"
    );
    assert_eq!(usage_breakdown_value(&result, "local_storage"), 0);
    assert_eq!(json_number_as_u64(&result["usage"]), indexed_db_usage);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_usage_and_quota_reports_storage_bucket_usage_for_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-usage-storage-buckets".into()));
    let indexed_db_manager = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .indexed_db_manager_for_test()
        .clone();

    let origin = Url::parse("https://bucket-usage.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let sibling_origin = Url::parse("https://bucket-usage-sibling.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);
    let sibling_storage_key = first_party_storage_key_for_origin(&sibling_origin);
    let cache_root = HttpCacheTestRoot::new("storage-bucket-cache-usage");
    let storage_buckets_path = cache_root.path.join("storage-buckets.json");
    let cache_storage_root = cache_root.path.join("cache-storage");
    let cache_usage = 83u64;
    let sibling_cache_usage = 997u64;

    let storage_bucket_store = {
        let store = moli_core::storage::new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager(
            &storage_buckets_path,
            &cache_storage_root,
            &indexed_db_manager,
        )
        .expect("profile-backed StorageBucket store should open");
        let (bucket_cache_id, sibling_cache_id) = {
            let mut store = store.lock();
            store
                .open_bucket(&storage_key, "bucket-a")
                .expect("bucket-a should open");
            store
                .open_bucket(&storage_key, "bucket-b")
                .expect("bucket-b should open");
            store
                .open_bucket(&storage_key, "bucket-cache")
                .expect("bucket-cache should open");
            store
                .open_bucket(&sibling_storage_key, "bucket-c")
                .expect("sibling bucket should open");
            store
                .open_bucket(&sibling_storage_key, "bucket-cache")
                .expect("sibling cache bucket should open");
            (
                store
                    .bucket_id(&storage_key, "bucket-cache")
                    .expect("cache bucket should have identity"),
                store
                    .bucket_id(&sibling_storage_key, "bucket-cache")
                    .expect("sibling cache bucket should have identity"),
            )
        };
        write_storage_bucket_cache_file(
            &cache_storage_root,
            &storage_key,
            bucket_cache_id,
            "receipts",
            "/receipt.txt",
            b"bucket cache",
            cache_usage,
        );
        write_storage_bucket_cache_file(
            &cache_storage_root,
            &sibling_storage_key,
            sibling_cache_id,
            "receipts",
            "/receipt.txt",
            b"sibling cache",
            sibling_cache_usage,
        );
        moli_core::storage::new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager(
            &storage_buckets_path,
            &cache_storage_root,
            &indexed_db_manager,
        )
        .expect("profile-backed StorageBucket store should reopen with cache entries")
    };
    let (bucket_a_identity, bucket_a_key, bucket_b_key, sibling_bucket_key) = {
        let store = storage_bucket_store.lock();
        let bucket_a_identity = store
            .bucket_identity(&storage_key, "bucket-a")
            .expect("bucket-a should have identity");
        (
            bucket_a_identity.clone(),
            bucket_a_identity.indexed_db_storage_key(),
            store
                .bucket_identity(&storage_key, "bucket-b")
                .expect("bucket-b should have identity")
                .indexed_db_storage_key(),
            store
                .bucket_identity(&sibling_storage_key, "bucket-c")
                .expect("sibling bucket should have identity")
                .indexed_db_storage_key(),
        )
    };
    let opfs_usage = {
        let storage_service = storage_bucket_store.lock().storage_service();
        let locator = bucket_a_identity.locator();
        let bucket_key = moli_storage_service::StorageService::opfs_bucket_key(&locator)
            .expect("bucket OPFS key should derive");
        let root = storage_service
            .ensure_opfs_root(&locator)
            .expect("bucket OPFS root should open");
        let file = storage_service
            .with_opfs(|opfs| opfs.get_file(&bucket_key, &root, "usage.txt", true))
            .expect("bucket OPFS file should open");
        storage_service
            .with_opfs(|opfs| opfs.write_file(&bucket_key, &file, b"opfs usage", None))
            .expect("bucket OPFS file should write");
        storage_service
            .opfs_usage(&locator)
            .expect("bucket OPFS usage should load")
    };

    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        bc.replace_storage_bucket_store_for_test(storage_bucket_store);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &bucket_a_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &bucket_b_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &sibling_bucket_key);
    }

    ctx.process_async(json!({
        "id": 12_533,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://bucket-usage.example"
        }
    }))
    .await;

    let result = take_result_by_id(&mut ctx, 12_533);
    let bucket_usage = usage_breakdown_value(&result, "storage_buckets");
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let bucket_a_usage = moli_core::storage::indexed_db_origin_usage_bytes(
        bc.indexed_db_manager_for_test(),
        &bucket_a_key,
    )
    .expect("bucket-a usage should be readable");
    let bucket_b_usage = moli_core::storage::indexed_db_origin_usage_bytes(
        bc.indexed_db_manager_for_test(),
        &bucket_b_key,
    )
    .expect("bucket-b usage should be readable");
    assert_eq!(
        bucket_usage,
        bucket_a_usage
            .saturating_add(bucket_b_usage)
            .saturating_add(cache_usage)
            .saturating_add(opfs_usage)
    );
    assert_eq!(usage_breakdown_value(&result, "local_storage"), 0);
    assert_eq!(usage_breakdown_value(&result, "indexeddb"), 0);
    assert_eq!(json_number_as_u64(&result["usage"]), bucket_usage);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_usage_and_quota_targets_command_session_browser_context() {
    let mut ctx = TestContext::new();
    let origin = Url::parse("https://session-usage.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);

    let mut active = BrowserContext::new("BID-usage-active".into());
    active.attach_active_session("SID-usage-active");
    {
        let mut store = active.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "aa"));
    }

    let mut inactive = BrowserContext::new("BID-usage-inactive".into());
    inactive.attach_active_session("SID-usage-inactive");
    {
        let mut store = inactive.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "bbbb"));
    }

    ctx.conn.browser_context = Some(active);
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 12_529,
        "sessionId": "SID-usage-inactive",
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://session-usage.example"
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 12_529);
    assert_eq!(response["sessionId"], json!("SID-usage-inactive"));
    let result = response
        .get("result")
        .expect("response should include result");
    assert_eq!(json_number_as_u64(&result["usage"]), 4);
    assert_eq!(usage_breakdown_value(result, "local_storage"), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_override_quota_for_origin_affects_get_usage_and_quota() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-quota-override".into()));

    ctx.process_async(json!({
        "id": 12_531,
        "method": "Storage.overrideQuotaForOrigin",
        "params": {
            "origin": "https://quota.example/app",
            "quotaSize": 4096
        }
    }))
    .await;
    ctx.expect_result(12_531, json!({}), None);

    ctx.process_async(json!({
        "id": 12_532,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://quota.example"
        }
    }))
    .await;

    let result = take_result_by_id(&mut ctx, 12_532);
    assert_eq!(json_number_as_u64(&result["quota"]), 4096);
    assert_eq!(result["overrideActive"], json!(true));

    ctx.process_async(json!({
        "id": 12_533,
        "method": "Storage.overrideQuotaForOrigin",
        "params": {
            "origin": "https://quota.example/app"
        }
    }))
    .await;
    ctx.expect_result(12_533, json!({}), None);

    ctx.process_async(json!({
        "id": 12_534,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://quota.example"
        }
    }))
    .await;

    let result = take_result_by_id(&mut ctx, 12_534);
    assert_eq!(
        json_number_as_u64(&result["quota"]),
        moli_core::storage::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES
    );
    assert_eq!(result["overrideActive"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_override_quota_for_origin_targets_command_session_browser_context() {
    let mut ctx = TestContext::new();

    let mut active = BrowserContext::new("BID-quota-active".into());
    active.attach_active_session("SID-quota-active");
    let mut inactive = BrowserContext::new("BID-quota-inactive".into());
    inactive.attach_active_session("SID-quota-inactive");

    ctx.conn.browser_context = Some(active);
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 12_535,
        "sessionId": "SID-quota-inactive",
        "method": "Storage.overrideQuotaForOrigin",
        "params": {
            "origin": "https://session-quota.example/app",
            "quotaSize": 8192
        }
    }))
    .await;
    ctx.expect_result(12_535, json!({}), Some("SID-quota-inactive"));

    ctx.process_async(json!({
        "id": 12_536,
        "sessionId": "SID-quota-active",
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://session-quota.example"
        }
    }))
    .await;
    let active_response = take_response_by_id(&mut ctx, 12_536);
    assert_eq!(active_response["sessionId"], json!("SID-quota-active"));
    let active_result = active_response
        .get("result")
        .expect("active response should include result");
    assert_eq!(
        json_number_as_u64(&active_result["quota"]),
        moli_core::storage::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES
    );
    assert_eq!(active_result["overrideActive"], json!(false));

    ctx.process_async(json!({
        "id": 12_537,
        "sessionId": "SID-quota-inactive",
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "https://session-quota.example"
        }
    }))
    .await;
    let inactive_response = take_response_by_id(&mut ctx, 12_537);
    assert_eq!(inactive_response["sessionId"], json!("SID-quota-inactive"));
    let inactive_result = inactive_response
        .get("result")
        .expect("inactive response should include result");
    assert_eq!(json_number_as_u64(&inactive_result["quota"]), 8192);
    assert_eq!(inactive_result["overrideActive"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_usage_and_quota_rejects_invalid_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-usage-invalid".into()));

    ctx.process_async(json!({
        "id": 12_530,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "not a url"
        }
    }))
    .await;

    ctx.expect_error(12_530, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_get_usage_and_quota_rejects_opaque_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-usage-opaque".into()));

    ctx.process_async(json!({
        "id": 12_538,
        "method": "Storage.getUsageAndQuota",
        "params": {
            "origin": "data:text/html,opaque"
        }
    }))
    .await;

    ctx.expect_error(12_538, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_override_quota_for_origin_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-quota-invalid".into()));

    ctx.process_async(json!({
        "id": 12_539,
        "method": "Storage.overrideQuotaForOrigin",
        "params": {
            "origin": "not a url",
            "quotaSize": 4096
        }
    }))
    .await;
    ctx.expect_error(12_539, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 12_540,
        "method": "Storage.overrideQuotaForOrigin",
        "params": {
            "origin": "data:text/html,opaque",
            "quotaSize": 4096
        }
    }))
    .await;
    ctx.expect_error(12_540, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 12_541,
        "method": "Storage.overrideQuotaForOrigin",
        "params": {
            "origin": "https://quota-invalid.example",
            "quotaSize": -1
        }
    }))
    .await;
    ctx.expect_error(12_541, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_targets_command_session_browser_context() {
    let mut ctx = TestContext::new();
    let origin = Url::parse("https://session-clear.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);

    let mut active = BrowserContext::new("BID-session-clear-active".into());
    active.attach_active_session("SID-session-clear-active");
    {
        let mut store = active.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "active"));
    }

    let mut inactive = BrowserContext::new("BID-session-clear-inactive".into());
    inactive.attach_active_session("SID-session-clear-inactive");
    {
        let mut store = inactive.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "inactive"));
    }

    ctx.conn.browser_context = Some(active);
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 12_527,
        "sessionId": "SID-session-clear-inactive",
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://session-clear.example",
            "storageTypes": "local_storage"
        }
    }))
    .await;
    ctx.expect_result(12_527, json!({}), Some("SID-session-clear-inactive"));

    let mut active_store = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .web_storage_store_for_test()
        .lock();
    assert_eq!(
        active_store.get_item(&storage_key, "local"),
        Some("active".to_owned())
    );
    drop(active_store);

    let mut inactive_store = ctx.conn.inactive_browser_contexts[0]
        .web_storage_store_for_test()
        .lock();
    assert_eq!(inactive_store.get_item(&storage_key, "local"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_cookies_and_local_storage_together() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-all".into()));

    let origin = Url::parse("https://app.example.com/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "1"));
    }

    ctx.process_async(json!({
        "id": 12_526,
        "method": "Storage.setCookies",
        "params": {
            "cookies": [
                { "name": "sid", "value": "1", "url": "https://app.example.com/app" },
                { "name": "other", "value": "1", "url": "https://other.example.com/app" }
            ]
        }
    }))
    .await;
    ctx.expect_result(12_526, json!({}), None);

    ctx.process_async(json!({
        "id": 12_527,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://app.example.com",
            "storageTypes": "cookies,local_storage"
        }
    }))
    .await;
    ctx.expect_result(12_527, json!({}), None);

    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.web_storage_store_for_test().lock();
        assert_eq!(store.len(&storage_key), 0);
        assert_eq!(store.get_item(&storage_key, "local"), None);
    }

    ctx.process_async(json!({
        "id": 12_528,
        "method": "Storage.getCookies"
    }))
    .await;
    ctx.expect_result(
        12_528,
        json!({
            "cookies": [{
                "name": "other",
                "value": "1",
                "domain": "other.example.com",
                "path": "/",
                "size": 6,
                "secure": true
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_indexed_db_backend() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-origin-indexeddb".into());
    browser_context.set_active_target_id("TID-origin-indexeddb");
    ctx.conn.browser_context = Some(browser_context);
    let url = Url::parse("https://idb-clear.example/app").unwrap();
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url.clone(),
        "<!doctype html><html><body>idb</body></html>".into(),
        None,
    )
    .await;

    install_storage_test_completion_binding(&mut ctx).await;

    {
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .active_target
            .runtime_slot
            .loaded_page_mut()
            .unwrap();
        let scheduled = page
            .evaluate_runtime_expression_async(
                r#"
(() => {
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__moliStorageTestComplete(
      `open-error:${open.error && open.error.name}`
    );
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const put = tx.objectStore("kv").put("value", "key");
    put.onerror = () => {
      globalThis.__moliStorageTestComplete(
        `put-error:${put.error && put.error.name}`
      );
    };
    tx.oncomplete = () => {
      db.close();
      globalThis.__moliStorageTestComplete("stored");
    };
  };
  return "scheduled";
})()
"#,
            )
            .await
            .expect("indexeddb setup should evaluate");
        assert_eq!(scheduled["value"], json!("scheduled"));
    }
    assert_eq!(wait_for_storage_test_completion(&mut ctx).await, "stored");

    ctx.process_async(json!({
        "id": 12_529,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://idb-clear.example",
            "storageTypes": "all"
        }
    }))
    .await;
    ctx.expect_result(12_529, json!({}), None);

    {
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .active_target
            .runtime_slot
            .loaded_page_mut()
            .unwrap();
        let scheduled = page
            .evaluate_runtime_expression_async(
                r#"
(() => {
  let oldVersion = "no-upgrade";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
    globalThis.__moliStorageTestComplete(
      `open-error:${open.error && open.error.name}`
    );
  };
  open.onupgradeneeded = (event) => {
    oldVersion = String(event.oldVersion);
    open.result.createObjectStore("fresh");
  };
  open.onsuccess = () => {
    const db = open.result;
    const result = [
      oldVersion,
      String(db.objectStoreNames.contains("kv")),
      String(db.objectStoreNames.contains("fresh"))
    ].join("|");
    db.close();
    globalThis.__moliStorageTestComplete(result);
  };
  return "scheduled";
})()
"#,
            )
            .await
            .expect("indexeddb reopen should evaluate");
        assert_eq!(scheduled["value"], json!("scheduled"));
    }
    assert_eq!(
        wait_for_storage_test_completion(&mut ctx).await,
        "0|false|true"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_storage_bucket_names_for_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-storage-buckets".into()));

    let origin = Url::parse("https://bucket-clear.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let sibling_origin = Url::parse("https://other-bucket-clear.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);
    let sibling_storage_key = first_party_storage_key_for_origin(&sibling_origin);
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let mut store = bc.storage_bucket_store_for_test().lock();
        store
            .open_bucket(&storage_key, "bucket-a")
            .expect("bucket-a should open");
        store
            .open_bucket(&storage_key, "bucket-b")
            .expect("bucket-b should open");
        store
            .open_bucket(&sibling_storage_key, "bucket-c")
            .expect("bucket-c should open");
    }

    ctx.process_async(json!({
        "id": 12_530,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://bucket-clear.example",
            "storageTypes": "local_storage"
        }
    }))
    .await;
    ctx.expect_result(12_530, json!({}), None);
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let store = bc.storage_bucket_store_for_test().lock();
        assert_eq!(store.keys(&storage_key), vec!["bucket-a", "bucket-b"]);
        assert_eq!(store.keys(&sibling_storage_key), vec!["bucket-c"]);
    }

    ctx.process_async(json!({
        "id": 12_531,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://bucket-clear.example",
            "storageTypes": "storage_buckets"
        }
    }))
    .await;
    ctx.expect_result(12_531, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let store = bc.storage_bucket_store_for_test().lock();
    assert_eq!(store.keys(&storage_key), Vec::<String>::new());
    assert_eq!(store.keys(&sibling_storage_key), vec!["bucket-c"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_storage_bucket_indexeddb_for_origin() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-storage-bucket-idb".into()));

    let origin = Url::parse("https://bucket-idb-clear.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let sibling_origin = Url::parse("https://other-bucket-idb-clear.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key = first_party_storage_key_for_origin(&origin);
    let sibling_storage_key = first_party_storage_key_for_origin(&sibling_origin);
    {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        {
            let mut store = bc.storage_bucket_store_for_test().lock();
            store
                .open_bucket(&storage_key, "bucket-a")
                .expect("bucket-a should open");
            store
                .open_bucket(&storage_key, "bucket-b")
                .expect("bucket-b should open");
            store
                .open_bucket(&sibling_storage_key, "bucket-c")
                .expect("bucket-c should open");
        }
        let bucket_a_key = bucket_indexed_db_storage_key_for_test(bc, &storage_key, "bucket-a");
        let bucket_b_key = bucket_indexed_db_storage_key_for_test(bc, &storage_key, "bucket-b");
        let sibling_bucket_key =
            bucket_indexed_db_storage_key_for_test(bc, &sibling_storage_key, "bucket-c");
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &bucket_a_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &bucket_b_key);
        seed_indexed_db_usage(bc.indexed_db_manager_for_test(), &sibling_bucket_key);
        assert!(
            moli_core::storage::indexed_db_origin_usage_bytes(
                bc.indexed_db_manager_for_test(),
                &bucket_a_key,
            )
            .expect("bucket-a usage should be readable")
                > 0
        );
        assert!(
            moli_core::storage::indexed_db_origin_usage_bytes(
                bc.indexed_db_manager_for_test(),
                &bucket_b_key,
            )
            .expect("bucket-b usage should be readable")
                > 0
        );
    }

    let (
        bucket_a_identity,
        bucket_b_identity,
        sibling_bucket_identity,
        bucket_a_key,
        bucket_b_key,
        sibling_bucket_key,
    ) = {
        let bc = ctx.conn.browser_context.as_ref().unwrap();
        let store = bc.storage_bucket_store_for_test();
        let store = store.lock();
        let bucket_a = store
            .bucket_identity(&storage_key, "bucket-a")
            .expect("bucket-a should have identity");
        let bucket_b = store
            .bucket_identity(&storage_key, "bucket-b")
            .expect("bucket-b should have identity");
        let sibling = store
            .bucket_identity(&sibling_storage_key, "bucket-c")
            .expect("bucket-c should have identity");
        (
            bucket_a.clone(),
            bucket_b.clone(),
            sibling.clone(),
            bucket_a.indexed_db_storage_key(),
            bucket_b.indexed_db_storage_key(),
            sibling.indexed_db_storage_key(),
        )
    };
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(seed_storage_bucket_opfs_usage_for_test(bc, &bucket_a_identity, "a.txt") > 0);
    assert!(seed_storage_bucket_opfs_usage_for_test(bc, &bucket_b_identity, "b.txt") > 0);
    assert!(seed_storage_bucket_opfs_usage_for_test(bc, &sibling_bucket_identity, "c.txt") > 0);

    ctx.process_async(json!({
        "id": 12_532,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "https://bucket-idb-clear.example",
            "storageTypes": "storage_buckets"
        }
    }))
    .await;
    ctx.expect_result(12_532, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let store = bc.storage_bucket_store_for_test().lock();
    assert_eq!(store.keys(&storage_key), Vec::<String>::new());
    assert_eq!(store.keys(&sibling_storage_key), vec!["bucket-c"]);
    drop(store);
    assert_eq!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &bucket_a_key
        )
        .expect("bucket-a usage should be readable"),
        0
    );
    assert_eq!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &bucket_b_key
        )
        .expect("bucket-b usage should be readable"),
        0
    );
    assert!(
        moli_core::storage::indexed_db_origin_usage_bytes(
            bc.indexed_db_manager_for_test(),
            &sibling_bucket_key,
        )
        .expect("sibling bucket usage should be readable")
            > 0
    );
    let storage_service = bc.storage_bucket_store_for_test().lock().storage_service();
    assert_eq!(
        storage_service
            .opfs_usage(&bucket_a_identity.locator())
            .expect("bucket-a OPFS usage should load"),
        0
    );
    assert_eq!(
        storage_service
            .opfs_usage(&bucket_b_identity.locator())
            .expect("bucket-b OPFS usage should load"),
        0
    );
    assert!(
        storage_service
            .opfs_usage(&sibling_bucket_identity.locator())
            .expect("sibling OPFS usage should load")
            > 0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_clears_http_cache_entries_for_origin() {
    let cache_root = HttpCacheTestRoot::new("clear-data-for-origin");
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_root.path.display().to_string()));

    let (app_url, app_hits, app_server) = spawn_cacheable_text_server("app-cache").await;
    let (other_url, other_hits, other_server) = spawn_cacheable_text_server("other-cache").await;
    let client = moli_fetch::FetchClient::new(
        &fetch_config,
        moli_cookie_jar::new_shared_browser_cookie_store(),
    );

    assert_eq!(
        client
            .fetch(moli_fetch::Request::get(&app_url).expect("app request"))
            .await
            .expect("app cache seed should fetch")
            .body_text(),
        "app-cache"
    );
    assert_eq!(
        client
            .fetch(moli_fetch::Request::get(&other_url).expect("other request"))
            .await
            .expect("other cache seed should fetch")
            .body_text(),
        "other-cache"
    );
    assert_eq!(app_hits.load(Ordering::SeqCst), 1);
    assert_eq!(other_hits.load(Ordering::SeqCst), 1);

    let mut ctx = TestContext::from_conn(crate::conn::CdpConnection::new_with_fetch_config(
        fetch_config.clone(),
    ));
    let browser_context = ctx.conn.new_browser_context("BID-cache-clear".into());
    ctx.conn.browser_context = Some(browser_context);

    let app_origin = Url::parse(&app_url)
        .expect("app url should parse")
        .origin()
        .ascii_serialization();
    ctx.process_async(json!({
        "id": 12_531,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": app_origin,
            "storageTypes": "cache_storage"
        }
    }))
    .await;
    ctx.expect_result(12_531, json!({}), None);

    app_server.abort();
    other_server.abort();
    let _ = app_server.await;
    let _ = other_server.await;

    let client = moli_fetch::FetchClient::new(
        &fetch_config,
        moli_cookie_jar::new_shared_browser_cookie_store(),
    );
    assert_eq!(
        client
            .fetch(moli_fetch::Request::get(&other_url).expect("other cached request"))
            .await
            .expect("other origin cache entry should remain")
            .body_text(),
        "other-cache"
    );
    let app_error = client
        .fetch(moli_fetch::Request::get(&app_url).expect("app cached request"))
        .await
        .expect_err("cleared app origin cache entry should miss after server abort");
    assert!(
        format!("{app_error:#}").contains("curl request failed"),
        "unexpected app cache miss error: {app_error:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_uses_browser_context_http_cache_owner() {
    let cache_root = HttpCacheTestRoot::new("clear-data-for-origin-context-owner");
    let mut seed_config = moli_fetch::FetchConfig::default();
    seed_config.set_http_cache_dir(Some(cache_root.path.display().to_string()));

    let (app_url, app_hits, app_server) = spawn_cacheable_text_server("context-owner-cache").await;
    let client = moli_fetch::FetchClient::new(
        &seed_config,
        moli_cookie_jar::new_shared_browser_cookie_store(),
    );
    assert_eq!(
        client
            .fetch(moli_fetch::Request::get(&app_url).expect("app request"))
            .await
            .expect("app cache seed should fetch")
            .body_text(),
        "context-owner-cache"
    );
    assert_eq!(app_hits.load(Ordering::SeqCst), 1);

    let mut clear_config = moli_fetch::FetchConfig::default();
    clear_config.set_http_cache_dir(None);
    let mut ctx = TestContext::from_conn(crate::conn::CdpConnection::new_with_fetch_config(
        clear_config,
    ));
    let mut browser_context = BrowserContext::new("BID-cache-owner".into());
    browser_context.http_cache_root = Some(cache_root.path.clone());
    browser_context.http_cache_max_bytes = seed_config.http_cache_max_bytes();
    ctx.conn.browser_context = Some(browser_context);

    let app_origin = Url::parse(&app_url)
        .expect("app url should parse")
        .origin()
        .ascii_serialization();
    ctx.process_async(json!({
        "id": 12_532,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": app_origin,
            "storageTypes": "cache_storage"
        }
    }))
    .await;
    ctx.expect_result(12_532, json!({}), None);

    app_server.abort();
    let _ = app_server.await;

    let client = moli_fetch::FetchClient::new(
        &seed_config,
        moli_cookie_jar::new_shared_browser_cookie_store(),
    );
    let app_error = client
        .fetch(moli_fetch::Request::get(&app_url).expect("app cached request"))
        .await
        .expect_err("context-owner-cleared app origin cache entry should miss");
    assert!(
        format!("{app_error:#}").contains("curl request failed"),
        "unexpected app cache miss error: {app_error:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_clear_data_for_origin_rejects_invalid_origin_even_for_noop_types() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-origin-invalid".into()));

    ctx.process_async(json!({
        "id": 12_530,
        "method": "Storage.clearDataForOrigin",
        "params": {
            "origin": "not an origin",
            "storageTypes": "local_storage"
        }
    }))
    .await;
    ctx.expect_error(12_530, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_set_cookies_reports_missing_cookie_url_when_no_default_scope_exists() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-MISS".into()));

    ctx.process_async(json!({
        "id": 126,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-MISS",
            "cookies": [{
                "name": "sid",
                "value": "1"
            }]
        }
    }))
    .await;
    ctx.expect_result(
        126,
        json!({
            "success": false,
            "cookieReports": [{
                "status": {
                    "kind": "Rejected",
                    "reason": "MissingCookieUrl"
                },
                "rejectionReasons": ["MissingCookieUrl"],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_delete_cookies_respects_optional_path_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-D".into()));

    ctx.process_async(json!({
        "id": 20,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-D",
            "cookies": [
                { "name": "sid", "value": "root", "domain": "example.com", "path": "/" },
                { "name": "sid", "value": "app", "domain": "example.com", "path": "/app" }
            ]
        }
    }))
    .await;
    ctx.expect_result(20, json!({}), None);

    ctx.process_async(json!({
        "id": 21,
        "method": "Storage.deleteCookies",
        "params": {
            "browserContextId": "BID-D",
            "name": "sid",
            "domain": "example.com",
            "path": "/app"
        }
    }))
    .await;
    ctx.expect_result(21, json!({}), None);

    ctx.process_async(json!({
        "id": 22,
        "method": "Storage.getCookies",
        "params": { "browserContextId": "BID-D" }
    }))
    .await;
    ctx.expect_result(
        22,
        json!({
            "cookies": [{
                "name": "sid",
                "value": "root",
                "domain": ".example.com",
                "path": "/",
                "size": 7
            }]
        }),
        None,
    );
}
