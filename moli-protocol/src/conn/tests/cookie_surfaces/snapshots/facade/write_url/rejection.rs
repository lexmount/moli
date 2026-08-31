use super::*;
#[test]
fn browser_context_document_cookie_facade_snapshot_keeps_backend_available_after_lock_holder_panic()
{
    let mut bc = BrowserContext::new_with_page_for_test("BID-cookie-facade", "TID-cookie-facade");
    bc.set_target_url("https://example.com/app".into());

    let cookie_store = bc.cookie_store_for_test().clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = cookie_store.lock();
        panic!("panic while holding cookie store lock");
    }));

    let snapshot = bc.document_cookie_facade_snapshot();
    let manager_snapshot = bc.cookie_manager_surface_snapshot();
    assert_eq!(
        manager_snapshot.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
    assert_eq!(
        snapshot
            .structured_write
            .default_cookie_write_url
            .as_ref()
            .map(Url::as_str),
        Some("https://example.com/app")
    );
    assert_eq!(
        snapshot.structured_write.default_cookie_write_url_source,
        BrowserContextDefaultCookieWriteUrlSource::BrowserContextUrl
    );
    assert_eq!(
        snapshot.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingBrowserContextUrl
    );
    assert_eq!(
        snapshot.structured_write.default_command_verdict,
        BrowserContextStructuredCookieCommandVerdict::Ready
    );
    assert_eq!(
        snapshot.structured_write.backend_status,
        BrowserContextStructuredCookieWriteBackendStatus::Available
    );
    assert_eq!(
        (
            snapshot
                .structured_write
                .normalized_write_capability
                .write_enabled,
            snapshot
                .structured_write
                .normalized_write_capability
                .primary_rejection_reason,
            snapshot
                .structured_write
                .normalized_write_capability
                .blocked_reasons
                .clone()
        ),
        (true, None, Vec::<StoredCookieSetRejectionReason>::new())
    );
    assert_eq!(manager_snapshot.structured_write, snapshot.structured_write);

    assert!(
        snapshot
            .structured_write
            .normalized_cookie_facade_rejection(&StoredCookie {
                name: "sid".into(),
                value: "1".into(),
                domain: "example.com".into(),
                host_only: true,
                path: "/".into(),
                secure: true,
                http_only: false,
                expires: None,
                same_site: StoredCookieSameSite::Lax,
                priority: None,
                partition_key: None,
                source_scheme: StoredCookieSourceScheme::Unset,
                source_port: -1,
                creation_index: 0,
                last_access_index: 0,
            })
            .is_none(),
        "parking_lot mutexes do not poison, so backend availability must not project a rejection"
    );
}

#[test]
fn structured_cookie_write_snapshot_projects_any_primary_rejection_reason_into_report() {
    let snapshot =
        crate::conn::cookie_manager_surface::BrowserContextStructuredCookieWriteSnapshot {
            default_cookie_write_url: Some(Url::parse("https://example.com/app").unwrap()),
            default_cookie_write_url_source: BrowserContextDefaultCookieWriteUrlSource::LoadedPage,
            readiness_status:
                BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingLoadedPageUrl,
            backend_status: BrowserContextStructuredCookieWriteBackendStatus::Available,
            default_command_verdict: BrowserContextStructuredCookieCommandVerdict::Blocked(
                StoredCookieSetRejectionReason::CookiesDisabled,
            ),
            normalized_write_capability:
                crate::conn::cookie_manager_surface::BrowserContextCookieWriteCapabilitySnapshot {
                    write_enabled: false,
                    primary_rejection_reason: Some(StoredCookieSetRejectionReason::CookiesDisabled),
                    blocked_reasons: vec![StoredCookieSetRejectionReason::CookiesDisabled],
                },
        };

    let report = snapshot
        .normalized_cookie_facade_rejection(&StoredCookie {
            name: "sid".into(),
            value: "1".into(),
            domain: "example.com".into(),
            host_only: true,
            path: "/".into(),
            secure: true,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Lax,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Unset,
            source_port: -1,
            creation_index: 0,
            last_access_index: 0,
        })
        .expect("primary browser-boundary rejection should always produce a report");
    assert_eq!(
        report.status,
        moli_cookie_jar::StoredCookieSetStatus::Rejected(
            StoredCookieSetRejectionReason::CookiesDisabled
        )
    );
    assert_eq!(
        report.rejection_reasons,
        vec![StoredCookieSetRejectionReason::CookiesDisabled]
    );
}
