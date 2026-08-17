use super::*;
#[tokio::test]
async fn browser_context_document_cookie_facade_overrides_apply_to_new_loaded_page() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie-facade".into()));
    conn.browser_context
        .as_mut()
        .unwrap()
        .apply_cookie_manager_policy_overrides_async(
            &BrowserCookieFacadeOverrides::default().with_cookies_enabled(false),
        )
        .await;

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
        .expect("expression should evaluate");
    assert_eq!(payload["value"], json!(false));
    assert_eq!(
        conn.browser_context
            .as_mut()
            .unwrap()
            .cookie_manager_surface_snapshot_async()
            .await
            .policy
            .overrides
            .cookies_enabled,
        Some(false)
    );
    assert_eq!(
        conn.browser_context
            .as_mut()
            .unwrap()
            .cookie_manager_surface_snapshot_async()
            .await
            .policy
            .overrides
            .cookies_enabled,
        Some(false)
    );
}
#[tokio::test]
async fn browser_context_document_cookie_facade_overrides_update_live_page() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie-facade".into()));
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
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("expression should evaluate");
    assert_eq!(before["value"], json!(true));

    conn.browser_context
        .as_mut()
        .unwrap()
        .apply_cookie_manager_policy_overrides_async(
            &BrowserCookieFacadeOverrides::default().with_cookies_enabled(false),
        )
        .await;
    let after_apply = conn
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("expression should evaluate");
    assert_eq!(after_apply["value"], json!(false));
    let blocked_snapshot = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        blocked_snapshot.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::PolicyBlocked
    );
    assert_eq!(
        blocked_snapshot.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::PolicyBlocked
    );

    conn.browser_context
        .as_mut()
        .unwrap()
        .clear_cookie_manager_policy_overrides_async()
        .await;
    let after_clear = conn
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("expression should evaluate");
    assert_eq!(after_clear["value"], json!(true));
}

#[tokio::test]
async fn browser_context_document_cookie_browser_context_overrides_update_live_page() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie-context-overrides".into()));
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
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("expression should evaluate");
    assert_eq!(before["value"], json!(true));

    let cross_site = Url::parse("https://other.test/embedder").unwrap();
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_browser_context_overrides_async(
            &BrowserCookieFacadeContextOverrides::default()
                .with_site_for_cookies_url(&cross_site)
                .with_top_frame_origin_url(&cross_site),
        )
        .await;

    let after_apply = conn
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("expression should evaluate");
    assert_eq!(after_apply["value"], json!(false));
    let blocked_snapshot = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        blocked_snapshot
            .capability_surface
            .manager_surface
            .policy
            .browser_context_overrides,
        BrowserCookieFacadeContextOverrides::default()
            .with_site_for_cookies_url(&cross_site)
            .with_top_frame_origin_url(&cross_site)
    );
    assert_eq!(
        blocked_snapshot.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::PolicyBlocked
    );
    assert_eq!(
        blocked_snapshot.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::PolicyBlocked
    );

    conn.browser_context
        .as_mut()
        .unwrap()
        .clear_cookie_manager_policy_browser_context_overrides_async()
        .await;
    let after_clear = conn
        .evaluate_runtime_expression_with_await_async("navigator.cookieEnabled", false)
        .await
        .expect("expression should evaluate");
    assert_eq!(after_clear["value"], json!(true));
}

#[tokio::test]
async fn browser_context_document_cookie_policy_surface_tracks_override_generation() {
    let mut bc = BrowserContext::new("BID-cookie-policy".into());

    let initial = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(initial.generation, 0);
    assert_eq!(initial.overrides, BrowserCookieFacadeOverrides::default());

    let overrides = BrowserCookieFacadeOverrides::default().with_cookies_enabled(false);
    bc.apply_cookie_manager_policy_overrides_async(&overrides)
        .await;
    let after_apply = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(after_apply.generation, 1);
    assert_eq!(after_apply.overrides, overrides);

    bc.apply_cookie_manager_policy_overrides_async(&overrides)
        .await;
    let after_noop = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(after_noop.generation, 1);
    assert_eq!(after_noop.overrides, overrides);

    bc.clear_cookie_manager_policy_overrides_async().await;
    let after_clear = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(after_clear.generation, 2);
    assert_eq!(
        after_clear.overrides,
        BrowserCookieFacadeOverrides::default()
    );
}

