use moli_test_support as support;

use super::{
    Browser, FetchDeadline, FetchedDocument, NavigationEngine, NavigationPageStorageHandles,
    NavigationResourceStorageHandles, RenderedDomWaitUntil,
    external_raw_document_body_from_streaming_response_with_body_eof_observer,
};
use crate::{
    RendererOutputItem, RendererOutputResidenceIdentity, RendererOutputTransportMessage,
    RendererOutputTransportReceiver, RendererOwnerAction,
    page::{
        DocumentNodeClientRectResolution, Page, PendingPageCommand,
        PendingSubresourceContinueOutcome, PendingSubresourceFetchInfo,
        RendererDocumentQuerySelectorNode, RendererDocumentQuerySelectorResolution,
        RendererRuntimeInspectorMessage, SubresourceResourceType, SubresourceResponseWaitCriteria,
    },
    renderer::RendererPageCommand,
    renderer::{PageId, RendererOwnerCommand, RendererOwnerHandle, materialize_page_created_reply},
    runtime::BrowserConfig as AppConfig,
};
use anyhow::{Context, Result, anyhow};
use moli_browser_profile::{BrowserProfileLock, BrowserProfilePaths, load_profile_manifest};
use moli_cookie_jar::new_shared_browser_cookie_store;
use moli_fetch::{
    BrowserNavigationRequestKind, FetchCancelHandle, FetchConfig, Request, ResponseHead,
    StreamingRawResponse,
};
use moli_page_types::{LayoutPolicy, OptionalResourceFetchMask};
use moli_renderer_v8::new_shared_web_storage_store;
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::FixtureServer;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Mutex, oneshot},
    task::JoinHandle,
};
use url::Url;

#[test]
fn browser_clone_keeps_shared_renderer_and_fetch_owners_live_after_source_drop() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;
    let expected_runtime_id = browser
        .resource_request_client()
        .resource_runtime_diagnostics()
        .runtime_id;
    let expected_renderer_owner_id = browser.js_runtime.renderer_owner_id_for_diagnostics();
    let clone = browser.clone();

    drop(browser);

    assert_eq!(
        clone
            .resource_request_client()
            .resource_runtime_diagnostics()
            .runtime_id,
        expected_runtime_id
    );
    assert_eq!(
        clone.js_runtime.renderer_owner_id_for_diagnostics(),
        expected_renderer_owner_id
    );
    Ok(())
}

#[tokio::test]
async fn external_raw_bridge_drop_after_body_eof_cancels_pending_fetch_completion() -> Result<()> {
    let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
    body_tx.send(b"complete body".to_vec())?;
    drop(body_tx);
    let cancel_handle = FetchCancelHandle::new();
    let (mut fetch_completion_tx, fetch_completion_rx) = oneshot::channel();
    let response = StreamingRawResponse::new_with_head(
        ResponseHead {
            final_url: Url::parse("https://bridge.test/document")?,
            status: 200,
            headers: vec![("content-type".to_owned(), "text/html".to_owned())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body_rx,
        cancel_handle.clone(),
        fetch_completion_rx,
    );
    let (body_eof_tx, body_eof_rx) = oneshot::channel();
    let outward = external_raw_document_body_from_streaming_response_with_body_eof_observer(
        response,
        Some(body_eof_tx),
    );

    body_eof_rx
        .await
        .context("external bridge should observe body EOF before fetch completion")?;
    assert!(
        !fetch_completion_tx.is_closed(),
        "the test must hold fetch completion pending at the EOF barrier"
    );

    drop(outward);
    fetch_completion_tx.closed().await;
    assert!(
        cancel_handle.is_cancelled(),
        "dropping the external bridge in the EOF-to-completion window must cancel the fetch"
    );
    Ok(())
}

async fn recv_subresource_fetch_pause_for_page(
    output_rx: &mut RendererOutputTransportReceiver,
    page: &Page,
) -> Result<PendingSubresourceFetchInfo> {
    let owner_local_host_id = page.renderer_owner_local_host_id();
    let page_id = page.renderer_page_id();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let message = output_rx
                .recv()
                .await
                .context("renderer output transport closed before Fetch pause")?;
            let RendererOutputTransportMessage::Publication(publication) = message else {
                continue;
            };
            if !matches!(
                publication.cursor().stream().residence(),
                RendererOutputResidenceIdentity::Page {
                    owner_local_host_id: output_owner,
                    page_id: output_page,
                } if output_owner == owner_local_host_id && output_page == page_id
            ) {
                continue;
            }
            if let Some(info) =
                publication
                    .records()
                    .iter()
                    .find_map(|record| match record.item() {
                        RendererOutputItem::OwnerAction(
                            RendererOwnerAction::SubresourceFetchPause { info, .. },
                        ) => Some(info.as_ref().clone()),
                        _ => None,
                    })
            {
                return Ok(info);
            }
        }
    })
    .await
    .context("timed out waiting for concrete subresource Fetch pause")?
}

async fn query_selector_node_from_live_document(
    page: &mut Page,
    selector: &str,
) -> Result<Option<RendererDocumentQuerySelectorNode>> {
    let pending = page.start_document_query_selector_for_document(selector.to_owned(), false)?;
    let completion = pending.wait().await?;
    match page.finish_document_query_selector(completion)? {
        RendererDocumentQuerySelectorResolution::Found(nodes) => Ok(nodes.into_iter().next()),
        RendererDocumentQuerySelectorResolution::MissingRoot => Ok(None),
        RendererDocumentQuerySelectorResolution::InvalidSelector(message) => {
            Err(anyhow!("invalid selector {selector:?}: {message}"))
        }
    }
}

async fn resolve_runtime_object_for_backend_node_id(
    page: &mut Page,
    backend_node_id: u32,
    execution_context_id: Option<i64>,
    object_group: Option<&str>,
) -> Result<crate::page::DocumentNodeRuntimeObjectResolution> {
    let pending = page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
        None,
        backend_node_id,
        execution_context_id,
        object_group,
    )?;
    let completion = pending.wait().await?;
    page.finish_resolve_runtime_object_for_backend_node_id(completion)
}

#[tokio::test(flavor = "multi_thread")]
async fn page_creation_diagnostics_include_default_runtime_context_event() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: Url = server.url("/runtime-context-diagnostics").parse()?;
    let html = "<!doctype html><html><body>ok</body></html>".to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url.clone(),
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let (handle, page_state, diagnostics, artifacts, pending_download) =
        renderer_owner.materialize_page_created_reply_parts(reply)?;
    assert!(pending_download.is_none());
    let _page = Page::from_attached_handle(handle, page_state);

    let default_context = diagnostics
        .initial_runtime_realms
        .iter()
        .find(|realm| realm.is_default && realm.context_type == "default")
        .context("page creation should report the default runtime context")?;
    assert!(default_context.context_id > 0);
    assert_eq!(default_context.name, page_url.as_str());
    assert!(
        default_context
            .realm_id
            .as_deref()
            .is_some_and(|id| !id.is_empty()),
        "page creation should expose the native V8 realm id"
    );
    assert_eq!(
        artifacts.active_document.page_id,
        artifacts.lifecycle_snapshot.frame.page_id
    );
    assert_eq!(artifacts.active_epoch, artifacts.lifecycle_snapshot.epoch);
    assert_eq!(
        artifacts
            .initial_lifecycle_events
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![
            crate::page::RendererDocumentLifecycleEventKind::Started {
                reason: crate::page::RendererLifecycleStartReason::InitialDocument,
            },
            crate::page::RendererDocumentLifecycleEventKind::Milestone(
                crate::page::RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            crate::page::RendererDocumentLifecycleEventKind::Milestone(
                crate::page::RendererDocumentLifecycleMilestone::Load,
            ),
        ]
    );
    assert!(artifacts.lifecycle_snapshot.dom_content_loaded.is_some());
    assert!(artifacts.lifecycle_snapshot.load.is_some());
    assert!(
        artifacts
            .initial_lifecycle_events
            .windows(2)
            .all(|events| events[0].sequence < events[1].sequence
                && events[0].timestamp_micros <= events[1].timestamp_micros)
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn page_creation_diagnostics_fence_initial_runtime_observable_output() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: Url = server.url("/runtime-observable-diagnostics").parse()?;
    let html = "<!doctype html><script>console.warn('creation warning')</script>".to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url.clone(),
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let (handle, page_state, diagnostics, _artifacts, pending_download) =
        renderer_owner.materialize_page_created_reply_parts(reply)?;
    assert!(pending_download.is_none());
    let _page = Page::from_attached_handle(handle, page_state);

    assert!(
        diagnostics.renderer_output_predecessor.is_some(),
        "page creation must fence the concrete Page-stream batch containing initial console output"
    );

    server.shutdown().await;
    Ok(())
}

struct TempProfileDir {
    path: PathBuf,
}

#[test]
fn browser_without_profile_does_not_create_indexeddb_sibling_for_http_cache() -> Result<()> {
    let profile = TempProfileDir::new("no-profile-indexeddb");
    let cache_dir = profile.path.join("http-cache");
    let indexeddb_dir = profile.path.join("indexeddb");
    let mut config = AppConfig::default();
    config
        .fetch_mut()
        .set_http_cache_dir(Some(cache_dir.to_string_lossy().into_owned()));

    let browser = Browser::new(config)?;
    drop(browser);

    assert!(
        !indexeddb_dir.exists(),
        "no-profile Browser::new should keep IndexedDB in memory, not create {}",
        indexeddb_dir.display()
    );
    Ok(())
}

impl TempProfileDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-profile-{name}-{}-{nonce}",
            std::process::id()
        ));
        Self { path }
    }
}

impl Drop for TempProfileDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn indexed_db_origin_file(root: &std::path::Path, origin: &str) -> PathBuf {
    let mut encoded = String::with_capacity(origin.len() * 2);
    for byte in origin.as_bytes() {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    root.join(format!("{encoded}.json"))
}

fn indexed_db_origin_exists(root: &std::path::Path, origin: &str) -> bool {
    if indexed_db_origin_file(root, origin).exists() {
        return true;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.filter_map(|entry| entry.ok()).any(|entry| {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            return false;
        }
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("origin")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some(origin)
    })
}

fn first_party_storage_key_for_url(url: &Url) -> String {
    moli_storage_key::MoliStorageKey::first_party_from_url(url, None).serialized_storage_key()
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_about_blank_case_variants_materialize_empty_documents() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;
    for raw_url in [
        "about:blank",
        "about:BLANK",
        "about:Blank#fragment",
        "ABOUT:bLaNk?query#fragment",
    ] {
        let expected_url = Url::parse(raw_url)?;
        let mut page = browser
            .fetch_with_wait_until(raw_url, RenderedDomWaitUntil::Load, Duration::from_secs(1))
            .await?;

        assert_eq!(page.requested_url(), &expected_url, "{raw_url}");
        assert_eq!(page.final_url(), &expected_url, "{raw_url}");
        assert_eq!(page.status(), 200, "{raw_url}");
        assert_eq!(
            page.evaluate_runtime_expression_async("document.URL")
                .await?,
            serde_json::json!({"type": "string", "value": expected_url.as_str()}),
            "{raw_url}"
        );
        assert_eq!(
            page.evaluate_runtime_expression_async("document.readyState")
                .await?,
            serde_json::json!({"type": "string", "value": "complete"}),
            "{raw_url}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_fetch_rejects_file_navigation_without_reading_local_file() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;

    let error = browser
        .fetch_with_wait_until(
            "file:///moli-policy-must-not-open",
            RenderedDomWaitUntil::Load,
            Duration::from_secs(1),
        )
        .await
        .expect_err("hosted Browser::fetch must not receive local file capability");

    assert_eq!(
        error.to_string(),
        "Navigation to a local file URL requires an explicitly granted browser capability."
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_done_does_not_wait_for_future_interval_timers() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = tokio::time::timeout(
        Duration::from_secs(2),
        browser.fetch_with_wait_until(
            &server.url("/runtime/future-interval-done"),
            RenderedDomWaitUntil::Done,
            Duration::from_millis(500),
        ),
    )
    .await
    .context("fetch wait_until Done should not wait for future interval ticks")??;

    assert_eq!(
        page.evaluate_runtime_expression_async("document.body.dataset.ready")
            .await?,
        serde_json::json!({"type": "string", "value": "yes"})
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_allow_http_error_materializes_error_status_pages() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let not_found_url = server.url("/net/upstream/xhr/404");
    let server_error_url = server.url("/net/upstream/xhr/500");

    let default_error = browser
        .fetch(&not_found_url)
        .await
        .expect_err("default fetch should reject HTTP error statuses");
    assert!(
        default_error.to_string().contains("404 Not Found"),
        "{default_error:?}"
    );

    let mut not_found_page = browser.fetch_allow_http_error(&not_found_url).await?;
    assert_eq!(not_found_page.status(), 404);
    assert_eq!(
        not_found_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "Not Found"})
    );

    let mut server_error_page = browser
        .fetch_allow_http_error_with_wait_until(
            &server_error_url,
            RenderedDomWaitUntil::Load,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(server_error_page.status(), 500);
    assert_eq!(
        server_error_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "Internal Server Error"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_fetch_main_resource_navigation_uses_service_worker_response() -> Result<()> {
    let (base_url, requests, server) = spawn_main_resource_service_worker_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut registration_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/register.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut registration_page,
            "String(globalThis.__mainResourceSwReady).startsWith('ready:')",
            Duration::from_secs(5),
        )
        .await?;
    let ready_state = registration_page
        .evaluate_runtime_expression_async("String(globalThis.__mainResourceSwReady)")
        .await?;
    assert_eq!(
        ready_state,
        serde_json::json!({"type": "string", "value": "ready:true"})
    );

    let mut controlled_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/controlled.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(controlled_page.status(), 200);
    assert_eq!(
        controlled_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "sw-main:document:navigate"})
    );

    let mut fallback_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/fallback.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        fallback_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "network fallback"})
    );

