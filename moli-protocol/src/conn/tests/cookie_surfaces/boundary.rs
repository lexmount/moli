use super::*;

#[test]
fn browser_context_document_cookie_facade_snapshot_preview_does_not_bump_cookie_store_generation() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-cookie-facade", "TID-cookie-facade");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

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

    let before_preview = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();
    let before_generation = before_preview.cookie_store_generation;

    conn.preview_clear_cookie_storage_with_target_and_scope(
        moli_cookie_jar::CookieStorageClearTarget::WholeStore,
        moli_cookie_jar::CookieSiteDataClearScope::Persistent,
    )
    .expect("preview should succeed");

    let after_preview = conn
        .browser_context
        .as_ref()
        .unwrap()
        .document_cookie_facade_snapshot();
    assert_eq!(after_preview.cookie_store_generation, before_generation);
}

#[test]
fn browser_context_cookie_boundary_snapshot_aligns_facade_and_storage_generation() {
    let mut conn = CdpConnection::new();
    let mut bc =
        BrowserContext::new_with_page_for_test("BID-cookie-boundary", "TID-cookie-boundary");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-cookie-boundary".into()),
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
    .expect("structured write should succeed");

    let snapshot = conn
        .cookie_boundary_snapshot()
        .expect("boundary snapshot should exist");
    assert_eq!(
        snapshot.facade.cookie_store_generation,
        snapshot.storage_state.store_generation
    );
    assert_eq!(snapshot.storage_state.live_cookie_count, 1);
    assert_eq!(
        snapshot.storage_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
}

#[test]
fn connection_cookie_boundary_snapshot_for_sites_keeps_facade_but_filters_storage_slice() {
    let mut conn = CdpConnection::new();
    let mut bc =
        BrowserContext::new_with_page_for_test("BID-cookie-boundary", "TID-cookie-boundary");
    bc.set_target_url("https://sub.example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-cookie-boundary".into()),
        vec![
            crate::domains::storage::CdpCookieParam {
                name: "example".into(),
                value: "1".into(),
                url: Some("https://deep.example.com/app".into()),
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
            },
            crate::domains::storage::CdpCookieParam {
                name: "other".into(),
                value: "1".into(),
                url: Some("https://foo.co.uk/app".into()),
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
            },
        ],
    )
    .expect("structured writes should succeed");

    let full = conn
        .cookie_boundary_snapshot()
        .expect("full boundary snapshot should exist");
    let filtered = conn
        .cookie_boundary_snapshot_for_sites(&["deep.example.com"])
        .expect("site-scoped boundary snapshot should exist");

    assert_eq!(filtered.facade, full.facade);
    assert_eq!(
        filtered.storage_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert_eq!(filtered.storage_state.live_cookie_count, 1);
    assert_eq!(
        full.storage_state.live_site_data,
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 0)
        ]
    );
}

#[test]
fn browser_context_preview_cookie_boundary_operation_projects_hypothetical_storage_only() {
    let mut conn = CdpConnection::new();
    let mut bc =
        BrowserContext::new_with_page_for_test("BID-cookie-boundary", "TID-cookie-boundary");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-cookie-boundary".into()),
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
    .expect("structured write should succeed");

    let preview = conn
        .browser_context
        .as_ref()
        .unwrap()
        .preview_cookie_boundary_operation(&CookieSiteDataOperation::Clear {
            target: CookieStorageClearTarget::WholeStore,
            scope: CookieSiteDataClearScope::All,
        })
        .expect("preview should succeed");
    assert_eq!(
        preview.current_boundary.facade,
        preview.resulting_boundary.facade
    );
    assert!(preview.current_boundary.storage_state.live_cookie_count > 0);
    assert_eq!(
        preview.resulting_boundary.storage_state.live_cookie_count,
        0
    );
}

#[test]
fn browser_context_preview_cookie_boundary_operation_with_site_target_projects_target_slice() {
    let bc = BrowserContext::new("BID-cookie-boundary".into());
    bc.store_response_cookie_headers_for_test(
        &Url::parse("https://app.example.com/app/index.html").unwrap(),
        &[
            ("set-cookie".to_owned(), "session=1; Path=/app".to_owned()),
            (
                "set-cookie".to_owned(),
                "persist=1; Path=/app; Max-Age=3600".to_owned(),
            ),
        ],
    );
    bc.store_response_cookie_headers_for_test(
        &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
        &[("set-cookie".to_owned(), "other=1; Path=/app".to_owned())],
    );

    let preview = bc
        .preview_cookie_boundary_operation(&CookieSiteDataOperation::Clear {
            target: CookieStorageClearTarget::RegistrableSites(vec!["Deep.Example.com".to_owned()]),
            scope: CookieSiteDataClearScope::Persistent,
        })
        .expect("preview should succeed");

    assert_eq!(
        preview.current_target_boundary.storage_state.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
    assert_eq!(
        preview
            .current_target_boundary
            .storage_state
            .live_cookie_count,
        2
    );
    assert_eq!(
        preview
            .resulting_target_boundary
            .storage_state
            .live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert_eq!(
        preview.resulting_boundary.storage_state.live_site_data,
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 0)
        ]
    );
    assert_eq!(
        preview.current_boundary.facade,
        preview.current_target_boundary.facade
    );
    assert_eq!(
        preview.resulting_boundary.facade,
        preview.resulting_target_boundary.facade
    );
}

