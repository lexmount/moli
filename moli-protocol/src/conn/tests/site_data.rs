use super::*;

#[test]
fn browser_context_cookie_sites_and_clear_for_sites_use_site_keys() {
    let bc = BrowserContext::new("BID-sites".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[("set-cookie".to_owned(), "b=1; Path=/assets".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "c=1; Path=/app".to_owned())],
        );
    }

    assert_eq!(
        bc.cookie_sites(),
        vec!["example.com".to_owned(), "foo.co.uk".to_owned()]
    );
    assert_eq!(bc.clear_cookies_for_sites(&["deep.example.com"]), 2);
    assert_eq!(bc.cookie_sites(), vec!["foo.co.uk".to_owned()]);
}

#[test]
fn browser_context_clear_cookies_for_sites_report_projects_replaced_and_remaining_state() {
    let bc = BrowserContext::new("BID-site-clear-report".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persistent=1; Path=/assets; Max-Age=3600".to_owned(),
            )],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let report = bc.clear_cookies_for_sites_with_report(&["deep.example.com"]);

    assert_eq!(report.requested_sites, vec!["example.com".to_owned()]);
    assert_eq!(report.removed_cookie_count, 2);
    assert_eq!(
        report.replaced_state.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
    assert!(report.resulting_state.live_site_data.is_empty());
    assert_eq!(bc.cookie_site_data(), vec![site_summary("foo.co.uk", 1, 1)]);
}

#[test]
fn browser_context_cookie_site_data_summarizes_counts_by_site() {
    let bc = BrowserContext::new("BID-site-data".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[("set-cookie".to_owned(), "b=1; Path=/assets".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[
                ("set-cookie".to_owned(), "c=1; Path=/app".to_owned()),
                ("set-cookie".to_owned(), "d=1; Path=/app".to_owned()),
            ],
        );
    }

    assert_eq!(
        bc.cookie_site_data(),
        vec![
            site_summary("example.com", 2, 0),
            site_summary("foo.co.uk", 2, 0),
        ]
    );
}

#[test]
fn browser_context_preview_clear_cookies_for_sites_reports_targeted_removal_without_mutation() {
    let bc = BrowserContext::new("BID-site-clear-preview".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let preview = bc.preview_clear_cookies_for_sites(&["deep.example.com"]);

    assert_eq!(preview.requested_sites, vec!["example.com".to_owned()]);
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert!(preview.replaced_state.store_generation.is_some());
    assert_eq!(preview.resulting_state.store_generation, None);
    assert_eq!(preview.state_diff.live_site_changes.len(), 1);
    assert_eq!(preview.state_diff.persistent_site_changes.len(), 0);
    assert_eq!(
        preview.replaced_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("foo.co.uk", 1, 1)]
    );
    assert_eq!(
        bc.cookie_site_data(),
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 1),
        ]
    );
}

#[test]
fn browser_context_preview_clear_cookies_for_sites_with_persistent_scope_keeps_session_slice() {
    let bc = BrowserContext::new("BID-site-clear-preview-persistent".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persist=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let preview = bc.preview_clear_cookies_for_sites_with_scope(
        &["deep.example.com"],
        CookieSiteDataClearScope::Persistent,
    );

    assert_eq!(preview.scope, CookieSiteDataClearScope::Persistent);
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert_eq!(preview.state_diff.live_site_changes.len(), 1);
    assert_eq!(preview.state_diff.persistent_site_changes.len(), 1);
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert_eq!(
        bc.cookie_site_data(),
        vec![site_summary("example.com", 2, 1)]
    );
}

#[test]
fn browser_context_preview_clear_cookie_store_defaults_to_all_scope() {
    let bc = BrowserContext::new("BID-store-clear-preview-all".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persist=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let preview = bc.preview_clear_cookie_store();

    assert_eq!(preview.target, CookieStorageClearTarget::WholeStore);
    assert_eq!(preview.scope, CookieSiteDataClearScope::All);
    assert_eq!(preview.would_remove_cookie_count, 2);
    assert!(preview.resulting_state.live_site_data.is_empty());
    assert_eq!(
        bc.cookie_site_data(),
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 1),
        ]
    );
}

#[tokio::test]
async fn connection_preview_clear_cookie_store_with_persistent_scope_does_not_invalidate_live_document_cookie_cache()
 {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-store-clear-preview-live",
        "TID-store-clear-preview-live",
    ));
    let url = Url::parse("https://app.example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![
                ("set-cookie".into(), "theme=dark; Path=/app".into()),
                (
                    "set-cookie".into(),
                    "persist=1; Path=/app; Max-Age=3600".into(),
                ),
            ],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    let before = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("initial cookie read should succeed");
    assert_eq!(before["value"], json!("theme=dark; persist=1"));

    let preview = conn
        .preview_clear_cookie_store_with_scope(CookieSiteDataClearScope::Persistent)
        .expect("store clear preview should succeed");

    assert_eq!(preview.scope, CookieSiteDataClearScope::Persistent);
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );

    let after = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("cookie read after preview should succeed");
    assert_eq!(after["value"], json!("theme=dark; persist=1"));
}