    let mut preload_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_page.status(), 200);
    assert_eq!(
        preload_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "preload:200:preload:core-preload:network-preload-body"})
    );

    let mut preload_headers_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-headers.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_headers_page.status(), 200);
    assert_eq!(
        preload_headers_page
            .evaluate_runtime_expression_async(
                r#"
(() => {
  const headers = JSON.parse(document.body.textContent);
  return JSON.stringify({
    preload: headers["SERVICE-WORKER-NAVIGATION-PRELOAD"],
    upgrade: headers["UPGRADE-INSECURE-REQUESTS"]
  });
})()
"#
            )
            .await?,
        serde_json::json!({
            "type": "string",
            "value": "{\"preload\":[\"core-preload\"],\"upgrade\":[\"1\"]}"
        })
    );

    let mut preload_gzip_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-gzip.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_gzip_page.status(), 200);
    assert_eq!(
        preload_gzip_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "Hello World"})
    );

    let mut preload_chunked_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-chunked.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_chunked_page.status(), 200);
    assert_eq!(
        preload_chunked_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "0123456789"})
    );

    let mut preload_cookie_lax_first_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-cookie-lax.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_cookie_lax_first_page.status(), 200);
    assert_eq!(
        preload_cookie_lax_first_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "0"})
    );

    let mut preload_cookie_lax_second_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-cookie-lax.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_cookie_lax_second_page.status(), 200);
    assert_eq!(
        preload_cookie_lax_second_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "1"})
    );

    let mut preload_cookie_strict_first_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-cookie-strict.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_cookie_strict_first_page.status(), 200);
    assert_eq!(
        preload_cookie_strict_first_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "0"})
    );

    let mut preload_cookie_strict_second_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-cookie-strict.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_cookie_strict_second_page.status(), 200);
    assert_eq!(
        preload_cookie_strict_second_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "1"})
    );

    let mut preload_empty_body_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-empty-body.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_empty_body_page.status(), 200);
    assert_eq!(
        preload_empty_body_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "[]"})
    );

    let mut preload_broken_unused_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-broken-body-unused.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_broken_unused_page.status(), 200);
    assert_eq!(
        preload_broken_unused_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "PASS: preloadResponse resolved"})
    );

    let mut preload_redirect_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-redirect.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_redirect_page.status(), 200);
    assert_eq!(
        preload_redirect_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({
            "type": "string",
            "value": format!(
                "preload-redirect:0:opaqueredirect:false:{base_url}/app/preload-redirect.html:"
            )
        })
    );

    let mut preload_redirect_direct_body_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-redirect-direct-body.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        preload_redirect_direct_body_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "BODY"})
    );

    let mut preload_redirect_follow_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-redirect-follow.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        preload_redirect_follow_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "redirected\n"})
    );

    let mut preload_redirect_to_scope_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-redirect-to-scope.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        preload_redirect_to_scope_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "redirected\n"})
    );

    let mut preload_body_error_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-body-error.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_body_error_page.status(), 200);
    assert_eq!(
        preload_body_error_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({
            "type": "string",
            "value": "{\"hasResponse\":true,\"status\":200,\"body\":null,\"bodyError\":{\"name\":\"TypeError\",\"message\":\"The service worker navigation preload request failed due to a network error. This may have been an actual network error, or caused by the browser simulating offline to see if the page works offline: see https://w3c.github.io/manifest/#installability-signals\",\"isTypeError\":true}}"
        })
    );

    let mut preload_cancel_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/preload-cancel.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(preload_cancel_page.status(), 200);
    assert_eq!(
        preload_cancel_page
            .evaluate_runtime_expression_async("document.body.textContent")
            .await?,
        serde_json::json!({"type": "string", "value": "preload cancel handled"})
    );
    browser
        .wait_for_script_truthy(
            &mut preload_cancel_page,
            "String(globalThis.__preloadCancelProbe).startsWith('{')",
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        preload_cancel_page
            .evaluate_runtime_expression_async("String(globalThis.__preloadCancelProbe)")
            .await?,
        serde_json::json!({
            "type": "string",
            "value": "{\"name\":\"NetworkError\",\"message\":\"The service worker navigation preload request was cancelled before 'preloadResponse' settled. If you intend to use 'preloadResponse', use waitUntil() or respondWith() to wait for the promise to settle.\",\"isDomException\":true}"
        })
    );

    server.abort();
    let requests = requests.lock().await.clone();
    assert!(
        requests
            .iter()
            .filter(|request| request_path(request).as_deref() == Some("/app/controlled.html"))
            .all(|request| request_has_navigation_preload_header(request, "core-preload")),
        "controlled main resource should only hit network as a navigation preload request, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(
                |request| request_path(request).as_deref() == Some("/app/fallback.html")
                    && !request_has_navigation_preload_header(request, "core-preload")
            ),
        "fallback main resource should return to network after the preload request, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(
                |request| request_path(request).as_deref() == Some("/app/preload.html")
                    && request_has_navigation_preload_header(request, "core-preload")
            ),
        "navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(
                |request| request_path(request).as_deref() == Some("/app/preload-headers.html")
                    && request_has_navigation_preload_header(request, "core-preload")
                    && request_has_header_value(request, "Upgrade-Insecure-Requests", "1")
            ),
        "preload headers path should include navigation preload and upgrade-insecure headers, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(
                |request| request_path(request).as_deref() == Some("/app/preload-gzip.html")
                    && request_has_navigation_preload_header(request, "core-preload")
            ),
        "gzip navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(
                |request| request_path(request).as_deref() == Some("/app/preload-chunked.html")
                    && request_has_navigation_preload_header(request, "core-preload")
            ),
        "chunked navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-cookie-lax.html")
                && request_has_navigation_preload_header(request, "core-preload")
                && request_has_header_containing(request, "Cookie", "preload_lax=1")),
        "same-site Lax cookie navigation preload should send the stored cookie on the second visit, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-cookie-strict.html")
                && request_has_navigation_preload_header(request, "core-preload")
                && request_has_header_containing(request, "Cookie", "preload_strict=1")),
        "same-site Strict cookie navigation preload should send the stored cookie on the second visit, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-empty-body.html")
                && request_has_navigation_preload_header(request, "core-preload")),
        "empty-body navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-broken-body-unused.html")
                && request_has_navigation_preload_header(request, "core-preload")),
        "broken-body navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(
                |request| request_path(request).as_deref() == Some("/app/preload-redirect.html")
                    && request_has_navigation_preload_header(request, "core-preload")
            ),
        "redirect navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-redirect-direct-body.html")
                && request_has_navigation_preload_header(request, "core-preload")),
        "direct no-location redirect-body navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-redirect-follow.html")
                && request_has_navigation_preload_header(request, "core-preload")),
        "direct Location redirect navigation preload should issue an initial network request with the configured header, requests: {requests:?}"
    );
    for path in [
        "/app/preload-redirect-to-scope.html",
        "/app/preload-redirect-to-scope-2.html",
        "/app/preload-redirect-to-scope-3.html",
    ] {
        assert!(
            requests
                .iter()
                .any(|request| request_path(request).as_deref() == Some(path)
                    && request_has_navigation_preload_header(request, "core-preload")),
            "same-scope redirect hop {path} should issue a navigation preload request with the configured header, requests: {requests:?}"
        );
    }
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/outside/preload-redirected.html")
                && !request_has_navigation_preload_header(request, "core-preload")),
        "direct Location redirect should follow the internal redirect to network without a preload header outside the controlled scope, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request_path(request).as_deref()
                == Some("/app/preload-body-error.html")
                && request_has_navigation_preload_header(request, "core-preload")),
        "body-error navigation preload should issue a network request with the configured header, requests: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .all(|request| request_path(request).as_deref() != Some("/app/preload-final.html")),
        "navigation preload must expose the redirect response instead of following to the final URL, requests: {requests:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_persists_local_storage_but_not_session_storage() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let profile = TempProfileDir::new("localstorage");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));

    {
        let browser = Browser::new(config.clone())?;
        let mut page = browser.fetch(&server.url("/static")).await?;
        let write = page
            .evaluate_runtime_expression_async(
                "localStorage.clear(); sessionStorage.clear(); localStorage.setItem('persisted', 'yes'); sessionStorage.setItem('ephemeral', 'yes'); 'ok'",
            )
            .await?;
        assert_eq!(write["value"], serde_json::json!("ok"));
    }

    let profile_paths = BrowserProfilePaths::new(&profile.path);
    let profile_json = fs::read_to_string(&profile_paths.local_storage_path)
        .context("expected localStorage profile json to be written")?;
    assert!(
        profile_json.contains("\"persisted\"") && profile_json.contains("\"yes\""),
        "profile localStorage file should contain persisted key: {profile_json}"
    );

    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let read = page
        .evaluate_runtime_expression_async(
            "`${localStorage.getItem('persisted')}|${String(sessionStorage.getItem('ephemeral'))}`",
        )
        .await?;
    assert_eq!(read["value"], serde_json::json!("yes|null"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_persists_storage_bucket_names() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let profile = TempProfileDir::new("storage-buckets");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));
    let page_url = server.url("/static");
    let page_url = Url::parse(&page_url)?;
    let storage_key = first_party_storage_key_for_url(&page_url);
    let page_url = page_url.to_string();

    {
        let browser = Browser::new(config.clone())?;
        let mut page = browser.fetch(&page_url).await?;
        let write = page
            .evaluate_runtime_expression_with_await_async(
                r#"
(async () => {
  await navigator.storageBuckets.open("bucket-b");
  await navigator.storageBuckets.open("bucket-a");
  return (await navigator.storageBuckets.keys()).join("|");
})()
"#,
                true,
            )
            .await?;
        assert_eq!(write["value"], serde_json::json!("bucket-a|bucket-b"));
    }

    let profile_paths = BrowserProfilePaths::new(&profile.path);
    let bucket_json = fs::read_to_string(&profile_paths.storage_buckets_path)
        .context("expected Storage Buckets profile json to be written")?;
    let persisted: serde_json::Value = serde_json::from_str(&bucket_json)?;
    assert_eq!(persisted["version"], serde_json::json!(5));
    assert_eq!(persisted["nextBucketId"], serde_json::json!(3));
    let bucket_a_id = persisted["origins"][storage_key.as_str()]["bucket-a"]["bucketId"]
        .as_u64()
        .context("expected bucket-a to have a persistent bucket ID")?;
    let bucket_b_id = persisted["origins"][storage_key.as_str()]["bucket-b"]["bucketId"]
        .as_u64()
        .context("expected bucket-b to have a persistent bucket ID")?;
    assert_ne!(bucket_a_id, 0);
    assert_ne!(bucket_b_id, 0);
    assert_ne!(bucket_a_id, bucket_b_id);

    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&page_url).await?;
    let read = page
        .evaluate_runtime_expression_with_await_async(
            r#"
(async () => (await navigator.storageBuckets.keys()).join("|"))()
"#,
            true,
        )
        .await?;
    assert_eq!(read["value"], serde_json::json!("bucket-a|bucket-b"));

    let reopened: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&profile_paths.storage_buckets_path)?)?;
    assert_eq!(
        reopened["origins"][storage_key.as_str()]["bucket-a"]["bucketId"],
        serde_json::json!(bucket_a_id)
    );
    assert_eq!(
        reopened["origins"][storage_key.as_str()]["bucket-b"]["bucketId"],
        serde_json::json!(bucket_b_id)
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_persists_storage_bucket_indexeddb_separately() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let profile = TempProfileDir::new("storage-bucket-indexeddb");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));
    let page_url = server.url("/static");
    let page_url = Url::parse(&page_url)?;
    let origin = page_url.origin().ascii_serialization();
    let storage_key = first_party_storage_key_for_url(&page_url);
    let page_url = page_url.to_string();
    {
        let browser = Browser::new(config.clone())?;
        let mut page = browser.fetch(&page_url).await?;
        let write = page
            .evaluate_runtime_expression_with_await_async(
                r#"
(async () => {
  const bucket = await navigator.storageBuckets.open("idbbucket");
  const db = await new Promise((resolve, reject) => {
    const open = bucket.indexedDB.open("bucket-db", 1);
    open.onerror = () => reject(`open:${open.error && open.error.name}`);
    open.onupgradeneeded = () => {
      open.result.createObjectStore("kv");
    };
    open.onsuccess = () => resolve(open.result);
  });
  await new Promise((resolve, reject) => {
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put("bucket-value", "key");
    tx.onerror = () => reject(`tx:${tx.error && tx.error.name}`);
    tx.oncomplete = resolve;
  });
  db.close();
  return "stored";
})()
"#,
                true,
            )
            .await?;
        assert_eq!(write["value"], serde_json::json!("stored"));
    }

    let profile_paths = BrowserProfilePaths::new(&profile.path);
    let bucket_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&profile_paths.storage_buckets_path)?)?;
    let bucket_id = bucket_json["origins"][storage_key.as_str()]["idbbucket"]["bucketId"]
        .as_u64()
        .and_then(moli_storage_service::StorageBucketId::new)
        .context("expected idbbucket to have a persistent bucket ID")?;
    let bucket_storage_key =
        crate::storage::storage_bucket_indexed_db_storage_key(&storage_key, bucket_id);
    assert!(
        indexed_db_origin_exists(&profile_paths.indexeddb_root, &bucket_storage_key),
        "bucket IndexedDB should write under its bucket storage key"
    );
    assert!(
        !indexed_db_origin_exists(&profile_paths.indexeddb_root, &origin),
        "bucket IndexedDB should not write under the origin-wide IndexedDB key"
    );

    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&page_url).await?;
    let read = page
        .evaluate_runtime_expression_with_await_async(
            r#"
(async () => {
  const bucket = await navigator.storageBuckets.open("idbbucket");
  const bucketValue = await new Promise((resolve, reject) => {
    const open = bucket.indexedDB.open("bucket-db", 1);
    open.onerror = () => reject(`bucket-open:${open.error && open.error.name}`);
    open.onupgradeneeded = () => reject("bucket-missing");
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction("kv", "readonly");
      const get = tx.objectStore("kv").get("key");
      get.onerror = () => reject(`bucket-get:${get.error && get.error.name}`);
      get.onsuccess = () => {
        const value = get.result;
        db.close();
        resolve(value);
      };
    };
  });
  const globalState = await new Promise((resolve, reject) => {
    const open = indexedDB.open("bucket-db", 1);
    let upgraded = false;
    open.onerror = () => reject(`global-open:${open.error && open.error.name}`);
    open.onupgradeneeded = () => {
      upgraded = true;
      open.result.createObjectStore("kv");
    };
    open.onsuccess = () => {
      open.result.close();
      resolve(upgraded ? "missing" : "leaked");
    };
  });
  return `${bucketValue}|${globalState}`;
})()
"#,
            true,
        )
        .await?;
    assert_eq!(read["value"], serde_json::json!("bucket-value|missing"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_prunes_expired_storage_bucket_indexeddb_on_open() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let profile = TempProfileDir::new("storage-bucket-expired-indexeddb");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));
    let page_url = server.url("/static");
    let page_url = Url::parse(&page_url)?;
    let storage_key = first_party_storage_key_for_url(&page_url);
    let page_url = page_url.to_string();
    let bucket_name = "expired-idbbucket";
    {
        let browser = Browser::new(config.clone())?;
        let mut page = browser.fetch(&page_url).await?;
        let write = page
            .evaluate_runtime_expression_with_await_async(
                r#"
(async () => {
  const bucket = await navigator.storageBuckets.open("expired-idbbucket", {
    expires: Date.now() + 60000
  });
  const db = await new Promise((resolve, reject) => {
    const open = bucket.indexedDB.open("bucket-db", 1);
    open.onerror = () => reject(`open:${open.error && open.error.name}`);
    open.onupgradeneeded = () => {
      open.result.createObjectStore("kv");
    };
    open.onsuccess = () => resolve(open.result);
  });
  await new Promise((resolve, reject) => {
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").put("expired-bucket-value", "key");
    tx.onerror = () => reject(`tx:${tx.error && tx.error.name}`);
    tx.oncomplete = resolve;
  });
  db.close();
  return "stored";
})()
"#,
                true,
            )
            .await?;
        assert_eq!(write["value"], serde_json::json!("stored"));
    }

    let profile_paths = BrowserProfilePaths::new(&profile.path);
    let mut bucket_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&profile_paths.storage_buckets_path)?)?;
    let bucket_id = bucket_json["origins"][storage_key.as_str()][bucket_name]["bucketId"]
        .as_u64()
        .and_then(moli_storage_service::StorageBucketId::new)
        .context("expected expired bucket to have a persistent bucket ID")?;
    let bucket_storage_key =
        crate::storage::storage_bucket_indexed_db_storage_key(&storage_key, bucket_id);
    assert!(
        indexed_db_origin_exists(&profile_paths.indexeddb_root, &bucket_storage_key),
        "bucket IndexedDB should exist before profile-open pruning"
    );

    bucket_json["origins"][storage_key.as_str()][bucket_name]["expires"] = serde_json::json!(0.0);
    fs::write(
        &profile_paths.storage_buckets_path,
        serde_json::to_vec_pretty(&bucket_json)?,
    )?;

    {
        let browser = Browser::new(config)?;
        drop(browser);
    }

    assert!(
        !indexed_db_origin_exists(&profile_paths.indexeddb_root, &bucket_storage_key),
        "profile-open pruning should remove expired bucket IndexedDB backing"
    );
    let pruned_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&profile_paths.storage_buckets_path)?)?;
    assert!(
        pruned_json["origins"]
            .get(storage_key.as_str())
            .and_then(|buckets| buckets.get(bucket_name))
            .is_none(),
        "profile-open pruning should remove expired bucket metadata"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_persists_storage_bucket_cache_entries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let profile = TempProfileDir::new("storage-bucket-cache");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));
    let page_url = server.url("/static");

    {
        let browser = Browser::new(config.clone())?;
        let mut page = browser.fetch(&page_url).await?;
        let write = page
            .evaluate_runtime_expression_with_await_async(
                r#"
(async () => {
  const bucket = await navigator.storageBuckets.open("cachebucket");
  const cache = await bucket.caches.open("receipts");
  await cache.put("receipt.txt", new Response("profile cache body", {
    status: 202,
    statusText: "Accepted",
    headers: [["x-profile-cache", "hit"]]
  }));
  const estimate = await bucket.estimate();
  return estimate.usageDetails.caches > 0;
})()
"#,
                true,
            )
            .await?;
        assert_eq!(write["value"], serde_json::json!(true));
    }

    let profile_paths = BrowserProfilePaths::new(&profile.path);
    assert!(
        profile_paths.cache_storage_root.exists(),
        "StorageBucket CacheStorage root should be profile-backed"
    );

    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&page_url).await?;
    let read = page
        .evaluate_runtime_expression_with_await_async(
            r#"
(async () => {
  const bucket = await navigator.storageBuckets.open("cachebucket");
  const cache = await bucket.caches.open("receipts");
  const matched = await cache.match("receipt.txt");
  const missing = await cache.match("missing.txt");
  const estimate = await bucket.estimate();
  return JSON.stringify({
    keys: await bucket.caches.keys(),
    status: matched.status,
    statusText: matched.statusText,
    header: matched.headers.get("x-profile-cache"),
    text: await matched.text(),
    missing: typeof missing,
    cacheUsage: estimate.usageDetails.caches > 0
  });
})()
"#,
            true,
        )
        .await?;
    assert_eq!(
        read["value"],
        serde_json::json!(
            r#"{"keys":["receipts"],"status":202,"statusText":"Accepted","header":"hit","text":"profile cache body","missing":"undefined","cacheUsage":true}"#
        )
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_initializes_profile_manifest() -> Result<()> {
    let profile = TempProfileDir::new("manifest");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));
    let profile_paths = BrowserProfilePaths::new(&profile.path);

    {
        let browser = Browser::new(config)?;
        drop(browser);
    }

    let manifest = load_profile_manifest(&profile_paths)?;
    assert_eq!(
        manifest.version,
        moli_browser_profile::PROFILE_MANIFEST_VERSION
    );
    assert_eq!(manifest.partitions.len(), 1);
    let partition = &manifest.partitions[0];
    assert_eq!(
        partition.id,
        moli_browser_profile::DEFAULT_PROFILE_PARTITION_ID
    );
    assert_eq!(partition.root, "partitions/default");
    assert_eq!(
        partition.cookies.path.as_deref(),
        Some("partitions/default/cookies.json")
    );
    assert_eq!(
        partition.local_storage.path.as_deref(),
        Some("partitions/default/localstorage.json")
    );
    assert_eq!(partition.session_storage.path, None);
    assert_eq!(
        partition.storage_buckets.path.as_deref(),
        Some("partitions/default/storage-buckets.json")
    );
    assert_eq!(
        partition.service_worker_resources.path.as_deref(),
        Some("partitions/default/service-worker-resources.json")
    );
    assert_eq!(
        partition.cache_storage.path.as_deref(),
        Some("partitions/default/cache-storage")
    );
    assert_eq!(
        partition.indexed_db.path.as_deref(),
        Some("partitions/default/indexeddb")
    );
    assert_eq!(
        partition.http_cache.path.as_deref(),
        Some("partitions/default/http-cache")
    );
    Ok(())
}

