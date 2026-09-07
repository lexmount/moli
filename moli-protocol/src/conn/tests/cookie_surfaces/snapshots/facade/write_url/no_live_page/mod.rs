use super::*;

mod capability_surface;
mod freshness;
mod structured_write;

async fn configured_connection_without_live_page() -> CdpConnection {
    let mut conn = crate::test_support::connection();
    conn.browser_context = Some(conn.new_browser_context_fixture_for_test("BID-cookie-facade"));
    conn.browser_context
        .as_mut()
        .unwrap()
        .apply_cookie_manager_policy_overrides_async(
            &BrowserCookieFacadeOverrides::default()
                .with_cookies_enabled(false)
                .with_storage_access_status(
                    moli_cookie_jar::BrowserCookieStorageAccessStatus::Granted,
                ),
        )
        .await;
    conn
}