#[tokio::test]
async fn browser_context_document_cookie_policy_surface_tracks_split_overrides() {
    let mut bc = BrowserContext::new("BID-cookie-policy-split".into());
    let override_url = Url::parse("https://embedder.example/root").unwrap();

    bc.set_cookie_manager_policy_cookies_enabled_override_async(false)
        .await;
    bc.set_cookie_manager_policy_browser_context_overrides_async(
        &BrowserCookieFacadeContextOverrides::default()
            .with_site_for_cookies_url(&override_url)
            .with_top_frame_origin_url(&override_url)
            .with_storage_access_status(moli_cookie_jar::BrowserCookieStorageAccessStatus::Granted),
    )
    .await;

    let snapshot = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(snapshot.cookies_enabled_override, Some(false));
    assert_eq!(
        snapshot.browser_context_overrides,
        BrowserCookieFacadeContextOverrides::default()
            .with_site_for_cookies_url(&override_url)
            .with_top_frame_origin_url(&override_url)
            .with_storage_access_status(moli_cookie_jar::BrowserCookieStorageAccessStatus::Granted)
    );

    bc.clear_cookie_manager_policy_browser_context_overrides_async()
        .await;
    let after_browser_context_clear = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(
        after_browser_context_clear.cookies_enabled_override,
        Some(false)
    );
    assert_eq!(
        after_browser_context_clear.browser_context_overrides,
        BrowserCookieFacadeContextOverrides::default()
    );

    bc.clear_cookie_manager_policy_cookies_enabled_override_async()
        .await;
    let after_all_clear = bc.cookie_manager_surface_snapshot().policy;
    assert_eq!(after_all_clear.cookies_enabled_override, None);
    assert_eq!(
        after_all_clear.browser_context_overrides,
        BrowserCookieFacadeContextOverrides::default()
    );
}

