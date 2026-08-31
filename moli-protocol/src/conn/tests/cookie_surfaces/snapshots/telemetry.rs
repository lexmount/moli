use super::*;
#[tokio::test]
async fn browser_context_document_cookie_snapshots_reflect_live_page_state() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-facade",
        "TID-cookie-facade",
    ));
    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://example.com/app").unwrap(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_loaded_page_async(navigation.page)
        .await;

    let before = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_telemetry_snapshot_async()
        .await
        .expect("live page telemetry snapshot");
    let live_before = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap()
        .document_cookie_telemetry_snapshot_async()
        .await
        .unwrap();
    assert_eq!(
        before.last_operation_was_set,
        live_before.last_operation_was_set
    );
    assert_eq!(before.cache_hits, live_before.cache_hits);
    assert_eq!(before.store_reads, live_before.store_reads);
    assert_eq!(before.blocked_reads, live_before.blocked_reads);
    assert_eq!(before.unavailable_reads, live_before.unavailable_reads);
    assert_eq!(before.applied_writes, live_before.applied_writes);
    assert_eq!(before.rejected_writes, live_before.rejected_writes);
    assert_eq!(
        before.facade_blocked_writes,
        live_before.facade_blocked_writes
    );
    assert_eq!(before.last_cache_lookup_result, None);

    conn.evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .unwrap();
    conn.evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .unwrap();

    let after = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_telemetry_snapshot_async()
        .await
        .expect("live page telemetry snapshot");
    let live_after = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap()
        .document_cookie_telemetry_snapshot_async()
        .await
        .unwrap();
    // BrowserContext should be a thin owner/view seam over the live page's
    // document-cookie facade state instead of keeping a parallel counter
    // set of its own.
    let expected_lookup = match live_after.last_cache_lookup_result {
        Some(moli_core::page::DocumentCookieCacheLookupResult::CacheMissFirstAccess) => {
            Some(BrowserContextDocumentCookieCacheLookupResult::CacheMissFirstAccess)
        }
        Some(moli_core::page::DocumentCookieCacheLookupResult::CacheHitAfterGet) => {
            Some(BrowserContextDocumentCookieCacheLookupResult::CacheHitAfterGet)
        }
        Some(moli_core::page::DocumentCookieCacheLookupResult::CacheHitAfterSet) => {
            Some(BrowserContextDocumentCookieCacheLookupResult::CacheHitAfterSet)
        }
        Some(moli_core::page::DocumentCookieCacheLookupResult::CacheMissAfterGet) => {
            Some(BrowserContextDocumentCookieCacheLookupResult::CacheMissAfterGet)
        }
        Some(moli_core::page::DocumentCookieCacheLookupResult::CacheMissAfterSet) => {
            Some(BrowserContextDocumentCookieCacheLookupResult::CacheMissAfterSet)
        }
        None => None,
    };
    assert_eq!(after.last_cache_lookup_result, expected_lookup);
    assert_eq!(
        after.last_operation_was_set,
        live_after.last_operation_was_set
    );
    assert_eq!(after.cache_hits, live_after.cache_hits);
    assert_eq!(after.store_reads, live_after.store_reads);
    assert_eq!(after.blocked_reads, live_after.blocked_reads);
    assert_eq!(after.unavailable_reads, live_after.unavailable_reads);
    assert_eq!(after.applied_writes, live_after.applied_writes);
    assert_eq!(after.rejected_writes, live_after.rejected_writes);
    assert_eq!(
        after.facade_blocked_writes,
        live_after.facade_blocked_writes
    );
    assert!(after.store_reads >= before.store_reads);
}
#[tokio::test]
async fn browser_context_document_cookie_facade_snapshot_projects_probe_telemetry_into_owner_view()
{
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-cookie-facade", "TID-cookie-facade");
    bc.set_target_url("https://example.com/app".into());
    conn.browser_context = Some(bc);

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://example.com/app").unwrap(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_loaded_page_async(navigation.page)
        .await;

    let payload = conn
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("cookieEnabled probe should succeed");
    assert_eq!(payload["value"], json!(true));

    let snapshot = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        snapshot.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        snapshot.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoEntry
    );
    assert_eq!(
        snapshot.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
    );
    assert_eq!(
        snapshot.capability_surface.first_cookie_request,
        Some(BrowserContextFirstCookieRequest::CookiesEnabled)
    );
    assert!(snapshot.freshness.cookie_get_would_need_backend_access);
    assert!(!snapshot.freshness.cookie_get_would_need_backend_reconnect);
    assert!(!snapshot.freshness.cookie_get_would_hit_cache);
    assert!(!snapshot.freshness.cookie_get_would_revalidate_after_write);
}
