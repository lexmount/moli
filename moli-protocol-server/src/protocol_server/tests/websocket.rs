use super::*;
use std::path::Path;

fn assert_cdp_event_precedes_response(
    messages: &[serde_json::Value],
    method: &str,
    response_id: u64,
) {
    let event_index = messages
        .iter()
        .position(|message| message["method"] == json!(method))
        .unwrap_or_else(|| panic!("expected {method} before response {response_id}: {messages:?}"));
    let response_index = messages
        .iter()
        .position(|message| message["id"] == json!(response_id))
        .unwrap_or_else(|| panic!("expected response {response_id}: {messages:?}"));
    assert!(
        event_index < response_index,
        "{method} must precede CDP response {response_id}: {messages:?}"
    );
}

async fn wait_for_cookie_profile(
    path: &Path,
    predicate: impl Fn(&[StoredCookie]) -> bool,
) -> Vec<StoredCookie> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let cookies = if path.exists() {
            cookie_cache::load_cookie_cache(path).expect("profiled cookie cache should load")
        } else {
            Vec::new()
        };
        if predicate(&cookies) {
            return cookies;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for cookie profile predicate; last cookies: {cookies:?}"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_profile_lock_release(paths: &BrowserProfilePaths) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        // Unix advisory locks keep the metadata file after drop; reacquiring
        // the lock is the observable release signal across platforms.
        match BrowserProfileLock::acquire(paths) {
            Ok(lock) => {
                drop(lock);
                return;
            }
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for profile lock release: {}; last error: {error}",
                    paths.lock_path.display()
                );
            }
        }
        sleep(Duration::from_millis(25)).await;
    }
}

#[derive(Clone, Copy)]
enum ProfileCookieDeleteCommand {
    NetworkClearBrowserCookies,
    StorageClearCookies,
    StorageDeleteCookies,
    StorageClearDataForOrigin,
}

impl ProfileCookieDeleteCommand {
    fn label(self) -> &'static str {
        match self {
            Self::NetworkClearBrowserCookies => "Network.clearBrowserCookies",
            Self::StorageClearCookies => "Storage.clearCookies",
            Self::StorageDeleteCookies => "Storage.deleteCookies",
            Self::StorageClearDataForOrigin => "Storage.clearDataForOrigin",
        }
    }

    fn temp_name(self) -> &'static str {
        match self {
            Self::NetworkClearBrowserCookies => "cookie-profile-network-clear-browser-cookies",
            Self::StorageClearCookies => "cookie-profile-storage-clear-cookies",
            Self::StorageDeleteCookies => "cookie-profile-storage-delete-cookies",
            Self::StorageClearDataForOrigin => "cookie-profile-storage-clear-data-origin",
        }
    }

    fn method(self) -> &'static str {
        self.label()
    }

    fn params(self, page_origin: &str) -> serde_json::Value {
        match self {
            Self::NetworkClearBrowserCookies | Self::StorageClearCookies => json!({}),
            Self::StorageDeleteCookies => json!({ "name": "sid" }),
            Self::StorageClearDataForOrigin => {
                json!({ "origin": page_origin, "storageTypes": "cookies" })
            }
        }
    }
}

async fn assert_profile_cookie_delete_command_persists_across_restart(
    command: ProfileCookieDeleteCommand,
) {
    let profile = TempDir::new(command.temp_name());
    let paths = BrowserProfilePaths::new(&profile.path);
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_origin = format!("http://{fixture_addr}");
    let page_url = format!("{page_origin}/page");

    let (cdp_addr, cdp_server) =
        spawn_profiled_test_protocol_server_with_cookie_profile(profile.path.clone(), Vec::new())
            .await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap_or_else(|error| panic!("connect to profiled cookie cdp websocket: {error}"));
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        "document.cookie = 'sid=keep; path=/'; document.cookie",
    )
    .await;
    assert!(
        write.contains("sid=keep"),
        "{} scenario document.cookie after write: {write}",
        command.label()
    );
    let protocol_cookies = send_cdp_command(
        &mut socket,
        7,
        "Storage.getCookies",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        protocol_cookies.iter().any(|message| {
            message["id"] == json!(7_u64)
                && message["result"]["cookies"]
                    .as_array()
                    .is_some_and(|cookies| {
                        cookies.iter().any(|cookie| {
                            cookie["name"] == json!("sid") && cookie["value"] == json!("keep")
                        })
                    })
        }),
        "{} scenario Storage.getCookies should see document.cookie write: {protocol_cookies:?}",
        command.label()
    );
    let _ = socket.close(None).await;
    let persisted = wait_for_cookie_profile(&paths.cookies_path, |cookies| {
        cookies
            .iter()
            .any(|cookie| cookie.name == "sid" && cookie.value == "keep")
    })
    .await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    assert!(
        persisted
            .iter()
            .any(|cookie| cookie.name == "sid" && cookie.value == "keep"),
        "{} scenario cookie profile should contain sid after first shutdown: {persisted:?}",
        command.label()
    );

    let (cdp_addr, cdp_server) =
        spawn_profiled_test_protocol_server_with_cookie_profile(profile.path.clone(), Vec::new())
            .await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap_or_else(|error| panic!("reconnect to profiled cookie cdp websocket: {error}"));
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read = cdp_runtime_evaluate_string(&mut socket, &session_id, 6, "document.cookie").await;
    assert!(
        read.contains("sid=keep"),
        "{} scenario document.cookie should restore profile cookie: {read}",
        command.label()
    );
    let delete_response = send_cdp_command(
        &mut socket,
        7,
        command.method(),
        Some(&session_id),
        command.params(&page_origin),
    )
    .await;
    assert!(
        delete_response
            .iter()
            .any(|message| message["id"] == json!(7_u64) && message.get("result").is_some()),
        "{} scenario delete command should return success: {delete_response:?}",
        command.label()
    );
    let _ = socket.close(None).await;
    let after_clear = wait_for_cookie_profile(&paths.cookies_path, |cookies| {
        cookies.iter().all(|cookie| cookie.name != "sid")
    })
    .await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    assert!(
        after_clear.iter().all(|cookie| cookie.name != "sid"),
        "{} should remove sid from profile: {after_clear:?}",
        command.label()
    );

    let (cdp_addr, cdp_server) =
        spawn_profiled_test_protocol_server_with_cookie_profile(profile.path.clone(), Vec::new())
            .await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap_or_else(|error| panic!("reconnect after cookie clear: {error}"));
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read_after_restart =
        cdp_runtime_evaluate_string(&mut socket, &session_id, 6, "document.cookie").await;
    assert_eq!(
        read_after_restart,
        "",
        "{} scenario should not restore deleted profile cookie",
        command.label()
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

mod dialogs_and_target_lifecycle;
mod frame_network_and_automation_events;
mod navigation_and_load_progress;
mod parser_replacement_and_debugger;
mod profiles_cache_and_interrupts;
