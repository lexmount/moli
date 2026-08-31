use super::*;
#[tokio::test]
async fn browser_context_cookie_manager_surface_projects_live_effective_browser_context() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-manager-context",
        "TID-cookie-manager-context",
    ));
    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://child.example/app").unwrap(),
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

    conn.browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_browser_context_overrides_async(
            &BrowserCookieFacadeContextOverrides::default()
                .with_site_for_cookies_url(&Url::parse("https://embedder.example/frame").unwrap())
                .with_top_frame_origin_url(&Url::parse("https://top.example/root").unwrap())
                .with_storage_access_status(
                    moli_cookie_jar::BrowserCookieStorageAccessStatus::Granted,
                ),
        )
        .await;

    let manager_surface = conn
        .browser_context
        .as_mut()
        .unwrap()
        .cookie_manager_surface_snapshot_async()
        .await;
    let context = manager_surface
        .effective_context
        .expect("live page should project effective manager context");
    let navigation = &context.navigation;
    assert_eq!(
        navigation.current_document_url.as_str(),
        "https://child.example/app"
    );
    assert_eq!(navigation.navigation_initiator_url, None);
    assert_eq!(navigation.navigation_initiator_site, None);
    assert_eq!(navigation.navigation_initiator_requested_relationship, None);
    assert_eq!(
        navigation.schemeful_navigation_initiator_requested_relationship,
        None
    );
    assert_eq!(
        navigation.effective_navigation_relationship_source,
        super::cookie_manager_surface::BrowserContextCookieManagerEffectiveNavigationRelationshipSource::RequestedDocument
    );
    assert_eq!(
        navigation.effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.schemeful_effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(navigation.navigation_initiator_relationship, None);
    assert_eq!(navigation.schemeful_navigation_initiator_relationship, None);
    assert_eq!(
        navigation.current_document_site.as_deref(),
        Some("child.example")
    );
    assert_eq!(
        navigation.requested_document_url.as_str(),
        "https://child.example/app"
    );
    assert_eq!(
        navigation.requested_document_site.as_deref(),
        Some("child.example")
    );
    assert!(!navigation.requested_document_differs_from_current);
    assert!(!navigation.navigation_was_redirected);
    assert_eq!(navigation.navigation_redirect_count, 0);
    assert_eq!(
        navigation.navigation_transition_kind,
        super::cookie_manager_surface::BrowserContextCookieManagerNavigationTransitionKind::DirectNavigation
    );
    assert_eq!(
        navigation.requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.schemeful_requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        context.site_for_cookies_url.as_ref().map(Url::as_str),
        Some("https://embedder.example/frame")
    );
    assert_eq!(
        context.site_for_cookies_site.as_deref(),
        Some("embedder.example")
    );
    assert_eq!(
        context.site_for_cookies_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        context.schemeful_site_for_cookies_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        context.site_for_cookies_source,
        moli_cookie_jar::StoredCookieBrowserContextValueSource::FacadeOverride
    );
    assert_eq!(
        context.top_frame_origin_url.as_ref().map(Url::as_str),
        Some("https://top.example/root")
    );
    assert_eq!(
        context.top_frame_origin_site.as_deref(),
        Some("top.example")
    );
    assert_eq!(
        context.top_frame_origin_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        context.schemeful_top_frame_origin_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        context.document_frame_relationship,
        Some(
            super::cookie_manager_surface::BrowserContextCookieManagerDocumentFrameRelationship::CrossSiteSubframe
        )
    );
    assert_eq!(
        context.schemeful_document_frame_relationship,
        Some(
            super::cookie_manager_surface::BrowserContextCookieManagerDocumentFrameRelationship::CrossSiteSubframe
        )
    );
    assert_eq!(
        context.top_frame_origin_source,
        moli_cookie_jar::StoredCookieBrowserContextValueSource::FacadeOverride
    );
    assert_eq!(
        context.storage_access_status,
        moli_cookie_jar::BrowserCookieStorageAccessStatus::Granted
    );
    assert_eq!(
        context.storage_access_source,
        moli_cookie_jar::StoredCookieBrowserContextValueSource::FacadeOverride
    );
}