#[test]
fn browser_profile_dir_supplies_default_http_cache_root_without_overriding_explicit_cache()
-> Result<()> {
    let profile = TempProfileDir::new("partition-http-cache");
    let explicit_cache = TempProfileDir::new("explicit-http-cache");
    let profile_paths = BrowserProfilePaths::new(&profile.path);

    let mut default_config = AppConfig::default();
    default_config.set_profile_dir(Some(profile.path.clone()));
    let browser = Browser::new(default_config)?;
    assert_eq!(
        browser.config().fetch().http_cache_dir(),
        Some(profile_paths.http_cache_root.to_string_lossy().as_ref())
    );
    drop(browser);

    let mut explicit_config = AppConfig::default();
    explicit_config.set_profile_dir(Some(profile.path.clone()));
    explicit_config
        .fetch_mut()
        .set_http_cache_dir(Some(explicit_cache.path.to_string_lossy().into_owned()));
    let browser = Browser::new(explicit_config)?;
    assert_eq!(
        browser.config().fetch().http_cache_dir(),
        Some(explicit_cache.path.to_string_lossy().as_ref())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_dir_refuses_second_writer_until_first_browser_drops() -> Result<()> {
    let profile = TempProfileDir::new("writer-lock");
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));

    let first = Browser::new(config.clone())?;
    let error = Browser::new(config.clone()).expect_err("second profile writer should fail");
    let error_chain = format!("{error:?}");
    assert!(error_chain.contains("already locked"), "error: {error:?}");

    drop(first);
    let second = Browser::new(config)?;
    drop(second);

    Ok(())
}