#[test]
fn browser_context_preview_clear_cookie_storage_with_site_target_projects_target() {
    let bc = BrowserContext::new("BID-store-clear-target-preview".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let preview = bc.preview_clear_cookie_storage_with_target_and_scope(
        CookieStorageClearTarget::RegistrableSites(vec!["deep.example.com".to_owned()]),
        CookieSiteDataClearScope::All,
    );

    assert_eq!(
        preview.target,
        CookieStorageClearTarget::RegistrableSites(vec!["example.com".to_owned()])
    );
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert_eq!(
        preview.replaced_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
}

#[tokio::test]
async fn connection_clear_cookie_store_with_session_scope_invalidates_live_document_cookie_cache_but_preserves_persistent_cookie()
 {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-store-clear-session-live",
        "TID-store-clear-session-live",
    ));
    let url = Url::parse("https://app.example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![
                ("set-cookie".into(), "theme=dark; Path=/app".into()),
                (
                    "set-cookie".into(),
                    "persist=1; Path=/app; Max-Age=3600".into(),
                ),
            ],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    let before = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("initial cookie read should succeed");
    assert_eq!(before["value"], json!("theme=dark; persist=1"));

    let report = conn
        .clear_cookie_store_with_scope_and_report(CookieSiteDataClearScope::Session)
        .expect("store clear should succeed");

    assert_eq!(report.scope, CookieSiteDataClearScope::Session);
    assert_eq!(report.removed_cookie_count, 1);
    assert_eq!(
        report.resulting_state.live_site_data,
        vec![site_summary("example.com", 1, 1)]
    );

    let after = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("cookie read after clear should succeed");
    assert_eq!(after["value"], json!("persist=1"));
}

#[test]
fn connection_clear_cookie_storage_with_site_target_projects_targeted_report() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-store-clear-target-report".into()));

    {
        let mut store = conn
            .browser_context
            .as_ref()
            .unwrap()
            .cookie_store_for_test()
            .lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persist=1; Path=/assets; Max-Age=3600".to_owned(),
            )],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "other=1; Path=/app".to_owned())],
        );
    }

    let report = conn
        .clear_cookie_storage_with_target_and_scope_and_report(
            CookieStorageClearTarget::RegistrableSites(vec!["deep.example.com".to_owned()]),
            CookieSiteDataClearScope::Persistent,
        )
        .expect("targeted clear should succeed");

    assert_eq!(
        report.target,
        CookieStorageClearTarget::RegistrableSites(vec!["example.com".to_owned()])
    );
    assert_eq!(report.scope, CookieSiteDataClearScope::Persistent);
    assert_eq!(report.removed_cookie_count, 1);
    assert_eq!(
        conn.cookie_site_data()
            .expect("cookie site data should succeed"),
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 0),
        ]
    );
}

#[test]
fn browser_context_preview_cookie_site_data_operation_clear_projects_generic_owner_seam() {
    let bc = BrowserContext::new("BID-op-preview-clear".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let preview = bc
        .preview_cookie_site_data_operation(&CookieSiteDataOperation::Clear {
            target: CookieStorageClearTarget::RegistrableSites(vec!["deep.example.com".to_owned()]),
            scope: CookieSiteDataClearScope::All,
        })
        .expect("site-data operation preview should succeed");

    let CookieSiteDataOperationPreviewReport::Clear(report) = preview;
    assert_eq!(
        report.target,
        CookieStorageClearTarget::RegistrableSites(vec!["example.com".to_owned()])
    );
    assert_eq!(report.would_remove_cookie_count, 1);
}

#[test]
fn browser_context_cookie_storage_state_snapshot_distinguishes_live_and_persistent_views() {
    let bc = BrowserContext::new("BID-site-state".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persistent=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    assert_eq!(
        bc.cookie_site_data_with_scope(CookieSiteDataScope::Persistent),
        vec![site_summary("foo.co.uk", 1, 1)]
    );

    let snapshot = bc.cookie_storage_state_snapshot();
    assert_eq!(snapshot.live_cookie_count, 2);
    assert_eq!(snapshot.persistent_cookie_count, 1);
    assert_eq!(
        snapshot.live_site_data,
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 1),
        ]
    );
    assert_eq!(
        snapshot.persistent_site_data,
        vec![site_summary("foo.co.uk", 1, 1)]
    );
}

#[test]
fn browser_context_cookie_storage_state_snapshot_for_sites_filters_views() {
    let bc = BrowserContext::new("BID-site-state-scoped".into());
    {
        let mut store = bc.cookie_store_for_test().lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persistent=1; Path=/assets; Max-Age=3600".to_owned(),
            )],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let snapshot = bc.cookie_storage_state_snapshot_for_sites(&["deep.example.com"]);
    assert_eq!(snapshot.live_cookie_count, 2);
    assert_eq!(snapshot.persistent_cookie_count, 1);
    assert_eq!(
        snapshot.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
}