#[tokio::test]
async fn browser_context_cookie_manager_surface_tracks_schemeful_site_relationships() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-manager-schemeful-context",
        "TID-cookie-manager-schemeful-context",
    ));
    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://app.example.com/app").unwrap(),
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

    conn.browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_browser_context_overrides_async(
            &BrowserCookieFacadeContextOverrides::default()
                .with_site_for_cookies_url(&Url::parse("http://img.example.com/frame").unwrap())
                .with_top_frame_origin_url(&Url::parse("https://shell.example.com/root").unwrap()),
        )
        .await;

    let context = conn
        .browser_context
        .as_mut()
        .unwrap()
        .cookie_manager_surface_snapshot_async()
        .await
        .effective_context
        .expect("live page should project effective manager context");
    let navigation = &context.navigation;

    assert!(!navigation.requested_document_differs_from_current);
    assert_eq!(navigation.navigation_initiator_url, None);
    assert_eq!(navigation.navigation_initiator_site, None);
    assert_eq!(navigation.navigation_initiator_requested_relationship, None);
    assert_eq!(
        navigation.schemeful_navigation_initiator_requested_relationship,
        None
    );
    assert_eq!(
        navigation.effective_navigation_relationship_source,
        super::cookie_manager_surface::BrowserContextCookieManagerEffectiveNavigationRelationshipSource::RequestedDocument
    );
    assert_eq!(
        navigation.effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.schemeful_effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(navigation.navigation_initiator_relationship, None);
    assert_eq!(navigation.schemeful_navigation_initiator_relationship, None);
    assert!(!navigation.navigation_was_redirected);
    assert_eq!(navigation.navigation_redirect_count, 0);
    assert_eq!(
        navigation.navigation_transition_kind,
        super::cookie_manager_surface::BrowserContextCookieManagerNavigationTransitionKind::DirectNavigation
    );
    assert_eq!(
        navigation.requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.schemeful_requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        context.site_for_cookies_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        context.schemeful_site_for_cookies_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        context.top_frame_origin_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        context.schemeful_top_frame_origin_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        context.document_frame_relationship,
        Some(
            super::cookie_manager_surface::BrowserContextCookieManagerDocumentFrameRelationship::SameSiteSubframe
        )
    );
    assert_eq!(
        context.schemeful_document_frame_relationship,
        Some(
            super::cookie_manager_surface::BrowserContextCookieManagerDocumentFrameRelationship::SameSiteSubframe
        )
    );
}

#[tokio::test]
async fn browser_context_cookie_manager_surface_tracks_redirected_navigation_transition() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-manager-redirected-navigation",
        "TID-cookie-manager-redirected-navigation",
    ));
    let requested_url = Url::parse("https://origin.example/start").unwrap();
    let final_url = Url::parse("https://redirected.example/final").unwrap();
    let mut response = moli_core::page::NavigationResponse::from_text_body(
        final_url.clone(),
        200,
        Vec::new(),
        "<!doctype html><html><body>ok</body></html>".into(),
    );
    response.redirected = true;
    response.redirect_chain = vec![moli_core::page::NavigationRedirect {
        from_url: requested_url.clone(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("location".into(), final_url.as_str().into())],
        network_extra_info_available: true,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: true,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }];
    let navigation = conn
        .build_navigation_from_network_response_async(
            requested_url,
            "GET".into(),
            Vec::new(),
            moli_fetch::NetworkFetchResult::without_request_observation(response),
        )
        .await
        .expect("redirected navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_loaded_page_async(navigation.page)
        .await;

    let context = conn
        .browser_context
        .as_mut()
        .unwrap()
        .cookie_manager_surface_snapshot_async()
        .await
        .effective_context
        .expect("live page should project effective manager context");
    let navigation = &context.navigation;

    assert!(navigation.requested_document_differs_from_current);
    assert!(navigation.navigation_was_redirected);
    assert_eq!(navigation.navigation_redirect_count, 1);
    assert_eq!(
        navigation.navigation_transition_kind,
        super::cookie_manager_surface::BrowserContextCookieManagerNavigationTransitionKind::RedirectedNavigation
    );
    assert_eq!(
        navigation.effective_navigation_relationship_source,
        super::cookie_manager_surface::BrowserContextCookieManagerEffectiveNavigationRelationshipSource::RequestedDocument
    );
    assert_eq!(
        navigation.effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite
    );
    assert_eq!(
        navigation.schemeful_effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite
    );
    assert_eq!(
        navigation.requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite
    );
    assert_eq!(
        navigation.schemeful_requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite
    );
}