#[test]
fn browser_profile_lock_refuses_browser_start_when_external_guard_is_held() -> Result<()> {
    let profile = TempProfileDir::new("external-lock");
    let paths = BrowserProfilePaths::new(&profile.path);
    let _lock = BrowserProfileLock::acquire(&paths)?;
    let mut config = AppConfig::default();
    config.set_profile_dir(Some(profile.path.clone()));

    let error = Browser::new(config).expect_err("held profile lock should reject Browser::new");
    let error_chain = format!("{error:?}");

    assert!(error_chain.contains("already locked"), "error: {error:?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_engine_keeps_network_runtime_when_web_storage_changes() -> Result<()> {
    let mut engine = NavigationEngine::new();
    let cookie_store = new_shared_browser_cookie_store();
    let session_storage_store = new_shared_web_storage_store();
    let first_store = new_shared_web_storage_store();
    let second_store = new_shared_web_storage_store();
    let url = Url::parse("http://example.test/")?;

    let mut first_page = engine
        .build_inline_html_document_page_with_storage_best_effort_async(
            NavigationPageStorageHandles::new(
                cookie_store.clone(),
                first_store,
                session_storage_store.clone(),
                None,
                None,
            ),
            url.clone(),
            None,
            "<!doctype html><html><body>first</body></html>".to_owned(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await?
        .page;
    first_page
        .evaluate_runtime_expression_async("localStorage.clear(); localStorage.setItem('ctx', 'a')")
        .await?;
    let first_resource_runtime_id = engine
        .resource_request_client()
        .expect("first page should initialize the resource runtime")
        .resource_runtime_diagnostics()
        .runtime_id;

    let mut second_page = engine
        .build_inline_html_document_page_with_storage_best_effort_async(
            NavigationPageStorageHandles::new(
                cookie_store,
                second_store,
                session_storage_store,
                None,
                None,
            ),
            url,
            None,
            "<!doctype html><html><body>second</body></html>".to_owned(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await?
        .page;

    assert_eq!(
        engine
            .resource_request_client()
            .expect("second page should retain the resource runtime")
            .resource_runtime_diagnostics()
            .runtime_id,
        first_resource_runtime_id,
        "changing only Web Storage identity must not rebuild the browser network runtime",
    );
    assert_eq!(
        second_page
            .evaluate_runtime_expression_async("String(localStorage.getItem('ctx'))")
            .await?,
        serde_json::json!({"type": "string", "value": "null"})
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_engine_bypasses_service_worker_for_main_resource() -> Result<()> {
    let (base_url, requests, server) = spawn_main_resource_service_worker_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut registration_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/register.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut registration_page,
            "String(globalThis.__mainResourceSwReady).startsWith('ready:')",
            Duration::from_secs(5),
        )
        .await?;

    let mut engine = NavigationEngine::new_with_fetch_config_and_browser_context_access(
        FetchConfig::default(),
        browser._lifetime_owner.browser_context_owner.owner_access(),
        OptionalResourceFetchMask::NONE,
        true,
    )?;
    engine.set_bypass_service_worker(true);
    let navigation = engine
        .fetch_navigation_streaming_raw_response_with_storage_async(
            NavigationResourceStorageHandles::new(
                browser.partition.cookie_store(),
                browser.partition.web_storage_store(),
                browser.partition.session_storage_store(),
            ),
            None,
            BrowserNavigationRequestKind::Navigate,
            false,
            "GET",
            &format!("{base_url}/app/controlled.html"),
            None,
            Vec::new(),
            None,
        )
        .await?;
    assert!(
        navigation.reserved_service_worker_client.is_some(),
        "bypass should retain an uncontrolled client reservation for document commit"
    );
    assert!(
        navigation.fetch_result.request_observation().is_some(),
        "the bypassed main resource should retain direct network transport metadata"
    );
    let response = navigation
        .fetch_result
        .into_response()
        .into_lossy_materialized_text_response()
        .await?;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body_text(),
        "<!doctype html><body>network controlled</body>"
    );

    server.abort();
    let requests = requests.lock().await.clone();
    assert!(
        requests.iter().any(|request| {
            request_path(request).as_deref() == Some("/app/controlled.html")
                && !request_has_navigation_preload_header(request, "core-preload")
        }),
        "bypassed main resource should use a direct network request, requests: {requests:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_engine_service_worker_main_resource_has_no_network_transport_metadata()
-> Result<()> {
    let (base_url, _requests, server) = spawn_main_resource_service_worker_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut registration_page = browser
        .fetch_allow_http_error_with_wait_until(
            &format!("{base_url}/app/register.html"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut registration_page,
            "String(globalThis.__mainResourceSwReady).startsWith('ready:')",
            Duration::from_secs(5),
        )
        .await?;

    let mut engine = NavigationEngine::new_with_fetch_config_and_browser_context_access(
        FetchConfig::default(),
        browser._lifetime_owner.browser_context_owner.owner_access(),
        OptionalResourceFetchMask::NONE,
        true,
    )?;
    let navigation = engine
        .fetch_navigation_streaming_raw_response_with_storage_async(
            NavigationResourceStorageHandles::new(
                browser.partition.cookie_store(),
                browser.partition.web_storage_store(),
                browser.partition.session_storage_store(),
            ),
            None,
            BrowserNavigationRequestKind::Navigate,
            false,
            "GET",
            &format!("{base_url}/app/controlled.html"),
            None,
            Vec::new(),
            None,
        )
        .await?;
    assert!(
        navigation.fetch_result.request_observation().is_none(),
        "a Service Worker synthetic response must not look like a network-service raw callback"
    );
    let response = navigation
        .fetch_result
        .into_response()
        .into_lossy_materialized_text_response()
        .await?;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body_text(),
        "<!doctype html><body>sw-main:document:navigate</body>"
    );

    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profile_indexeddb_root_is_page_local_when_default_changes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let profile_a = TempProfileDir::new("indexeddb-a");
    let profile_b = TempProfileDir::new("indexeddb-b");
    let mut config_a = AppConfig::default();
    config_a.set_profile_dir(Some(profile_a.path.clone()));
    let mut config_b = AppConfig::default();
    config_b.set_profile_dir(Some(profile_b.path.clone()));

    let browser_a = Browser::new(config_a)?;
    let _browser_b = Browser::new(config_b)?;
    let page_url = server.url("/static");
    let origin = Url::parse(&page_url)?.origin().ascii_serialization();
    let mut page = browser_a.fetch(&page_url).await?;

    page.evaluate_runtime_expression_async(
        r#"
(() => {
  globalThis.__indexedDbRootResult = "pending";
  const open = indexedDB.open("app", 1);
  open.onerror = () => {
globalThis.__indexedDbRootResult = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
open.result.close();
globalThis.__indexedDbRootResult = "ok";
  };
  return "scheduled";
})()
"#,
    )
    .await?;
    browser_a
        .wait_for_script_truthy(
            &mut page,
            "globalThis.__indexedDbRootResult === 'ok'",
            Duration::from_secs(2),
        )
        .await?;

    let profile_paths_a = BrowserProfilePaths::new(&profile_a.path);
    let profile_paths_b = BrowserProfilePaths::new(&profile_b.path);
    let storage_key = first_party_storage_key_for_url(&Url::parse(&format!("{origin}/"))?);
    assert!(
        indexed_db_origin_exists(&profile_paths_a.indexeddb_root, &storage_key),
        "browser A page should write IndexedDB under browser A profile"
    );
    assert!(
        !indexed_db_origin_exists(&profile_paths_b.indexeddb_root, &storage_key),
        "browser B default root should not receive browser A page writes"
    );

    server.shutdown().await;
    Ok(())
}

async fn wait_for_renderer_owner_state(
    renderer_owner: &RendererOwnerHandle,
    mut ready: impl FnMut(&RendererOwnerHandle) -> bool,
    context: &str,
) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if ready(renderer_owner) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{context}; final owner state len={}", renderer_owner.len()));
}

async fn wait_for_renderer_owner_page_removed(
    renderer_owner: &RendererOwnerHandle,
    page_id: PageId,
    context: &str,
) {
    wait_for_renderer_owner_state(
        renderer_owner,
        |renderer_owner| renderer_owner.record(page_id).is_none(),
        context,
    )
    .await;
}

async fn assert_unrelated_evaluate_runs_while_wait_command_is_pending(
    page: &mut Page,
    pending_wait: PendingPageCommand,
    marker: &str,
) -> Result<()> {
    let expression = format!("document.body.dataset.{marker} = 'ok'; 'ok'");
    let value = tokio::time::timeout(
        Duration::from_secs(2),
        page.evaluate_runtime_expression_async(&expression),
    )
    .await
    .context("unrelated evaluate timed out behind a pending wait command")??;
    assert_eq!(value["value"], serde_json::json!("ok"));
    drop(pending_wait);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn networkidle_times_out_while_intercepted_fetch_request_is_paused() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let (output_tx, mut output_rx) = crate::renderer_output_transport_channel();
    browser
        .js_runtime
        .set_renderer_output_transport_sender(output_tx);

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.set_fetch_subresource_interception_async(true, Some(SubresourceResourceType::Fetch))
        .await?;
    page.evaluate_runtime_expression_async(
        "fetch('/wait-until-data').then(r => r.text()).then(text => { document.body.setAttribute('data-state', text); }); 'scheduled';",
    )
    .await?;

    let error = page
        .wait_for_network_idle(
            &browser.resource_request_client(),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    let _pending = recv_subresource_fetch_pause_for_page(&mut output_rx, &page).await?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn networkidle_recovers_after_continuing_intercepted_fetch_request() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let (output_tx, mut output_rx) = crate::renderer_output_transport_channel();
    browser
        .js_runtime
        .set_renderer_output_transport_sender(output_tx);

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.set_fetch_subresource_interception_async(true, Some(SubresourceResourceType::Fetch))
        .await?;
    page.evaluate_runtime_expression_async(
        "fetch('/wait-until-data').then(r => r.text()).then(text => { document.body.setAttribute('data-state', text); }); 'scheduled';",
    )
    .await?;

    let pending = recv_subresource_fetch_pause_for_page(&mut output_rx, &page).await?;
    let outcome = page
        .continue_pending_subresource_fetch_async(
            pending.internal_id,
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .await?;
    assert!(matches!(
        outcome,
        PendingSubresourceContinueOutcome::Started
    ));

    page.wait_for_network_idle(&browser.resource_request_client(), Duration::from_secs(2))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-state=\"settled\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_selector_finds_late_element_via_renderer_wait_loop() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let node = browser
        .wait_for_selector(&mut page, "#late", Duration::from_secs(2))
        .await?;
    assert_eq!(
        query_selector_node_from_live_document(&mut page, "#late")
            .await?
            .map(|node| node.backend_node_id),
        Some(node.backend_node_id)
    );
    assert!(crate::page::is_renderer_backend_node_id(
        node.backend_node_id
    ));
    assert!(page.serialize_html_async().await?.contains("id=\"late\""));
    assert!(page.serialize_html_async().await?.contains("settled"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_selector_observes_attribute_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.evaluate_runtime_expression_async(
        "setTimeout(() => { document.body.classList.add('ready'); }, 50); 'scheduled';",
    )
    .await?;

    let node = browser
        .wait_for_selector(&mut page, "body.ready", Duration::from_secs(2))
        .await?;
    assert_eq!(
        query_selector_node_from_live_document(&mut page, "body.ready")
            .await?
            .map(|node| node.backend_node_id),
        Some(node.backend_node_id)
    );
    assert!(crate::page::is_renderer_backend_node_id(
        node.backend_node_id
    ));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("<body class=\"ready\">")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_selector_times_out_for_missing_selector() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let error = browser
        .wait_for_selector(&mut page, "#does-not-exist", Duration::from_millis(150))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("timed out waiting for selector `#does-not-exist`")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_selector_cancellation_restores_entry_for_close() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let canceled = tokio::time::timeout(
        Duration::from_millis(150),
        browser.wait_for_selector(&mut page, "#never-appears", Duration::from_secs(5)),
    )
    .await;
    assert!(
        canceled.is_err(),
        "selector wait should be cancelled by the test timeout"
    );
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after cancelled selector wait should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_deadline_spans_lifecycle_selector_and_script_without_reset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let started = std::time::Instant::now();
    let deadline = FetchDeadline::new(Duration::from_millis(600))?;
    let fetched = browser
        .fetch_request_document_allow_http_error_with_wait_until_deadline(
            Request::get(&server.url("/static"))?,
            RenderedDomWaitUntil::Load,
            deadline,
        )
        .await?;
    let FetchedDocument::Page(mut page) = fetched else {
        panic!("static HTML fixture unexpectedly produced a raw document");
    };
    let page_id = page.renderer_page_id();

    browser
        .wait_for_selector_with_deadline(&mut page, "body", deadline)
        .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let error = browser
        .wait_for_script_truthy_with_deadline(&mut page, "false", deadline)
        .await
        .unwrap_err();

    assert!(
        error
            .chain()
            .any(|cause| cause.to_string().contains("timed out")),
        "unexpected deadline error: {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "the script phase appears to have received a fresh timeout: elapsed={:?}",
        started.elapsed()
    );
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "a deadline-cancelled wait must return the Page owner entry for close"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn best_effort_page_readiness_consumes_only_the_remaining_fetch_deadline() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    for (wait_until, path, expected_html) in [
        (
            RenderedDomWaitUntil::NetworkIdle,
            "/wait-until-slow-interval-fetch",
            "data-state=\"init\"",
        ),
        (
            RenderedDomWaitUntil::DomStable,
            "/wait-until-slow-interval-dom-mutation",
            "id=\"mutation-count\"",
        ),
    ] {
        // The main response costs 500 ms. Readiness must receive only the
        // roughly 500 ms left in this one-second plan, not a fresh second.
        let deadline = FetchDeadline::new(Duration::from_secs(1))?;
        let started = std::time::Instant::now();
        let fetched = browser
            .fetch_request_document_allow_http_error_with_wait_until_deadline(
                Request::get(&server.url(path))?,
                wait_until,
                deadline,
            )
            .await?;
        let elapsed = started.elapsed();
        let FetchedDocument::Page(mut page) = fetched else {
            panic!("HTML fixture unexpectedly produced a raw document");
        };
        let page_id = page.renderer_page_id();

        assert!(
            elapsed < Duration::from_millis(1_300),
            "{wait_until:?} appears to have restarted its timeout after the slow main response: {elapsed:?}"
        );
        assert!(
            page.serialize_html_async().await?.contains(expected_html),
            "the best-effort Page must remain usable after {wait_until:?} expires"
        );

        let post_wait_started = std::time::Instant::now();
        let error = browser
            .wait_for_selector_with_deadline(&mut page, "#never-appears", deadline)
            .await
            .unwrap_err();
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("timed out")),
            "unexpected exhausted-deadline error: {error:?}"
        );
        assert!(
            post_wait_started.elapsed() < Duration::from_millis(200),
            "a post-readiness selector must not receive a fresh timeout"
        );

        tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;
        assert!(
            renderer_owner.record(page_id).is_none(),
            "best-effort deadline cancellation must return the Page entry for close"
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_wait_until_apis_do_not_restart_best_effort_timeout() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for (wait_until, path) in [
        (
            RenderedDomWaitUntil::NetworkIdle,
            "/wait-until-slow-interval-fetch",
        ),
        (
            RenderedDomWaitUntil::DomStable,
            "/wait-until-slow-interval-dom-mutation",
        ),
    ] {
        let started = std::time::Instant::now();
        let page = browser
            .fetch_with_wait_until(&server.url(path), wait_until, Duration::from_secs(1))
            .await?;

        assert!(
            started.elapsed() < Duration::from_millis(1_300),
            "the Page-returning {wait_until:?} API appears to have started a fresh timeout"
        );
        page.close_async().await?;
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn best_effort_readiness_does_not_soften_a_base_lifecycle_timeout() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for wait_until in [
        RenderedDomWaitUntil::NetworkIdle,
        RenderedDomWaitUntil::DomStable,
    ] {
        // No Page exists at 200 ms because the main response itself takes
        // 500 ms. Best effort applies only after the Load/DCL base boundary.
        let started = std::time::Instant::now();
        let error = browser
            .fetch_request_document_allow_http_error_with_wait_until(
                Request::get(&server.url("/wait-until-slow-static"))?,
                wait_until,
                Duration::from_millis(200),
            )
            .await
            .unwrap_err();

        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("timed out")),
            "unexpected base lifecycle error: {error:?}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "{wait_until:?} incorrectly softened or extended the base lifecycle timeout"
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_selector_wait_allows_unrelated_evaluate_while_pending() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let pending_wait = page.start_page_command(RendererPageCommand::WaitForSelector {
        selector: "#never-appears".to_owned(),
        timeout_ms: 10_000,
        loader: browser.resource_request_client(),
    })?;
    assert_unrelated_evaluate_runs_while_wait_command_is_pending(
        &mut page,
        pending_wait,
        "selectorWaitProbe",
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_selector_reports_invalid_selector_errors() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let error = browser
        .wait_for_selector(&mut page, "div[", Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("wait_for_selector `div[` failed inside renderer")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_observes_delayed_fetch_state_change() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-state') === 'settled'",
            Duration::from_secs(2),
        )
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-state=\"settled\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_advances_slow_runtime_script_started_from_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script-very-slow"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !page
            .serialize_html_async()
            .await?
            .contains("id=\"late-dcl-script-very-slow\"")
    );

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.querySelector('#late-dcl-script-very-slow') !== null",
            Duration::from_secs(7),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await?
            .contains("id=\"late-dcl-script-very-slow\"")
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains(">script-loaded-very-slow<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_observes_attribute_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.evaluate_runtime_expression_async(
        "setTimeout(() => { document.body.dataset.ready = 'yes'; }, 50); 'scheduled';",
    )
    .await?;

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.dataset.ready === 'yes'",
            Duration::from_secs(2),
        )
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-ready=\"yes\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_page_delay_advances_runtime_work() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.evaluate_runtime_expression_async(
        "setTimeout(() => { document.body.dataset.delayReady = 'yes'; }, 50); 'scheduled';",
    )
    .await?;

    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(100))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-delay-ready=\"yes\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_page_turn_follows_post_load_cookie_reload() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/location-nav/post-load-cookie-reload-challenge"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(400))
        .await?;
    let html = page.serialize_html_async().await?;

    assert!(
        html.contains("post-load-cookie-reload=done"),
        "standalone owner left the post-load reload pending: {html}"
    );
    assert!(
        html.contains("data-final-script=\"done\""),
        "observer command ran before the replacement document committed: {html}"
    );
    assert!(
        !html.contains("post-load-cookie-reload=source"),
        "challenge document survived its post-load reload: {html}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_selector_observes_replacement_document_before_load() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind replacement document commit fixture")?;
    let addr = listener
        .local_addr()
        .context("read replacement document commit fixture address")?;
    let (blocking_script_requested_tx, blocking_script_requested_rx) = oneshot::channel();
    let blocking_script_requested_tx = Arc::new(Mutex::new(Some(blocking_script_requested_tx)));
    let (release_blocking_script_tx, release_blocking_script_rx) = oneshot::channel();
    let release_blocking_script_rx = Arc::new(Mutex::new(Some(release_blocking_script_rx)));
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(_) => break,
            };
            let blocking_script_requested_tx = blocking_script_requested_tx.clone();
            let release_blocking_script_rx = release_blocking_script_rx.clone();
            tokio::spawn(async move {
                let request = match read_http_request_head(&mut stream).await {
                    Ok(request) => request,
                    Err(_) => return,
                };
                let path = request_path(&request).unwrap_or_else(|| "/".to_owned());
                if path == "/replacement-document-blocking.js" {
                    if let Some(requested) = blocking_script_requested_tx.lock().await.take() {
                        let _ = requested.send(());
                    }
                    if let Some(release) = release_blocking_script_rx.lock().await.take() {
                        let _ = release.await;
                    }
                    let body = "document.documentElement.dataset.finalScript = 'done';";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }

                let replacement = request_has_header_containing(
                    &request,
                    "Cookie",
                    "replacement_document_commit=1",
                );
                let (body, document_header) = if replacement {
                    (
                        concat!(
                            "<!doctype html><html><body>",
                            "<main id=\"replacement-target\">replacement</main>",
                            "<script src=\"/replacement-document-blocking.js\"></script>",
                            "</body></html>"
                        ),
                        "replacement",
                    )
                } else {
                    (
                        "<!doctype html><html><body><main id=\"source-target\">source</main></body></html>",
                        "source",
                    )
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Fixture-Document: {document_header}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let browser = Browser::new(AppConfig::default())?;
    let url = format!("http://127.0.0.1:{}/replacement-document", addr.port());
    let mut page = browser
        .fetch_with_wait_until(&url, RenderedDomWaitUntil::Load, Duration::from_secs(5))
        .await?;
    let initial_view = page.handle_for_testing().renderer_page_view_async().await?;
    let pending_selector = page.start_page_command(RendererPageCommand::WaitForSelector {
        selector: "#replacement-target".to_owned(),
        timeout_ms: 5_000,
        loader: browser.resource_request_client(),
    })?;

    page.evaluate_runtime_expression_async(
        "setTimeout(() => { document.cookie = 'replacement_document_commit=1; path=/'; location.reload(); }, 0); 'scheduled';",
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(2), blocking_script_requested_rx)
        .await
        .context("replacement document did not request its parser-blocking script")?
        .context("replacement document fixture dropped the script-request signal")?;

    let selector_completion = tokio::time::timeout(Duration::from_secs(2), pending_selector.wait())
        .await
        .context("selector remained blocked behind replacement load")??;
    assert!(matches!(
        page.finish_page_command(selector_completion),
        crate::renderer::RendererPageReply::DocumentQuerySelectorNode(_)
    ));
    let committed_view = page.handle_for_testing().renderer_page_view_async().await?;
    assert_ne!(committed_view.vm_creation_id, initial_view.vm_creation_id);
    assert!(committed_view.view_generation > initial_view.view_generation);
    assert_eq!(committed_view.page_state.requested_url.as_str(), url);
    assert_eq!(committed_view.page_state.status, 200);
    assert!(
        committed_view
            .page_state
            .headers
            .iter()
            .any(|(name, value)| {
                name.eq_ignore_ascii_case("X-Fixture-Document") && value == "replacement"
            })
    );

    release_blocking_script_tx
        .send(())
        .map_err(|()| anyhow!("blocking script response was no longer waiting"))?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.documentElement.dataset.finalScript === 'done' && document.readyState === 'complete'",
            Duration::from_secs(5),
        )
        .await?;

    page.close_async().await?;
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_no_document_response_keeps_current_document() -> Result<()> {
    let (base_url, server) = spawn_non_document_navigation_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch_with_wait_until(
            &format!("{base_url}/source"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let initial_view = page.handle_for_testing().renderer_page_view_async().await?;

    for path in ["/no-document", "/reset-content"] {
        let expression = format!(
            "location.assign({}); 'requested'",
            serde_json::to_string(path)?
        );
        let value = page.evaluate_runtime_expression_async(&expression).await?;
        assert_eq!(value["value"], serde_json::json!("requested"));
        let settled_view = page.handle_for_testing().renderer_page_view_async().await?;
        assert_eq!(settled_view.vm_creation_id, initial_view.vm_creation_id);
        assert_eq!(
            settled_view.page_state.requested_url,
            initial_view.page_state.requested_url
        );
        assert_eq!(
            page.evaluate_runtime_expression_async("location.href")
                .await?["value"],
            serde_json::json!(initial_view.page_state.requested_url.as_str())
        );
        assert!(
            page.serialize_html_async()
                .await?
                .contains("source-document")
        );
    }

    page.close_async().await?;
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_precommit_navigation_failure_keeps_current_document() -> Result<()> {
    let (base_url, server) = spawn_non_document_navigation_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch_with_wait_until(
            &format!("{base_url}/source"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let initial_view = page.handle_for_testing().renderer_page_view_async().await?;
    let unused_listener = TcpListener::bind("127.0.0.1:0").await?;
    let unavailable_url = format!("http://{}/unavailable", unused_listener.local_addr()?);
    drop(unused_listener);
    let expression = format!(
        "location.assign({}); 'requested'",
        serde_json::to_string(&unavailable_url)?
    );

    let error = page
        .evaluate_runtime_expression_async(&expression)
        .await
        .expect_err("connection failure should reject the navigation initiator");
    assert!(
        error.to_string().contains("Cannot navigate")
            || error.to_string().contains("curl request failed")
            || error.to_string().contains("connect")
            || error.to_string().contains("Connection"),
        "unexpected navigation failure: {error:#}"
    );
    let settled_view = page.handle_for_testing().renderer_page_view_async().await?;
    assert_eq!(settled_view.vm_creation_id, initial_view.vm_creation_id);
    assert_eq!(
        page.evaluate_runtime_expression_async("location.href")
            .await?["value"],
        serde_json::json!(initial_view.page_state.requested_url.as_str())
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("source-document")
    );

    page.close_async().await?;
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_download_response_keeps_current_document() -> Result<()> {
    let (base_url, server) = spawn_non_document_navigation_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch_with_wait_until(
            &format!("{base_url}/source"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let initial_view = page.handle_for_testing().renderer_page_view_async().await?;

    page.evaluate_runtime_expression_async("location.assign('/download'); 'requested'")
        .await?;
    let settled_view = page.handle_for_testing().renderer_page_view_async().await?;
    assert_eq!(settled_view.vm_creation_id, initial_view.vm_creation_id);
    assert_eq!(
        page.evaluate_runtime_expression_async("location.href")
            .await?["value"],
        serde_json::json!(initial_view.page_state.requested_url.as_str())
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("source-document")
    );

    page.close_async().await?;
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_awaits_promise_result() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.evaluate_runtime_expression_async(
        "globalThis.__waitTruthy = new Promise(resolve => setTimeout(() => resolve(true), 50)); 'scheduled';",
    )
    .await?;

    browser
        .wait_for_script_truthy(&mut page, "globalThis.__waitTruthy", Duration::from_secs(2))
        .await?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_invokes_function_predicate_each_poll() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.evaluate_runtime_expression_async("globalThis.__waitPredicateCalls = 0")
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.__waitPredicateCalls += 1, globalThis.__waitPredicateCalls >= 2",
            Duration::from_secs(2),
        )
        .await?;
    let expression_calls = page
        .evaluate_runtime_expression_async("globalThis.__waitPredicateCalls")
        .await?["value"]
        .as_f64()
        .expect("expression call count should be numeric");
    assert!(
        expression_calls >= 2.0,
        "comma expression changed meaning inside the predicate wrapper: {expression_calls}"
    );

    page.evaluate_runtime_expression_async("globalThis.__waitPredicateCalls = 0")
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "() => ++globalThis.__waitPredicateCalls >= 2",
            Duration::from_secs(2),
        )
        .await?;

    let calls = page
        .evaluate_runtime_expression_async("globalThis.__waitPredicateCalls")
        .await?["value"]
        .as_f64()
        .expect("predicate call count should be numeric");
    assert!(calls >= 2.0, "function predicate was not polled: {calls}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_wakes_after_async_wasm_compile_tasks() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    page.evaluate_runtime_expression_async(
        r#"
        globalThis.__wasmCompileDone = false;
        (async () => {
          const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
          for (let index = 0; index < 5; index += 1) {
            await WebAssembly.compile(bytes);
          }
          globalThis.__wasmCompileDone = true;
        })();
        "scheduled";
        "#,
    )
    .await?;

    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.__wasmCompileDone === true",
            Duration::from_millis(300),
        )
        .await?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_times_out_for_false_expression_and_function_predicate() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    for predicate in ["false", "() => false", "async () => false"] {
        let error = browser
            .wait_for_script_truthy(&mut page, predicate, Duration::from_millis(150))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("timed out waiting for script to become truthy"),
            "unexpected error for `{predicate}`: {error:#}"
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_cancellation_restores_entry_for_close() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let canceled = tokio::time::timeout(
        Duration::from_millis(150),
        browser.wait_for_script_truthy(&mut page, "false", Duration::from_secs(5)),
    )
    .await;
    assert!(
        canceled.is_err(),
        "script truthy wait should be cancelled by the test timeout"
    );
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after cancelled script-truthy wait should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_script_truthy_reports_invalid_expression_errors() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let error = browser
        .wait_for_script_truthy(&mut page, "(() => {", Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("wait_for_script_truthy `(() => {` failed inside renderer")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_script_truthy_wait_allows_unrelated_evaluate_while_pending() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let pending_wait = page.start_page_command(RendererPageCommand::WaitForScriptTruthy {
        expression: "false".to_owned(),
        timeout_ms: 10_000,
        loader: browser.resource_request_client(),
    })?;
    assert_unrelated_evaluate_runs_while_wait_command_is_pending(
        &mut page,
        pending_wait,
        "scriptTruthyWaitProbe",
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_network_idle_waits_for_delayed_fetch_completion() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    browser
        .wait_for_page_network_idle(&mut page, Duration::from_secs(2))
        .await?;
    assert!(page.serialize_html_async().await?.contains("id=\"late\""));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-state=\"settled\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_subresource_response_wait_allows_unrelated_evaluate_while_pending() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let pending_wait =
        page.start_page_command(RendererPageCommand::WaitForSubresourceResponse {
            criteria: SubresourceResponseWaitCriteria {
                url_contains: Some("/never-matches-this-response".to_owned()),
                ..SubresourceResponseWaitCriteria::default()
            },
            timeout_ms: 10_000,
            loader: browser.resource_request_client(),
        })?;
    assert_unrelated_evaluate_runs_while_wait_command_is_pending(
        &mut page,
        pending_wait,
        "subresourceResponseWaitProbe",
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn window_fetch_returns_before_slow_network_response() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let started = std::time::Instant::now();
    let value = page
        .evaluate_runtime_expression_async(
            "fetch('/wait-until-slow-data').then(r => r.text()).then(text => { document.body.setAttribute('data-fetch-state', text); }); document.body.setAttribute('data-fetch-returned', '1'); 'returned';",
        )
        .await?;
    assert_eq!(value["value"], serde_json::json!("returned"));
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "fetch() should return a promise without waiting for the network response"
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-fetch-returned=\"1\"")
    );

    browser
        .wait_for_page_network_idle(&mut page, Duration::from_secs(2))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-fetch-state=\"settled-slow\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn async_xhr_send_returns_before_slow_network_response() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let started = std::time::Instant::now();
    let value = page
        .evaluate_runtime_expression_async(
            "const xhr = new XMLHttpRequest(); xhr.open('GET', '/wait-until-slow-data'); xhr.onload = () => { document.body.setAttribute('data-xhr-state', xhr.responseText); }; xhr.send(); document.body.setAttribute('data-xhr-returned', '1'); 'returned';",
        )
        .await?;
    assert_eq!(value["value"], serde_json::json!("returned"));
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "async XMLHttpRequest.send() should not wait for the network response"
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-xhr-returned=\"1\"")
    );

    browser
        .wait_for_page_network_idle(&mut page, Duration::from_secs(2))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-xhr-state=\"settled-slow\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_network_idle_times_out_on_interval_fetch_page() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let error = browser
        .wait_for_page_network_idle(&mut page, Duration::from_millis(700))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("timed out waiting for networkidle")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_networkidle_timeout_restores_entry_for_close() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let error = browser
        .wait_for_page_network_idle(&mut page, Duration::from_millis(700))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("timed out waiting for networkidle")
    );
    assert!(
        renderer_owner.record(page_id).is_some(),
        "networkidle timeout should restore the owner-local page entry"
    );

    page.close_async().await?;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after networkidle timeout should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_networkidle_cancellation_restores_entry_for_close() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let canceled = tokio::time::timeout(
        Duration::from_millis(150),
        browser.wait_for_page_network_idle(&mut page, Duration::from_secs(5)),
    )
    .await;
    assert!(
        canceled.is_err(),
        "networkidle wait should be cancelled by the test timeout"
    );
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after cancelled networkidle wait should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_networkidle_cancellation_allows_detached_drop_cleanup() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let canceled = tokio::time::timeout(
        Duration::from_millis(150),
        browser.wait_for_page_network_idle(&mut page, Duration::from_secs(5)),
    )
    .await;
    assert!(
        canceled.is_err(),
        "networkidle wait should be cancelled by the test timeout"
    );

    drop(page);
    wait_for_renderer_owner_page_removed(
        &renderer_owner,
        page_id,
        "dropping page after cancelled networkidle wait should eventually remove the owner entry",
    )
    .await;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "detached drop cleanup after cancelled networkidle wait should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_domstable_timeout_restores_entry_for_close() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-dom-mutation"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let error = browser
        .wait_for_page_dom_stable(&mut page, Duration::from_millis(700))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("timed out waiting for domstable")
    );
    assert!(
        renderer_owner.record(page_id).is_some(),
        "domstable timeout should restore the owner-local page entry"
    );

    page.close_async().await?;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after domstable timeout should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_domstable_cancellation_restores_entry_for_close() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-dom-mutation"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let canceled = tokio::time::timeout(
        Duration::from_millis(150),
        browser.wait_for_page_dom_stable(&mut page, Duration::from_secs(5)),
    )
    .await;
    assert!(
        canceled.is_err(),
        "domstable wait should be cancelled by the test timeout"
    );
    tokio::time::timeout(Duration::from_secs(3), page.close_async()).await??;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after cancelled domstable wait should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_domstable_wait_parks_entry_while_pending() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-dom-mutation"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    let page_id = page.renderer_page_id();

    let mut wait_future =
        Box::pin(browser.wait_for_page_dom_stable(&mut page, Duration::from_secs(5)));
    tokio::select! {
        result = &mut wait_future => panic!("domstable unexpectedly completed before the interval page became stable: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(150)) => {}
    }

    wait_for_renderer_owner_state(
        &renderer_owner,
        |renderer_owner| renderer_owner.record(page_id).is_some(),
        "pending domstable wait should park the page entry back in the owner store",
    )
    .await;

    drop(wait_future);
    tokio::time::timeout(Duration::from_secs(1), page.close_async()).await??;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after cancelled parked domstable wait should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_network_idle_resets_quiet_window_when_new_fetch_starts() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-staggered-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    browser
        .wait_for_page_network_idle(&mut page, Duration::from_secs(2))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-first=\"settled\"")
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-second=\"settled-second\"")
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("id=\"late-second\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_registers_page_in_renderer_registry_until_page_is_dropped() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    assert_eq!(renderer_owner.len(), 0);

    let page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let record = renderer_owner
        .record(page_id)
        .expect("page should be registered");
    assert_eq!(record.requested_url, server.url("/static").parse()?);
    assert_eq!(record.final_url, page.final_url().clone());
    assert_eq!(record.status, 200);
    assert_eq!(renderer_owner.len(), 1);

    drop(page);
    wait_for_renderer_owner_page_removed(
        &renderer_owner,
        page_id,
        "dropping page should eventually remove the renderer owner entry",
    )
    .await;
    assert!(renderer_owner.record(page_id).is_none());
    assert_eq!(renderer_owner.len(), 0);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_registers_page_in_renderer_registry_until_page_is_closed_async() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    assert_eq!(renderer_owner.len(), 0);

    let page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    assert!(
        renderer_owner.record(page_id).is_some(),
        "page should be registered before explicit close"
    );
    assert_eq!(renderer_owner.len(), 1);

    page.close_async().await?;

    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit async close should remove the owner entry immediately"
    );
    assert_eq!(renderer_owner.len(), 0);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn location_navigation_keeps_same_renderer_page_id() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let page = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;
    let page_id = page.renderer_page_id();
    let record = renderer_owner
        .record(page_id)
        .expect("page should remain registered");

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=assign"));
    assert_eq!(record.final_url, page.final_url().clone());
    assert_eq!(record.requested_url, page.requested_url().clone());

    drop(page);
    wait_for_renderer_owner_page_removed(
        &renderer_owner,
        page_id,
        "dropping navigated page should eventually remove the renderer owner entry",
    )
    .await;
    assert!(renderer_owner.record(page_id).is_none());

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn location_navigation_refreshes_owner_vm_incarnation_for_dispatch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(
        page.evaluate_runtime_expression_async("location.pathname")
            .await?,
        serde_json::json!({"type": "string", "value": "/location-nav/target"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_create_page_command_produces_page() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let request = Request::get(&server.url("/static"))?;
    let requested_url = request.url.clone();
    let response = browser.resource_request_client().fetch(request).await?;
    let (response_head, response_body) = response.into_text_parts();
    let create_page_request = renderer_owner.build_create_html_page_request(
        requested_url.clone(),
        None,
        false,
        0,
        response_head.status,
        response_head.headers,
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        response_head.final_url,
        response_body,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    assert_eq!(
        renderer_owner.len(),
        1,
        "owner create-page should register the page before the reply is consumed"
    );
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    assert_eq!(page.requested_url(), &requested_url);
    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().path(), "/static");
    let creation_artifacts = page
        .take_page_creation_artifacts()
        .expect("directly materialized page should retain creation artifacts");
    assert_eq!(
        creation_artifacts.active_document.page_id.as_u64(),
        page.page_id()
    );
    assert!(!creation_artifacts.initial_lifecycle_events.is_empty());
    assert!(page.take_page_creation_artifacts().is_none());

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_common_page_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let request = Request::get(&server.url("/static"))?;
    let requested_url = request.url.clone();
    let response = browser.resource_request_client().fetch(request).await?;
    let (response_head, response_body) = response.into_text_parts();
    let create_page_request = renderer_owner.build_create_html_page_request(
        requested_url,
        None,
        false,
        0,
        response_head.status,
        response_head.headers,
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        response_head.final_url,
        response_body,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    assert_eq!(
        page.evaluate_runtime_expression_async("1 + 2").await?,
        serde_json::json!({"type": "number", "value": 3, "description": "3"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn page_runtime_evaluation_can_return_json_compatible_objects_by_value() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;

    let by_value = page
        .evaluate_runtime_expression_by_value_async("({ title: 'Moli', count: 2 })")
        .await?;
    assert_eq!(
        by_value["value"],
        serde_json::json!({ "title": "Moli", "count": 2 })
    );

    let by_reference = page
        .evaluate_runtime_expression_async("({ title: 'Moli' })")
        .await?;
    assert!(by_reference.get("objectId").is_some());
    assert!(by_reference.get("value").is_none());

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn async_page_command_snapshot_follow_can_adopt_pending_location_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/static")).await?;
    let _ = page
        .evaluate_runtime_expression_with_await_async(
            "location.assign('/location-nav/target?from=async-eval')",
            false,
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=async-eval"));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("location-target=async-eval")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn async_page_command_snapshot_follows_chained_location_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/static")).await?;
    let _ = page
        .evaluate_runtime_expression_with_await_async(
            "location.assign('/location-nav/chain-source')",
            false,
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=chain-mid"));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("location-target=chain-mid")
    );
    assert!(!page.serialize_html_async().await?.contains("chain-source"));
    assert!(
        !page
            .serialize_html_async()
            .await?
            .contains("<main id=\"mid\">chain-mid</main>")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn async_page_command_snapshot_rejects_chained_location_navigation_loop() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let error = match page
        .evaluate_runtime_expression_with_await_async(
            "location.assign('/location-nav/loop-a')",
            false,
        )
        .await
    {
        Ok(_) => panic!("location navigation loop should fail"),
        Err(error) => error,
    };
    let error_message = format!("{error:#}");
    assert!(
        error_message.contains("too many chained location navigations"),
        "unexpected error: {error:#}"
    );
    assert!(
        renderer_owner.record(page_id).is_some(),
        "failed navigation follow should restore the owner-local page entry"
    );

    page.close_async().await?;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after failed navigation follow should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_networkidle_rejects_chained_location_navigation_loop_and_restores_entry()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let _ = page
        .evaluate_runtime_expression_with_await_async(
            "setTimeout(() => location.assign('/location-nav/loop-a'), 0)",
            false,
        )
        .await?;
    let error = match browser
        .wait_for_page_network_idle(&mut page, Duration::from_secs(5))
        .await
    {
        Ok(_) => panic!("location navigation loop should fail during live networkidle wait"),
        Err(error) => error,
    };
    let error_message = format!("{error:#}");
    assert!(
        error_message.contains("too many chained location navigations"),
        "unexpected error: {error:#}"
    );
    assert!(
        renderer_owner.record(page_id).is_some(),
        "failed live networkidle navigation follow should restore the owner-local page entry"
    );

    page.close_async().await?;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after failed live networkidle follow should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_networkidle_follows_chained_location_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/static")).await?;
    let _ = page
        .evaluate_runtime_expression_with_await_async(
            "setTimeout(() => location.assign('/location-nav/chain-source'), 0)",
            false,
        )
        .await?;
    browser
        .wait_for_page_network_idle(&mut page, Duration::from_secs(5))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=chain-mid"));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("location-target=chain-mid")
    );
    assert!(!page.serialize_html_async().await?.contains("chain-source"));
    assert!(
        !page
            .serialize_html_async()
            .await?
            .contains("<main id=\"mid\">chain-mid</main>")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_domstable_rejects_chained_location_navigation_loop_and_restores_entry()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let _ = page
        .evaluate_runtime_expression_with_await_async(
            "setTimeout(() => location.assign('/location-nav/loop-a'), 0)",
            false,
        )
        .await?;
    let error = match browser
        .wait_for_page_dom_stable(&mut page, Duration::from_secs(5))
        .await
    {
        Ok(_) => panic!("location navigation loop should fail during live domstable wait"),
        Err(error) => error,
    };
    let error_message = format!("{error:#}");
    assert!(
        error_message.contains("too many chained location navigations"),
        "unexpected error: {error:#}"
    );
    assert!(
        renderer_owner.record(page_id).is_some(),
        "failed live domstable navigation follow should restore the owner-local page entry"
    );

    page.close_async().await?;
    assert!(
        renderer_owner.record(page_id).is_none(),
        "explicit close after failed live domstable follow should remove the page entry"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_domstable_follows_chained_location_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/static")).await?;
    let _ = page
        .evaluate_runtime_expression_with_await_async(
            "setTimeout(() => location.assign('/location-nav/chain-source'), 0)",
            false,
        )
        .await?;
    browser
        .wait_for_page_dom_stable(&mut page, Duration::from_secs(5))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=chain-mid"));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("location-target=chain-mid")
    );
    assert!(!page.serialize_html_async().await?.contains("chain-source"));
    assert!(
        !page
            .serialize_html_async()
            .await?
            .contains("<main id=\"mid\">chain-mid</main>")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_tracks_page_command_epoch_progress() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();

    assert_eq!(
        renderer_owner.command_epoch(page_id),
        Some(0),
        "freshly adopted pages should start with command epoch 0"
    );

    let _ = page.evaluate_runtime_expression_async("1 + 2").await?;
    assert_eq!(
        renderer_owner.command_epoch(page_id),
        Some(1),
        "first page command should advance owner-side command epoch"
    );

    let _ = page.runtime_enable_events_async().await?;
    assert_eq!(
        renderer_owner.command_epoch(page_id),
        Some(2),
        "subsequent page commands should keep advancing owner-side command epoch"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn page_html_serializes_on_demand_from_renderer_owner() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let initial_html = page.serialize_html_async().await?;
    assert!(initial_html.contains("<html"));

    let _ = page
        .evaluate_runtime_expression_async(
            "document.body.innerHTML = '<main id=\"owner-slot-state\">updated</main>'; true",
        )
        .await?;

    let updated_html = page.serialize_html_async().await?;
    assert!(updated_html.contains("owner-slot-state"));
    assert!(updated_html.contains("updated"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_reuses_shared_local_host_for_pages_on_same_owner() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page_a = browser.fetch(&server.url("/static")).await?;
    let page_b = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;

    let handle_a = page_a.handle_for_testing();
    let handle_b = page_b.handle_for_testing();

    assert!(
        handle_a.shares_local_host(&handle_b),
        "pages created by the same renderer owner on the same thread should share one local host"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_does_not_share_local_host_across_owners() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser_a = Browser::new(AppConfig::default())?;
    let browser_b = Browser::new(AppConfig::default())?;

    let page_a = browser_a.fetch(&server.url("/static")).await?;
    let page_b = browser_b
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;

    let handle_a = page_a.handle_for_testing();
    let handle_b = page_b.handle_for_testing();

    assert!(
        !handle_a.shares_local_host(&handle_b),
        "pages created by different renderer owners should not share one local host"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_can_create_isolated_world_on_older_page_after_second_page_exists()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page_a = browser.fetch(&server.url("/static")).await?;
    let _page_b = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;

    let world_id = page_a
        .create_isolated_world_async("reactivated-world", false)
        .await?;
    assert!(
        page_a
            .has_isolated_execution_context_id_async(world_id)
            .await?
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_can_evaluate_on_older_page_after_second_page_exists() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page_a = browser.fetch(&server.url("/static")).await?;
    let _page_b = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;

    assert_eq!(
        page_a.evaluate_runtime_expression_async("1 + 1").await?,
        serde_json::json!({"type": "number", "value": 2, "description": "2"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_page_only_removes_its_entry_from_shared_local_host() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let page_a = browser.fetch(&server.url("/static")).await?;
    let page_b = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;

    let page_a_id = page_a.renderer_page_id();
    let page_b_id = page_b.renderer_page_id();

    assert_eq!(renderer_owner.len(), 2);

    drop(page_a);

    wait_for_renderer_owner_state(
        &renderer_owner,
        |renderer_owner| {
            renderer_owner.record(page_a_id).is_none()
                && renderer_owner.record(page_b_id).is_some()
                && renderer_owner.len() == 1
        },
        "dropping one shared-host page should eventually remove only that page entry",
    )
    .await;
    assert!(
        renderer_owner.record(page_a_id).is_none(),
        "dropping one page should remove its owner entry"
    );
    assert!(
        renderer_owner.record(page_b_id).is_some(),
        "dropping one shared-host page should keep the other page registered"
    );
    assert_eq!(renderer_owner.len(), 1);

    drop(page_b);
    wait_for_renderer_owner_state(
        &renderer_owner,
        |renderer_owner| renderer_owner.is_empty(),
        "dropping the last shared-host page should eventually empty the renderer owner table",
    )
    .await;
    assert_eq!(renderer_owner.len(), 0);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_recreates_local_host_after_last_page_drops() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page_a = browser.fetch(&server.url("/static")).await?;
    let host_instance_a = page_a
        .handle_for_testing()
        .host_instance_key_async()
        .await?;
    drop(page_a);
    wait_for_renderer_owner_state(
        &browser.js_runtime.renderer_owner_handle(),
        |renderer_owner| renderer_owner.is_empty(),
        "dropping the last page should eventually release the current renderer owner host",
    )
    .await;

    let page_b = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;
    let host_instance_b = page_b
        .handle_for_testing()
        .host_instance_key_async()
        .await?;

    assert_ne!(
        host_instance_a, host_instance_b,
        "dropping the last page should let the local runtime recreate a fresh host instance"
    );

    drop(page_b);
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn aborting_async_page_command_restores_local_host_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let loader = browser.resource_request_client();
    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    {
        let future = page.wait_for_script_truthy(&loader, "false", Duration::from_millis(100));
        tokio::pin!(future);
        tokio::select! {
            result = &mut future => panic!("wait_for_script_truthy unexpectedly completed early: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
        }
    }

    assert!(
        renderer_owner.record(page_id).is_some(),
        "dropping an in-flight async page command should restore the page entry into the shared local host"
    );
    let value = page.evaluate_runtime_expression_async("1 + 1").await?;
    assert_eq!(value["value"], 2);

    drop(page);
    server.shutdown().await;
    Ok(())
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn panicking_async_page_command_restores_local_host_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let error = page
        .panic_renderer_command_for_testing()
        .await
        .expect_err("test panic command should surface as a page command error");
    assert!(
        error
            .to_string()
            .contains("panicked before restoring its page entry"),
        "unexpected panic command error: {error:#}"
    );

    assert!(
        renderer_owner.record(page_id).is_some(),
        "a panicking spawned local task should restore the page entry into the shared local host"
    );
    let value = page.evaluate_runtime_expression_async("1 + 1").await?;
    assert_eq!(value["value"], 2);

    drop(page);
    server.shutdown().await;
    Ok(())
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn panicking_wait_for_selector_restores_local_host_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let loader = browser.resource_request_client();
    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let error = page
        .panic_wait_for_selector_for_testing(&loader)
        .await
        .expect_err("test selector wait panic should surface as a page command error");
    assert!(
        error
            .to_string()
            .contains("panicked before restoring its page entry"),
        "unexpected selector wait panic error: {error:#}"
    );

    assert!(
        renderer_owner.record(page_id).is_some(),
        "a panicking wait-for-selector local task should restore the page entry into the shared local host"
    );
    let value = page.evaluate_runtime_expression_async("1 + 1").await?;
    assert_eq!(value["value"], 2);

    drop(page);
    server.shutdown().await;
    Ok(())
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn panicking_wait_for_script_truthy_restores_local_host_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let loader = browser.resource_request_client();
    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let error = page
        .panic_wait_for_script_truthy_for_testing(&loader)
        .await
        .expect_err("test script wait panic should surface as a page command error");
    assert!(
        error
            .to_string()
            .contains("panicked before restoring its page entry"),
        "unexpected script wait panic error: {error:#}"
    );

    assert!(
        renderer_owner.record(page_id).is_some(),
        "a panicking wait-for-script-truthy local task should restore the page entry into the shared local host"
    );
    let value = page.evaluate_runtime_expression_async("1 + 1").await?;
    assert_eq!(value["value"], 2);

    drop(page);
    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn renderer_page_testing_handle_page_state_refresh_works_on_current_thread_runtime()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let page = browser.fetch(&server.url("/static")).await?;
    let handle = page.handle_for_testing();

    let page_state = handle.current_page_state_async().await?;
    let view = handle.renderer_page_view_async().await?;
    let _owner_slot = handle.owner_slot_async().await?;
    let _host_instance_key = handle.host_instance_key_async().await?;

    assert_eq!(page_state.requested_url, page.requested_url().clone());
    assert_eq!(page_state.status, page.status());
    assert_eq!(view.page_id.as_u64(), page.page_id());
    assert_eq!(view.page_state.requested_url, page.requested_url().clone());

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_removed_page_releases_last_page_state() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();

    let _ = page
        .evaluate_runtime_expression_async(
            "document.body.innerHTML = '<main id=\"removed-owner-slot-state\">removed</main>'; true",
        )
        .await?;

    let page_state = page.handle_for_testing().current_page_state_async().await?;
    let page_state_weak = Arc::downgrade(&page_state);
    drop(page_state);

    page.close_async().await?;

    assert!(renderer_owner.record(page_id).is_none());
    assert!(
        page_state_weak.upgrade().is_none(),
        "removed-page tombstones must not retain the full page state"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_clears_in_flight_command_after_successful_dispatch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();

    assert_eq!(renderer_owner.in_flight_command_epoch(page_id), None);

    let _ = page.evaluate_runtime_expression_async("1 + 2").await?;
    assert_eq!(
        renderer_owner.in_flight_command_epoch(page_id),
        None,
        "successful page commands should still clear owner-side in-flight bookkeeping"
    );
    assert_eq!(
        renderer_owner.command_epoch(page_id),
        Some(1),
        "successful page commands should advance owner-side command epoch"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_rejects_commands_for_removed_page() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();

    renderer_owner.remove_page_for_testing(page_id);
    let error = page
        .evaluate_runtime_expression_async("1 + 2")
        .await
        .expect_err("removed pages should not keep dispatching through the owner surface");
    assert!(
        error
            .to_string()
            .contains("renderer owner no longer tracks active page"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn page_renderer_page_record_returns_error_for_removed_owner_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let page = browser.fetch(&server.url("/static")).await?;
    let handle = page.handle_for_testing();
    let page_id = page.renderer_page_id();

    renderer_owner.remove_page_for_testing(page_id);

    let error = handle
        .current_page_state_async()
        .await
        .expect_err("removed owner entry should fail page-view refresh");
    assert!(
        error
            .to_string()
            .contains("failed to refresh renderer owner page view"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_does_not_revive_removed_page_from_plain_view_refresh() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let page = browser.fetch(&server.url("/static")).await?;
    let handle = page.handle_for_testing();
    let page_id = page.renderer_page_id();
    let page_view = handle
        .renderer_page_view_async()
        .await
        .expect("renderer owner should provide current page view for testing");

    renderer_owner.remove_page_for_testing(page_id);
    let error = renderer_owner
        .refresh_page_view_for_testing(page_view)
        .expect_err("removed pages should not be reactivated by plain view refresh");
    assert!(
        error
            .to_string()
            .contains("renderer owner no longer tracks active page"),
        "unexpected error: {error:#}"
    );
    assert!(
        renderer_owner.record(page_id).is_none(),
        "removed page metadata should remain inactive after failed refresh"
    );
    assert_eq!(renderer_owner.len(), 0);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_rejects_stale_page_view_refresh_generation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let mut page = browser.fetch(&server.url("/static")).await?;
    let handle = page.handle_for_testing();
    let stale_view = handle
        .renderer_page_view_async()
        .await
        .expect("renderer owner should provide current page view for testing");

    let _ = page.evaluate_runtime_expression_async("1 + 2").await?;

    let error = renderer_owner
        .refresh_page_view_for_testing(stale_view)
        .expect_err("owner should reject stale page-view refresh generations");
    assert!(
        error
            .to_string()
            .contains("renderer owner received stale page view refresh"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_remove_unknown_page_keeps_never_tracked_state() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let unknown_page_id = crate::renderer::PageId::new_for_testing(999_999);

    renderer_owner.remove_page_for_testing(unknown_page_id);

    let error = renderer_owner
        .refresh_page_view_for_testing(crate::renderer::RendererPageView {
            page_id: unknown_page_id,
            vm_creation_id: 0,
            view_generation: 0,
            page_state: std::sync::Arc::new(crate::renderer::RendererPageState {
                requested_url: Url::parse("https://example.com/requested")?,
                navigation_initiator_url: None,
                navigation_redirected: false,
                navigation_redirect_count: 0,
                navigation_redirect_chain: Vec::new(),
                final_url: Url::parse("https://example.com/final")?,
                document_title: String::new(),
                status: 200,
                headers: Vec::new(),
                idle_override: None,
                service_worker_client_id: 0,
                dedicated_worker_running_worker_isolate_count: 0,
                performance_metric_snapshot: Default::default(),
                script_execution: Arc::new(crate::page::ScriptExecutionReport::default()),
            }),
        })
        .expect_err("removing an unknown page should not create a removed tombstone");
    assert!(
        error
            .to_string()
            .contains("renderer owner has never tracked page"),
        "unexpected error: {error:#}"
    );
    assert!(renderer_owner.record(unknown_page_id).is_none());
    assert_eq!(renderer_owner.len(), 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn page_drop_removes_owner_entry_via_bound_slot() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let page = browser.fetch(&server.url("/static")).await?;
    let page_id = page.renderer_page_id();
    assert_eq!(renderer_owner.len(), 1);
    assert!(
        renderer_owner.record(page_id).is_some(),
        "fetched page should register an active owner entry before drop"
    );

    drop(page);

    wait_for_renderer_owner_state(
        &renderer_owner,
        |renderer_owner| renderer_owner.is_empty() && renderer_owner.record(page_id).is_none(),
        "dropping page should eventually remove its bound-slot owner entry",
    )
    .await;
    assert_eq!(
        renderer_owner.len(),
        0,
        "dropping page should remove its owner entry through the bound slot"
    );
    assert!(
        renderer_owner.record(page_id).is_none(),
        "dropped page should no longer expose an active owner record"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_rejects_slot_owned_by_another_renderer() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser_a = Browser::new(AppConfig::default())?;
    let browser_b = Browser::new(AppConfig::default())?;
    let renderer_owner_b = browser_b.js_runtime.renderer_owner_handle();

    let page = browser_a.fetch(&server.url("/static")).await?;
    let handle = page.handle_for_testing();
    let foreign_slot = handle.owner_slot_async().await?;
    let error = renderer_owner_b
        .refresh_page_view_on_slot_for_testing(
            &foreign_slot,
            handle
                .renderer_page_view_async()
                .await
                .expect("renderer owner should provide current page view for testing"),
        )
        .expect_err("owner should reject slot belonging to another renderer");
    assert!(
        error
            .to_string()
            .contains("renderer owner does not own slot"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_runtime_protocol_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let request = Request::get(&server.url("/static"))?;
    let response = browser.resource_request_client().fetch(request).await?;
    let (response_head, response_body) = response.into_text_parts();
    let create_page_request = renderer_owner.build_create_html_page_request(
        server.url("/static").parse()?,
        None,
        false,
        0,
        response_head.status,
        response_head.headers,
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        response_head.final_url,
        response_body,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let runtime_messages = page.runtime_enable_events_async().await?;
    assert!(
        !runtime_messages.is_empty(),
        "runtime enable should emit at least one event-like message"
    );

    let world_id = page
        .create_isolated_world_async("test-world", false)
        .await?;
    assert!(
        page.has_isolated_execution_context_id_async(world_id)
            .await?
    );

    page.add_runtime_binding_async("testBinding", None, None)
        .await?;
    let binding_type = page
        .evaluate_runtime_expression_async("typeof globalThis.testBinding")
        .await?;
    assert_eq!(binding_type["value"], serde_json::json!("function"));
    page.remove_runtime_binding_async("testBinding").await?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_document_start_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let request = Request::get(&server.url("/static"))?;
    let response = browser.resource_request_client().fetch(request).await?;
    let (response_head, response_body) = response.into_text_parts();
    let create_page_request = renderer_owner.build_create_html_page_request(
        server.url("/static").parse()?,
        None,
        false,
        0,
        response_head.status,
        response_head.headers,
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        response_head.final_url,
        response_body,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    page.run_page_surface_override_script_async(
        "document.body.setAttribute('data-doc-start', 'ran');",
    )
    .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-doc-start=\"ran\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_document_start_script_keeps_bootstrap_bridge_safe_for_core_window_reads()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(
        AppConfig::default().with_document_start_script("window.__docStartBridgeOk = 'ok';"),
    )?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    assert_eq!(
        page.evaluate_runtime_expression_async("window.__docStartBridgeOk")
            .await?,
        serde_json::json!({"type": "string", "value": "ok"})
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("location.href")
            .await?,
        serde_json::json!({"type": "string", "value": server.url("/static")})
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("typeof navigator.userAgent")
            .await?,
        serde_json::json!({"type": "string", "value": "string"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_document_start_keeps_live_document_html_surface_stable() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_document_start_script(
        r#"
        const snap = () => JSON.stringify({
          ctor: document.constructor && document.constructor.name,
          tag: Object.prototype.toString.call(document),
          contentType: document.contentType
        });
        window.__docStartShape = snap();
        document.addEventListener("DOMContentLoaded", () => {
          window.__docDomContentLoadedShape = snap();
        });
        window.addEventListener("load", () => {
          window.__docLoadShape = snap();
        });
        "#,
    ))?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let expected = serde_json::json!({
        "type": "string",
        "value": r#"{"ctor":"HTMLDocument","tag":"[object HTMLDocument]","contentType":"text/html"}"#
    });
    assert_eq!(
        page.evaluate_runtime_expression_async("window.__docStartShape")
            .await?,
        expected
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("window.__docDomContentLoadedShape")
            .await?,
        expected
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("window.__docLoadShape")
            .await?,
        expected
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_streaming_chunked_html_parses_incrementally_across_utf8_boundaries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/streaming/chunked-html"))
        .await?;

    assert!(page.serialize_html_async().await?.contains("naive-你好"));
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-stream-script=\"seen\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn canvas_get_context_reuses_same_context_object_and_state() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;

    let value = page
        .evaluate_runtime_expression_async(
            "(() => { const canvas = document.createElement('canvas'); const first = canvas.getContext('2d'); first.fillStyle = 'red'; const second = canvas.getContext('2d'); return JSON.stringify([first === second, second.fillStyle, canvas.getContext('webgl') === null]); })()",
        )
        .await?;
    assert_eq!(
        value,
        serde_json::json!({"type": "string", "value": "[true,\"#ff0000\",true]"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_input_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: url::Url = server.url("/input-commands").parse()?;
    let html = r#"
        <!doctype html>
        <html>
          <body style="margin:0">
            <button id="mouse" style="position:absolute;left:0;top:0;width:80px;height:40px"
              onclick="document.body.dataset.clicked='yes'">mouse</button>
            <div id="touch" style="position:absolute;left:0;top:50px;width:80px;height:40px"
              ontouchstart="document.body.dataset.touched='yes'">touch</div>
            <input id="field" style="position:absolute;left:0;top:100px;width:120px;height:24px"
              onkeydown="document.body.dataset.key=event.key">
          </body>
        </html>
    "#
    .to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url,
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let _ = page
        .dispatch_mouse_event_at_point_async(10.0, 10.0, "mousedown", 0, None, 0.0, 0.0)
        .await?;
    let _ = page
        .dispatch_touch_event_at_point_async(10.0, 60.0, "touchstart", false)
        .await?;

    page.evaluate_runtime_expression_async("document.getElementById('field').focus(); 'focused';")
        .await?;
    assert!(page.insert_text_into_active_control_async("abc").await?);
    assert_eq!(
        page.evaluate_runtime_expression_async("document.getElementById('field').value")
            .await?,
        serde_json::json!({"type": "string", "value": "abc"})
    );

    assert!(
        page.dispatch_key_event_async("keydown", "Enter", "Enter", "", 0, false, false)
            .await?
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("document.body.dataset.key || ''")
            .await?,
        serde_json::json!({"type": "string", "value": "Enter"})
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_input_query_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::OnDemand))?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: url::Url = server.url("/input-query-commands").parse()?;
    let html = r#"
        <!doctype html>
        <html>
          <body style="margin:0">
            <input id="field" style="position:absolute;left:12px;top:8px;width:120px;height:24px" value="seed">
          </body>
        </html>
    "#
    .to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url,
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let input_backend_node_id = query_selector_node_from_live_document(&mut page, "#field")
        .await?
        .expect("input node should exist")
        .backend_node_id;
    let rect_pending = page.start_client_rect_for_backend_node_id(input_backend_node_id)?;
    let rect_completion = rect_pending.wait().await?;
    let rect = match page
        .finish_client_rect_for_backend_node_id(rect_completion)?
        .expect("input rect should be available")
    {
        DocumentNodeClientRectResolution::Found(rect) => rect,
        DocumentNodeClientRectResolution::FoundNonElement(_) => {
            panic!("input node should be an element")
        }
        DocumentNodeClientRectResolution::NotElement => {
            panic!("input node should be an element")
        }
    };
    assert_eq!(rect.left, 12.0);
    assert_eq!(rect.top, 8.0);
    assert_eq!(rect.width, 120.0);
    assert_eq!(rect.height, 24.0);

    let _ = page.runtime_enable_events_async().await?;
    let default_context_id = page
        .default_execution_context_id_async()
        .await?
        .expect("default execution context should exist");
    assert_eq!(
        page.evaluate_runtime_expression_in_execution_context_with_await_async(
            default_context_id,
            "1 + 4",
            false,
        )
        .await?,
        serde_json::json!({"type": "number", "value": 5, "description": "5"})
    );

    let isolated_world_id = page
        .create_isolated_world_async("query-world", false)
        .await?;
    assert!(
        page.has_isolated_execution_context_id_async(isolated_world_id)
            .await?
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_reports_real_client_rect_for_positioned_node() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::OnDemand))?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: url::Url = server.url("/input-query-geometry").parse()?;
    let html = r#"
        <!doctype html>
        <html>
          <body style="margin:0">
            <div id="target"
              style="position:absolute;left:12px;top:8px;width:120px;height:24px;margin:0;padding:0;border:none;background:#c00"></div>
          </body>
        </html>
    "#
    .to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url,
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let target_backend_node_id = query_selector_node_from_live_document(&mut page, "#target")
        .await?
        .expect("target div should exist")
        .backend_node_id;
    let rect_pending = page.start_client_rect_for_backend_node_id(target_backend_node_id)?;
    let rect_completion = rect_pending.wait().await?;
    let rect = match page
        .finish_client_rect_for_backend_node_id(rect_completion)?
        .expect("target rect should be available")
    {
        DocumentNodeClientRectResolution::Found(rect) => rect,
        DocumentNodeClientRectResolution::FoundNonElement(_) => {
            panic!("target node should be an element")
        }
        DocumentNodeClientRectResolution::NotElement => {
            panic!("target node should be an element")
        }
    };

    let assert_close = |actual: f64, expected: f64, label: &str| {
        let delta = (actual - expected).abs();
        assert!(
            delta <= 0.01,
            "{label} expected {expected}, got {actual} (delta {delta})"
        );
    };

    assert_close(rect.left, 12.0, "left");
    assert_close(rect.top, 8.0, "top");
    assert_close(rect.right, 132.0, "right");
    assert_close(rect.bottom, 32.0, "bottom");
    assert_close(rect.width, 120.0, "width");
    assert_close(rect.height, 24.0, "height");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_resolves_backend_node_runtime_object() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: url::Url = server.url("/object-resolution-commands").parse()?;
    let html = r#"
        <!doctype html>
        <html>
          <body>
            <div id="target">ok</div>
          </body>
        </html>
    "#
    .to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url,
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let backend_node_id = query_selector_node_from_live_document(&mut page, "#target")
        .await?
        .context("target div should exist")?
        .backend_node_id;
    let _ = page.runtime_enable_events_async().await?;
    let execution_context_id = page
        .default_execution_context_id_async()
        .await?
        .context("default execution context should exist")?;
    let resolved_object = resolve_runtime_object_for_backend_node_id(
        &mut page,
        backend_node_id,
        Some(execution_context_id),
        Some("roundtrip"),
    )
    .await?;
    let resolved_object = match resolved_object {
        crate::page::DocumentNodeRuntimeObjectResolution::Found(value) => {
            value.into_protocol_value()
        }
        crate::page::DocumentNodeRuntimeObjectResolution::MissingContext => {
            return Err(anyhow!(
                "document node resolution unexpectedly lost its context"
            ));
        }
        crate::page::DocumentNodeRuntimeObjectResolution::MissingNode => {
            return Err(anyhow!(
                "document node should resolve back to a runtime object"
            ));
        }
    };
    assert_eq!(resolved_object["type"], serde_json::json!("object"));
    resolved_object["objectId"]
        .as_str()
        .context("resolved runtime object should carry an objectId")?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_object_resolve_uses_live_backend_node_for_stale_snapshot_path() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: url::Url = server.url("/object-resolution-live-node-id").parse()?;
    let html = r#"
        <!doctype html>
        <html>
          <body>
            <div id="target">target</div>
          </body>
        </html>
    "#
    .to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url,
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let backend_node_id = query_selector_node_from_live_document(&mut page, "#target")
        .await?
        .context("target div should exist")?
        .backend_node_id;
    let _ = page.runtime_enable_events_async().await?;
    let execution_context_id = page
        .default_execution_context_id_async()
        .await?
        .context("default execution context should exist")?;

    let mutation = serde_json::json!({
        "id": 91,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "const target = document.getElementById('target'); const inserted = document.createElement('div'); inserted.id = 'inserted'; target.before(inserted); 'mutated';",
            "returnByValue": true
        }
    });
    let mutation_pending =
        page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
    let mutation_completion = mutation_pending.wait().await?;

    let resolved_object = resolve_runtime_object_for_backend_node_id(
        &mut page,
        backend_node_id,
        Some(execution_context_id),
        Some("stale-path-probe"),
    )
    .await?;
    let resolved_object = match resolved_object {
        crate::page::DocumentNodeRuntimeObjectResolution::Found(value) => {
            value.into_protocol_value()
        }
        crate::page::DocumentNodeRuntimeObjectResolution::MissingContext => {
            return Err(anyhow!(
                "document node resolution unexpectedly lost its context"
            ));
        }
        crate::page::DocumentNodeRuntimeObjectResolution::MissingNode => {
            return Err(anyhow!(
                "document node should resolve back to a runtime object"
            ));
        }
    };
    let object_id = resolved_object["objectId"]
        .as_str()
        .context("resolved runtime object should carry an objectId")?;

    let _mutation_messages = page.finish_runtime_protocol_message(mutation_completion)?;
    let call = serde_json::json!({
        "id": 92,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function () { return this.id; }",
            "returnByValue": true
        }
    });
    let messages = page
        .dispatch_runtime_protocol_message_async(&serde_json::to_string(&call)?)
        .await?;
    let response =
        runtime_protocol_response_by_id(&messages, 92).context("callFunctionOn should respond")?;
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("target"),
        "backend-node runtime object resolution must follow the live node, not the stale document path"
    );

    server.shutdown().await;
    Ok(())
}

fn runtime_protocol_response_by_id(
    messages: &[RendererRuntimeInspectorMessage],
    response_id: i64,
) -> Option<&serde_json::Value> {
    messages.iter().find_map(|message| match message {
        RendererRuntimeInspectorMessage::Protocol(value)
            if value.get("id").and_then(serde_json::Value::as_i64) == Some(response_id) =>
        {
            Some(value.value())
        }
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_inline_stylesheet_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let renderer_owner = browser.js_runtime.renderer_owner_handle();
    let page_url: url::Url = server.url("/stylesheet-commands").parse()?;
    let html = r#"
        <!doctype html>
        <html>
          <head>
            <style id="sheet">#target { display: block; }</style>
          </head>
          <body>
            <div id="target" style="width:120px;height:24px">styled text</div>
          </body>
        </html>
    "#
    .to_owned();
    let create_page_request = renderer_owner.build_create_html_page_request(
        page_url.clone(),
        None,
        false,
        0,
        200,
        vec![],
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        page_url,
        html,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    let style_sheet_inventory_pending = page.start_style_sheet_inventory_for_document()?;
    let style_sheet_inventory_completion = style_sheet_inventory_pending.wait().await?;
    let style_sheet_id = page
        .finish_style_sheet_inventory_for_document(style_sheet_inventory_completion)?
        .added
        .into_iter()
        .find(|header| header.is_inline)
        .map(|header| header.style_sheet_id)
        .context("inline stylesheet header should be available")?;
    let target_backend_node_id = query_selector_node_from_live_document(&mut page, "#target")
        .await?
        .expect("target div should exist")
        .backend_node_id;

    let computed_before_pending =
        page.start_computed_style_properties_for_backend_node_id(target_backend_node_id)?;
    let computed_before_completion = computed_before_pending.wait().await?;
    let computed_before = page
        .finish_computed_style_properties(computed_before_completion)?
        .unwrap_or_default();
    let display_before = computed_before
        .iter()
        .find(|(name, _)| name == "display")
        .map(|(_, value)| value.clone())
        .expect("computed display should exist before stylesheet edit");
    let width_before = computed_before
        .iter()
        .find(|(name, _)| name == "width")
        .map(|(_, value)| value.clone())
        .expect("computed width should exist before stylesheet edit");
    let height_before = computed_before
        .iter()
        .find(|(name, _)| name == "height")
        .map(|(_, value)| value.clone())
        .expect("computed height should exist before stylesheet edit");
    assert_eq!(display_before, "block");
    assert_eq!(width_before, "120px");
    assert_eq!(height_before, "24px");

    let payload_before_pending =
        page.start_style_sheet_payload_for_style_sheet_id(&style_sheet_id)?;
    let payload_before_completion = payload_before_pending.wait().await?;
    assert_eq!(
        page.finish_style_sheet_payload(payload_before_completion)?
            .expect("inline stylesheet payload should be available")
            .text,
        "#target { display: block; }"
    );

    let edit_pending = page.start_set_inline_style_sheet_text_for_style_sheet_id(
        &style_sheet_id,
        "#target { display: none; }",
    )?;
    let edit_completion = edit_pending.wait().await?;
    assert!(page.finish_set_inline_style_sheet_text(edit_completion)?);

    let payload_after_pending =
        page.start_style_sheet_payload_for_style_sheet_id(&style_sheet_id)?;
    let payload_after_completion = payload_after_pending.wait().await?;
    assert_eq!(
        page.finish_style_sheet_payload(payload_after_completion)?
            .expect("inline stylesheet payload should still be available")
            .text,
        "#target { display: none; }"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_owner_created_page_runs_subresource_interception_commands() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let (output_tx, mut output_rx) = crate::renderer_output_transport_channel();
    browser
        .js_runtime
        .set_renderer_output_transport_sender(output_tx);
    let renderer_owner = browser.js_runtime.renderer_owner_handle();

    let request = Request::get(&server.url("/static"))?;
    let requested_url = request.url.clone();
    let response = browser.resource_request_client().fetch(request).await?;
    let (response_head, response_body) = response.into_text_parts();
    let create_page_request = renderer_owner.build_create_html_page_request(
        requested_url,
        None,
        false,
        0,
        response_head.status,
        response_head.headers,
        &browser.resource_request_client(),
        moli_renderer_v8::RendererWebStorageHandles::new(
            browser.partition.web_storage_store(),
            browser.partition.session_storage_store(),
        ),
        response_head.final_url,
        response_body,
        vec![],
        vec![],
        vec![],
        vec![],
        false,
        Vec::new(),
        false,
        None,
        super::PageVmInitStage::Load,
    );

    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(create_page_request))
        .await?;
    let mut page = materialize_page_created_reply(&renderer_owner, reply)?;

    page.set_fetch_subresource_interception_async(true, Some(SubresourceResourceType::Fetch))
        .await?;
    page.evaluate_runtime_expression_async(
        "fetch('/wait-until-data').then(r => r.text()).then(text => { document.body.setAttribute('data-state', text); }); 'scheduled';",
    )
    .await?;

    let pending = recv_subresource_fetch_pause_for_page(&mut output_rx, &page).await?;
    let outcome = page
        .continue_pending_subresource_fetch_async(
            pending.internal_id,
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .await?;
    assert!(matches!(
        outcome,
        PendingSubresourceContinueOutcome::Started
    ));

    page.wait_for_network_idle(&browser.resource_request_client(), Duration::from_secs(2))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("data-state=\"settled\"")
    );

    server.shutdown().await;
    Ok(())
}

async fn spawn_main_resource_service_worker_server()
-> Result<(String, Arc<Mutex<Vec<String>>>, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind main-resource service worker test server")?;
    let addr = listener
        .local_addr()
        .context("main-resource service worker test server addr")?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_for_server = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(_) => break,
            };
            let requests = requests_for_server.clone();
            tokio::spawn(async move {
                let request = match read_http_request_head(&mut stream).await {
                    Ok(request) => request,
                    Err(_) => return,
                };
                let path = request_path(&request).unwrap_or_else(|| "/".to_owned());
                let preload_header_value = request
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("Service-Worker-Navigation-Preload")
                                .then(|| value.trim().to_owned())
                        })
                    })
                    .unwrap_or_else(|| "missing".to_owned());
                requests.lock().await.push(request.clone());
                if path == "/app/preload-cancel.html" && preload_header_value != "missing" {
                    let mut buf = [0_u8; 1024];
                    while matches!(stream.read(&mut buf).await, Ok(n) if n != 0) {}
                    return;
                }
                if path == "/app/preload-headers.html" && preload_header_value != "missing" {
                    let mut headers: std::collections::BTreeMap<String, Vec<String>> =
                        std::collections::BTreeMap::new();
                    for line in request.lines().skip(1) {
                        let Some((name, value)) = line.split_once(':') else {
                            continue;
                        };
                        headers
                            .entry(name.trim().to_ascii_uppercase())
                            .or_default()
                            .push(value.trim().to_owned());
                    }
                    let body = serde_json::to_string(&headers)
                        .expect("preload request headers should serialize");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                if path == "/app/preload-gzip.html" && preload_header_value != "missing" {
                    let body = [
                        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 243, 72, 205, 201, 201, 87, 8, 207, 47,
                        202, 73, 1, 0, 86, 177, 23, 74, 11, 0, 0, 0,
                    ];
                    let response_head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response_head.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                    return;
                }
                if path == "/app/preload-chunked.html" && preload_header_value != "missing" {
                    let response_head = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/html; charset=UTF-8\r\n",
                        "Transfer-Encoding: chunked\r\n",
                        "Connection: close\r\n",
                        "\r\n"
                    );
                    let _ = stream.write_all(response_head.as_bytes()).await;
                    for digit in 0..10 {
                        let chunk = format!("1\r\n{digit}\r\n");
                        let _ = stream.write_all(chunk.as_bytes()).await;
                    }
                    let _ = stream.write_all(b"0\r\n\r\n").await;
                    return;
                }
                if path == "/app/preload-cookie-lax.html" && preload_header_value != "missing" {
                    let response =
                        navigation_preload_cookie_response(&request, "preload_lax", "Lax");
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                if path == "/app/preload-cookie-strict.html" && preload_header_value != "missing" {
                    let response =
                        navigation_preload_cookie_response(&request, "preload_strict", "Strict");
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                if path == "/app/preload-empty-body.html" && preload_header_value != "missing" {
                    let response = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/html\r\n",
                        "Content-Length: 0\r\n",
                        "Connection: close\r\n",
                        "\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                if path == "/app/preload-broken-body-unused.html"
                    && preload_header_value != "missing"
                {
                    let response = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/html\r\n",
                        "Content-Length: 64\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "partial unused preload body"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                if path == "/app/preload-body-error.html" && preload_header_value != "missing" {
                    let response = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/html\r\n",
                        "Content-Length: 64\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "partial preload body"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                if path == "/app/preload-redirect-direct-body.html"
                    && preload_header_value != "missing"
                {
                    let body = "<body>BODY</body>";
                    let response = format!(
                        "HTTP/1.1 302 Found\r\nContent-Type: text/html\r\nCustom-Header: hello\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                let (status, content_type, body) = main_resource_service_worker_response(&path);
                let extra_headers = match path.as_str() {
                    "/app/preload.html" => {
                        format!("X-Network: preload\r\nX-Seen-Preload: {preload_header_value}\r\n")
                    }
                    "/app/preload-redirect.html" => {
                        format!(
                            "Location: /app/preload-final.html\r\nX-Seen-Preload: {preload_header_value}\r\n"
                        )
                    }
                    "/app/preload-redirect-follow.html" => {
                        "Location: /outside/preload-redirected.html\r\n".to_owned()
                    }
                    "/app/preload-redirect-to-scope.html" => {
                        "Location: /app/preload-redirect-to-scope-2.html\r\n".to_owned()
                    }
                    "/app/preload-redirect-to-scope-2.html" => {
                        "Location: /app/preload-redirect-to-scope-3.html\r\n".to_owned()
                    }
                    "/app/preload-redirect-to-scope-3.html" => {
                        "Location: /outside/preload-redirected.html\r\n".to_owned()
                    }
                    _ => String::new(),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Ok((
        format!("http://127.0.0.1:{}", addr.port()),
        requests,
        server,
    ))
}

async fn spawn_non_document_navigation_server() -> Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind no-Document navigation fixture")?;
    let addr = listener
        .local_addr()
        .context("read no-Document navigation fixture address")?;
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(_) => break,
            };
            tokio::spawn(async move {
                let request = match read_http_request_head(&mut stream).await {
                    Ok(request) => request,
                    Err(_) => return,
                };
                if request_path(&request).as_deref() == Some("/no-document") {
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    return;
                }
                if request_path(&request).as_deref() == Some("/reset-content") {
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 205 Reset Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    return;
                }
                if request_path(&request).as_deref() == Some("/download") {
                    let body = "download";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=fixture.txt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return;
                }
                let body = "<!doctype html><body><main id=\"source-document\">source</main></body>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), server))
}

fn request_path(request: &str) -> Option<String> {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(ToOwned::to_owned)
}

fn request_has_navigation_preload_header(request: &str, expected_value: &str) -> bool {
    request_has_header_value(request, "Service-Worker-Navigation-Preload", expected_value)
}

fn request_has_header_value(request: &str, expected_name: &str, expected_value: &str) -> bool {
    request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case(expected_name) && value.trim() == expected_value
        })
    })
}

fn request_has_header_containing(request: &str, expected_name: &str, expected_value: &str) -> bool {
    request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case(expected_name) && value.contains(expected_value)
        })
    })
}

fn navigation_preload_cookie_response(request: &str, cookie_name: &str, same_site: &str) -> String {
    let body = if request_has_header_containing(request, "Cookie", &format!("{cookie_name}=1")) {
        "1"
    } else {
        "0"
    };
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: {cookie_name}=1; Path=/app; SameSite={same_site}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> Result<String> {
    let mut bytes = Vec::new();
    let mut buf = [0_u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        let read = stream
            .read(&mut buf)
            .await
            .context("read service worker test request")?;
        if read == 0 {
            break;
        }
        bytes.push(buf[0]);
        if bytes.len() > 16 * 1024 {
            return Err(anyhow!("service worker test request head too large"));
        }
    }
    String::from_utf8(bytes).context("service worker test request should be utf-8")
}

fn main_resource_service_worker_response(path: &str) -> (&'static str, &'static str, &'static str) {
    match path {
        "/app/register.html" => (
            "200 OK",
            "text/html",
            r#"<!doctype html><script>
	globalThis.__mainResourceSwReady = "pending";
	(async () => {
	  const registration = await navigator.serviceWorker.register("worker.js", { scope: "./" });
	  await navigator.serviceWorker.ready;
	  await registration.navigationPreload.setHeaderValue("core-preload");
	  await registration.navigationPreload.enable();
	  globalThis.__mainResourceSwReady =
	    "ready:" + String(Boolean(navigator.serviceWorker.controller));
	})().catch(error => {
  globalThis.__mainResourceSwReady = "error:" + error.name + ":" + error.message;
});
</script>"#,
        ),
        "/app/worker.js" => (
            "200 OK",
            "text/javascript",
            r#"let preloadCancelProbe = Promise.resolve(null);
let preloadCancelProbeResolve = null;
function resetPreloadCancelProbe() {
  preloadCancelProbe = new Promise(resolve => {
    preloadCancelProbeResolve = resolve;
  });
}
resetPreloadCancelProbe();
self.addEventListener("install", event => {
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});
self.addEventListener("message", event => {
  if (event.data !== "preload-cancel-error") {
    return;
  }
  event.waitUntil(preloadCancelProbe.then(result => {
    event.source.postMessage(JSON.stringify(result));
  }));
});
self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
	  if (url.pathname === "/app/controlled.html") {
	    event.respondWith(new Response(
	      "<!doctype html><body>sw-main:" +
	        event.request.destination + ":" + event.request.mode +
	      "</body>",
	      { headers: { "Content-Type": "text/html" } }
	    ));
	  } else if (url.pathname === "/app/preload.html") {
	    event.respondWith((async () => {
	      const preload = await event.preloadResponse;
	      return new Response(
	        "<!doctype html><body>preload:" +
	          preload.status + ":" +
	          preload.headers.get("x-network") + ":" +
	          preload.headers.get("x-seen-preload") + ":" +
	          await preload.text() +
	        "</body>",
	        { headers: { "Content-Type": "text/html" } }
	      );
	    })());
	  } else if (url.pathname === "/app/preload-headers.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-gzip.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-chunked.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-cookie-lax.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-cookie-strict.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-empty-body.html") {
	    event.respondWith((async () => {
	      const preload = await event.preloadResponse;
	      return new Response(
	        "<!doctype html><body>[" + await preload.text() + "]</body>",
	        { headers: { "Content-Type": "text/html" } }
	      );
	    })());
	  } else if (url.pathname === "/app/preload-broken-body-unused.html") {
	    event.respondWith(event.preloadResponse.then(
	      _ => new Response(
	        "<!doctype html><body>PASS: preloadResponse resolved</body>",
	        { headers: { "Content-Type": "text/html" } }
	      ),
	      _ => new Response(
	        "<!doctype html><body>FAIL: preloadResponse rejected</body>",
	        { headers: { "Content-Type": "text/html" } }
	      )
	    ));
	  } else if (url.pathname === "/app/preload-redirect.html") {
	    event.respondWith((async () => {
	      const preload = await event.preloadResponse;
	      return new Response(
	        "<!doctype html><body>preload-redirect:" +
	          preload.status + ":" +
	          preload.type + ":" +
	          preload.redirected + ":" +
	          preload.url + ":" +
	          await preload.text() +
	        "</body>",
	        { headers: { "Content-Type": "text/html" } }
	      );
	    })());
	  } else if (url.pathname === "/app/preload-redirect-direct-body.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-redirect-follow.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-redirect-to-scope.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-redirect-to-scope-2.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-redirect-to-scope-3.html") {
	    event.respondWith(event.preloadResponse);
	  } else if (url.pathname === "/app/preload-body-error.html") {
	    event.respondWith((async () => {
	      const preload = await event.preloadResponse;
	      let body = null;
	      let bodyError = null;
	      try {
	        body = await preload.text();
	      } catch (error) {
	        bodyError = {
	          name: error && error.name,
	          message: error && error.message,
	          isTypeError: error instanceof TypeError
	        };
	      }
	      return new Response(
	        "<!doctype html><body>" +
	          JSON.stringify({
	            hasResponse: preload instanceof Response,
	            status: preload.status,
	            body,
	            bodyError
	          }) +
	        "</body>",
	        { headers: { "Content-Type": "text/html" } }
	      );
	    })());
	  } else if (url.pathname === "/app/preload-cancel.html") {
	    resetPreloadCancelProbe();
	    event.preloadResponse.catch(error => {
	      preloadCancelProbeResolve({
	        name: error && error.name,
	        message: error && error.message,
	        isDomException: error instanceof DOMException
	      });
	    });
	    event.respondWith(new Response(`<!doctype html><script>
globalThis.__preloadCancelProbe = "pending";
navigator.serviceWorker.onmessage = event => {
  globalThis.__preloadCancelProbe = event.data;
};
(async () => {
  await navigator.serviceWorker.ready;
  const controller = navigator.serviceWorker.controller;
  if (!controller) {
    globalThis.__preloadCancelProbe = "missing-controller";
    return;
  }
  controller.postMessage("preload-cancel-error");
})().catch(error => {
  globalThis.__preloadCancelProbe = "error:" + error.name + ":" + error.message;
});
</script><body>preload cancel handled</body>`,
	      { headers: { "Content-Type": "text/html" } }
	    ));
	  }
	});
	"#,
        ),
        "/app/controlled.html" => (
            "200 OK",
            "text/html",
            "<!doctype html><body>network controlled</body>",
        ),
        "/app/fallback.html" => (
            "200 OK",
            "text/html",
            "<!doctype html><body>network fallback</body>",
        ),
        "/app/preload.html" => ("200 OK", "text/plain", "network-preload-body"),
        "/app/preload-redirect.html" => ("302 Found", "text/plain", ""),
        "/app/preload-redirect-follow.html" => ("302 Found", "text/plain", ""),
        "/app/preload-redirect-to-scope.html" => ("302 Found", "text/plain", ""),
        "/app/preload-redirect-to-scope-2.html" => ("302 Found", "text/plain", ""),
        "/app/preload-redirect-to-scope-3.html" => ("302 Found", "text/plain", ""),
        "/app/preload-final.html" => ("200 OK", "text/plain", "preload-final-body"),
        "/outside/preload-redirected.html" => (
            "200 OK",
            "text/html",
            "<!doctype html><body>redirected\n</body>",
        ),
        _ => ("404 Not Found", "text/plain", "not found"),
    }
}
