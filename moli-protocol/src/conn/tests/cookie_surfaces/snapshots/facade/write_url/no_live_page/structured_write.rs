use super::*;

#[tokio::test]
async fn browser_context_document_cookie_facade_snapshot_keeps_structured_write_overrides_without_live_page()
 {
    let conn = configured_connection_without_live_page().await;
    let snapshot = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();

    assert!(!snapshot.has_loaded_page);
    assert_eq!(snapshot.document_id, None);
    assert_eq!(snapshot.cookie_store_generation, Some(0));
    assert_eq!(snapshot.structured_write.default_cookie_write_url, None);
    assert_eq!(
        snapshot.structured_write.default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::Unavailable
    );
    assert_eq!(
        snapshot.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::MissingScopedUrl
    );
}