#[test]
fn connection_apply_cookie_boundary_operation_reports_replaced_and_resulting_boundary() {
    let mut conn = CdpConnection::new();
    let mut bc =
        BrowserContext::new_with_page_for_test("BID-cookie-boundary", "TID-cookie-boundary");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-cookie-boundary".into()),
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
    .expect("structured write should succeed");

    let report = conn
        .apply_cookie_boundary_operation(&CookieSiteDataOperation::Clear {
            target: CookieStorageClearTarget::WholeStore,
            scope: CookieSiteDataClearScope::All,
        })
        .expect("apply should succeed");

    assert!(report.replaced_boundary.storage_state.live_cookie_count > 0);
    assert_eq!(report.resulting_boundary.storage_state.live_cookie_count, 0);
    assert_eq!(
        report.resulting_boundary.storage_state.store_generation,
        report.resulting_boundary.facade.cookie_store_generation
    );
}

#[test]
fn browser_context_site_data_manager_surface_wraps_cookie_boundary_with_reserved_future_storage() {
    let mut conn = CdpConnection::new();
    let mut bc =
        BrowserContext::new_with_page_for_test("BID-site-data-manager", "TID-site-data-manager");
    bc.set_target_url("https://sub.example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-site-data-manager".into()),
        vec![
            crate::domains::storage::CdpCookieParam {
                name: "example".into(),
                value: "1".into(),
                url: Some("https://deep.example.com/app".into()),
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
            },
            crate::domains::storage::CdpCookieParam {
                name: "other".into(),
                value: "1".into(),
                url: Some("https://foo.co.uk/app".into()),
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
            },
        ],
    )
    .expect("structured writes should succeed");

    let full_boundary = conn
        .cookie_boundary_snapshot()
        .expect("cookie boundary snapshot should exist");
    let filtered_boundary = conn
        .cookie_boundary_snapshot_for_sites(&["deep.example.com"])
        .expect("filtered cookie boundary snapshot should exist");

    let full_surface = conn
        .site_data_manager_surface_snapshot()
        .expect("site-data manager surface should exist");
    let filtered_surface = conn
        .site_data_manager_surface_snapshot_for_sites(&["deep.example.com"])
        .expect("filtered site-data manager surface should exist");

    assert_eq!(
        full_surface.owner_state,
        BrowserContextSiteDataManagerOwnerState::CookieOnly
    );
    assert_eq!(
        full_surface.reserved_additional_storage,
        BrowserContextReservedSiteDataOwnerState::Reserved
    );
    assert_eq!(full_surface.cookie_boundary, full_boundary);
    assert_eq!(filtered_surface.cookie_boundary, filtered_boundary);
}

#[test]
fn browser_context_site_data_manager_operation_wraps_cookie_boundary_preview_and_report() {
    let mut conn = CdpConnection::new();
    let mut bc =
        BrowserContext::new_with_page_for_test("BID-site-data-manager", "TID-site-data-manager");
    bc.set_target_url("https://example.com/app".into());
    conn.install_browser_context_fixture_for_test(bc);

    crate::domains::storage::set_cookies_for_browser_context(
        &mut conn,
        Some("BID-site-data-manager".into()),
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
    .expect("structured write should succeed");

    let preview = conn
        .preview_site_data_manager_operation(&CookieSiteDataOperation::Clear {
            target: CookieStorageClearTarget::WholeStore,
            scope: CookieSiteDataClearScope::All,
        })
        .expect("site-data manager preview should succeed");
    assert_eq!(
        preview.current_surface.cookie_boundary,
        preview.cookie_boundary_preview.current_boundary
    );
    assert_eq!(
        preview.resulting_surface.cookie_boundary,
        preview.cookie_boundary_preview.resulting_boundary
    );
    assert_eq!(
        preview.current_surface.cookie_boundary.facade,
        preview.resulting_surface.cookie_boundary.facade
    );

    let report = conn
        .apply_site_data_manager_operation(&CookieSiteDataOperation::Clear {
            target: CookieStorageClearTarget::WholeStore,
            scope: CookieSiteDataClearScope::All,
        })
        .expect("site-data manager apply should succeed");
    assert_eq!(
        report.replaced_surface.cookie_boundary,
        report.cookie_boundary_report.replaced_boundary
    );
    assert_eq!(
        report.resulting_surface.cookie_boundary,
        report.cookie_boundary_report.resulting_boundary
    );
    assert_eq!(
        report
            .resulting_surface
            .cookie_boundary
            .storage_state
            .live_cookie_count,
        0
    );
}
