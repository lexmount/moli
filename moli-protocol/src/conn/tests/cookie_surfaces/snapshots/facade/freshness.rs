use super::*;
#[test]
fn browser_context_document_cookie_facade_snapshot_tracks_shared_cookie_store_generation() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-cookie-facade", "TID-cookie-facade");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    let before = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();
    assert_eq!(before.cookie_store_generation, Some(0));

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-cookie-facade".into()),
        vec![crate::domains::storage::CdpCookieParam {
            name: "sid".into(),
            value: "1".into(),
            url: None,
            domain: None,
            path: None,
            secure: None,
            http_only: false,
            same_site: None,
            priority: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
            partition_key_opaque: None,
            expires: None,
        }],
    )
    .expect("structured write should succeed via browser context default url");

    let after = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();
    assert_eq!(
        after.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
    assert_eq!(
        after.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoLivePage
    );
    assert_eq!(
        after.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::NoLivePage
    );
    assert!(after.cookie_store_generation > before.cookie_store_generation);
    assert!(!after.freshness.cookie_get_would_need_backend_access);
    assert!(!after.freshness.cookie_get_would_need_backend_reconnect);
    assert!(!after.freshness.cookie_get_would_hit_cache);
    assert!(!after.freshness.cookie_get_would_revalidate_after_write);
}

#[tokio::test]
async fn browser_context_document_cookie_facade_snapshot_projects_cookie_get_freshness_state() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-cookie-facade", "TID-cookie-facade");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://example.com/app").unwrap(),
            "GET".into(),
            vec![],
            200,
            vec![("set-cookie".into(), "theme=dark; Path=/".into())],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_loaded_page_async(navigation.page)
        .await;

    let before_read = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        before_read.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        before_read.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoEntry
    );
    assert_eq!(
        before_read.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
    );
    assert!(before_read.freshness.cookie_get_would_need_backend_access);
    assert!(
        !before_read
            .freshness
            .cookie_get_would_need_backend_reconnect
    );
    assert!(!before_read.freshness.cookie_get_would_hit_cache);
    assert!(
        !before_read
            .freshness
            .cookie_get_would_revalidate_after_write
    );
    assert_eq!(before_read.capability_surface.first_cookie_request, None);

    let payload = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("cookie read should succeed");
    assert_eq!(payload["value"], json!("theme=dark"));

    let after_read = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        after_read.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        after_read.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoEntry
    );
    assert_eq!(
        after_read.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
    );
    assert!(after_read.freshness.cookie_get_would_need_backend_access);
    assert!(!after_read.freshness.cookie_get_would_need_backend_reconnect);
    assert!(!after_read.freshness.cookie_get_would_hit_cache);
    assert!(!after_read.freshness.cookie_get_would_revalidate_after_write);
    assert_eq!(
        after_read.capability_surface.first_cookie_request,
        Some(BrowserContextFirstCookieRequest::Get)
    );

    crate::domains::storage::set_cookies_for_browser_context_async(
        &mut conn,
        Some("BID-cookie-facade".into()),
        vec![crate::domains::storage::CdpCookieParam {
            name: "sid".into(),
            value: "1".into(),
            url: None,
            domain: None,
            path: None,
            secure: None,
            http_only: false,
            same_site: None,
            priority: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
            partition_key_opaque: None,
            expires: None,
        }],
    )
    .await
    .expect("structured write should succeed");

    let after_write = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        after_write.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        after_write.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoEntry
    );
    assert_eq!(
        after_write.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
    );
    assert!(after_write.freshness.cookie_get_would_need_backend_access);
    assert!(
        !after_write
            .freshness
            .cookie_get_would_need_backend_reconnect
    );
    assert!(!after_write.freshness.cookie_get_would_hit_cache);
    // A backend-side write invalidates freshness, but it is not a
    // document-facing cookie write. Keep the "after write revalidation"
    // bit tied to facade-visible `document.cookie` ownership instead of
    // treating every backend mutation as a pending document write.
    assert!(
        !after_write
            .freshness
            .cookie_get_would_revalidate_after_write
    );
    assert_eq!(
        after_write.capability_surface.first_cookie_request,
        Some(BrowserContextFirstCookieRequest::Get)
    );

    conn.evaluate_runtime_expression_with_await_async("document.cookie = 'lang=en; Path=/'", false)
        .await
        .expect("document cookie write should succeed");

    let after_document_write = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        after_document_write
            .capability_surface
            .backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        after_document_write.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NeedsRevalidationAfterDocumentWrite
    );
    assert_eq!(
        after_document_write.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
    );
    assert!(
        after_document_write
            .freshness
            .cookie_get_would_need_backend_access
    );
    assert!(
        !after_document_write
            .freshness
            .cookie_get_would_need_backend_reconnect
    );
    assert!(!after_document_write.freshness.cookie_get_would_hit_cache);
    assert!(
        after_document_write
            .freshness
            .cookie_get_would_revalidate_after_write
    );
    assert_eq!(
        after_document_write.capability_surface.first_cookie_request,
        Some(BrowserContextFirstCookieRequest::Get)
    );
}