#[tokio::test]
async fn browser_context_cookie_manager_surface_distinguishes_same_document_url_updates_from_redirects()
 {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-manager-same-document-transition",
        "TID-cookie-manager-same-document-transition",
    ));
    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://app.example.test/app").unwrap(),
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

    conn.evaluate_runtime_expression_with_await_async(
        "history.pushState({}, '', '/next'); 'ok';",
        false,
    )
    .await
    .expect("same-document url update should succeed");

    let context = conn
        .browser_context
        .as_mut()
        .unwrap()
        .cookie_manager_surface_snapshot_async()
        .await
        .effective_context
        .expect("live page should project effective manager context");
    let navigation = &context.navigation;

    assert_eq!(
        navigation.requested_document_url.as_str(),
        "https://app.example.test/app"
    );
    assert_eq!(
        navigation.current_document_url.as_str(),
        "https://app.example.test/next"
    );
    assert!(navigation.requested_document_differs_from_current);
    assert!(!navigation.navigation_was_redirected);
    assert_eq!(navigation.navigation_redirect_count, 0);
    assert_eq!(
        navigation.navigation_transition_kind,
        super::cookie_manager_surface::BrowserContextCookieManagerNavigationTransitionKind::SameDocumentUrlUpdate
    );
    assert_eq!(
        navigation.effective_navigation_relationship_source,
        super::cookie_manager_surface::BrowserContextCookieManagerEffectiveNavigationRelationshipSource::RequestedDocument
    );
    assert_eq!(
        navigation.effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.schemeful_effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
    assert_eq!(
        navigation.schemeful_requested_document_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite
    );
}

#[tokio::test]
async fn browser_context_cookie_manager_surface_projects_navigation_initiator_relationships() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-cookie-manager-navigation-initiator",
        "TID-cookie-manager-navigation-initiator",
    ));

    let initial_navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://initiator.example/home").unwrap(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>start</body></html>".into(),
        )
        .await
        .expect("initial navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_loaded_page_async(initial_navigation.page)
        .await;

    let next_navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            Url::parse("https://target.example/app").unwrap(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>next</body></html>".into(),
        )
        .await
        .expect("next navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_loaded_page_async(next_navigation.page)
        .await;

    let context = conn
        .browser_context
        .as_mut()
        .unwrap()
        .cookie_manager_surface_snapshot_async()
        .await
        .effective_context
        .expect("live page should project effective manager context");
    let navigation = &context.navigation;

    assert_eq!(
        navigation
            .navigation_initiator_url
            .as_ref()
            .map(Url::as_str),
        Some("https://initiator.example/home")
    );
    assert_eq!(
        navigation.effective_navigation_relationship_source,
        super::cookie_manager_surface::BrowserContextCookieManagerEffectiveNavigationRelationshipSource::Initiator
    );
    assert_eq!(
        navigation.effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite
    );
    assert_eq!(
        navigation.schemeful_effective_navigation_relationship,
        super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite
    );
    assert_eq!(
        navigation.navigation_initiator_site.as_deref(),
        Some("initiator.example")
    );
    assert_eq!(
        navigation.navigation_initiator_requested_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        navigation.schemeful_navigation_initiator_requested_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        navigation.navigation_initiator_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        navigation.schemeful_navigation_initiator_relationship,
        Some(super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::CrossSite)
    );
    assert_eq!(
        navigation.navigation_transition_kind,
        super::cookie_manager_surface::BrowserContextCookieManagerNavigationTransitionKind::DirectNavigation
    );
}