#[tokio::test]
async fn browser_context_cookie_manager_surface_tracks_policy_without_a_live_page() {
    let mut bc = BrowserContext::new("BID-cookie-manager".into());
    let initial = bc.cookie_manager_surface_snapshot();
    assert_eq!(initial.policy.generation, 0);
    assert_eq!(
        initial.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
    bc.apply_cookie_manager_policy_overrides_async(
        &BrowserCookieFacadeOverrides::default().with_cookies_enabled(false),
    )
    .await;
    let after_policy_change = bc.cookie_manager_surface_snapshot();
    assert_eq!(after_policy_change.policy.generation, 1);
    assert_eq!(
        after_policy_change.backend_connection_state,
        BrowserContextCookieBackendConnectionState::NoLivePage
    );
}

#[tokio::test]
async fn browser_context_document_cookie_capability_and_freshness_snapshots_project_owner_surfaces()
{
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie-capability-surface".into()));
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

    conn.browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_cookies_enabled_override_async(false)
        .await;

    let capability = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_capability_surface_snapshot_async()
        .await;
    assert_eq!(
        capability.manager_surface.policy.cookies_enabled_override,
        Some(false)
    );
    assert_eq!(
        capability.manager_surface.capability.cookie_access_enabled,
        Some(false)
    );
    assert_eq!(
        capability.manager_surface.capability.cookie_access_verdict,
        super::cookie_manager_surface::BrowserContextCookieManagerAccessVerdict::Blocked(
            moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .policy_gating
            .cookie_access_policy_verdict,
        super::cookie_manager_surface::BrowserContextCookieManagerAccessVerdict::Blocked(
            moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .policy_gating
            .cookie_access_primary_block_reason,
        Some(moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled)
    );
    assert_eq!(
        capability
            .manager_surface
            .policy_gating
            .cookie_access_blocked_reasons,
        vec![moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled]
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_gating
            .cookie_access_policy_verdict,
        super::cookie_manager_surface::BrowserContextCookieManagerAccessVerdict::Blocked(
            moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_gating
            .cookie_access_primary_block_reason,
        Some(moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_gating
            .cookie_access_blocked_reasons,
        vec![moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled]
    );
    assert_eq!(
        capability.manager_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        capability
            .manager_surface
            .capability
            .cookies_enabled_preference,
        Some(false)
    );
    assert_eq!(
        capability.manager_surface.capability.store_available,
        Some(true)
    );
    assert_eq!(
        capability
            .manager_surface
            .capability
            .cookie_access_primary_block_reason,
        Some(moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled)
    );
    assert_eq!(
        capability
            .manager_surface
            .capability
            .cookie_access_blocked_reasons,
        vec![moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled]
    );
    assert_eq!(
        capability.manager_surface.capability.cookie_write_enabled,
        Some(false)
    );
    assert_eq!(
        capability.manager_surface.capability.cookie_write_verdict,
        super::cookie_manager_surface::BrowserContextCookieManagerWriteVerdict::Blocked(
            moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .policy_gating
            .cookie_write_policy_verdict,
        super::cookie_manager_surface::BrowserContextCookieManagerWriteVerdict::Blocked(
            moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .policy_gating
            .cookie_write_primary_rejection_reason,
        Some(moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled)
    );
    assert_eq!(
        capability
            .manager_surface
            .policy_gating
            .cookie_write_blocked_reasons,
        vec![moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled]
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_gating
            .cookie_write_policy_verdict,
        super::cookie_manager_surface::BrowserContextCookieManagerWriteVerdict::Blocked(
            moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_gating
            .cookie_write_primary_rejection_reason,
        Some(moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_gating
            .cookie_write_blocked_reasons,
        vec![moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled]
    );
    assert_eq!(
        capability
            .manager_surface
            .capability
            .cookie_write_primary_rejection_reason,
        Some(moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled)
    );
    assert_eq!(
        capability.manager_surface.capability.view_generation,
        capability
            .capability
            .as_ref()
            .map(|capability| capability.view_generation)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.navigation.current_document_url.as_str()),
        Some("https://example.com/app")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.navigation.current_document_site.as_deref()),
        Some("example.com")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.navigation.requested_document_url.as_str()),
        Some("https://example.com/app")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.navigation.requested_document_site.as_deref()),
        Some("example.com")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.navigation.requested_document_differs_from_current),
        Some(false)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| &context.navigation.requested_document_relationship),
        Some(&super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| &context.navigation.schemeful_requested_document_relationship),
        Some(&super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.site_for_cookies_url.as_ref())
            .map(Url::as_str),
        Some("https://example.com/app")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.site_for_cookies_site.as_deref()),
        Some("example.com")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.site_for_cookies_relationship.as_ref()),
        Some(&super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.schemeful_site_for_cookies_relationship.as_ref()),
        Some(&super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.document_frame_relationship.as_ref()),
        Some(
            &super::cookie_manager_surface::BrowserContextCookieManagerDocumentFrameRelationship::TopLevelDocument
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.schemeful_document_frame_relationship.as_ref()),
        Some(
            &super::cookie_manager_surface::BrowserContextCookieManagerDocumentFrameRelationship::TopLevelDocument
        )
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.site_for_cookies_source),
        Some(moli_cookie_jar::StoredCookieBrowserContextValueSource::FacadeDefault)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.top_frame_origin_url.as_ref())
            .map(Url::as_str),
        Some("https://example.com/app")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.top_frame_origin_site.as_deref()),
        Some("example.com")
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.top_frame_origin_relationship.as_ref()),
        Some(&super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .and_then(|context| context.schemeful_top_frame_origin_relationship.as_ref()),
        Some(&super::cookie_manager_surface::BrowserContextCookieManagerSiteRelationship::SameSite)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.top_frame_origin_source),
        Some(moli_cookie_jar::StoredCookieBrowserContextValueSource::FacadeDefault)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.storage_access_status),
        Some(moli_cookie_jar::BrowserCookieStorageAccessStatus::None)
    );
    assert_eq!(
        capability
            .manager_surface
            .effective_context
            .as_ref()
            .map(|context| context.storage_access_source),
        Some(moli_cookie_jar::StoredCookieBrowserContextValueSource::FacadeDefault)
    );
    assert_eq!(
        capability.write_capability.as_ref().map(|capability| (
            capability.write_enabled,
            capability.primary_rejection_reason,
            capability.blocked_reasons.clone(),
        )),
        Some((
            false,
            Some(moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled),
            vec![moli_cookie_jar::StoredCookieSetRejectionReason::CookiesDisabled],
        ))
    );
    assert_eq!(
        capability.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Attached
    );
    assert_eq!(
        capability.capability.as_ref().map(|capability| (
            capability.cookies_enabled_preference,
            capability.cookie_access_enabled,
            capability.store_available,
            capability.primary_block_reason,
            capability.blocked_reasons.clone(),
        )),
        Some((
            false,
            false,
            true,
            Some(moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled),
            vec![moli_cookie_jar::StoredCookieExclusionReason::CookiesDisabled],
        ))
    );

    let freshness = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_freshness_snapshot_async()
        .await;
    assert_eq!(
        freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::PolicyBlocked
    );
    assert_eq!(
        freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::PolicyBlocked
    );
    assert!(!freshness.cookie_get_would_need_backend_reconnect);
}

#[tokio::test]
async fn browser_context_cookie_manager_surface_projects_document_capability_and_activity() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie-manager-projection".into()));
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
    conn.browser_context
        .as_mut()
        .unwrap()
        .set_cookie_manager_policy_cookies_enabled_override_async(false)
        .await;

    let manager_surface = conn
        .browser_context
        .as_mut()
        .unwrap()
        .cookie_manager_surface_snapshot_async()
        .await;
    let capability_surface = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_capability_surface_snapshot_async()
        .await;

    assert_eq!(
        manager_surface.document_cookie_capability_snapshot(),
        capability_surface.capability
    );
    assert_eq!(
        manager_surface.document_cookie_write_capability_snapshot(),
        capability_surface.write_capability
    );
    assert_eq!(
        manager_surface.document_cookie_telemetry_snapshot(),
        capability_surface.telemetry
    );
    assert_eq!(
        manager_surface.first_cookie_request(),
        capability_surface.first_cookie_request
    );
}