#[tokio::test]
async fn connection_cookie_site_clear_invalidates_live_document_cookie_cache() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-live-sites",
        "TID-live-sites",
    ));
    let url = Url::parse("https://app.example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![("set-cookie".into(), "theme=dark; Path=/app".into())],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    let before = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("initial cookie read should succeed");
    assert_eq!(before["value"], json!("theme=dark"));

    assert_eq!(conn.clear_cookies_for_sites(&["example.com"]).unwrap(), 1);

    let after = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("cookie read after site clear should succeed");
    assert_eq!(after["value"], json!(""));
}

#[test]
fn connection_clear_cookies_for_sites_report_projects_targeted_state() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-live-site-clear-report".into()));
    {
        let cookie_store = conn.ensure_cookie_store().unwrap();
        let mut store = cookie_store.lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let report = conn
        .clear_cookies_for_sites_with_report(&["deep.example.com"])
        .expect("site clear report should succeed");

    assert_eq!(report.requested_sites, vec!["example.com".to_owned()]);
    assert_eq!(report.removed_cookie_count, 1);
    assert_eq!(
        report.replaced_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert!(report.resulting_state.live_site_data.is_empty());
    assert_eq!(
        conn.cookie_site_data().unwrap(),
        vec![site_summary("foo.co.uk", 1, 1)]
    );
}

#[tokio::test]
async fn connection_preview_clear_cookies_for_sites_does_not_invalidate_live_document_cookie_cache()
{
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-live-site-clear-preview",
        "TID-live-site-clear-preview",
    ));
    let url = Url::parse("https://app.example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![("set-cookie".into(), "theme=dark; Path=/app".into())],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    let before = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("initial cookie read should succeed");
    assert_eq!(before["value"], json!("theme=dark"));

    {
        let cookie_store = conn.ensure_cookie_store().unwrap();
        let mut store = cookie_store.lock();
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let preview = conn
        .preview_clear_cookies_for_sites(&["deep.example.com"])
        .expect("site clear preview should succeed");

    assert_eq!(preview.requested_sites, vec!["example.com".to_owned()]);
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert!(preview.replaced_state.store_generation.is_some());
    assert_eq!(preview.resulting_state.store_generation, None);
    assert_eq!(preview.state_diff.live_site_changes.len(), 1);
    assert_eq!(preview.state_diff.persistent_site_changes.len(), 0);
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("foo.co.uk", 1, 1)]
    );

    let after = conn
        .evaluate_runtime_expression_with_await_async("document.cookie", false)
        .await
        .expect("cookie read after preview should succeed");
    assert_eq!(after["value"], json!("theme=dark"));
}

#[test]
fn connection_cookie_sites_reflect_active_browser_store() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-live-sites".into()));
    {
        let cookie_store = conn.ensure_cookie_store().unwrap();
        let mut store = cookie_store.lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "b=1; Path=/app".to_owned())],
        );
    }

    assert_eq!(
        conn.cookie_sites().unwrap(),
        vec!["example.com".to_owned(), "foo.co.uk".to_owned()]
    );
}

#[test]
fn connection_cookie_site_data_reflects_active_browser_store() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-live-site-data".into()));
    {
        let cookie_store = conn.ensure_cookie_store().unwrap();
        let mut store = cookie_store.lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[("set-cookie".to_owned(), "b=1; Path=/assets".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "c=1; Path=/app".to_owned())],
        );
    }

    assert_eq!(
        conn.cookie_site_data().unwrap(),
        vec![
            site_summary("example.com", 2, 0),
            site_summary("foo.co.uk", 1, 0),
        ]
    );
}

#[test]
fn connection_cookie_storage_state_snapshot_reflects_active_browser_store() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-live-site-state".into()));
    {
        let cookie_store = conn.ensure_cookie_store().unwrap();
        let mut store = cookie_store.lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persistent=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let snapshot = conn
        .cookie_storage_state_snapshot()
        .expect("cookie storage snapshot should succeed");
    assert_eq!(snapshot.live_cookie_count, 2);
    assert_eq!(snapshot.persistent_cookie_count, 1);
}

#[test]
fn connection_cookie_storage_state_snapshot_for_sites_filters_active_store() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-live-site-state-scoped".into()));
    {
        let cookie_store = conn.ensure_cookie_store().unwrap();
        let mut store = cookie_store.lock();
        store.store_response_headers(
            &Url::parse("https://app.example.com/app/index.html").unwrap(),
            &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
        );
        store.store_response_headers(
            &Url::parse("https://cdn.example.com/assets/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "persistent=1; Path=/assets; Max-Age=3600".to_owned(),
            )],
        );
        store.store_response_headers(
            &Url::parse("https://foo.co.uk/app/index.html").unwrap(),
            &[(
                "set-cookie".to_owned(),
                "other=1; Path=/app; Max-Age=3600".to_owned(),
            )],
        );
    }

    let snapshot = conn
        .cookie_storage_state_snapshot_for_sites(&["sub.example.com"])
        .expect("scoped cookie storage snapshot should succeed");
    assert_eq!(snapshot.live_cookie_count, 2);
    assert_eq!(snapshot.persistent_cookie_count, 1);
    assert_eq!(
        snapshot.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
}
