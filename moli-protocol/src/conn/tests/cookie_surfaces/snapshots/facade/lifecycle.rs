use super::*;
#[tokio::test]
async fn browser_context_page_attachment_id_tracks_attach_and_detach() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-facade",
        "TID-cookie-facade",
    ));

    let before = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();
    assert!(!before.has_loaded_page);
    assert_eq!(before.page_attachment_id, None);
    assert_eq!(before.cookie_store_generation, Some(0));
    assert_eq!(before.structured_write.default_cookie_write_url, None);
    assert_eq!(
        before.structured_write.default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::Unavailable
    );
    assert_eq!(
        before.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::MissingScopedUrl
    );
    assert_eq!(
        before.structured_write.default_command_verdict,
        BrowserContextStructuredCookieCommandVerdict::MissingScopedUrl
    );
    assert_eq!(
        before.structured_write.backend_status,
        BrowserContextStructuredCookieWriteBackendStatus::Available
    );
    assert!(
        before
            .structured_write
            .normalized_write_capability
            .write_enabled
    );
    assert_eq!(
        before
            .structured_write
            .normalized_write_capability
            .primary_rejection_reason,
        None
    );
    assert!(
        before
            .structured_write
            .normalized_write_capability
            .blocked_reasons
            .is_empty()
    );
    assert!(before.capability_surface.capability.is_none());
    assert!(before.freshness.cache.is_none());
    assert_eq!(
        before.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
    assert_eq!(
        before.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoLivePage
    );
    assert_eq!(
        before.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::NoLivePage
    );
    assert!(!before.freshness.cookie_get_would_need_backend_access);
    assert!(!before.freshness.cookie_get_would_need_backend_reconnect);
    assert!(!before.freshness.cookie_get_would_hit_cache);
    assert!(!before.freshness.cookie_get_would_revalidate_after_write);
    assert_eq!(before.capability_surface.first_cookie_request, None);
    assert!(before.capability_surface.telemetry.is_none());

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

    let after_attach = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert!(after_attach.has_loaded_page);
    let attached_page_id = after_attach
        .page_attachment_id
        .expect("attached Page must expose an attachment id");
    assert_eq!(after_attach.cookie_store_generation, Some(0));
    assert_eq!(
        after_attach
            .structured_write
            .default_cookie_write_url
            .as_ref()
            .map(Url::as_str),
        Some("https://example.com/app")
    );
    assert_eq!(
        after_attach
            .structured_write
            .default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::LoadedPage
    );
    assert_eq!(
        after_attach.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingLoadedPageUrl
    );
    assert_eq!(
        after_attach.structured_write.default_command_verdict,
        BrowserContextStructuredCookieCommandVerdict::Ready
    );
    assert_eq!(
        after_attach.structured_write.backend_status,
        BrowserContextStructuredCookieWriteBackendStatus::Available
    );
    assert!(after_attach.capability_surface.capability.is_some());
    assert_eq!(
        after_attach
            .freshness
            .cache
            .as_ref()
            .map(|cache| cache.status),
        Some(moli_core::page::DocumentCookieCacheStatus::NoEntry)
    );
    assert_eq!(
        after_attach.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        after_attach.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoEntry
    );
    assert_eq!(
        after_attach.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
    );
    assert!(after_attach.freshness.cookie_get_would_need_backend_access);
    assert!(
        !after_attach
            .freshness
            .cookie_get_would_need_backend_reconnect
    );
    assert!(!after_attach.freshness.cookie_get_would_hit_cache);
    assert!(
        !after_attach
            .freshness
            .cookie_get_would_revalidate_after_write
    );
    assert_eq!(after_attach.capability_surface.first_cookie_request, None);
    assert!(after_attach.capability_surface.telemetry.is_some());

    conn.browser_context.as_mut().unwrap().clear_loaded_page();
    let after_detach = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();
    assert!(!after_detach.has_loaded_page);
    assert_eq!(after_detach.page_attachment_id, None);
    assert_ne!(attached_page_id, 0);
    assert_eq!(after_detach.cookie_store_generation, Some(0));
    assert_eq!(after_detach.structured_write.default_cookie_write_url, None);
    assert_eq!(
        after_detach
            .structured_write
            .default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::Unavailable
    );
    assert_eq!(
        after_detach.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::MissingScopedUrl
    );
    assert_eq!(
        after_detach.structured_write.default_command_verdict,
        BrowserContextStructuredCookieCommandVerdict::MissingScopedUrl
    );
    assert_eq!(
        after_detach.structured_write.backend_status,
        BrowserContextStructuredCookieWriteBackendStatus::Available
    );
    assert!(
        after_detach
            .structured_write
            .normalized_write_capability
            .write_enabled
    );
    assert_eq!(
        after_detach
            .structured_write
            .normalized_write_capability
            .primary_rejection_reason,
        None
    );
    assert!(
        after_detach
            .structured_write
            .normalized_write_capability
            .blocked_reasons
            .is_empty()
    );
    assert!(after_detach.capability_surface.capability.is_none());
    assert!(after_detach.freshness.cache.is_none());
    assert_eq!(
        after_detach.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
    assert_eq!(
        after_detach.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NoLivePage
    );
    assert_eq!(
        after_detach.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::NoLivePage
    );
    assert!(!after_detach.freshness.cookie_get_would_need_backend_access);
    assert!(
        !after_detach
            .freshness
            .cookie_get_would_need_backend_reconnect
    );
    assert!(!after_detach.freshness.cookie_get_would_hit_cache);
    assert!(
        !after_detach
            .freshness
            .cookie_get_would_revalidate_after_write
    );
    assert_eq!(after_detach.capability_surface.first_cookie_request, None);
    assert!(after_detach.capability_surface.telemetry.is_none());
}
