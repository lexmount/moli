use super::*;
#[tokio::test]
async fn browser_context_document_cookie_facade_snapshot_projects_default_cookie_write_url_owner() {
    let mut bc = BrowserContext::new_with_page_for_test("BID-cookie-facade", "TID-cookie-facade");
    bc.set_target_url("https://example.com/app".into());

    let before_load = bc.document_cookie_facade_snapshot();
    let before_load_manager = bc.cookie_manager_surface_snapshot();
    assert_eq!(
        before_load_manager.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
    assert_eq!(
        before_load
            .structured_write
            .default_cookie_write_url
            .as_ref()
            .map(Url::as_str),
        Some("https://example.com/app")
    );
    assert_eq!(
        before_load.structured_write.default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::BrowserContextUrl
    );
    assert_eq!(
        before_load.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingBrowserContextUrl
    );
    assert_eq!(
        before_load.structured_write.backend_status,
        BrowserContextStructuredCookieWriteBackendStatus::Available
    );
    assert_eq!(
        (
            before_load
                .structured_write
                .normalized_write_capability
                .write_enabled,
            before_load
                .structured_write
                .normalized_write_capability
                .primary_rejection_reason,
            before_load
                .structured_write
                .normalized_write_capability
                .blocked_reasons
                .clone()
        ),
        (true, None, Vec::<StoredCookieSetRejectionReason>::new())
    );
    assert_eq!(
        before_load_manager.structured_write,
        before_load.structured_write
    );

    let mut conn = CdpConnection::new();
    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://live.example.com/page").unwrap(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    bc.set_loaded_page_async(navigation.page).await;

    let after_load = bc.document_cookie_facade_snapshot_async().await;
    let after_load_manager = bc.cookie_manager_surface_snapshot_async().await;
    assert_eq!(
        after_load_manager.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        after_load
            .structured_write
            .default_cookie_write_url
            .as_ref()
            .map(Url::as_str),
        Some("https://live.example.com/page")
    );
    assert_eq!(
        after_load.structured_write.default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::LoadedPage
    );
    assert_eq!(
        after_load.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingLoadedPageUrl
    );
    assert_eq!(
        after_load.structured_write.backend_status,
        BrowserContextStructuredCookieWriteBackendStatus::Available
    );
    assert_eq!(
        (
            after_load
                .structured_write
                .normalized_write_capability
                .write_enabled,
            after_load
                .structured_write
                .normalized_write_capability
                .primary_rejection_reason,
            after_load
                .structured_write
                .normalized_write_capability
                .blocked_reasons
                .clone()
        ),
        (true, None, Vec::<StoredCookieSetRejectionReason>::new())
    );
    assert_eq!(
        after_load_manager.structured_write,
        after_load.structured_write
    );

    bc.set_target_url("about:blank".into());
    bc.clear_loaded_page();

    let after_detach = bc.document_cookie_facade_snapshot();
    let after_detach_manager = bc.cookie_manager_surface_snapshot();
    assert_eq!(
        after_detach_manager.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
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
    assert_eq!(
        after_detach_manager.structured_write,
        after_detach.structured_write
    );
}
