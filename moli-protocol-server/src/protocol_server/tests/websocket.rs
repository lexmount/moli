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

#[tokio::test]
async fn websocket_cdp_file_navigation_returns_stable_error_without_events_or_replacement() {
    let (cdp_addr, cdp_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect browser CDP websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;

    let _ = send_cdp_command(
        &mut socket,
        4,
        "Page.enable",
        Some(&target.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Network.enable",
        Some(&target.session_id),
        json!({}),
    )
    .await;

    let rejected = send_cdp_command(
        &mut socket,
        6,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": "file:///moli-policy-must-not-open" }),
    )
    .await;
    let response = rejected
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("rejected Page.navigate response");
    assert_eq!(response["sessionId"], json!(target.session_id));
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Navigation to a local file URL requires an explicitly granted browser capability.")
    );
    assert!(
        rejected.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Page.frameStartedNavigating")
                    | Some("Page.frameStartedLoading")
                    | Some("Page.domContentEventFired")
                    | Some("Page.loadEventFired")
                    | Some("Network.requestWillBeSent")
                    | Some("Network.loadingFailed")
            )
        }),
        "rejected file navigation must not emit CDP load events: {rejected:?}"
    );

    let location_messages = send_cdp_command(
        &mut socket,
        7,
        "Runtime.evaluate",
        Some(&target.session_id),
        json!({
            "expression": "location.href",
            "returnByValue": true
        }),
    )
    .await;
    assert!(
        location_messages.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Page.frameStartedNavigating")
                    | Some("Page.frameStartedLoading")
                    | Some("Page.domContentEventFired")
                    | Some("Page.loadEventFired")
                    | Some("Network.requestWillBeSent")
                    | Some("Network.loadingFailed")
            )
        }),
        "rejected file navigation must not leak delayed CDP events: {location_messages:?}"
    );
    let location = location_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate location response");
    assert_eq!(location["result"]["result"]["value"], json!("about:blank"));

    let _ = send_cdp_command(
        &mut socket,
        8,
        "Target.closeTarget",
        None,
        json!({ "targetId": target.target_id }),
    )
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
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

#[tokio::test]
async fn websocket_cdp_localstorage_profile_persists_across_server_restart() {
    let profile = TempDir::new("localstorage-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to profiled cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        "localStorage.clear(); sessionStorage.clear(); localStorage.setItem('persisted', 'yes'); sessionStorage.setItem('ephemeral', 'yes'); 'ok'",
    )
    .await;
    assert_eq!(write, "ok");
    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;

    let persisted = std::fs::read_to_string(&paths.local_storage_path)
        .expect("profiled localStorage json should be written");
    assert!(
        persisted.contains("\"persisted\"") && persisted.contains("\"yes\""),
        "profile file should contain persisted localStorage entry: {persisted}"
    );

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("reconnect to profiled cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        "`${localStorage.getItem('persisted')}|${String(sessionStorage.getItem('ephemeral'))}`",
    )
    .await;
    assert_eq!(read, "yes|null");

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_imported_cookies_with_profile_dir_persist_across_server_restart() {
    let profile = TempDir::new("imported-cookie-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");
    let mut imported = stored_cookie("session", "fixture");
    imported.domain = fixture_addr.ip().to_string();
    imported.host_only = true;

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server_with_cookie_profile(
        profile.path.clone(),
        vec![imported],
    )
    .await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to profiled cdp websocket with imported cookies");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read_imported =
        cdp_runtime_evaluate_string(&mut socket, &session_id, 6, "document.cookie").await;
    assert!(
        read_imported.contains("session=fixture"),
        "default context should see imported cookie: {read_imported}"
    );
    let _ = socket.close(None).await;
    let persisted = wait_for_cookie_profile(&paths.cookies_path, |cookies| {
        cookies
            .iter()
            .any(|cookie| cookie.name == "session" && cookie.value == "fixture")
    })
    .await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    assert!(
        persisted
            .iter()
            .any(|cookie| cookie.name == "session" && cookie.value == "fixture"),
        "profile should contain imported cookie after socket shutdown: {persisted:?}"
    );

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("reconnect to profiled cdp websocket without imported cookies");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read_profile =
        cdp_runtime_evaluate_string(&mut socket, &session_id, 6, "document.cookie").await;
    assert!(
        read_profile.contains("session=fixture"),
        "default context should restore imported cookie from profile: {read_profile}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_indexeddb_profile_persists_across_server_restart() {
    let profile = TempDir::new("indexeddb-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to profiled cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        r#"
(() => {
  globalThis.__idbProfileWrite = "pending";
  const open = indexedDB.open("profile-db", 1);
  open.onerror = () => {
    globalThis.__idbProfileWrite = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const put = tx.objectStore("kv").put("persisted", "answer");
    put.onerror = () => {
      globalThis.__idbProfileWrite = `put-error:${put.error && put.error.name}`;
    };
    tx.oncomplete = () => {
      db.close();
      globalThis.__idbProfileWrite = "stored";
    };
    tx.onerror = () => {
      globalThis.__idbProfileWrite = `tx-error:${tx.error && tx.error.name}`;
    };
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(write, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &session_id,
        7,
        "String(globalThis.__idbProfileWrite)",
        "stored",
    )
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;

    assert!(
        std::fs::read_dir(&paths.indexeddb_root)
            .expect("profiled IndexedDB root should exist")
            .next()
            .is_some(),
        "profiled IndexedDB root should contain persisted origin data"
    );

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("reconnect to profiled cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        8,
        r#"
(() => {
  globalThis.__idbProfileRead = "pending";
  const open = indexedDB.open("profile-db", 1);
  open.onerror = () => {
    globalThis.__idbProfileRead = `open-error:${open.error && open.error.name}`;
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readonly");
    const get = tx.objectStore("kv").get("answer");
    get.onsuccess = () => {
      globalThis.__idbProfileRead = String(get.result);
    };
    get.onerror = () => {
      globalThis.__idbProfileRead = `get-error:${get.error && get.error.name}`;
    };
    tx.oncomplete = () => db.close();
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(read, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &session_id,
        9,
        "String(globalThis.__idbProfileRead)",
        "persisted",
    )
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_clear_data_for_origin_indexeddb_persists_across_server_restart() {
    let profile = TempDir::new("indexeddb-clear-data-origin-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_origin = format!("http://{fixture_addr}");
    let page_url = format!("{page_origin}/page");

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to profiled cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        r#"
(() => {
  globalThis.__idbClearWrite = "pending";
  const open = indexedDB.open("profile-clear-db", 1);
  open.onerror = () => {
    globalThis.__idbClearWrite = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const put = tx.objectStore("kv").put("persisted", "answer");
    put.onerror = () => {
      globalThis.__idbClearWrite = `put-error:${put.error && put.error.name}`;
    };
    tx.oncomplete = () => {
      db.close();
      globalThis.__idbClearWrite = "stored";
    };
    tx.onerror = () => {
      globalThis.__idbClearWrite = `tx-error:${tx.error && tx.error.name}`;
    };
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(write, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &session_id,
        7,
        "String(globalThis.__idbClearWrite)",
        "stored",
    )
    .await;

    let clear = send_cdp_command(
        &mut socket,
        8,
        "Storage.clearDataForOrigin",
        Some(&session_id),
        json!({
            "origin": page_origin,
            "storageTypes": "indexeddb",
        }),
    )
    .await;
    let clear_response = clear
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("clearDataForOrigin response");
    assert_eq!(clear_response["result"], json!({}));

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("reconnect to profiled cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        9,
        r#"
(() => {
  globalThis.__idbClearRead = "pending";
  let oldVersion = "no-upgrade";
  const open = indexedDB.open("profile-clear-db", 1);
  open.onerror = () => {
    globalThis.__idbClearRead = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = (event) => {
    oldVersion = String(event.oldVersion);
    open.result.createObjectStore("fresh");
  };
  open.onsuccess = () => {
    const db = open.result;
    globalThis.__idbClearRead = [
      oldVersion,
      String(db.objectStoreNames.contains("kv")),
      String(db.objectStoreNames.contains("fresh"))
    ].join("|");
    db.close();
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(read, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &session_id,
        10,
        "String(globalThis.__idbClearRead)",
        "0|false|true",
    )
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_cookie_profile_delete_commands_persist_across_server_restart() {
    for command in [
        ProfileCookieDeleteCommand::NetworkClearBrowserCookies,
        ProfileCookieDeleteCommand::StorageClearCookies,
        ProfileCookieDeleteCommand::StorageDeleteCookies,
        ProfileCookieDeleteCommand::StorageClearDataForOrigin,
    ] {
        assert_profile_cookie_delete_command_persists_across_restart(command).await;
    }
}

#[tokio::test]
async fn websocket_cdp_ephemeral_context_cookie_changes_do_not_clear_cookie_profile() {
    let profile = TempDir::new("ephemeral-cookie-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    cookie_cache::save_cookie_cache(&paths.cookies_path, vec![stored_cookie("sid", "profile")])
        .expect("seed cookie profile");

    let (cdp_addr, cdp_server) =
        spawn_profiled_test_protocol_server_with_cookie_profile(profile.path.clone(), Vec::new())
            .await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to profiled cdp websocket");

    let context_id = cdp_create_browser_context(&mut socket, 1).await;
    let cookies = send_cdp_command(
        &mut socket,
        2,
        "Storage.getCookies",
        None,
        json!({ "browserContextId": context_id }),
    )
    .await;
    assert!(
        cookies.iter().any(|message| {
            message["id"] == json!(2_u64) && message["result"]["cookies"] == json!([])
        }),
        "ephemeral context should not inherit profile cookies: {cookies:?}"
    );

    let _ = socket.close(None).await;
    sleep(Duration::from_millis(100)).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;

    let persisted = cookie_cache::load_cookie_cache(&paths.cookies_path)
        .expect("profile cookie cache should survive ephemeral-only connection");
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].name, "sid");
    assert_eq!(persisted[0].value, "profile");
}

#[tokio::test]
async fn websocket_cdp_browser_contexts_isolate_localstorage() {
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");
    let (cdp_addr, cdp_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let context_a = cdp_create_browser_context(&mut socket, 1).await;
    let context_b = cdp_create_browser_context(&mut socket, 2).await;
    let target_a = cdp_create_attached_target(&mut socket, 3, &context_a).await;
    let target_b = cdp_create_attached_target(&mut socket, 5, &context_b).await;
    cdp_navigate_and_wait_for_load(&mut socket, 7, &target_a.session_id, &page_url).await;
    cdp_navigate_and_wait_for_load(&mut socket, 8, &target_b.session_id, &page_url).await;

    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        9,
        "localStorage.clear(); localStorage.setItem('ctx', 'a'); 'ok'",
    )
    .await;
    assert_eq!(write, "ok");

    let read_a = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        10,
        "String(localStorage.getItem('ctx'))",
    )
    .await;
    assert_eq!(read_a, "a");
    let read_b = cdp_runtime_evaluate_string(
        &mut socket,
        &target_b.session_id,
        11,
        "String(localStorage.getItem('ctx'))",
    )
    .await;
    assert_eq!(read_b, "null");

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_same_context_targets_share_localstorage_and_isolate_sessionstorage() {
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");
    let (cdp_addr, cdp_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let context_id = cdp_create_browser_context(&mut socket, 1).await;
    let target_a = cdp_create_attached_target(&mut socket, 2, &context_id).await;
    let target_b = cdp_create_attached_target(&mut socket, 4, &context_id).await;
    cdp_navigate_and_wait_for_load(&mut socket, 6, &target_a.session_id, &page_url).await;
    cdp_navigate_and_wait_for_load(&mut socket, 7, &target_b.session_id, &page_url).await;

    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        8,
        "localStorage.clear(); sessionStorage.clear(); localStorage.setItem('shared', 'yes'); sessionStorage.setItem('target', 'a'); 'ok'",
    )
    .await;
    assert_eq!(write, "ok");

    let read_b = cdp_runtime_evaluate_string(
        &mut socket,
        &target_b.session_id,
        9,
        "`${localStorage.getItem('shared')}|${String(sessionStorage.getItem('target'))}`",
    )
    .await;
    assert_eq!(read_b, "yes|null");

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_browser_contexts_isolate_cookies_sessionstorage_and_indexeddb() {
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");
    let (cdp_addr, cdp_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let context_a = cdp_create_browser_context(&mut socket, 1).await;
    let context_b = cdp_create_browser_context(&mut socket, 2).await;
    let target_a = cdp_create_attached_target(&mut socket, 3, &context_a).await;
    let target_b = cdp_create_attached_target(&mut socket, 5, &context_b).await;
    cdp_navigate_and_wait_for_load(&mut socket, 7, &target_a.session_id, &page_url).await;
    cdp_navigate_and_wait_for_load(&mut socket, 8, &target_b.session_id, &page_url).await;

    let write_cookie_and_session = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        9,
        "document.cookie = 'ctxcookie=a; path=/'; sessionStorage.setItem('ctx', 'a'); 'ok'",
    )
    .await;
    assert_eq!(write_cookie_and_session, "ok");

    let write_idb = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        10,
        r#"
(() => {
  globalThis.__idbContextWrite = "pending";
  const open = indexedDB.open("ctx-db", 1);
  open.onerror = () => {
    globalThis.__idbContextWrite = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const put = tx.objectStore("kv").put("a", "ctx");
    put.onerror = () => {
      globalThis.__idbContextWrite = `put-error:${put.error && put.error.name}`;
    };
    tx.oncomplete = () => {
      db.close();
      globalThis.__idbContextWrite = "stored";
    };
    tx.onerror = () => {
      globalThis.__idbContextWrite = `tx-error:${tx.error && tx.error.name}`;
    };
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(write_idb, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &target_a.session_id,
        11,
        "String(globalThis.__idbContextWrite)",
        "stored",
    )
    .await;

    let read_a = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        12,
        "`${document.cookie.includes('ctxcookie=a')}|${sessionStorage.getItem('ctx')}`",
    )
    .await;
    assert_eq!(read_a, "true|a");
    let read_b = cdp_runtime_evaluate_string(
        &mut socket,
        &target_b.session_id,
        13,
        "`${document.cookie.includes('ctxcookie=a')}|${String(sessionStorage.getItem('ctx'))}`",
    )
    .await;
    assert_eq!(read_b, "false|null");

    let cookies_a = send_cdp_command(
        &mut socket,
        14,
        "Storage.getCookies",
        None,
        json!({ "browserContextId": context_a }),
    )
    .await;
    assert!(
        cookies_a.iter().any(|message| {
            message["id"] == json!(14_u64)
                && message["result"]["cookies"]
                    .as_array()
                    .is_some_and(|cookies| {
                        cookies.iter().any(|cookie| {
                            cookie["name"] == json!("ctxcookie") && cookie["value"] == json!("a")
                        })
                    })
        }),
        "context A Storage.getCookies should see ctxcookie: {cookies_a:?}"
    );
    let cookies_b = send_cdp_command(
        &mut socket,
        15,
        "Storage.getCookies",
        None,
        json!({ "browserContextId": context_b }),
    )
    .await;
    assert!(
        cookies_b.iter().any(|message| {
            message["id"] == json!(15_u64) && message["result"]["cookies"] == json!([])
        }),
        "context B Storage.getCookies should not see context A cookie: {cookies_b:?}"
    );

    let read_idb_a = cdp_runtime_evaluate_string(
        &mut socket,
        &target_a.session_id,
        16,
        r#"
(() => {
  globalThis.__idbContextReadA = "pending";
  const open = indexedDB.open("ctx-db", 1);
  open.onerror = () => {
    globalThis.__idbContextReadA = `open-error:${open.error && open.error.name}`;
  };
  open.onsuccess = () => {
    const db = open.result;
    const tx = db.transaction("kv", "readonly");
    const get = tx.objectStore("kv").get("ctx");
    get.onsuccess = () => {
      globalThis.__idbContextReadA = String(get.result);
    };
    get.onerror = () => {
      globalThis.__idbContextReadA = `get-error:${get.error && get.error.name}`;
    };
    tx.oncomplete = () => db.close();
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(read_idb_a, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &target_a.session_id,
        17,
        "String(globalThis.__idbContextReadA)",
        "a",
    )
    .await;

    let read_idb_b = cdp_runtime_evaluate_string(
        &mut socket,
        &target_b.session_id,
        18,
        r#"
(() => {
  globalThis.__idbContextReadB = "pending";
  const open = indexedDB.open("ctx-db", 1);
  open.onerror = () => {
    globalThis.__idbContextReadB = `open-error:${open.error && open.error.name}`;
  };
  open.onupgradeneeded = () => {
    globalThis.__idbContextReadB = "missing";
  };
  open.onsuccess = () => {
    const db = open.result;
    if (!db.objectStoreNames.contains("kv")) {
      db.close();
      globalThis.__idbContextReadB = "missing";
      return;
    }
    const tx = db.transaction("kv", "readonly");
    const get = tx.objectStore("kv").get("ctx");
    get.onsuccess = () => {
      globalThis.__idbContextReadB = String(get.result);
    };
    get.onerror = () => {
      globalThis.__idbContextReadB = `get-error:${get.error && get.error.name}`;
    };
    tx.oncomplete = () => db.close();
  };
  return "scheduled";
})()
"#,
    )
    .await;
    assert_eq!(read_idb_b, "scheduled");
    wait_for_cdp_runtime_string(
        &mut socket,
        &target_b.session_id,
        19,
        "String(globalThis.__idbContextReadB)",
        "missing",
    )
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_clear_browser_cache_clears_configured_http_cache_dir() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-http-cache-clear-{}-{nonce}",
        std::process::id()
    ));
    let entry_dir = cache_dir.join("0123456789abcdef.entry");
    fs::create_dir_all(&entry_dir).expect("cache entry dir should be created");
    fs::write(entry_dir.join("body.test.bin"), b"cached")
        .expect("cache body fixture should be written");
    fs::write(cache_dir.join("owner.lock"), b"keep")
        .expect("unrelated cache root file should be written");

    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");
    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_fetch_config(fetch_config).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Network.clearBrowserCache",
                "sessionId": session_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send clearBrowserCache");
    let clear_response = recv_until_id(&mut socket, 6).await;
    assert!(
        clear_response
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message["result"] == json!({})),
        "clearBrowserCache should succeed: {clear_response:?}"
    );

    assert!(!entry_dir.exists());
    assert!(cache_dir.join("owner.lock").exists());

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
    let _ = fs::remove_dir_all(cache_dir);
}

#[tokio::test]
async fn websocket_cdp_create_isolated_world_resolves_during_about_blank_prewarm() {
    // Regression test for the createIsolatedWorld fast-ack defer path.
    // When createIsolatedWorld arrives while the about:blank prewarm
    // started by createTarget is still in flight, the handler must
    // return an empty reply set immediately and the deferred task must
    // produce a valid executionContextId once the prewarm resolves.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    // Send Page.createIsolatedWorld immediately — the about:blank
    // prewarm kicked off by createTarget is still in flight on the
    // renderer thread. The handler should defer and the socket loop
    // should drain the deferred completion when the prewarm resolves.
    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.createIsolatedWorld",
                "sessionId": session_id,
                "params": {
                    "frameId": target_id,
                    "worldName": "utility-deferred"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createIsolatedWorld");
    let created = recv_until_id(&mut socket, 4).await;
    let response = created
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .expect("createIsolatedWorld response");
    assert_eq!(response["sessionId"].as_str(), Some(session_id.as_str()));
    let execution_context_id = response["result"]["executionContextId"]
        .as_i64()
        .expect("executionContextId from deferred createIsolatedWorld");
    assert!(
        execution_context_id != 0,
        "executionContextId should be non-zero, got {execution_context_id}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_create_isolated_world_during_prewarm_then_close_target_does_not_panic() {
    // Race test: createIsolatedWorld arrives during prewarm, then
    // closeTarget is sent before the prewarm resolves. The deferred
    // completion must be silently dropped (target no longer current)
    // and the socket must remain healthy enough to reply to the close.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.createIsolatedWorld",
                "sessionId": session_id,
                "params": {
                    "frameId": target_id,
                    "worldName": "utility-pre-close"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createIsolatedWorld");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Target.closeTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send closeTarget");
    // Wait for the closeTarget reply. The createIsolatedWorld deferred
    // completion may or may not have arrived first; either way the
    // socket must remain healthy and not panic.
    let _ = recv_until_match(&mut socket, |message| message["id"] == json!(5_u64)).await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_debugger_pause_interrupts_in_flight_runtime_evaluate() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(
        &mut socket,
        "data:text/html,<body>debugger pause</body>",
    )
    .await;

    let enabled = send_cdp_command(
        &mut socket,
        6,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        enabled
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message.get("error").is_none()),
        "Debugger.enable should succeed: {enabled:#?}"
    );

    // Local Chromium and V8's inspector tests acknowledge Debugger.pause
    // before the next JavaScript statement enters the nested pause loop.
    let pause = send_cdp_command(
        &mut socket,
        7,
        "Debugger.pause",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        pause.iter().all(|message| {
            message["sessionId"].as_str() != Some(session_id.as_str())
                || message["method"] != json!("Debugger.paused")
        }),
        "Debugger.paused must not precede the Debugger.pause response: {pause:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "globalThis.__moliDebuggerPauseProbe = 1",
                    "returnByValue": true,
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that enters the debugger pause");
    let paused = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Debugger.paused")
    })
    .await;
    assert!(
        paused.iter().all(|message| message["id"] != json!(8_u64)),
        "Runtime.evaluate must not complete before Debugger.paused: {paused:#?}"
    );

    let mut resumed = send_cdp_command(
        &mut socket,
        9,
        "Debugger.resume",
        Some(&session_id),
        json!({}),
    )
    .await;
    if resumed.iter().all(|message| message["id"] != json!(8_u64)) {
        resumed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(8_u64)).await);
    }
    assert!(
        resumed.iter().any(|message| {
            message["id"] == json!(8_u64) && message["result"]["result"]["value"] == json!(1_u64)
        }),
        "Debugger.resume should release the pending Runtime.evaluate: {resumed:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_io_terminate_interrupts_busy_main_thread_and_skips_main_follower() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(
        &mut socket,
        "data:text/html,<body>IO interrupt</body>",
    )
    .await;

    let enabled = send_cdp_command(
        &mut socket,
        6,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        enabled
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message.get("error").is_none()),
        "Debugger.enable should succeed: {enabled:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "debugger; for (;;) {}",
                    "returnByValue": true,
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send the non-yielding MainThread Runtime.evaluate");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Debugger.paused")
    })
    .await;
    assert!(
        observed.iter().all(|message| message["id"] != json!(7_u64)),
        "the busy Runtime.evaluate must still be in flight at its debugger barrier: {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Debugger.resume",
            Some(&session_id),
            json!({}),
        )
        .await,
    );
    if observed.iter().all(|message| {
        message["sessionId"].as_str() != Some(session_id.as_str())
            || message["method"] != json!("Debugger.resumed")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(session_id.as_str())
                    && message["method"] == json!("Debugger.resumed")
            })
            .await,
        );
    }
    assert!(
        observed.iter().all(|message| message["id"] != json!(7_u64)),
        "the resumed Runtime.evaluate must enter its non-yielding loop: {observed:#?}"
    );

    // This MainThread follower is deliberately queued before the IO command.
    // An interrupt callback must skip it, dispatch terminateExecution, and
    // leave the follower for ordinary owner dispatch after V8 unwinds.
    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "6 * 7",
                    "returnByValue": true,
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("queue the MainThread follower");
    let terminated = tokio::time::timeout(
        Duration::from_secs(10),
        send_cdp_command(
            &mut socket,
            10,
            "Runtime.terminateExecution",
            Some(&session_id),
            json!({}),
        ),
    )
    .await
    .expect("IO terminateExecution must interrupt non-yielding MainThread JavaScript");
    observed.extend(terminated);

    let mut saw_busy_response = observed.iter().any(|message| message["id"] == json!(7_u64));
    let mut saw_follower_response = observed.iter().any(|message| message["id"] == json!(9_u64));
    if !saw_busy_response || !saw_follower_response {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                saw_busy_response |= message["id"] == json!(7_u64);
                saw_follower_response |= message["id"] == json!(9_u64);
                saw_busy_response && saw_follower_response
            })
            .await,
        );
    }

    let terminate_response = observed
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("terminateExecution response");
    assert_eq!(
        terminate_response["result"],
        json!({}),
        "terminateExecution must complete through the IO V8 interrupt: {observed:#?}"
    );
    let busy_response = observed
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("terminated Runtime.evaluate response");
    assert!(
        busy_response.get("error").is_some()
            || busy_response["result"]["exceptionDetails"].is_object(),
        "the non-yielding evaluation must report termination: {busy_response:#?}"
    );
    let follower_response = observed
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("MainThread follower response");
    assert_eq!(
        follower_response["result"]["result"]["value"],
        json!(42),
        "the skipped MainThread follower must run normally after termination: {observed:#?}"
    );
    let terminate_response_index = observed
        .iter()
        .position(|message| message["id"] == json!(10_u64))
        .expect("terminateExecution response position");
    let follower_response_index = observed
        .iter()
        .position(|message| message["id"] == json!(9_u64))
        .expect("MainThread follower response position");
    assert!(
        terminate_response_index < follower_response_index,
        "the MainThread follower must not first-dispatch ahead of IO termination: {observed:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_active_js_interrupt_preserves_main_and_io_lanes_across_sessions() {
    let entered_busy_loop = Arc::new(tokio::sync::Notify::new());
    let entered_busy_loop_route = Arc::clone(&entered_busy_loop);
    let fixture_app = Router::new()
        .route(
            "/",
            get(|| async {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>active JavaScript interrupt</body></html>",
                )
            }),
        )
        .route(
            "/entered",
            get(move || {
                let entered_busy_loop = Arc::clone(&entered_busy_loop_route);
                async move {
                    entered_busy_loop.notify_one();
                    "entered"
                }
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "active-js-interrupt");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let primary = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&primary.session_id),
        json!({}),
    )
    .await;
    let _ = cdp_navigate_and_wait_for_load(
        &mut socket,
        5,
        &primary.session_id,
        &format!("http://{fixture_addr}/"),
    )
    .await;

    let auxiliary_attach = send_cdp_command(
        &mut socket,
        6,
        "Target.attachToTarget",
        None,
        json!({ "targetId": primary.target_id, "flatten": true }),
    )
    .await;
    let auxiliary_session_id = auxiliary_attach
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("auxiliary session id")
        .to_owned();
    for (id, method, session_id) in [
        (7, "Debugger.enable", primary.session_id.as_str()),
        (8, "Runtime.enable", auxiliary_session_id.as_str()),
        (9, "Debugger.enable", auxiliary_session_id.as_str()),
    ] {
        let enabled = send_cdp_command(&mut socket, id, method, Some(session_id), json!({})).await;
        assert!(
            enabled
                .iter()
                .any(|message| message["id"] == json!(id) && message.get("error").is_none()),
            "{method} should succeed before the active-JS matrix: {enabled:#?}"
        );
    }

    let busy_source = r#"const xhr = new XMLHttpRequest();
xhr.open('GET', '/entered', false);
xhr.send();
console.log('moli-active-js-loop-entered');
for (;;) {}"#;
    let compiled = send_cdp_command(
        &mut socket,
        10,
        "Runtime.compileScript",
        Some(&primary.session_id),
        json!({
            "expression": busy_source,
            "sourceURL": "moli-active-js-interrupt.js",
            "persistScript": true,
        }),
    )
    .await;
    let script_id = compiled
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .and_then(|message| message["result"]["scriptId"].as_str())
        .unwrap_or_else(|| panic!("Runtime.compileScript should return scriptId: {compiled:#?}"))
        .to_owned();

    send_cdp_command_without_wait(
        &mut socket,
        11,
        "Runtime.runScript",
        Some(&primary.session_id),
        json!({ "scriptId": script_id }),
    )
    .await;
    timeout(Duration::from_secs(5), entered_busy_loop.notified())
        .await
        .expect("compiled JavaScript should complete its synchronous external witness");

    // These Main commands are queued on a different DevTools session while
    // the primary session owns the renderer in non-yielding JavaScript.
    send_cdp_command_without_wait(
        &mut socket,
        12,
        "Runtime.evaluate",
        Some(&auxiliary_session_id),
        json!({
            "expression": "(globalThis.__moliMainLane ??= []).push('m1')",
            "returnByValue": true,
        }),
    )
    .await;
    send_cdp_command_without_wait(
        &mut socket,
        13,
        "Runtime.evaluate",
        Some(&auxiliary_session_id),
        json!({
            "expression": "globalThis.__moliMainLane.push('m2')",
            "returnByValue": true,
        }),
    )
    .await;
    let mut observed = recv_cdp_messages_for(&mut socket, Duration::from_millis(250)).await;
    assert!(
        observed
            .iter()
            .all(|message| !matches!(message["id"].as_u64(), Some(11..=13))),
        "the active script and its Main followers must remain blocked before IO arrives: \
         {observed:#?}"
    );

    // All three commands use the auxiliary session's IO lane. The two source
    // lookups prove FIFO before terminateExecution releases the Main owner.
    for (id, method, params) in [
        (
            14,
            "Debugger.getScriptSource",
            json!({ "scriptId": script_id }),
        ),
        (
            15,
            "Debugger.getScriptSource",
            json!({ "scriptId": script_id }),
        ),
        (16, "Runtime.terminateExecution", json!({})),
    ] {
        send_cdp_command_without_wait(&mut socket, id, method, Some(&auxiliary_session_id), params)
            .await;
    }

    let expected_ids = [11_u64, 12, 13, 14, 15, 16];
    let mut response_ids = observed
        .iter()
        .filter_map(|message| message["id"].as_u64())
        .collect::<std::collections::BTreeSet<_>>();
    observed.extend(
        recv_until_match(&mut socket, |message| {
            if let Some(id) = message["id"].as_u64() {
                response_ids.insert(id);
            }
            expected_ids
                .iter()
                .all(|expected_id| response_ids.contains(expected_id))
        })
        .await,
    );

    for id in expected_ids {
        assert_eq!(
            observed
                .iter()
                .filter(|message| message["id"] == json!(id))
                .count(),
            1,
            "each command must produce exactly one response (id {id}): {observed:#?}"
        );
    }
    for id in [14_u64, 15] {
        let response = observed
            .iter()
            .find(|message| message["id"] == json!(id))
            .expect("getScriptSource response");
        assert_eq!(
            response["result"]["scriptSource"],
            json!(busy_source),
            "IO source lookup {id} must reach the live auxiliary Inspector session: \
             {observed:#?}"
        );
    }
    let position = |id| {
        observed
            .iter()
            .position(|message| message["id"] == json!(id))
            .unwrap_or_else(|| panic!("missing response position for id {id}: {observed:#?}"))
    };
    assert!(
        position(14) < position(15) && position(15) < position(16),
        "same-session IO commands must first-dispatch FIFO: {observed:#?}"
    );
    assert!(
        position(16) < position(12) && position(12) < position(13),
        "IO may overtake Main, but the auxiliary Main lane must remain FIFO: {observed:#?}"
    );
    assert_eq!(
        observed
            .iter()
            .find(|message| message["id"] == json!(16_u64))
            .expect("terminateExecution response")["result"],
        json!({}),
        "terminateExecution must complete through a true V8 interrupt: {observed:#?}"
    );
    let busy_response = observed
        .iter()
        .find(|message| message["id"] == json!(11_u64))
        .expect("terminated runScript response");
    assert!(
        busy_response.get("error").is_some()
            || busy_response["result"]["exceptionDetails"].is_object(),
        "the active Runtime.runScript must report termination: {busy_response:#?}"
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(primary.session_id.as_str())
                && message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("moli-active-js-loop-entered")
        }),
        "the buffered console witness must prove JavaScript passed XHR and entered the loop: \
         {observed:#?}"
    );
    for (id, value) in [(12_u64, 1_u64), (13, 2)] {
        assert_eq!(
            observed
                .iter()
                .find(|message| message["id"] == json!(id))
                .expect("Main follower response")["result"]["result"]["value"],
            json!(value),
            "Main follower {id} must execute once and in order: {observed:#?}"
        );
    }

    let recovered = send_cdp_command(
        &mut socket,
        17,
        "Runtime.evaluate",
        Some(&primary.session_id),
        json!({
            "expression": "globalThis.__moliMainLane.join(',')",
            "returnByValue": true,
        }),
    )
    .await;
    assert!(
        recovered.iter().any(|message| {
            message["id"] == json!(17_u64) && message["result"]["result"]["value"] == json!("m1,m2")
        }),
        "the isolate and owner must recover with exactly-once Main state: {recovered:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_raw_client_runtime_evaluate_immediately_after_page_navigate_succeeds() {
    // Regression test for the raw-CDP race: a raw client can pipeline
    // `Runtime.evaluate` directly behind `Page.navigate` without waiting for a
    // lifecycle event. The scheduler must finish the in-flight navigation
    // before dispatching evaluate; otherwise it observes `NoDocumentLoaded`.
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><a id='link' href='/next'>link</a></body></html>",
        )
    }
    let fixture_app = Router::new().route("/", get(page));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": { "browserContextId": browser_context_id, "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    // Send Page.navigate then Runtime.evaluate back-to-back without
    // waiting for any lifecycle event. This mirrors what raw-CDP clients
    // do. The socket loop must drain the pending background navigation
    // completion BEFORE dispatching Runtime.evaluate, otherwise evaluate
    // would observe `NoDocumentLoaded`.
    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "document.querySelector('#link') ? document.querySelector('#link').textContent : ''",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");

    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate_response = messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("Runtime.evaluate response");
    assert!(
        evaluate_response.get("error").is_none(),
        "Runtime.evaluate must not return NoDocumentLoaded after pipelined Page.navigate; got {evaluate_response}"
    );
    let value = evaluate_response["result"]["result"]["value"]
        .as_str()
        .expect("string value from evaluate");
    assert_eq!(
        value, "link",
        "evaluate must observe the new document body (got `{value}`)"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_control_command_waits_for_navigation_attachment_cutover() {
    let release_tail = Arc::new(tokio::sync::Notify::new());
    let (fixture_addr, fixture_server) =
        spawn_response_stage_streaming_document_fixture_server(Arc::clone(&release_tail)).await;

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let old_isolate_messages = send_cdp_command(
        &mut socket,
        4,
        "Runtime.getIsolateId",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let old_isolate_id = old_isolate_messages
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .and_then(|message| message["result"]["id"].as_str())
        .expect("initial Runtime.getIsolateId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.getIsolateId",
                "sessionId": session.session_id,
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send suspended Runtime.getIsolateId");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Browser.getVersion",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send navigation-independent Browser.getVersion");

    let before_resume = recv_until_id(&mut socket, 7).await;
    assert!(
        !before_resume
            .iter()
            .any(|message| message["id"] == json!(6_u64)),
        "Runtime control command must remain queued while the document attachment is suspended: \
         {before_resume:#?}"
    );

    release_tail.notify_one();
    let resumed_messages = recv_until_id(&mut socket, 6).await;
    assert!(
        before_resume
            .iter()
            .chain(&resumed_messages)
            .any(|message| message["id"] == json!(5_u64) && message.get("error").is_none()),
        "streaming Page.navigate should complete successfully after the body tail: \
         {resumed_messages:#?}"
    );
    let resumed = resumed_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("resumed Runtime.getIsolateId response");
    assert!(
        resumed.get("error").is_none(),
        "resumed Runtime.getIsolateId should succeed: {resumed_messages:#?}"
    );
    let new_isolate_id = resumed["result"]["id"]
        .as_str()
        .expect("replacement Runtime.getIsolateId");
    assert_ne!(
        new_isolate_id, old_isolate_id,
        "queued command must bind to the replacement document's renderer attachment"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_navigation_suspension_matches_chromium_io_command_routing() {
    let request_received = Arc::new(tokio::sync::Notify::new());
    let release_response = Arc::new(tokio::sync::Notify::new());
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pre-commit navigation fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("pre-commit navigation fixture addr");
    let fixture_server = {
        let request_received = Arc::clone(&request_received);
        let release_response = Arc::clone(&release_response);
        tokio::spawn(async move {
            let (mut stream, _) = fixture_listener
                .accept()
                .await
                .expect("accept pre-commit navigation request");
            let mut request = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let read = stream
                    .read(&mut buf)
                    .await
                    .expect("read pre-commit navigation request");
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            request_received.notify_one();
            release_response.notified().await;
            let body = b"<!doctype html><html><body><main>replacement</main></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write pre-commit navigation response head");
            stream
                .write_all(body)
                .await
                .expect("write pre-commit navigation response body");
        })
    };

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let debugger_enable = send_cdp_command(
        &mut socket,
        3,
        "Debugger.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    assert!(
        debugger_enable
            .iter()
            .any(|message| { message["id"] == json!(3_u64) && message.get("error").is_none() })
    );
    let performance_enable = send_cdp_command(
        &mut socket,
        4,
        "Performance.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    assert!(
        performance_enable
            .iter()
            .any(|message| { message["id"] == json!(4_u64) && message.get("error").is_none() })
    );
    let evaluated = send_cdp_command(
        &mut socket,
        10,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": "function moliSuspendedSocketSource() { return 10; }\n//# sourceURL=moli-suspended-socket-source.js"
        }),
    )
    .await;
    assert!(
        evaluated
            .iter()
            .any(|message| { message["id"] == json!(10_u64) && message.get("error").is_none() })
    );
    let script_id = evaluated
        .iter()
        .find(|message| {
            message["method"] == json!("Debugger.scriptParsed")
                && message["params"]["url"] == json!("moli-suspended-socket-source.js")
        })
        .and_then(|message| message["params"]["scriptId"].as_str())
        .map(str::to_owned)
        .expect("Debugger.scriptParsed for the pre-navigation source");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    timeout(Duration::from_secs(5), request_received.notified())
        .await
        .expect("main-document request should reach the pre-commit fixture");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Debugger.enable",
                "sessionId": session.session_id,
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send suspended Debugger.enable");

    let mut before_commit = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            7,
            "Performance.getMetrics",
            Some(&session.session_id),
            json!({}),
        ),
    )
    .await
    .expect("Performance.getMetrics must bypass navigation suspension");
    before_commit.extend(
        timeout(
            Duration::from_secs(5),
            send_cdp_command(
                &mut socket,
                8,
                "Runtime.terminateExecution",
                Some(&session.session_id),
                json!({}),
            ),
        )
        .await
        .expect("Runtime.terminateExecution must bypass navigation suspension"),
    );
    let suspended_source = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            11,
            "Debugger.getScriptSource",
            Some(&session.session_id),
            json!({"scriptId": script_id}),
        ),
    )
    .await
    .expect("Debugger.getScriptSource must address the suspended renderer");
    assert!(
        suspended_source.iter().any(|message| {
            message["id"] == json!(11_u64)
                && message["result"]["scriptSource"]
                    .as_str()
                    .is_some_and(|source| source.contains("moliSuspendedSocketSource"))
        }),
        "interruptible Debugger source lookup must finish before navigation commit: \
         {suspended_source:#?}"
    );
    before_commit.extend(suspended_source);
    before_commit
        .extend(send_cdp_command(&mut socket, 9, "Browser.getVersion", None, json!({})).await);
    for id in [7_u64, 8, 9, 11] {
        assert!(
            before_commit
                .iter()
                .any(|message| message["id"] == json!(id) && message.get("error").is_none()),
            "Chromium IO-route command {id} should complete before navigation commit: \
             {before_commit:#?}"
        );
    }
    let suspended_document_count = before_commit
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["metrics"].as_array())
        .and_then(|metrics| {
            metrics
                .iter()
                .find(|metric| metric["name"] == json!("Documents"))
        })
        .and_then(|metric| metric["value"].as_f64())
        .unwrap_or_default();
    assert!(
        suspended_document_count >= 1.0,
        "Performance.getMetrics must snapshot the suspended renderer rather than return the \
         default metric set: {before_commit:#?}"
    );
    assert!(
        before_commit
            .iter()
            .all(|message| message["id"] != json!(6_u64)),
        "ordinary Debugger commands must wait for the replacement attachment: \
         {before_commit:#?}"
    );

    release_response.notify_one();
    let mut after_commit = recv_until_id(&mut socket, 6).await;
    assert!(
        after_commit
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message.get("error").is_none()),
        "Debugger.enable should run on the replacement attachment: {after_commit:#?}"
    );
    if !before_commit
        .iter()
        .chain(&after_commit)
        .any(|message| message["id"] == json!(5_u64))
    {
        after_commit.extend(recv_until_id(&mut socket, 5).await);
    }
    assert!(
        before_commit
            .iter()
            .chain(&after_commit)
            .any(|message| message["id"] == json!(5_u64) && message.get("error").is_none()),
        "released navigation should complete successfully: {after_commit:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_replacement_retires_hanging_precommit_navigation() {
    const REPLACEMENT_TIMEOUT: Duration = Duration::from_secs(3);

    let hanging_request_received = Arc::new(tokio::sync::Notify::new());
    let request_received_for_route = Arc::clone(&hanging_request_received);
    let fixture_app = Router::new().route(
        "/hang",
        get(move || {
            let request_received_for_route = Arc::clone(&request_received_for_route);
            async move {
                request_received_for_route.notify_one();
                std::future::pending::<()>().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "unreachable",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "hanging-precommit-replacement");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (
            5_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/hang") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send hanging Page.navigate");
    timeout(REPLACEMENT_TIMEOUT, hanging_request_received.notified())
        .await
        .expect("hanging main-document request should reach the fixture");

    let marker = "moli-hanging-precommit-replacement";
    let replacement_url =
        format!("data:text/html,<title>{marker}</title><main id='marker'>{marker}</main>");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": replacement_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send replacement Page.navigate");

    let mut replacement_messages = timeout(REPLACEMENT_TIMEOUT, async {
        let mut messages = Vec::new();
        let mut replacement_loader_id = None::<String>;
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["id"] == json!(7_u64) {
                assert!(
                    message.get("error").is_none(),
                    "replacement Page.navigate must succeed: {message:#?}"
                );
                replacement_loader_id = message["result"]["loaderId"].as_str().map(str::to_owned);
            }
            let reached_replacement_load = replacement_loader_id.as_deref().is_some_and(|loader| {
                message["sessionId"].as_str() == Some(session.session_id.as_str())
                    && message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["name"] == json!("load")
                    && message["params"]["loaderId"].as_str() == Some(loader)
            });
            messages.push(message);
            if reached_replacement_load {
                break messages;
            }
        }
    })
    .await
    .expect("replacement Document must reach its own load without the hanging response");
    let replacement_loader_id = replacement_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("replacement Page.navigate loaderId");
    assert!(
        replacement_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() == Some(replacement_loader_id)
        }),
        "replacement DCL must precede its load: {replacement_messages:#?}"
    );
    if !replacement_messages
        .iter()
        .any(|message| message["id"] == json!(6_u64))
    {
        replacement_messages.extend(
            timeout(REPLACEMENT_TIMEOUT, recv_until_id(&mut socket, 6))
                .await
                .expect("superseded Page.navigate must receive a terminal response"),
        );
    }
    let superseded_response = replacement_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("superseded Page.navigate response");
    assert!(
        superseded_response.get("error").is_none(),
        "Chromium reports a superseded Page.navigate as a successful command: {superseded_response:#?}"
    );
    assert_eq!(
        superseded_response["result"]["frameId"],
        json!(session.target_id)
    );
    assert_eq!(
        superseded_response["result"]["errorText"],
        json!("net::ERR_ABORTED")
    );
    assert_eq!(superseded_response["result"]["isDownload"], json!(false));
    assert!(superseded_response["result"].get("loaderId").is_none());
    let replacement_document = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        8,
        "document.querySelector('#marker')?.textContent",
    )
    .await;
    assert_eq!(replacement_document, marker);

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_after_dcl_is_not_blocked_by_pending_load_stylesheet() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><link rel='stylesheet' href='/slow.css'></head><body><main id='ready'>ready</main></body></html>",
        )
    }
    async fn slow_css() -> impl IntoResponse {
        sleep(Duration::from_secs(60)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
            "body { color: black; }",
        )
    }
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/slow.css", get(slow_css));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 1).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 2).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    for (id, method) in [
        (3_u64, "Page.enable"),
        (4_u64, "Runtime.enable"),
        (5_u64, "Page.setLifecycleEventsEnabled"),
    ] {
        let params = if method == "Page.setLifecycleEventsEnabled" {
            json!({ "enabled": true })
        } else {
            json!({})
        };
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id,
                    "params": params
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap_or_else(|_| panic!("send {method}"));
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "document.querySelector('#ready')?.textContent",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");

    let evaluate_messages = recv_until_id(&mut socket, 7).await;
    let evaluate_response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert!(
        evaluate_response.get("error").is_none(),
        "Runtime.evaluate after DOMContentLoaded must not be blocked by pending load work: {evaluate_response}"
    );
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!("ready")
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_dom_query_after_dcl_runs_before_pending_load_and_load_later_fires() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><link rel='stylesheet' href='/slow.css'></head><body><main id='ready'>ready</main></body></html>",
        )
    }
    let stylesheet_requested = Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&stylesheet_requested);
    let release_for_route = Arc::clone(&release_stylesheet);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.css",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                    "body { color: black; }",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 1).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 2).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    for (id, method) in [
        (3_u64, "Page.enable"),
        (4_u64, "Runtime.enable"),
        (5_u64, "Page.setLifecycleEventsEnabled"),
    ] {
        let params = if method == "Page.setLifecycleEventsEnabled" {
            json!({ "enabled": true })
        } else {
            json!({})
        };
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id,
                    "params": params
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap_or_else(|_| panic!("send {method}"));
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    timeout(Duration::from_secs(2), stylesheet_requested.notified())
        .await
        .expect("stylesheet request should be pending before post-DCL DOM queries");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "DOM.getDocument",
                "sessionId": session_id,
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send DOM.getDocument");
    let document_messages =
        tokio::time::timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
            .await
            .expect("DOM.getDocument should return before pending stylesheet completes");
    assert!(
        !document_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "DOM.getDocument only proved non-blocking if it returned before load: {document_messages:#?}"
    );
    let document_response = document_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("DOM.getDocument response");
    assert!(
        document_response.get("error").is_none(),
        "DOM.getDocument after DOMContentLoaded must not be blocked by pending load work: {document_response}"
    );
    let root_id = document_response["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id");
    assert!(root_id > 0, "document node id should be non-zero");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "DOM.querySelector",
                "sessionId": session_id,
                "params": { "nodeId": root_id, "selector": "#ready" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send DOM.querySelector");
    let query_messages =
        tokio::time::timeout(Duration::from_secs(1), recv_until_id(&mut socket, 8))
            .await
            .expect("DOM.querySelector should return before pending stylesheet completes");
    assert!(
        !query_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "DOM.querySelector only proved non-blocking if it returned before load: {query_messages:#?}"
    );
    let query_response = query_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("DOM.querySelector response");
    assert!(
        query_response.get("error").is_none(),
        "DOM.querySelector after DOMContentLoaded must not be blocked by pending load work: {query_response}"
    );
    let node_id = query_response["result"]["nodeId"]
        .as_u64()
        .expect("querySelector node id");
    assert!(node_id > 0, "querySelector should find #ready");

    release_stylesheet.notify_one();
    let load_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    assert!(
        load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "deferred load completion should resume after pending stylesheet finishes"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_post_dcl_dynamic_script_completion_wakes_deferred_load() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.runtimeScriptOrder = [];
document.addEventListener('DOMContentLoaded', () => {
  window.runtimeScriptOrder.push('dcl');
  const script = document.createElement('script');
  script.async = false;
  script.onload = () => window.runtimeScriptOrder.push('onload');
  script.src = '/runtime-script.js';
  document.head.appendChild(script);
  window.runtimeScriptOrder.push('after-append');
});
window.addEventListener('load', () => {
  window.runtimeScriptOrder.push('window-load');
});
</script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }
    async fn runtime_script() -> impl IntoResponse {
        sleep(Duration::from_millis(200)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "window.runtimeScriptOrder.push('external');",
        )
    }
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/runtime-script.js", get(runtime_script));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (5_u64, "Runtime.enable", json!({})),
        (
            6_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let dcl_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    let dcl_timestamp = dcl_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.domContentEventFired")
        })
        .and_then(|message| message["params"]["timestamp"].as_f64())
        .expect("DOMContentLoaded timestamp");

    let load_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    let load_timestamp = load_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .and_then(|message| message["params"]["timestamp"].as_f64())
        .expect("load timestamp");
    assert!(
        load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "deferred load completion should resume after the post-DCL dynamic script finishes"
    );
    assert!(
        load_timestamp > dcl_timestamp,
        "post-DCL dynamic script should make load timestamp later than DCL timestamp; dcl={dcl_timestamp}, load={load_timestamp}"
    );

    let evaluate = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": "window.runtimeScriptOrder.join(',')",
            "returnByValue": true,
        }),
    )
    .await;
    let evaluate_response = evaluate
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!("dcl,after-append,external,onload,window-load")
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_after_dcl_runs_while_deferred_load_script_is_pending() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
document.addEventListener('DOMContentLoaded', () => {
  window.afterDclScriptAppendStarted = true;
  const script = document.createElement('script');
  script.src = '/runtime-script.js';
  document.head.appendChild(script);
});
window.addEventListener('load', () => {
  window.sawWindowLoad = true;
});
</script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&script_requested);
    let release_for_route = Arc::clone(&release_script);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/runtime-script.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "window.pendingRuntimeScriptExecuted = true;",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "post-dcl-load-script");
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (5_u64, "Runtime.enable", json!({})),
        (
            6_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    // The slow script request is started from the DOMContentLoaded handler.
    // Use that request as the readiness signal instead of requiring the CDP
    // DCL event to overtake the post-DCL resource wake under full-suite load.
    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("post-DCL dynamic script request should start before load");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.evaluate",
                "sessionId": session.session_id,
                "params": {
                    "expression": "JSON.stringify({afterAppend: window.afterDclScriptAppendStarted === true, scriptExecuted: window.pendingRuntimeScriptExecuted === true, loaded: window.sawWindowLoad === true})",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 8))
        .await
        .expect("Runtime.evaluate should return while the post-DCL load script is pending");
    assert!(
        !evaluate_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Runtime.evaluate only proves non-blocking if it returns before load: {evaluate_messages:#?}"
    );
    let evaluate_response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!("{\"afterAppend\":true,\"scriptExecuted\":false,\"loaded\":false}")
    );

    release_script.notify_one();
    let load_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    assert!(
        load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "deferred load completion should resume after the post-DCL dynamic script is released"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_defer_wait_runs_ready_tasks_while_next_defer_source_is_pending() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.mainDeferWaitTaskOrder = ['inline'];
addEventListener('DOMContentLoaded', () => {
  mainDeferWaitTaskOrder.push(`dcl:${document.readyState}`);
  globalThis.mainDeferWaitOrderAtDcl = mainDeferWaitTaskOrder.join(',');
});
</script>
<script defer src="/first-defer.js"></script>
<script defer src="/defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    async fn delayed_defer_script() -> impl IntoResponse {
        sleep(Duration::from_millis(120)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "mainDeferWaitTaskOrder.push(`second:${document.readyState}`);",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route(
            "/first-defer.js",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    r#"mainDeferWaitTaskOrder.push(`first:${document.readyState}`);
setTimeout(() => mainDeferWaitTaskOrder.push(`timer:${document.readyState}`), 0);
addEventListener('message', () => {
  mainDeferWaitTaskOrder.push(`message:${document.readyState}`);
}, { once: true });
postMessage('between-defer-scripts', '*');"#,
                )
            }),
        )
        .route("/defer.js", get(delayed_defer_script));
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "defer-event-loop-order");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = cdp_navigate_and_wait_for_load(
        &mut socket,
        6,
        &session.session_id,
        &format!("http://{fixture_addr}/"),
    )
    .await;

    let order = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        7,
        "mainDeferWaitOrderAtDcl",
    )
    .await;
    let events = order.split(',').collect::<Vec<_>>();
    let position = |event| {
        events
            .iter()
            .position(|candidate| candidate.starts_with(event))
            .unwrap_or_else(|| panic!("missing {event:?} in event order {events:?}"))
    };
    assert!(
        position("first") < position("timer") && position("first") < position("message"),
        "the first defer script must schedule both tasks before they run: {events:?}"
    );
    assert!(
        position("timer") < position("second"),
        "the timer task should run while the second defer source is pending: {events:?}"
    );
    assert!(
        position("message") < position("second"),
        "the posted-message task should run while the second defer source is pending: {events:?}"
    );
    assert!(
        position("second") < position("dcl"),
        "the second defer script must still execute before DOMContentLoaded: {events:?}"
    );
    for event in ["first", "timer", "message", "second", "dcl"] {
        assert_eq!(
            events[position(event)],
            format!("{event}:interactive"),
            "post-parse callbacks must observe the committed interactive transition: {events:?}"
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_defer_wait_runs_worker_message_arriving_after_lifecycle_parks() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.lateWorkerDeferOrder = ['inline'];
globalThis.lateWorker = new Worker('/worker.js');
lateWorker.addEventListener('message', () => {
  lateWorkerDeferOrder.push(`worker:${document.readyState}`);
  fetch('/release');
}, { once: true });
addEventListener('DOMContentLoaded', () => {
  lateWorkerDeferOrder.push(`dcl:${document.readyState}`);
  globalThis.lateWorkerDeferOrderAtDcl = lateWorkerDeferOrder.join(',');
});
</script>
<script defer src="/defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    async fn delayed_worker_script() -> impl IntoResponse {
        sleep(Duration::from_millis(80)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "postMessage('release-defer');",
        )
    }

    let release_defer = Arc::new(tokio::sync::Notify::new());
    let release_for_script = Arc::clone(&release_defer);
    let release_from_page = Arc::clone(&release_defer);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/worker.js", get(delayed_worker_script))
        .route(
            "/defer.js",
            get(move || {
                let release_for_script = Arc::clone(&release_for_script);
                async move {
                    release_for_script.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "lateWorkerDeferOrder.push(`defer:${document.readyState}`);",
                    )
                }
            }),
        )
        .route(
            "/release",
            get(move || {
                let release_from_page = Arc::clone(&release_from_page);
                async move {
                    release_from_page.notify_one();
                    "released"
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "late-worker-defer-event-loop-order");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    timeout(
        Duration::from_secs(5),
        cdp_navigate_and_wait_for_load(
            &mut socket,
            6,
            &session.session_id,
            &format!("http://{fixture_addr}/"),
        ),
    )
    .await
    .expect("late Worker message should release the deferred script and allow load");

    let order = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        7,
        "lateWorkerDeferOrderAtDcl",
    )
    .await;
    assert_eq!(
        order, "inline,worker:interactive,defer:interactive,dcl:interactive",
        "a Worker task arriving after the lifecycle wait parks must run before the defer source can finish"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_defer_wait_runs_indexeddb_task_started_by_late_timer() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.lateIndexedDbDeferOrder = ['inline'];
setTimeout(() => {
  lateIndexedDbDeferOrder.push(`timer:${document.readyState}`);
  const request = indexedDB.open('late-indexeddb-during-defer');
  request.addEventListener('success', () => {
    lateIndexedDbDeferOrder.push(`idb:${document.readyState}`);
    fetch('/release');
  }, { once: true });
}, 80);
addEventListener('DOMContentLoaded', () => {
  lateIndexedDbDeferOrder.push(`dcl:${document.readyState}`);
  globalThis.lateIndexedDbDeferOrderAtDcl = lateIndexedDbDeferOrder.join(',');
});
</script>
<script defer src="/defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    let release_defer = Arc::new(tokio::sync::Notify::new());
    let release_for_script = Arc::clone(&release_defer);
    let release_from_page = Arc::clone(&release_defer);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route(
            "/defer.js",
            get(move || {
                let release_for_script = Arc::clone(&release_for_script);
                async move {
                    release_for_script.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "lateIndexedDbDeferOrder.push(`defer:${document.readyState}`);",
                    )
                }
            }),
        )
        .route(
            "/release",
            get(move || {
                let release_from_page = Arc::clone(&release_from_page);
                async move {
                    release_from_page.notify_one();
                    "released"
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "late-indexeddb-defer-event-loop-order");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    timeout(
        Duration::from_secs(5),
        cdp_navigate_and_wait_for_load(
            &mut socket,
            6,
            &session.session_id,
            &format!("http://{fixture_addr}/"),
        ),
    )
    .await
    .expect("late IndexedDB task should release the deferred script and allow load");

    let order = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        7,
        "lateIndexedDbDeferOrderAtDcl",
    )
    .await;
    assert_eq!(
        order, "inline,timer:interactive,idb:interactive,defer:interactive,dcl:interactive",
        "an IndexedDB task started after the lifecycle wait parks must run before the defer source can finish"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_external_writer_with_two_document_writes_reaches_defer_and_dcl() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.documentWriteOrder = ['head'];
document.addEventListener('DOMContentLoaded', () => documentWriteOrder.push('dcl'));
</script>
<script src="/writer.js"></script>
<script>documentWriteOrder.push('tail');</script>
<script defer src="/defer.js"></script>
<script>documentWriteOrder.push('after-defer');</script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    async fn writer() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            r#"documentWriteOrder.push('writer-start');
document.write('<script src="/first.js"></' + 'script>');
document.write('<script src="/second.js"></' + 'script>');
documentWriteOrder.push('writer-end');"#,
        )
    }

    async fn first() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "documentWriteOrder.push('first');",
        )
    }

    async fn second() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "documentWriteOrder.push('second');",
        )
    }

    let defer_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let defer_requests_for_route = Arc::clone(&defer_requests);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/writer.js", get(writer))
        .route("/first.js", get(first))
        .route("/second.js", get(second))
        .route(
            "/defer.js",
            get(move || {
                let defer_requests_for_route = Arc::clone(&defer_requests_for_route);
                async move {
                    defer_requests_for_route.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "documentWriteOrder.push('defer');",
                    )
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "external-writer-two-document-writes");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Runtime.enable", json!({})),
        (5_u64, "Page.enable", json!({})),
        (
            6_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let dcl_messages = timeout(
        Duration::from_secs(3),
        recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
        }),
    )
    .await
    .expect("nested written scripts must not strand the main parser before DCL");
    let loader_id = dcl_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("Page.navigate should return the committed loaderId");
    assert!(
        dcl_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() == Some(loader_id)
        }),
        "DCL must belong to the exact navigation loader: {dcl_messages:#?}"
    );
    assert_eq!(
        defer_requests.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parser-deferred source must join the document preload and execute before DCL"
    );

    let evaluate_messages = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": "JSON.stringify(documentWriteOrder)",
            "returnByValue": true
        }),
    )
    .await;
    let response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(
            "[\"head\",\"writer-start\",\"writer-end\",\"first\",\"second\",\"tail\",\"after-defer\",\"defer\",\"dcl\"]"
        )
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_runs_while_parser_defer_source_is_blocked() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.blockedDeferProtocolMarker = 'committed-source';
document.addEventListener('readystatechange', () => {
    if (document.readyState === 'interactive') {
        void fetch('/parser-interactive');
    }
});
</script>
<script defer src="/blocked-defer.js"></script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    let defer_requested = Arc::new(tokio::sync::Notify::new());
    let parser_interactive = Arc::new(tokio::sync::Notify::new());
    let release_defer = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&defer_requested);
    let interactive_for_route = Arc::clone(&parser_interactive);
    let release_for_route = Arc::clone(&release_defer);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route(
            "/blocked-defer.js",
            get(move || {
                let requested_for_route = Arc::clone(&requested_for_route);
                let release_for_route = Arc::clone(&release_for_route);
                async move {
                    requested_for_route.notify_one();
                    release_for_route.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "globalThis.blockedDeferExecuted = true;",
                    )
                }
            }),
        )
        .route(
            "/parser-interactive",
            get(move || {
                let interactive_for_route = Arc::clone(&interactive_for_route);
                async move {
                    interactive_for_route.notify_one();
                    ""
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "blocked-defer-protocol-command");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Runtime.enable"), (5_u64, "Page.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    timeout(Duration::from_secs(1), defer_requested.notified())
        .await
        .expect("parser-deferred source request should start");
    timeout(Duration::from_secs(1), parser_interactive.notified())
        .await
        .expect("parser should reach interactive while the deferred source remains blocked");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session.session_id,
                "params": {
                    "expression": "JSON.stringify({ marker: globalThis.blockedDeferProtocolMarker, ready: document.readyState, deferExecuted: globalThis.blockedDeferExecuted === true })",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate while parser-deferred source is blocked");
    let evaluate_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
        .await
        .expect("Runtime.evaluate must not wait for the blocked parser-deferred source");
    assert!(
        !evaluate_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Runtime.evaluate only proves command independence if it returns before load: {evaluate_messages:#?}"
    );
    let response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(
            "{\"marker\":\"committed-source\",\"ready\":\"interactive\",\"deferExecuted\":false}"
        )
    );

    release_defer.notify_one();
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_uses_committed_page_while_parser_blocking_source_is_pending()
 {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.blockedParserProtocolMarker = 'committed-source';
</script>
<script src="/blocked-parser.js"></script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&script_requested);
    let release_for_route = Arc::clone(&release_script);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/blocked-parser.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "globalThis.blockedParserExecuted = true;",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "blocked-parser-protocol-command");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Runtime.enable"), (5_u64, "Page.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("parser-blocking source request should start");
    let _ = timeout(
        Duration::from_secs(1),
        recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
    )
    .await
    .expect("replacement default context should be published while its parser is blocked");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session.session_id,
                "params": {
                    "expression": "JSON.stringify({ marker: globalThis.blockedParserProtocolMarker, ready: document.readyState, bodyExists: document.body !== null, scriptExecuted: globalThis.blockedParserExecuted === true })",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate while parser-blocking source is pending");
    let evaluate_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
        .await
        .expect("Runtime.evaluate must use the committed page while its parser is blocked");
    assert!(
        !evaluate_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Runtime.evaluate must return before parser completion: {evaluate_messages:#?}"
    );
    let response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(
            "{\"marker\":\"committed-source\",\"ready\":\"loading\",\"bodyExists\":false,\"scriptExecuted\":false}"
        )
    );

    release_script.notify_one();
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_parser_script_navigation_progresses_without_followup_command() {
    const PASSIVE_PROGRESS_TIMEOUT: Duration = Duration::from_secs(5);

    async fn source_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head><script src="/navigate.js"></script></head>
<body><main id="source">source</main></body>
</html>"#,
        )
    }

    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let replacement_requested = Arc::new(tokio::sync::Notify::new());
    let requested_for_script = Arc::clone(&script_requested);
    let release_for_script = Arc::clone(&release_script);
    let requested_for_replacement = Arc::clone(&replacement_requested);
    let fixture_app = Router::new()
        .route("/source", get(source_page))
        .route(
            "/navigate.js",
            get(move || {
                let requested_for_script = Arc::clone(&requested_for_script);
                let release_for_script = Arc::clone(&release_for_script);
                async move {
                    requested_for_script.notify_one();
                    release_for_script.notified().await;
                    (
                        [(
                            axum::http::header::CONTENT_TYPE.as_str(),
                            "text/javascript",
                        )],
                        "location.href = '/replacement';",
                    )
                }
            }),
        )
        .route(
            "/replacement",
            get(move || {
                let requested_for_replacement = Arc::clone(&requested_for_replacement);
                async move {
                    requested_for_replacement.notify_one();
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id='replacement'>replacement</main></body></html>",
                    )
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "passive-parser-script-navigation");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.setLifecycleEventsEnabled",
        Some(&session.session_id),
        json!({ "enabled": true }),
    )
    .await;

    let source_url = format!("http://{fixture_addr}/source");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": source_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send source Page.navigate");
    timeout(PASSIVE_PROGRESS_TIMEOUT, script_requested.notified())
        .await
        .expect("parser-blocking navigation script request should start");
    let navigate_messages = timeout(PASSIVE_PROGRESS_TIMEOUT, recv_until_id(&mut socket, 6))
        .await
        .expect("Page.navigate should respond while the parser script is blocked");
    let source_loader_id = navigate_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("source Page.navigate loaderId")
        .to_owned();

    // No frontend command is sent after this release. The renderer-produced
    // owner action must receive an autonomous adapter turn.
    release_script.notify_one();
    timeout(PASSIVE_PROGRESS_TIMEOUT, replacement_requested.notified())
        .await
        .expect("replacement request must start without a follow-up CDP command");

    let replacement_url = format!("http://{fixture_addr}/replacement");
    let replacement_messages = timeout(
        PASSIVE_PROGRESS_TIMEOUT,
        recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() != Some(source_loader_id.as_str())
        }),
    )
    .await
    .expect("replacement DOMContentLoaded must arrive without a follow-up CDP command");
    let replacement_loader_id = replacement_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() != Some(source_loader_id.as_str())
        })
        .and_then(|message| message["params"]["loaderId"].as_str())
        .expect("replacement DOMContentLoaded loaderId");
    assert!(
        replacement_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"].as_str() == Some(replacement_url.as_str())
                && message["params"]["frame"]["loaderId"].as_str() == Some(replacement_loader_id)
        }),
        "replacement frame and DOMContentLoaded must use the same loader: {replacement_messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_replacement_retires_blocked_parser_defer_lifecycle() {
    async fn source_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.sourceDocumentMarker = 'source';
addEventListener('DOMContentLoaded', () => fetch('/stale-source-observed'));
</script>
<script defer src="/blocked-source-defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    async fn replacement_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head><script>globalThis.replacementDocumentMarker = 'replacement';</script></head>
<body><main id="replacement">replacement</main></body>
</html>"#,
        )
    }

    let defer_requested = Arc::new(tokio::sync::Notify::new());
    let release_defer = Arc::new(tokio::sync::Notify::new());
    let defer_response_sent = Arc::new(tokio::sync::Notify::new());
    let stale_source_observed = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&defer_requested);
    let release_for_route = Arc::clone(&release_defer);
    let response_sent_for_route = Arc::clone(&defer_response_sent);
    let stale_for_route = Arc::clone(&stale_source_observed);
    let fixture_app = Router::new()
        .route("/source", get(source_page))
        .route("/replacement", get(replacement_page))
        .route(
            "/blocked-source-defer.js",
            get(move || {
                let requested_for_route = Arc::clone(&requested_for_route);
                let release_for_route = Arc::clone(&release_for_route);
                let response_sent_for_route = Arc::clone(&response_sent_for_route);
                async move {
                    requested_for_route.notify_one();
                    release_for_route.notified().await;
                    response_sent_for_route.notify_one();
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "fetch('/stale-source-observed');",
                    )
                }
            }),
        )
        .route(
            "/stale-source-observed",
            get(move || {
                let stale_for_route = Arc::clone(&stale_for_route);
                async move {
                    stale_for_route.notify_one();
                    "stale"
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "blocked-defer-replacement");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Runtime.enable"), (5_u64, "Page.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/source") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send source Page.navigate");
    timeout(Duration::from_secs(1), defer_requested.notified())
        .await
        .expect("source parser-deferred request should start");

    timeout(
        Duration::from_secs(2),
        cdp_navigate_and_wait_for_load(
            &mut socket,
            7,
            &session.session_id,
            &format!("http://{fixture_addr}/replacement"),
        ),
    )
    .await
    .expect("replacement navigation must not wait for the source document's blocked defer");

    let replacement = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        8,
        "JSON.stringify({ marker: globalThis.replacementDocumentMarker, source: globalThis.sourceDocumentMarker, text: document.querySelector('#replacement')?.textContent })",
    )
    .await;
    assert_eq!(
        replacement,
        "{\"marker\":\"replacement\",\"text\":\"replacement\"}"
    );

    release_defer.notify_one();
    timeout(Duration::from_secs(1), defer_response_sent.notified())
        .await
        .expect("stale defer response should leave the fixture server");
    assert!(
        timeout(Duration::from_millis(250), stale_source_observed.notified())
            .await
            .is_err(),
        "the retired source defer script or DOMContentLoaded callback must not execute after replacement"
    );

    let replacement_after_stale_terminal = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        9,
        "JSON.stringify({ marker: globalThis.replacementDocumentMarker, source: globalThis.sourceDocumentMarker, text: document.querySelector('#replacement')?.textContent })",
    )
    .await;
    assert_eq!(replacement_after_stale_terminal, replacement);

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_replacement_cancels_source_document_xhrs() {
    const XHR_COUNT: usize = 4;
    const SOURCE_DOCUMENT_COUNT: usize = 2;

    async fn source_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<body>
<main>source</main>
<script>
const generation = new URL(location.href).searchParams.get('generation');
for (let index = 0; index < 4; index += 1) {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', `/held-xhr?generation=${generation}&index=${index}`);
  xhr.send();
}
</script>
</body>
</html>"#,
        )
    }

    let (xhr_requested_tx, mut xhr_requested_rx) = tokio::sync::mpsc::unbounded_channel();
    let release_xhrs = Arc::new(tokio::sync::Notify::new());
    let release_xhrs_for_route = Arc::clone(&release_xhrs);
    let fixture_app = Router::new().route("/source", get(source_page)).route(
        "/held-xhr",
        get(move || {
            let xhr_requested_tx = xhr_requested_tx.clone();
            let release_xhrs = Arc::clone(&release_xhrs_for_route);
            async move {
                xhr_requested_tx
                    .send(())
                    .expect("held XHR request observer should remain live");
                release_xhrs.notified().await;
                "late XHR response"
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "xhr-document-replacement");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Page.enable"), (5_u64, "Network.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    let mut messages = Vec::new();
    for generation in 0..SOURCE_DOCUMENT_COUNT {
        let source_url = format!("http://{fixture_addr}/source?generation={generation}");
        messages.extend(
            timeout(
                Duration::from_secs(5),
                cdp_navigate_and_wait_for_load(
                    &mut socket,
                    6 + generation as u64,
                    &session.session_id,
                    &source_url,
                ),
            )
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "source document generation {generation} should load while its XHRs remain pending"
                )
            }),
        );
        for request_index in 0..XHR_COUNT {
            timeout(Duration::from_secs(2), xhr_requested_rx.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!("held XHR {generation}:{request_index} should reach the fixture server")
                })
                .unwrap_or_else(|| {
                    panic!("held XHR observer closed at request {generation}:{request_index}")
                });
        }
    }

    messages.extend(
        send_cdp_command(
            &mut socket,
            20,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({ "expression": "document.querySelector('main').textContent" }),
        )
        .await,
    );

    let replacement_url =
        "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cmain%3Ereplacement%3C%2Fmain%3E";
    messages.extend(
        timeout(
            Duration::from_secs(5),
            cdp_navigate_and_wait_for_load(&mut socket, 21, &session.session_id, replacement_url),
        )
        .await
        .expect("replacement document should not wait for source-document XHRs"),
    );
    messages.extend(
        send_cdp_command(
            &mut socket,
            22,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({ "expression": "document.querySelector('main').textContent" }),
        )
        .await,
    );
    release_xhrs.notify_waiters();

    let xhr_requests = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let url = message["params"]["request"]["url"].as_str()?;
            (message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Network.requestWillBeSent")
                && url.contains("/held-xhr?generation="))
            .then(|| {
                let generation = usize::from(url.contains("generation=1"));
                message["params"]["requestId"]
                    .as_str()
                    .map(|request_id| (request_id.to_owned(), generation, index))
            })
            .flatten()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        xhr_requests.len(),
        XHR_COUNT * SOURCE_DOCUMENT_COUNT,
        "each source-document XHR should publish one request start: {messages:#?}"
    );
    let first_successor_xhr_index = xhr_requests
        .iter()
        .filter_map(|(_, generation, index)| (*generation == 1).then_some(*index))
        .min()
        .expect("second source Document should publish XHR starts");
    let final_load_index = messages
        .iter()
        .rposition(|message| message["method"] == json!("Page.loadEventFired"))
        .expect("final replacement should publish load");
    for (request_id, generation, start_index) in xhr_requests {
        let terminals = messages
            .iter()
            .enumerate()
            .filter(|message| {
                message.1["sessionId"].as_str() == Some(session.session_id.as_str())
                    && message.1["method"] == json!("Network.loadingFailed")
                    && message.1["params"]["requestId"] == json!(request_id)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminals.len(),
            1,
            "replacement must publish exactly one canceled terminal for old request {request_id}: {messages:#?}"
        );
        let (terminal_index, terminal) = terminals[0];
        assert_eq!(terminal["params"]["errorText"], json!("net::ERR_ABORTED"));
        assert_eq!(terminal["params"]["canceled"], json!(true));
        assert!(
            terminal_index > start_index,
            "request terminal must follow its announced start"
        );
        let successor_boundary = if generation == 0 {
            first_successor_xhr_index
        } else {
            final_load_index
        };
        assert!(
            terminal_index < successor_boundary,
            "old request {request_id} must terminate before its successor Document becomes observable: {messages:#?}"
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_continue_response_ack_does_not_wait_for_document_body_tail() {
    let release_tail = Arc::new(tokio::sync::Notify::new());
    let (fixture_addr, fixture_server) =
        spawn_response_stage_streaming_document_fixture_server(Arc::clone(&release_tail)).await;

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (5_u64, "Network.enable", json!({})),
        (
            6_u64,
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*",
                    "requestStage": "Request",
                    "resourceType": "Document"
                }]
            }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/page") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let request_stage_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
            && message["params"]["responseStatusCode"].is_null()
    })
    .await;
    let request_stage_pause = request_stage_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
                && message["params"]["responseStatusCode"].is_null()
        })
        .expect("request-stage document pause");
    let request_id = request_stage_pause["params"]["requestId"]
        .as_str()
        .expect("request-stage request id")
        .to_owned();

    let _ = send_cdp_command(
        &mut socket,
        8,
        "Fetch.continueRequest",
        Some(&session.session_id),
        json!({ "requestId": request_id, "interceptResponse": true }),
    )
    .await;

    let response_stage_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
            && message["params"]["responseStatusCode"] == json!(200)
    })
    .await;
    let response_stage_pause = response_stage_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .expect("response-stage document pause");
    let response_request_id = response_stage_pause["params"]["requestId"]
        .as_str()
        .expect("response-stage request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Fetch.continueResponse",
                "sessionId": session.session_id,
                "params": { "requestId": response_request_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Fetch.continueResponse");

    let ack_before_tail = timeout(Duration::from_millis(300), recv_until_id(&mut socket, 9)).await;
    if ack_before_tail.is_err() {
        release_tail.notify_one();
        let after_tail = timeout(Duration::from_secs(2), recv_until_id(&mut socket, 9))
            .await
            .expect("Fetch.continueResponse should eventually return after tail release");
        panic!(
            "Fetch.continueResponse ACK waited for main-document body tail; \
             Chromium replies before body EOF, after-tail messages: {after_tail:#?}"
        );
    }
    let ack_messages = ack_before_tail.expect("checked above");
    assert!(
        ack_messages.iter().any(|message| {
            message["id"] == json!(9_u64)
                && message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["result"] == json!({})
        }),
        "Fetch.continueResponse should ACK before the delayed body tail: {ack_messages:#?}"
    );

    release_tail.notify_one();
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_fetch_fulfill_bypasses_navigation_blocked_command() {
    let fixture_app = Router::new().route(
        "/page",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>unfulfilled fixture</main></body></html>",
            )
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "fetch-navigation-command-bypass");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (
            5_u64,
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*",
                    "requestStage": "Request",
                    "resourceType": "Document"
                }]
            }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/page") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let pause_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
    })
    .await;
    let request_id = pause_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("document request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.createIsolatedWorld",
                "sessionId": session.session_id,
                "params": {
                    "frameId": session.target_id,
                    "worldName": "__playwright_utility_world_page",
                    // Chromium's CDP schema intentionally retains this spelling.
                    "grantUniveralAccess": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send navigation-blocked Page.createIsolatedWorld");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Fetch.fulfillRequest",
                "sessionId": session.session_id,
                "params": {
                    "requestId": request_id,
                    "responseCode": 200,
                    "responseHeaders": [{
                        "name": "content-type",
                        "value": "text/html; charset=utf-8"
                    }],
                    "body": BASE64_STANDARD.encode(
                        "<!doctype html><main>fulfilled navigation</main>"
                    )
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Fetch.fulfillRequest");

    let mut messages = timeout(Duration::from_secs(2), recv_until_id(&mut socket, 8))
        .await
        .expect("Fetch.fulfillRequest must bypass the blocked navigation command");
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(8_u64)
                && message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["result"] == json!({})
        }),
        "Fetch.fulfillRequest response missing: {messages:#?}"
    );

    if !messages.iter().any(|message| message["id"] == json!(7_u64)) {
        messages.extend(
            timeout(Duration::from_secs(2), recv_until_id(&mut socket, 7))
                .await
                .expect("blocked Page.createIsolatedWorld should resume after navigation"),
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(7_u64)
                && message["result"]["executionContextId"].as_u64().is_some()
        }),
        "Page.createIsolatedWorld response missing after fulfill: {messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_pending_awaitpromise_does_not_block_later_command() {
    let fixture_app = Router::new().route(
        "/page",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>pending cdp command</main></body></html>",
            )
        }),
    );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "cdp-pending-awaitpromise");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(() => {})",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send permanently pending Runtime.evaluate");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "'later-command-ready'",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate while an earlier awaitPromise is pending");

    let mut messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
        .await
        .expect("later Runtime.evaluate should not be blocked by an earlier pending command");
    let later_response = messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("later Runtime.evaluate response");
    assert!(
        later_response.get("error").is_none(),
        "later Runtime.evaluate should succeed while the earlier command is pending: {messages:#?}"
    );
    assert_eq!(
        later_response["result"]["result"]["value"],
        json!("later-command-ready")
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "HeapProfiler.collectGarbage",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("collect an unreachable pending Runtime.evaluate promise");
    messages.extend(
        timeout(Duration::from_secs(1), recv_until_id(&mut socket, 8))
            .await
            .expect("HeapProfiler.collectGarbage should complete"),
    );

    let collected_responses = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["id"] == json!(6_u64))
        .collect::<Vec<_>>();
    assert_eq!(
        collected_responses.len(),
        1,
        "an unreachable pending promise should receive exactly one terminal response after explicit GC: {messages:#?}"
    );
    let (collected_index, collected) = collected_responses[0];
    assert_eq!(collected["error"]["code"], json!(-32000), "{collected:?}");
    assert_eq!(
        collected["error"]["message"],
        json!("Promise was collected"),
        "V8 Inspector should preserve Chromium's weak pending-promise collection semantics: {collected:?}"
    );
    let garbage_collection_index = messages
        .iter()
        .position(|message| message["id"] == json!(8_u64))
        .expect("HeapProfiler.collectGarbage response");
    assert!(
        collected_index < garbage_collection_index,
        "the weak Promise callback must report collection before collectGarbage completes: {messages:#?}"
    );
    assert_eq!(
        messages[garbage_collection_index]["result"],
        json!({}),
        "HeapProfiler.collectGarbage should succeed: {messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
// A Runtime.callFunctionOn(awaitPromise=true) command must park only its own
// response. Fetch events triggered by the awaited promise still need to reach
// the client so the request can be fulfilled and the promise can settle.
async fn websocket_cdp_runtime_call_function_awaitpromise_fetch_interception_unblocks() {
    let fixture_app = Router::new().route(
        "/page",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>runtime fetch interception</main></body></html>",
            )
        }),
    );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-fetch-interception");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    let _ = send_cdp_command(
        &mut socket,
        6,
        "Fetch.enable",
        Some(&session_id),
        json!({
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request"
            }]
        }),
    )
    .await;
    let utility_object = send_cdp_command(
        &mut socket,
        7,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "({})"
        }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.callFunctionOn",
                "sessionId": session_id,
                "params": {
                    "objectId": utility_object_id,
                    "functionDeclaration": "() => fetch('/popup-api').then(async response => ({ url: location.href, api: await response.json() }))",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.callFunctionOn awaitPromise fetch");

    let pause_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("XHR")
            && message["params"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/popup-api"))
    })
    .await;
    let paused = pause_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("XHR")
        })
        .expect("runtime fetch requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("requestPaused requestId")
        .to_owned();

    let fulfill_messages = send_cdp_command(
        &mut socket,
        9,
        "Fetch.fulfillRequest",
        Some(&session_id),
        json!({
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{
                "name": "content-type",
                "value": "application/json; charset=utf-8"
            }],
            "body": BASE64_STANDARD.encode(r#"{"source":"runtime route","ok":true}"#)
        }),
    )
    .await;
    let runtime_response = if let Some(message) = fulfill_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .cloned()
    {
        message
    } else {
        recv_until_id(&mut socket, 8)
            .await
            .into_iter()
            .find(|message| message["id"] == json!(8_u64))
            .expect("Runtime.callFunctionOn response")
    };

    assert_eq!(
        runtime_response["result"]["result"]["value"]["api"]["source"],
        "runtime route"
    );
    assert_eq!(
        runtime_response["result"]["result"]["value"]["api"]["ok"],
        true
    );
    assert_eq!(
        runtime_response["result"]["result"]["value"]["url"],
        page_url
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_fetch_routes_pending_background_parser_script_to_exact_session() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script src="/app.js?parser=1"></script>
</head>
<body data-app-loaded="false"><main>parser script fetch interception</main></body>
</html>"#,
        )
    }

    async fn app_js() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            "window.__moliParserScriptExecuted = true;",
        )
    }

    async fn blank() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>blank</title>",
        )
    }

    let fixture_app = Router::new()
        .route("/blank", get(blank))
        .route("/page", get(page))
        .route("/app.js", get(app_js));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "parser-script-fetch-abort");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let blank_url = format!("http://{fixture_addr}/blank");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let active = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let background = cdp_create_attached_target(&mut socket, 4, &browser_context_id).await;
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 6, &active.session_id, &blank_url).await;
    let _ =
        cdp_navigate_and_wait_for_load(&mut socket, 7, &background.session_id, &blank_url).await;
    for (id, method, params) in [
        (8_u64, "Runtime.enable", json!({})),
        (9_u64, "Page.enable", json!({})),
        (10_u64, "Network.enable", json!({})),
        (
            11_u64,
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*app.js*",
                    "requestStage": "Request"
                }]
            }),
        ),
    ] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&background.session_id),
            params,
        )
        .await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Page.navigate",
                "sessionId": background.session_id,
                "params": { "url": page_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(background.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Script")
            && message["params"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.contains("/app.js?parser=1"))
    })
    .await;
    let paused = observed
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(background.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Script")
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("/app.js?parser=1"))
        })
        .expect("parser script Fetch.requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("parser script requestId")
        .to_owned();
    assert!(
        !observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(active.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("/app.js?parser=1"))
        }),
        "a parser fetch from the pending background Page must not fall back to the active target: \
         {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            13,
            "Fetch.failRequest",
            Some(&background.session_id),
            json!({ "requestId": request_id, "errorReason": "BlockedByClient" }),
        )
        .await,
    );

    if !observed.iter().any(|message| {
        message["sessionId"].as_str() == Some(background.session_id.as_str())
            && message["method"] == json!("Network.loadingFailed")
            && message["params"]["type"] == json!("Script")
            && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(background.session_id.as_str())
                    && message["method"] == json!("Network.loadingFailed")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
            })
            .await,
        );
    }

    if !observed.iter().any(|message| {
        message["sessionId"].as_str() == Some(background.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(background.session_id.as_str())
                    && message["method"] == json!("Page.loadEventFired")
            })
            .await,
        );
    }

    assert!(
        observed
            .iter()
            .any(|message| message["id"] == json!(12_u64)),
        "Page.navigate should reply while parser script abort is handled: {observed:#?}"
    );

    let execution = cdp_runtime_evaluate_string(
        &mut socket,
        &background.session_id,
        14,
        "JSON.stringify({external: window.__moliParserScriptExecuted === true, bodyFlag: document.body && document.body.dataset.appLoaded, main: document.querySelector('main') && document.querySelector('main').textContent})",
    )
    .await;
    assert_eq!(
        execution,
        r#"{"external":false,"bodyFlag":"false","main":"parser script fetch interception"}"#,
        "aborted parser script must not execute while the parser still reaches later DOM"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_debugger_step_out_responds_before_resumed_and_caller_pause() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    let _ = send_cdp_command(
        &mut socket,
        3,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let enabled = send_cdp_command(
        &mut socket,
        4,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        enabled.iter().any(|message| message["id"] == json!(4_u64)),
        "Debugger.enable should complete before the pause witness: {enabled:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "(function outer(){ function inner(){ debugger; return 40; } return inner() + 2; })()",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that enters the debugger");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Debugger.paused")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(5_u64)),
        "Runtime.evaluate must remain pending while the renderer owner is paused: {observed:#?}"
    );
    let initial_pause = observed
        .iter()
        .find(|message| message["method"] == json!("Debugger.paused"))
        .expect("nested Runtime.evaluate should emit Debugger.paused");
    assert_eq!(initial_pause["params"]["reason"], json!("other"));
    assert_eq!(
        initial_pause["params"]["callFrames"][0]["functionName"],
        json!("inner")
    );

    let mut stepped = send_cdp_command(
        &mut socket,
        6,
        "Debugger.stepOut",
        Some(&session_id),
        json!({}),
    )
    .await;
    if !stepped.iter().any(|message| {
        message["method"] == json!("Debugger.paused")
            && message["params"]["reason"] == json!("step")
            && message["params"]["callFrames"][0]["functionName"] == json!("outer")
    }) {
        stepped.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(session_id.as_str())
                    && message["method"] == json!("Debugger.paused")
                    && message["params"]["reason"] == json!("step")
                    && message["params"]["callFrames"][0]["functionName"] == json!("outer")
            })
            .await,
        );
    }
    let response_position = stepped
        .iter()
        .position(|message| message["id"] == json!(6_u64))
        .expect("stepOut should respond");
    let resumed_position = stepped
        .iter()
        .position(|message| message["method"] == json!("Debugger.resumed"))
        .expect("stepOut should emit Debugger.resumed");
    let caller_pause_position = stepped
        .iter()
        .position(|message| {
            message["method"] == json!("Debugger.paused")
                && message["params"]["reason"] == json!("step")
                && message["params"]["callFrames"][0]["functionName"] == json!("outer")
        })
        .expect("stepOut should pause in the caller");
    assert!(
        response_position < resumed_position && resumed_position < caller_pause_position,
        "stepOut must preserve response -> resumed -> caller pause: {stepped:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            7,
            "Debugger.resume",
            Some(&session_id),
            json!({}),
        )
        .await,
    );
    if !observed.iter().any(|message| message["id"] == json!(5_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(5_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("Runtime.evaluate should complete after Debugger.resume");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(42),
        "resuming the exact pause must return to the blocked Runtime command: {observed:#?}"
    );
    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_debugger_pause_allows_auxiliary_main_thread_commands() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    let _ = send_cdp_command(
        &mut socket,
        3,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let enabled = send_cdp_command(
        &mut socket,
        4,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        enabled.iter().any(|message| message["id"] == json!(4_u64)),
        "Debugger.enable should complete before the pause witness: {enabled:#?}"
    );

    let auxiliary_attach = send_cdp_command(
        &mut socket,
        5,
        "Target.attachToTarget",
        None,
        json!({ "targetId": session.target_id, "flatten": true }),
    )
    .await;
    let auxiliary_session_id = auxiliary_attach
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("auxiliary session id")
        .to_owned();
    let auxiliary_enabled = send_cdp_command(
        &mut socket,
        50,
        "Debugger.enable",
        Some(&auxiliary_session_id),
        json!({}),
    )
    .await;
    assert!(
        auxiliary_enabled
            .iter()
            .any(|message| message["id"] == json!(50_u64) && message.get("error").is_none()),
        "auxiliary Debugger.enable should create its V8 inspector session: {auxiliary_enabled:#?}"
    );
    let isolated_world = send_cdp_command(
        &mut socket,
        49,
        "Page.createIsolatedWorld",
        Some(&auxiliary_session_id),
        json!({
            "frameId": session.target_id,
            "worldName": "nested-main-owner-boundary",
        }),
    )
    .await;
    let isolated_context_id = isolated_world
        .iter()
        .find(|message| message["id"] == json!(49_u64))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .unwrap_or_else(|| {
            panic!("Page.createIsolatedWorld should return executionContextId: {isolated_world:#?}")
        });

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "debugger; 42",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that enters the debugger");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Debugger.paused")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(6_u64)),
        "Runtime.evaluate must remain pending while the renderer owner is paused: {observed:#?}"
    );
    if observed.iter().all(|message| {
        message["sessionId"].as_str() != Some(auxiliary_session_id.as_str())
            || message["method"] != json!("Debugger.paused")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                    && message["method"] == json!("Debugger.paused")
            })
            .await,
        );
    }
    let auxiliary_call_frame_id = observed
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                && message["method"] == json!("Debugger.paused")
        })
        .and_then(|message| message["params"]["callFrames"][0]["callFrameId"].as_str())
        .expect("auxiliary Debugger.paused callFrameId")
        .to_owned();
    // Chromium's normal debugger loop pumps its main-thread DevTools receiver.
    // This command must therefore reach the auxiliary V8 session even though
    // the ordinary Page owner turn that entered the pause has not returned.
    let auxiliary_evaluate = tokio::time::timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            7,
            "Runtime.evaluate",
            Some(&auxiliary_session_id),
            json!({ "expression": "21 * 2", "returnByValue": true }),
        ),
    )
    .await
    .expect("auxiliary Main Runtime.evaluate must complete in the debugger loop");
    assert!(
        auxiliary_evaluate.iter().any(|message| {
            message["id"] == json!(7_u64)
                && message["sessionId"] == json!(auxiliary_session_id)
                && message["result"]["result"]["value"] == json!(42)
        }),
        "pause-loop Main Runtime.evaluate should complete before resume: {auxiliary_evaluate:#?}"
    );

    let auxiliary_object = tokio::time::timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            51,
            "Runtime.evaluate",
            Some(&auxiliary_session_id),
            json!({ "expression": "({ answer: 42 })" }),
        ),
    )
    .await
    .expect("object-valued Main Runtime.evaluate must complete in the debugger loop");
    assert!(
        auxiliary_object.iter().any(|message| {
            message["id"] == json!(51_u64)
                && message["sessionId"] == json!(auxiliary_session_id)
                && message["result"]["result"]["type"] == json!("object")
                && message["result"]["result"]["objectId"]
                    .as_str()
                    .is_some_and(|object_id| !object_id.is_empty())
        }),
        "pause-loop object response must remain owner-independent: {auxiliary_object:#?}"
    );
    let auxiliary_object_id = auxiliary_object
        .iter()
        .find(|message| message["id"] == json!(51_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("auxiliary Runtime.evaluate objectId")
        .to_owned();

    let properties = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            53,
            "Runtime.getProperties",
            Some(&auxiliary_session_id),
            json!({ "objectId": auxiliary_object_id, "ownProperties": true }),
        ),
    )
    .await
    .expect("Runtime.getProperties must complete in the nested Main loop");
    assert!(
        properties.iter().any(|message| {
            message["id"] == json!(53_u64)
                && message["result"]["result"]
                    .as_array()
                    .is_some_and(|properties| {
                        properties.iter().any(|property| {
                            property["name"] == json!("answer")
                                && property["value"]["value"] == json!(42)
                        })
                    })
        }),
        "nested Main must expose paused object properties: {properties:#?}"
    );

    let called = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            54,
            "Runtime.callFunctionOn",
            Some(&auxiliary_session_id),
            json!({
                "objectId": auxiliary_object_id,
                "functionDeclaration": "function () { return this.answer + 1; }",
                "returnByValue": true,
            }),
        ),
    )
    .await
    .expect("object-targeted Runtime.callFunctionOn must complete in the nested Main loop");
    assert!(
        called.iter().any(|message| {
            message["id"] == json!(54_u64) && message["result"]["result"]["value"] == json!(43)
        }),
        "nested Main must call a function on the paused object: {called:#?}"
    );

    let call_frame_evaluate = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            55,
            "Debugger.evaluateOnCallFrame",
            Some(&auxiliary_session_id),
            json!({
                "callFrameId": auxiliary_call_frame_id,
                "expression": "40 + 2",
                "returnByValue": true,
            }),
        ),
    )
    .await
    .expect("Debugger.evaluateOnCallFrame must complete in the nested Main loop");
    assert!(
        call_frame_evaluate.iter().any(|message| {
            message["id"] == json!(55_u64) && message["result"]["result"]["value"] == json!(42)
        }),
        "nested Main must evaluate against the auxiliary paused call frame: \
         {call_frame_evaluate:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 52_u64,
                "method": "Runtime.evaluate",
                "sessionId": auxiliary_session_id,
                "params": {
                    "expression": "Promise.resolve(43)",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pause-loop awaitPromise Runtime.evaluate");

    let explicit_context_evaluate = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            56,
            "Runtime.evaluate",
            Some(&auxiliary_session_id),
            json!({
                "contextId": isolated_context_id,
                "expression": "globalThis.__nestedInspectorContext = 44",
                "returnByValue": true,
            }),
        ),
    )
    .await
    .expect("explicit-context Runtime.evaluate must complete in the nested Main loop");
    assert!(
        explicit_context_evaluate.iter().any(|message| {
            message["id"] == json!(56_u64)
                && message["sessionId"] == json!(auxiliary_session_id)
                && message["result"]["result"]["value"] == json!(44)
        }),
        "nested Main must dispatch an explicit native Inspector context without Page owner: \
         {explicit_context_evaluate:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Debugger.resume",
            Some(&auxiliary_session_id),
            json!({}),
        )
        .await,
    );
    if !observed.iter().any(|message| message["id"] == json!(6_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(6_u64)).await);
    }
    if !observed
        .iter()
        .any(|message| message["id"] == json!(52_u64))
    {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(52_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("Runtime.evaluate should complete after Debugger.resume");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(42),
        "resuming the exact pause must return to the blocked Runtime command: {observed:#?}"
    );
    assert!(
        observed.iter().any(|message| {
            message["id"] == json!(52_u64)
                && message["sessionId"] == json!(auxiliary_session_id)
                && message["result"]["result"]["value"] == json!(43)
        }),
        "awaitPromise should settle after Debugger.resume without a deferred-reply deadlock: \
         {observed:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_handle_javascript_dialog_accept_resumes_confirm_with_true() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let enable =
        send_cdp_command(&mut socket, 6, "Page.enable", Some(&session_id), json!({})).await;
    assert!(
        enable
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message["result"] == json!({})),
        "Page.enable should resolve before dialog handling: {enable:#?}"
    );
    let _ = cdp_navigate_and_wait_for_load(
        &mut socket,
        11,
        &session_id,
        "data:text/html,<title>dialog replacement</title>",
    )
    .await;

    let scheduled = send_cdp_command(
        &mut socket,
        100,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "setTimeout(() => { window.__moliScheduledConfirm = confirm('moli scheduled confirm'); }, 0); 'scheduled'",
            "returnByValue": true
        }),
    )
    .await;
    assert!(
        scheduled.iter().any(|message| {
            message["id"] == json!(100_u64)
                && message["result"]["result"]["value"] == json!("scheduled")
        }),
        "scheduling a timer dialog must reply before the timer task blocks: {scheduled:#?}"
    );
    let mut scheduled_dialog = scheduled;
    if !scheduled_dialog.iter().any(|message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli scheduled confirm")
    }) {
        scheduled_dialog.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(session_id.as_str())
                    && message["method"] == json!("Page.javascriptDialogOpening")
                    && message["params"]["message"] == json!("moli scheduled confirm")
            })
            .await,
        );
    }
    scheduled_dialog.extend(
        send_cdp_command(
            &mut socket,
            101,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    if !scheduled_dialog
        .iter()
        .any(|message| message["id"] == json!(101_u64))
    {
        scheduled_dialog
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(101_u64)).await);
    }
    let scheduled_result = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        102,
        "String(window.__moliScheduledConfirm)",
    )
    .await;
    assert_eq!(scheduled_result, "true");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": r#"
                        confirm("moli confirm");
                    "#,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens confirm");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["type"] == json!("confirm")
            && message["params"]["message"] == json!("moli confirm")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(7_u64)),
        "Runtime.evaluate must remain pending while confirm is open: {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
        }),
        "accepting confirm should emit javascriptDialogClosed: {observed:#?}"
    );

    if !observed.iter().any(|message| message["id"] == json!(7_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(7_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate should resolve after confirm is handled");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(true),
        "accepted confirm should return true to page JavaScript: {observed:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "confirm('moli dismiss confirm')",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens dismissed confirm");
    let mut dismissed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli dismiss confirm")
    })
    .await;
    assert!(
        !dismissed
            .iter()
            .any(|message| message["id"] == json!(9_u64)),
        "dismissed confirm must also remain pending before handle: {dismissed:#?}"
    );
    dismissed.extend(
        send_cdp_command(
            &mut socket,
            10,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": false }),
        )
        .await,
    );
    if !dismissed
        .iter()
        .any(|message| message["id"] == json!(9_u64))
    {
        dismissed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(9_u64)).await);
    }
    let evaluate = dismissed
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("Runtime.evaluate should resolve after confirm is dismissed");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(false),
        "dismissed confirm should return false to page JavaScript: {dismissed:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(resolve => setTimeout(() => resolve(confirm('moli timer confirm')), 0))",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens confirm from a timer");
    let mut timer = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli timer confirm")
    })
    .await;
    assert!(
        !timer.iter().any(|message| message["id"] == json!(12_u64)),
        "awaitPromise evaluation must remain pending while timer confirm is open: {timer:#?}"
    );
    timer.extend(
        send_cdp_command(
            &mut socket,
            13,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    if !timer.iter().any(|message| message["id"] == json!(12_u64)) {
        timer.extend(recv_until_match(&mut socket, |message| message["id"] == json!(12_u64)).await);
    }
    let evaluate = timer
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .expect("awaitPromise Runtime.evaluate should resolve after timer confirm is accepted");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(true),
        "accepted timer confirm should resolve awaitPromise with true: {timer:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_handle_javascript_dialog_prompt_text_resumes_prompt() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(&mut socket, 6, "Page.enable", Some(&session_id), json!({})).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": r#"
                        prompt("moli prompt", "default answer");
                    "#,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens prompt");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["type"] == json!("prompt")
            && message["params"]["message"] == json!("moli prompt")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(7_u64)),
        "Runtime.evaluate must remain pending while prompt is open: {observed:#?}"
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["defaultPrompt"] == json!("default answer")
        }),
        "prompt opening should include defaultPrompt: {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true, "promptText": "typed answer" }),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
        }),
        "accepting prompt should emit javascriptDialogClosed: {observed:#?}"
    );

    if !observed.iter().any(|message| message["id"] == json!(7_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(7_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate should resolve after prompt is handled");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("typed answer"),
        "accepted prompt should return the supplied text to page JavaScript: {observed:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "prompt('moli dismiss prompt', 'default answer')",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens dismissed prompt");
    let mut dismissed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli dismiss prompt")
    })
    .await;
    assert!(
        !dismissed
            .iter()
            .any(|message| message["id"] == json!(9_u64)),
        "dismissed prompt must remain pending before handle: {dismissed:#?}"
    );
    dismissed.extend(
        send_cdp_command(
            &mut socket,
            10,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": false }),
        )
        .await,
    );
    if !dismissed
        .iter()
        .any(|message| message["id"] == json!(9_u64))
    {
        dismissed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(9_u64)).await);
    }
    let evaluate = dismissed
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("Runtime.evaluate should resolve after prompt is dismissed");
    assert_eq!(
        evaluate["result"]["result"]["subtype"],
        json!("null"),
        "dismissed prompt should return null to page JavaScript: {dismissed:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_lifecycle_events_include_load_marker() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>lifecycle load</main></body></html>",
        )
    }

    async fn blank() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>blank</title>",
        )
    }

    let fixture_app = Router::new()
        .route("/blank", get(blank))
        .route("/page", get(page));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "lifecycle-load-marker");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let blank_url = format!("http://{fixture_addr}/blank");
    let session_id = cdp_create_session_and_navigate(&mut socket, &blank_url).await;
    let _ = send_cdp_command(&mut socket, 6, "Page.enable", Some(&session_id), json!({})).await;
    let _ = send_cdp_command(
        &mut socket,
        7,
        "Page.setLifecycleEventsEnabled",
        Some(&session_id),
        json!({ "enabled": true }),
    )
    .await;

    let page_url = format!("http://{fixture_addr}/page");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": page_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["name"] == json!("load")
    })
    .await;
    let loader_id = messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("Page.navigate should return the committed loaderId");
    let init_index = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("init")
                && message["params"]["loaderId"].as_str() == Some(loader_id)
        })
        .expect("new document should emit lifecycle init for the navigation loader");
    let frame_navigated_index = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["loaderId"].as_str() == Some(loader_id)
        })
        .expect("new document should commit the same navigation loader");
    let load_index = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("load")
                && message["params"]["loaderId"].as_str() == Some(loader_id)
        })
        .expect("new document should load with the same navigation loader");
    assert!(
        init_index < frame_navigated_index && frame_navigated_index < load_index,
        "navigation lifecycle should order init before frame commit and load: {messages:#?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Page.loadEventFired should accompany lifecycle load: {messages:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_same_document_navigation_after_dcl_does_not_cancel_pending_load() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><link rel='stylesheet' href='/slow.css'></head><body><main id='ready'>ready</main></body></html>",
        )
    }
    let css_requested = Arc::new(tokio::sync::Notify::new());
    let release_css = Arc::new(tokio::sync::Notify::new());
    let css_requested_for_route = Arc::clone(&css_requested);
    let release_css_for_route = Arc::clone(&release_css);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.css",
        get(move || {
            let css_requested_for_route = Arc::clone(&css_requested_for_route);
            let release_css_for_route = Arc::clone(&release_css_for_route);
            async move {
                css_requested_for_route.notify_one();
                release_css_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                    "body { color: black; }",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "same-document-pending-load");
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 1).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 2).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    for (id, method) in [
        (3_u64, "Page.enable"),
        (4_u64, "Runtime.enable"),
        (5_u64, "Page.setLifecycleEventsEnabled"),
    ] {
        let params = if method == "Page.setLifecycleEventsEnabled" {
            json!({ "enabled": true })
        } else {
            json!({})
        };
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id,
                    "params": params
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap_or_else(|_| panic!("send {method}"));
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), css_requested.notified())
        .await
        .expect("stylesheet request should be pending before same-document navigation");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "location.hash = 'after-dcl'; location.href",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate_messages = recv_until_id(&mut socket, 7).await;
    let evaluate_response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert!(
        evaluate_response.get("error").is_none(),
        "same-document navigation after DCL should be accepted while load is still pending: {evaluate_response}"
    );
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!(format!("{fixture_url}#after-dcl"))
    );
    release_css.notify_one();

    let mut saw_load_event = evaluate_messages.iter().any(|message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    });
    if !saw_load_event {
        let load_messages = recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .await;
        saw_load_event = load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        });
    }
    assert!(
        saw_load_event,
        "same-document URL changes must not make the current document's deferred load completion stale"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_raw_client_second_navigate_does_not_cancel_subsequent_awaitpromise() {
    // Regression test for the cdp-session race that was missed by the
    // first navigation gate: `has_inflight_background_navigation` originally
    // checked `loaded_page.is_none()`, which is only true on the very
    // first Page.navigate per target (before any page is installed).
    //
    // On every SUBSEQUENT Page.navigate, the old loaded_page survives
    // until the new completion commits — so the flag returned false,
    // the drain hook did NOT fire, and:
    //   T0: client → Page.navigate (run N, N>=2).
    //   T1: client → Runtime.evaluate(awaitPromise=true). Handler registers
    //       a pending inspector await on the OLD page's context.
    //   T2: background completion arrives → commit_loaded_navigation_page
    //       swaps OLD → NEW, then fail_pending_inspector_awaits cancels the
    //       await from T1 with "Page navigated". Client sees command failed.
    //
    // The counter-based fix tracks "background nav spawned but not yet
    // drained" explicitly, so the gate applies to every in-flight navigation,
    // not just the first one. This test sends TWO sequential
    // navigate+evaluate cycles over the same session and asserts both
    // evaluates succeed.
    async fn page_a() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><span id='m'>A</span></body></html>",
        )
    }
    async fn page_b() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><span id='m'>B</span></body></html>",
        )
    }
    let fixture_app = Router::new()
        .route("/a", get(page_a))
        .route("/b", get(page_b));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    // Bring up a single session and reuse it for both cases.
    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let _ = recv_until_id(&mut socket, 1).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|m| m["id"] == json!(2_u64))
        .and_then(|m| m["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|m| m["id"] == json!(3_u64))
        .and_then(|m| m["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    // Cycle 1: navigate(/a) → evaluate awaitPromise that polls #m.
    let mut next_id = 4_u64;
    for (cycle, (path, expected)) in [("/a", "A"), ("/b", "B")].iter().enumerate() {
        let url = format!("http://{fixture_addr}{path}");
        let navigate_id = next_id;
        next_id += 1;
        socket
            .send(WsMessage::Text(
                json!({
                    "id": navigate_id,
                    "method": "Page.navigate",
                    "sessionId": session_id,
                    "params": { "url": url }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Page.navigate");

        let evaluate_id = next_id;
        next_id += 1;
        socket
            .send(WsMessage::Text(
                json!({
                    "id": evaluate_id,
                    "method": "Runtime.evaluate",
                    "sessionId": session_id,
                    "params": {
                        "expression": "new Promise(resolve => { const deadline = Date.now() + 3000; (function tick() { const n = document.querySelector('#m'); if (n && n.textContent) resolve(n.textContent); else if (Date.now() > deadline) resolve(''); else setTimeout(tick, 10); })(); })",
                        "awaitPromise": true,
                        "returnByValue": true
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Runtime.evaluate");

        let messages = recv_until_id(&mut socket, evaluate_id).await;
        let eval_response = messages
            .iter()
            .find(|m| m["id"] == json!(evaluate_id))
            .expect("evaluate response");
        assert!(
            eval_response.get("error").is_none(),
            "cycle {cycle}: Runtime.evaluate must not be cancelled by the preceding Page.navigate commit; got {eval_response}"
        );
        let value = eval_response["result"]["result"]["value"]
            .as_str()
            .expect("evaluate value should be a string");
        assert_eq!(
            value, *expected,
            "cycle {cycle}: evaluate must observe the new document body for {path}"
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_self_navigation_gate_is_not_applied_to_next_command() {
    async fn plain() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>plain</main></body></html>",
        )
    }
    async fn history_a() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>history a</main></body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/plain", get(plain))
        .route("/history-a", get(history_a));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = target.session_id;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(&mut socket, 5, "Page.enable", Some(&session_id), json!({})).await;
    let plain_url = format!("http://{fixture_addr}/plain");
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 6, &session_id, &plain_url).await;

    let history_url = format!("http://{fixture_addr}/history-a?open-self=1");
    let _ = send_cdp_command(
        &mut socket,
        7,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": format!("window.open('{}', '_self')", history_url),
            "returnByValue": true
        }),
    )
    .await;
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;

    let one_plus_one = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "1 + 1",
            "returnByValue": true
        }),
    )
    .await;
    assert!(
        one_plus_one.iter().any(|message| {
            message["id"] == json!(8_u64) && message["result"]["result"]["value"] == json!(2_u64)
        }),
        "sanity Runtime.evaluate should succeed after self navigation: {one_plus_one:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "typeof globalThis.__pwClock",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-navigation Runtime.evaluate");
    let typeof_clock = timeout(Duration::from_secs(3), recv_until_id(&mut socket, 9))
        .await
        .expect("post-navigation Runtime.evaluate must not wait on stale navigation gate");
    assert!(
        typeof_clock
            .iter()
            .any(|message| message["id"] == json!(9_u64) && message.get("result").is_some()),
        "post-navigation Runtime.evaluate should return a result: {typeof_clock:#?}"
    );

    let main_text = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        10,
        "document.querySelector('main') && document.querySelector('main').textContent",
    )
    .await;
    assert_eq!(main_text, "history a");

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_window_open_self_navigation_reaches_auxiliary_page_session() {
    async fn plain() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>plain</main></body></html>",
        )
    }
    async fn history_a() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>history a</main></body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/plain", get(plain))
        .route("/history-a", get(history_a));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let primary_session_id = target.session_id;
    let target_id = target.target_id;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&primary_session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&primary_session_id),
        json!({}),
    )
    .await;

    let browser_attach = send_cdp_command(
        &mut socket,
        6,
        "Target.attachToBrowserTarget",
        None,
        json!({}),
    )
    .await;
    let browser_session_id = browser_attach
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("browser session id")
        .to_owned();
    let auxiliary_attach = send_cdp_command(
        &mut socket,
        7,
        "Target.attachToTarget",
        Some(&browser_session_id),
        json!({ "targetId": target_id }),
    )
    .await;
    let auxiliary_session_id = auxiliary_attach
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("auxiliary session id")
        .to_owned();
    assert_ne!(auxiliary_session_id, primary_session_id);

    for (id, method) in [
        (8_u64, "Page.enable"),
        (9_u64, "Runtime.enable"),
        (10_u64, "Network.enable"),
    ] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&auxiliary_session_id),
            json!({}),
        )
        .await;
    }

    let plain_url = format!("http://{fixture_addr}/plain");
    let mut initial_navigation =
        cdp_navigate_and_wait_for_load(&mut socket, 11, &primary_session_id, &plain_url).await;
    if !initial_navigation.iter().any(|message| {
        message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
            && message["method"] == json!("Page.frameNavigated")
            && message["params"]["frame"]["url"] == json!(plain_url)
    }) {
        initial_navigation.append(
            &mut recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                    && message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(plain_url)
            })
            .await,
        );
    }
    if !initial_navigation.iter().any(|message| {
        message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    }) {
        let _ = recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .await;
    }

    let history_url = format!("http://{fixture_addr}/history-a?open-self=1");
    let isolated_world = send_cdp_command(
        &mut socket,
        120,
        "Page.createIsolatedWorld",
        Some(&primary_session_id),
        json!({
            "frameId": target_id,
            "worldName": "__moli_playwright_utility_world__",
            "grantUniversalAccess": true
        }),
    )
    .await;
    let isolated_context_id = isolated_world
        .iter()
        .find(|message| message["id"] == json!(120_u64))
        .and_then(|message| message["result"]["executionContextId"].as_u64())
        .expect("isolated utility world executionContextId");
    let utility_object = send_cdp_command(
        &mut socket,
        12,
        "Runtime.evaluate",
        Some(&primary_session_id),
        json!({
            "contextId": isolated_context_id,
            "expression": r#"({
                evaluate(isFunction, returnByValue, expression, argCount, ...args) {
                    let result = globalThis.eval(expression);
                    if (isFunction === true || (isFunction !== false && typeof result === "function")) {
                        result = result(...args.slice(0, argCount));
                    }
                    return returnByValue ? { v: result === null ? "null" : "undefined" } : result;
                }
            })"#
        }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();
    let mut navigation_messages = send_cdp_command(
        &mut socket,
        13,
        "Runtime.callFunctionOn",
        Some(&primary_session_id),
        json!({
            "objectId": utility_object_id,
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": utility_object_id },
                {},
                { "value": true },
                { "value": "(url) => window.open(url, '_self')" },
                { "value": 1 },
                { "value": history_url }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }),
    )
    .await;
    let runtime_response_index = navigation_messages
        .iter()
        .position(|message| message["id"] == json!(13_u64))
        .expect("Runtime.callFunctionOn response should be received");
    let runtime_response = &navigation_messages[runtime_response_index];
    assert!(
        runtime_response.get("result").is_some() && runtime_response.get("error").is_none(),
        "Runtime.callFunctionOn must complete successfully before its _self navigation is released: {navigation_messages:#?}"
    );
    assert!(
        !navigation_messages[..runtime_response_index]
            .iter()
            .any(|message| {
                let routed_to_target = matches!(
                    message["sessionId"].as_str(),
                    Some(session_id)
                        if session_id == primary_session_id || session_id == auxiliary_session_id
                );
                routed_to_target
                    && (message["method"] == json!("Runtime.executionContextsCleared")
                        || (message["method"] == json!("Page.frameStartedNavigating")
                            && message["params"]["url"] == json!(history_url))
                        || (message["method"] == json!("Page.frameNavigated")
                            && message["params"]["frame"]["url"] == json!(history_url))
                        || (message["method"] == json!("Network.requestWillBeSent")
                            && message["params"]["request"]["url"] == json!(history_url))
                        || message["method"] == json!("Page.loadEventFired"))
            }),
        "Runtime.callFunctionOn response must precede command-caused navigation output on every session attached to the target: {navigation_messages:#?}"
    );
    if !navigation_messages.iter().any(|message| {
        message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
            && message["method"] == json!("Page.frameNavigated")
            && message["params"]["frame"]["url"] == json!(history_url)
    }) {
        navigation_messages.append(
            &mut recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                    && message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(history_url)
            })
            .await,
        );
    }
    if !navigation_messages.iter().any(|message| {
        message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    }) {
        navigation_messages.append(
            &mut recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                    && message["method"] == json!("Page.loadEventFired")
            })
            .await,
        );
    }

    let started = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                && message["method"] == json!("Page.frameStartedNavigating")
                && message["params"]["url"] == json!(history_url)
        })
        .expect("auxiliary session should receive Page.frameStartedNavigating for _self URL");
    let navigated = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(history_url)
        })
        .expect("auxiliary session should receive Page.frameNavigated for _self URL");
    let loaded = navigation_messages
        .iter()
        .enumerate()
        .find_map(|(index, message)| {
            (index > navigated
                && message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                && message["method"] == json!("Page.loadEventFired"))
            .then_some(index)
        })
        .expect("auxiliary session should receive Page.loadEventFired after _self frameNavigated");
    assert!(
        started < navigated && navigated < loaded,
        "auxiliary Page event order should be started < navigated < load: {navigation_messages:#?}"
    );
    assert!(
        navigation_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(auxiliary_session_id.as_str())
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(history_url)
        }),
        "auxiliary Network session should observe the _self document request: {navigation_messages:#?}"
    );

    navigation_messages.append(
        &mut send_cdp_command(
            &mut socket,
            14,
            "Runtime.evaluate",
            Some(&primary_session_id),
            json!({
                "expression": "document.querySelector('main') && document.querySelector('main').textContent"
            }),
        )
        .await,
    );
    assert_eq!(
        navigation_messages
            .iter()
            .filter(|message| {
                message["id"] == json!(13_u64)
                    && message["sessionId"].as_str() == Some(primary_session_id.as_str())
            })
            .count(),
        1,
        "Runtime.callFunctionOn must receive exactly one terminal response: {navigation_messages:#?}"
    );
    let main_text = navigation_messages
        .iter()
        .find(|message| message["id"] == json!(14_u64))
        .and_then(|message| message["result"]["result"]["value"].as_str())
        .expect("post-navigation Runtime.evaluate string result");
    assert_eq!(main_text, "history a");

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_multi_target_sync_awaitpromise_precedes_realm_replacement() {
    const TARGET_COUNT: usize = 8;

    async fn start_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<body>
<main></main>
<script>
const target = new URL(location.href).searchParams.get('target');
globalThis.__realmMarker = `initial-${target}`;
document.querySelector('main').textContent = globalThis.__realmMarker;
</script>
</body>
</html>"#,
        )
    }

    async fn replacement_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<body>
<main></main>
<script>
const target = new URL(location.href).searchParams.get('target');
globalThis.__realmMarker = `replacement-${target}`;
document.querySelector('main').textContent = globalThis.__realmMarker;
</script>
</body>
</html>"#,
        )
    }

    let fixture_app = Router::new()
        .route("/start", get(start_page))
        .route("/replacement", get(replacement_page));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "multi-target-sync-awaitpromise");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to the single browser CDP websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let mut targets = Vec::with_capacity(TARGET_COUNT);
    for index in 0..TARGET_COUNT {
        let id_base = 10 + (index as u64 * 2);
        let target = cdp_create_attached_target(&mut socket, id_base, &browser_context_id).await;
        for (id, method) in [
            (100 + index as u64 * 2, "Runtime.enable"),
            (101 + index as u64 * 2, "Page.enable"),
        ] {
            let _ = send_cdp_command(&mut socket, id, method, Some(&target.session_id), json!({}))
                .await;
        }
        let start_url = format!("http://{fixture_addr}/start?target={index}");
        let _ = cdp_navigate_and_wait_for_load(
            &mut socket,
            200 + index as u64,
            &target.session_id,
            &start_url,
        )
        .await;
        targets.push(target);
    }

    const SCHEDULE_REPLACEMENT: &str = r#"(() => {
        const target = new URL(location.href).searchParams.get('target');
        const marker = document.querySelector('main').textContent;
        history.pushState(null, '', `${location.pathname}${location.search}#queued`);
        queueMicrotask(() => { location.href = `/replacement?target=${target}`; });
        return JSON.stringify({ marker, phase: 'scheduled' });
    })()"#;
    for (index, target) in targets.iter().enumerate() {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 1_000 + index as u64,
                    "method": "Runtime.evaluate",
                    "sessionId": target.session_id,
                    "params": {
                        "expression": SCHEDULE_REPLACEMENT,
                        "awaitPromise": true,
                        "returnByValue": true
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("pipeline synchronous awaitPromise Runtime.evaluate");
    }

    let mut navigation_messages = Vec::new();
    let mut saw_runtime_response = vec![false; TARGET_COUNT];
    let mut saw_replacement_load = vec![false; TARGET_COUNT];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while saw_runtime_response.iter().any(|seen| !seen)
        || saw_replacement_load.iter().any(|seen| !seen)
    {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(&mut socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for pipelined Runtime replies and realm replacements; \
                     responses={saw_runtime_response:?}, loads={saw_replacement_load:?}, \
                     messages={navigation_messages:#?}"
                )
            });
        if let Some(id) = message["id"].as_u64()
            && (1_000..1_000 + TARGET_COUNT as u64).contains(&id)
        {
            saw_runtime_response[(id - 1_000) as usize] = true;
        }
        for (index, target) in targets.iter().enumerate() {
            if message["sessionId"].as_str() == Some(target.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
            {
                saw_replacement_load[index] = true;
            }
        }
        navigation_messages.push(message);
    }

    assert!(
        !navigation_messages
            .iter()
            .any(|message| { message["error"]["message"] == json!("Promise was collected") }),
        "synchronous awaitPromise results must never be reported as collected: {navigation_messages:#?}"
    );
    for (index, target) in targets.iter().enumerate() {
        let response_id = 1_000 + index as u64;
        let response_indexes = navigation_messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message["id"] == json!(response_id)
                    && message["sessionId"].as_str() == Some(target.session_id.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            response_indexes.len(),
            1,
            "target {index} must receive exactly one terminal Runtime response: {navigation_messages:#?}"
        );
        let (response_index, response) = response_indexes[0];
        assert!(
            response.get("error").is_none(),
            "target {index} synchronous awaitPromise evaluation failed: {response:#?}"
        );
        assert_eq!(response["result"]["result"]["type"], json!("string"));
        assert_eq!(
            response["result"]["result"]["value"],
            json!(format!(
                r#"{{"marker":"initial-{index}","phase":"scheduled"}}"#
            ))
        );

        let same_document_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.navigatedWithinDocument")
                    && message["params"]["url"]
                        == json!(format!(
                            "http://{fixture_addr}/start?target={index}#queued"
                        ))
                    && message["params"]["navigationType"] == json!("historyApi")
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should expose the same-document history mutation: {navigation_messages:#?}"
                )
            });
        assert!(
            same_document_index < response_index,
            "same-document output produced inside the expression should precede its Runtime response for target {index}: {navigation_messages:#?}"
        );

        let replacement_url = format!("http://{fixture_addr}/replacement?target={index}");
        let contexts_cleared_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Runtime.executionContextsCleared")
            })
            .unwrap_or_else(|| {
                panic!("target {index} should clear the old realm: {navigation_messages:#?}")
            });
        let started_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.frameStartedNavigating")
                    && message["params"]["url"] == json!(replacement_url.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should start the replacement navigation: {navigation_messages:#?}"
                )
            });
        let navigated_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(replacement_url.as_str())
            })
            .unwrap_or_else(|| {
                panic!("target {index} should commit the replacement: {navigation_messages:#?}")
            });
        let replacement_context_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"]
                        == json!(target.target_id.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should publish its new default realm: {navigation_messages:#?}"
                )
            });
        let load_index = navigation_messages
            .iter()
            .enumerate()
            .find_map(|(message_index, message)| {
                (message_index > navigated_index
                    && message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.loadEventFired"))
                .then_some(message_index)
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should finish its replacement load: {navigation_messages:#?}"
                )
            });
        assert!(
            [contexts_cleared_index, started_index, navigated_index]
                .into_iter()
                .all(|navigation_index| response_index < navigation_index),
            "target {index} must flush its Runtime response before output that destroys the command realm: {navigation_messages:#?}"
        );
        assert!(
            contexts_cleared_index < replacement_context_index,
            "target {index} must clear the old realm before publishing the replacement realm: {navigation_messages:#?}"
        );
        assert!(
            started_index < navigated_index && navigated_index < load_index,
            "target {index} replacement order should be started < navigated < load: {navigation_messages:#?}"
        );
    }

    const OBSERVE_REPLACEMENT: &str = r#"JSON.stringify({
        marker: document.querySelector('main').textContent,
        realm: globalThis.__realmMarker
    })"#;
    for (index, target) in targets.iter().enumerate() {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 2_000 + index as u64,
                    "method": "Runtime.evaluate",
                    "sessionId": target.session_id,
                    "params": {
                        "expression": OBSERVE_REPLACEMENT,
                        "awaitPromise": true,
                        "returnByValue": true
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("pipeline post-navigation synchronous awaitPromise observation");
    }
    let mut replacement_messages = Vec::new();
    let mut saw_replacement_response = vec![false; TARGET_COUNT];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while saw_replacement_response.iter().any(|seen| !seen) {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(&mut socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for post-navigation Runtime replies; \
                     responses={saw_replacement_response:?}, messages={replacement_messages:#?}"
                )
            });
        if let Some(id) = message["id"].as_u64()
            && (2_000..2_000 + TARGET_COUNT as u64).contains(&id)
        {
            saw_replacement_response[(id - 2_000) as usize] = true;
        }
        replacement_messages.push(message);
    }
    assert!(
        !replacement_messages
            .iter()
            .any(|message| { message["error"]["message"] == json!("Promise was collected") }),
        "new-realm synchronous awaitPromise results must not be reported as collected: {replacement_messages:#?}"
    );
    for (index, target) in targets.iter().enumerate() {
        let response_id = 2_000 + index as u64;
        let responses = replacement_messages
            .iter()
            .filter(|message| {
                message["id"] == json!(response_id)
                    && message["sessionId"].as_str() == Some(target.session_id.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            responses.len(),
            1,
            "target {index} post-navigation observation should have one terminal response: {replacement_messages:#?}"
        );
        assert!(
            responses[0].get("error").is_none(),
            "target {index} should evaluate in its replacement realm: {replacement_messages:#?}"
        );
        assert_eq!(
            responses[0]["result"]["result"]["value"],
            json!(format!(
                r#"{{"marker":"replacement-{index}","realm":"replacement-{index}"}}"#
            ))
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_navigation_keeps_network_child_frame_visible_at_load_boundary() {
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-ws-cdp-visible</body></html>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Runtime.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.addScriptToEvaluateOnNewDocument",
                "sessionId": session_id,
                "params": {
                    "source": "globalThis.__lm_ws_cdp_child_world = true;",
                    "worldName": "utility-child"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.addScriptToEvaluateOnNewDocument");
    let _ = recv_until_id(&mut socket, 6).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/parent")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let mut navigation_messages = Vec::new();
    let mut saw_navigate_response = false;
    let mut saw_main_load_event = false;
    while !(saw_navigate_response && saw_main_load_event) {
        let message = recv_ws_json(&mut socket).await;
        if message["id"] == json!(7_u64) {
            saw_navigate_response = true;
        }
        if message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_main_load_event = true;
        }
        navigation_messages.push(message);
    }

    let child_frame_id = navigation_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .expect("child frame should emit Page.frameAttached")
        .to_owned();
    let child_url = format!("http://{fixture_addr}/child");
    let child_attached_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child attach index");
    let child_default_context_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!("child default execution context index: {navigation_messages:?}")
        });
    let child_named_context_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("child named execution context index");
    let child_navigated_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"] == json!(child_url.as_str())
        })
        .unwrap_or_else(|| {
            panic!("child final Page.frameNavigated before parent load: {navigation_messages:?}")
        });
    let main_load_event_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.loadEventFired")
        })
        .expect("main load event index");

    assert!(
        child_attached_index < main_load_event_index,
        "child Page.frameAttached should precede main Page.loadEventFired: {navigation_messages:?}"
    );
    assert!(
        child_default_context_index < main_load_event_index,
        "child default Runtime.executionContextCreated should precede main Page.loadEventFired: {navigation_messages:?}"
    );
    assert!(
        child_named_context_index < main_load_event_index,
        "child named Runtime.executionContextCreated should precede main Page.loadEventFired: {navigation_messages:?}"
    );
    assert!(
        child_navigated_index < main_load_event_index,
        "child final Page.frameNavigated should precede main Page.loadEventFired: {navigation_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Page.getFrameTree",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.getFrameTree after load boundary");
    let frame_tree_messages = recv_until_id(&mut socket, 8).await;
    let frame_tree = frame_tree_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("frame tree response");
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(child_frames.len(), 1);
    assert_eq!(child_frames[0]["frame"]["id"], json!(child_frame_id));
    assert_eq!(child_frames[0]["frame"]["url"], json!(child_url.as_str()));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_websocket_frame_events_are_emitted_without_followup_command() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>websocket async cdp event</body></html>",
        )
    }

    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(text) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text(text)).await;
                    }
                    Message::Close(frame) => {
                        let _ = socket.send(Message::Close(frame)).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/socket", get(socket)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Network.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Network.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": format!(
                        "(() => {{ const ws = new WebSocket({}); ws.addEventListener('open', () => ws.send('background frame')); ws.addEventListener('message', () => ws.close(1000, 'done')); return 'scheduled'; }})()",
                        serde_json::to_string(&format!("ws://{fixture_addr}/socket")).unwrap()
                    )
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate_messages = recv_until_id(&mut socket, 6).await;
    assert!(
        evaluate_messages
            .iter()
            .all(|message| { message["method"] != json!("Network.webSocketFrameReceived") }),
        "delayed echo should keep the received frame out of the command response batch: {evaluate_messages:?}"
    );

    let received = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.webSocketFrameReceived")
            {
                return message;
            }
        }
    })
    .await
    .expect("websocket frame event should be emitted without a follow-up CDP command");

    assert_eq!(
        received["params"]["response"]["payloadLength"],
        json!("background frame".len())
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_pending_runtime_await_completes_after_websocket_dom_update_without_followup_command()
 {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id='conversation'></main></body></html>",
        )
    }

    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(_) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text("OK".into())).await;
                    }
                    Message::Close(frame) => {
                        let _ = socket.send(Message::Close(frame)).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_app = Router::new()
        .route("/page", get(page))
        .route("/socket", get(socket));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-websocket-dom");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(resolve => { const poll = () => { const node = document.querySelector('[data-message-author-role=\"assistant\"]'); if (node) { resolve(node.textContent); return; } requestAnimationFrame(poll); }; poll(); })",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pending Runtime.evaluate awaitPromise");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": format!(
                        "(() => {{ const ws = new WebSocket({}); ws.addEventListener('open', () => ws.send('start')); ws.addEventListener('message', event => {{ const node = document.createElement('div'); node.dataset.messageAuthorRole = 'assistant'; node.textContent = event.data; document.getElementById('conversation').appendChild(node); ws.close(1000, 'done'); }}); return 'scheduled'; }})()",
                        serde_json::to_string(&format!("ws://{fixture_addr}/socket")).unwrap()
                    )
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send WebSocket scheduling Runtime.evaluate");

    let response_messages = recv_until_id(&mut socket, 6).await;
    let response = response_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("pending Runtime.evaluate response");
    assert!(
        response.get("error").is_none(),
        "pending Runtime.evaluate should resolve from WebSocket DOM mutation without follow-up command: {response_messages:?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("OK"));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_pending_runtime_await_completes_after_page_started_websocket_dom_update() {
    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(_) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text("OK".into())).await;
                    }
                    Message::Close(frame) => {
                        let _ = socket.send(Message::Close(frame)).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_app = Router::new().route("/socket", get(socket)).route(
        "/page",
        get(|headers: axum::http::HeaderMap| async move {
            let host = headers
                .get(axum::http::header::HOST)
                .and_then(|value| value.to_str().ok())
                .expect("fixture request should include Host header");
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                format!(
                    r#"<!doctype html>
<html>
<body>
<main id="conversation"></main>
<script>
window.addEventListener('load', () => {{
  setTimeout(() => {{
    const ws = new WebSocket({});
    ws.addEventListener('open', () => ws.send('start'));
    ws.addEventListener('message', event => {{
      const node = document.createElement('div');
      node.dataset.messageAuthorRole = 'assistant';
      node.textContent = event.data;
      document.getElementById('conversation').appendChild(node);
      history.pushState(null, '', '/c/smoke-live');
      ws.close(1000, 'done');
    }});
  }}, 0);
}});
</script>
</body>
</html>"#,
                    serde_json::to_string(&format!("ws://{host}/socket"))
                        .expect("websocket URL should serialize")
                ),
            )
        }),
    );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-page-websocket-dom");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(resolve => { const poll = () => { const node = document.querySelector('[data-message-author-role=\"assistant\"]'); if (location.pathname === '/c/smoke-live' && node && node.textContent === 'OK') { resolve(node.textContent); return; } requestAnimationFrame(poll); }; poll(); })",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pending Runtime.evaluate awaitPromise");

    let response_messages = recv_until_id(&mut socket, 8).await;
    let response = response_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("pending Runtime.evaluate response");
    assert_cdp_event_precedes_response(&response_messages, "Page.navigatedWithinDocument", 8);
    assert!(
        response.get("error").is_none(),
        "pending Runtime.evaluate should resolve from page-started WebSocket DOM mutation: {response_messages:?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("OK"));
    assert!(
        response_messages.iter().any(|message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["url"] == json!(format!("http://{fixture_addr}/c/smoke-live"))
                && message["params"]["navigationType"] == json!("historyApi")
        }),
        "history.pushState should be emitted as a same-document navigation: {response_messages:?}"
    );
    assert!(
        !response_messages.iter().any(|message| {
            message["method"] == json!("Page.frameStartedNavigating")
                && message["params"]["url"] == json!(format!("http://{fixture_addr}/c/smoke-live"))
        }),
        "history.pushState must not be projected as a full document navigation: {response_messages:?}"
    );
    assert!(
        !response_messages.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared" | "DOM.documentUpdated")
            )
        }),
        "same-document navigation must not clear runtime contexts or update the document: {response_messages:?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_network_and_websocket_outputs_are_session_isolated_without_followup_command()
{
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>owner isolated network output</body></html>",
        )
    }

    async fn api(request: Request<Body>) -> impl IntoResponse {
        let owner = request
            .uri()
            .query()
            .and_then(|query| query.strip_prefix("owner="))
            .unwrap_or("missing");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            format!("api:{owner}"),
        )
    }

    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(text) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text(text)).await;
                    }
                    Message::Close(frame) => {
                        let _ = socket.send(Message::Close(frame)).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api))
                .route("/socket", get(socket)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 200).await;
    let target_a = cdp_create_attached_target(&mut socket, 201, &browser_context_id).await;
    let target_b = cdp_create_attached_target(&mut socket, 203, &browser_context_id).await;

    for (id, session_id) in [
        (205_u64, target_a.session_id.as_str()),
        (206_u64, target_b.session_id.as_str()),
    ] {
        let _ = send_cdp_command(&mut socket, id, "Page.enable", Some(session_id), json!({})).await;
        let _ = send_cdp_command(
            &mut socket,
            id + 10,
            "Network.enable",
            Some(session_id),
            json!({}),
        )
        .await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 230, &target_a.session_id, &page_url).await;
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 231, &target_b.session_id, &page_url).await;

    let ws_url = format!("ws://{fixture_addr}/socket");
    let mut expected_evaluate_results = Vec::new();
    for (id, session_id, owner, payload) in [
        (
            240_u64,
            target_a.session_id.as_str(),
            "a",
            "context-a-frame",
        ),
        (241_u64, target_b.session_id.as_str(), "b", "b"),
    ] {
        let expression = format!(
            r#"(() => {{
                fetch('/api?owner={owner}');
                const ws = new WebSocket({});
                ws.addEventListener('open', () => ws.send({}));
                ws.addEventListener('message', () => ws.close(1000, 'done'));
                return 'scheduled-{owner}';
            }})()"#,
            serde_json::to_string(&ws_url).expect("serialize ws url"),
            serde_json::to_string(payload).expect("serialize ws payload"),
        );
        expected_evaluate_results.push(id);
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "Runtime.evaluate",
                    "sessionId": session_id,
                    "params": { "expression": expression }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Runtime.evaluate");
    }

    let api_a_url = format!("http://{fixture_addr}/api?owner=a");
    let api_b_url = format!("http://{fixture_addr}/api?owner=b");
    let payload_a_len = "context-a-frame".len();
    let payload_b_len = "b".len();
    let mut fetch_a_request_id = None::<String>;
    let mut fetch_b_request_id = None::<String>;
    let mut saw_fetch_a_finished = false;
    let mut saw_fetch_b_finished = false;
    let mut saw_ws_a_received = false;
    let mut saw_ws_b_received = false;
    let mut saw_evaluate_a_result = false;
    let mut saw_evaluate_b_result = false;
    let mut observed = Vec::new();

    timeout(Duration::from_secs(3), async {
        while !(saw_evaluate_a_result
            && saw_evaluate_b_result
            && saw_fetch_a_finished
            && saw_fetch_b_finished
            && saw_ws_a_received
            && saw_ws_b_received)
        {
            let message = recv_ws_json(&mut socket).await;
            observed.push(message.clone());
            if message["id"] == json!(expected_evaluate_results[0]) {
                saw_evaluate_a_result = true;
            }
            if message["id"] == json!(expected_evaluate_results[1]) {
                saw_evaluate_b_result = true;
            }
            let session_id = message["sessionId"].as_str();
            let method = message["method"].as_str();
            if session_id == Some(target_a.session_id.as_str()) {
                if message["params"]["request"]["url"] == json!(api_b_url)
                    || message["params"]["response"]["url"] == json!(api_b_url)
                {
                    panic!("context B fetch output leaked to context A session: {message:?}");
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_b_len)
                {
                    panic!(
                        "context B WebSocket frame leaked to context A session: {message:?}"
                    );
                }
                if method == Some("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(api_a_url)
                {
                    fetch_a_request_id =
                        message["params"]["requestId"].as_str().map(str::to_owned);
                }
                if method == Some("Network.loadingFinished")
                    && fetch_a_request_id.as_ref().is_some_and(|request_id| {
                        message["params"]["requestId"] == json!(request_id)
                    })
                {
                    saw_fetch_a_finished = true;
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_a_len)
                {
                    saw_ws_a_received = true;
                }
            } else if session_id == Some(target_b.session_id.as_str()) {
                if message["params"]["request"]["url"] == json!(api_a_url)
                    || message["params"]["response"]["url"] == json!(api_a_url)
                {
                    panic!("context A fetch output leaked to context B session: {message:?}");
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_a_len)
                {
                    panic!(
                        "context A WebSocket frame leaked to context B session: {message:?}"
                    );
                }
                if method == Some("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(api_b_url)
                {
                    fetch_b_request_id =
                        message["params"]["requestId"].as_str().map(str::to_owned);
                }
                if method == Some("Network.loadingFinished")
                    && fetch_b_request_id.as_ref().is_some_and(|request_id| {
                        message["params"]["requestId"] == json!(request_id)
                    })
                {
                    saw_fetch_b_finished = true;
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_b_len)
                {
                    saw_ws_b_received = true;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for isolated target network outputs; target_a={} session_a={} target_b={} session_b={} observed={observed:?}",
            target_a.target_id, target_a.session_id, target_b.target_id, target_b.session_id
        )
    });

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_parser_script_network_events_capture_response_body() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script src="/script.js"></script></body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [
                (
                    axum::http::header::CONTENT_TYPE.as_str(),
                    "application/javascript",
                ),
                ("x-script", "ok"),
            ],
            r#"globalThis.__lmParserScriptLoaded = "socket script body";"#,
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/script.js", get(script)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 10_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 10).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 11).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(11_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 12).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "Network.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Network.enable");
    let _ = recv_until_id(&mut socket, 13).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 14_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let mut navigation_messages = recv_until_id(&mut socket, 14).await;
    let page_url = format!("http://{fixture_addr}/page");
    let navigate_loader_id = navigation_messages
        .iter()
        .find(|message| message["id"] == json!(14_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("Page.navigate loaderId")
        .to_owned();
    if !navigation_messages.iter().any(|message| {
        message["sessionId"] == json!(session_id)
            && message["method"] == json!("Network.responseReceived")
            && message["params"]["type"] == json!("Document")
            && message["params"]["response"]["url"] == json!(page_url)
    }) {
        navigation_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.responseReceived")
                    && message["params"]["type"] == json!("Document")
                    && message["params"]["response"]["url"] == json!(page_url)
            })
            .await,
        );
    }
    let main_response_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Document")
                && message["params"]["response"]["url"] == json!(page_url)
        })
        .expect("main document responseReceived should be emitted for navigation");
    let main_request_id = navigation_messages[main_response_index]["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();
    assert_eq!(
        navigate_loader_id, main_request_id,
        "Page.navigate loaderId and main document Network requestId should share the navigation token"
    );
    timeout(Duration::from_secs(2), async {
        while !navigation_messages.iter().any(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(main_request_id)
        }) {
            navigation_messages.push(recv_ws_json(&mut socket).await);
        }
    })
    .await
    .expect("main document loadingFinished should arrive");
    let main_finished_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(main_request_id)
        })
        .expect("main document loadingFinished index");
    assert!(
        main_response_index < main_finished_index,
        "main document Network.responseReceived must precede terminal Network.loadingFinished for the same request: {navigation_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 15_u64,
                "method": "Network.getResponseBody",
                "sessionId": session_id,
                "params": {
                    "requestId": main_request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send main document Network.getResponseBody");
    let main_response_body = recv_until_id(&mut socket, 15).await;
    let main_body = main_response_body
        .iter()
        .find(|message| message["id"] == json!(15_u64))
        .and_then(|message| message["result"]["body"].as_str())
        .expect("main document response body");
    assert!(
        main_body.contains(r#"<script src="/script.js"></script>"#),
        "main document body should be readable after Network.loadingFinished: {main_response_body:?}"
    );
    navigation_messages.extend(main_response_body);

    let script_url = format!("http://{fixture_addr}/script.js");
    let script_request_id = match timeout(Duration::from_secs(2), async {
        loop {
            let request = navigation_messages.iter().find(|message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["request"]["url"] == json!(script_url)
            });
            let response = navigation_messages.iter().find(|message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.responseReceived")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["response"]["url"] == json!(script_url)
            });
            if let (Some(request), Some(response)) = (request, response) {
                let request_id = request["params"]["requestId"]
                    .as_str()
                    .expect("script request id");
                if response["params"]["requestId"] == json!(request_id)
                    && navigation_messages.iter().any(|message| {
                        message["sessionId"] == json!(session_id)
                            && message["method"] == json!("Network.loadingFinished")
                            && message["params"]["requestId"] == json!(request_id)
                    })
                {
                    return request_id.to_owned();
                }
            }
            navigation_messages.push(recv_ws_json(&mut socket).await);
        }
    })
    .await
    {
        Ok(request_id) => request_id,
        Err(error) => {
            panic!(
                "script network events should arrive: {error:?}; messages={navigation_messages:?}"
            )
        }
    };

    socket
        .send(WsMessage::Text(
            json!({
                "id": 16_u64,
                "method": "Network.getResponseBody",
                "sessionId": session_id,
                "params": {
                    "requestId": script_request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Network.getResponseBody");
    let response_body = recv_until_id(&mut socket, 16).await;
    let body = response_body
        .iter()
        .find(|message| message["id"] == json!(16_u64))
        .and_then(|message| message["result"]["body"].as_str())
        .expect("script response body");
    assert_eq!(
        body,
        r#"globalThis.__lmParserScriptLoaded = "socket script body";"#
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><head><script src="/script.js"></script></head><body>network-before-dcl</body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            r#"globalThis.__lmParserScriptBeforeDcl = true;"#,
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/script.js", get(script)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 10).await;
    let target = cdp_create_attached_target(&mut socket, 20, browser_context_id.as_str()).await;

    for (id, method, params) in [
        (30_u64, "Page.enable", json!({})),
        (31_u64, "Network.enable", json!({})),
        (
            32_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&target.session_id), params).await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let script_url = format!("http://{fixture_addr}/script.js");
    let mut messages = send_cdp_command(
        &mut socket,
        40,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": page_url }),
    )
    .await;
    messages.extend(
        recv_until_match(&mut socket, |message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Page.domContentEventFired")
        })
        .await,
    );

    let dcl_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Page.domContentEventFired")
        })
        .expect("Page.domContentEventFired should be emitted");
    let script_request = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Script")
                && message["params"]["request"]["url"] == json!(script_url)
        })
        .expect("parser script requestWillBeSent should be emitted before DCL");
    let script_request_id = messages[script_request]["params"]["requestId"]
        .as_str()
        .expect("script request id")
        .to_owned();
    let script_response = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Script")
                && message["params"]["requestId"] == json!(script_request_id)
        })
        .expect("parser script responseReceived should be emitted before DCL");
    let script_finished = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(script_request_id)
        })
        .expect("parser script loadingFinished should be emitted before DCL");

    assert!(
        script_request < dcl_index && script_response < dcl_index && script_finished < dcl_index,
        "parser script Network backlog should flush before DCL; messages={messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_inline_xhr_network_events_capture_response_body() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body>
<script>
globalThis.__lmInlineXhrDone = new Promise(resolve => {
  const xhr = new XMLHttpRequest();
  xhr.open("GET", "/xhr.bin", true);
  xhr.responseType = "arraybuffer";
  xhr.onload = () => resolve({ status: xhr.status, length: xhr.response.byteLength });
  xhr.onerror = () => resolve({ status: xhr.status, error: "xhr error" });
  xhr.send();
});
</script>
<main>inline xhr</main>
</body></html>"#,
        )
    }

    async fn xhr_bin() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/octet-stream",
            )],
            vec![0_u8, 255, b'l', b'm', b'-', b'x', b'h', b'r'],
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr.bin", get(xhr_bin)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 10).await;
    let target = cdp_create_attached_target(&mut socket, 20, browser_context_id.as_str()).await;

    for (id, method, params) in [
        (30_u64, "Page.enable", json!({})),
        (31_u64, "Network.enable", json!({})),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&target.session_id), params).await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let xhr_url = format!("http://{fixture_addr}/xhr.bin");
    let mut messages = send_cdp_command(
        &mut socket,
        40,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": page_url }),
    )
    .await;
    messages.extend(
        send_cdp_command(
            &mut socket,
            50,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": "globalThis.__lmInlineXhrDone",
                "awaitPromise": true,
                "returnByValue": true
            }),
        )
        .await,
    );

    let xhr_result = messages
        .iter()
        .find(|message| message["id"] == json!(50_u64))
        .expect("Runtime.evaluate response for inline XHR promise");
    assert_eq!(
        xhr_result["result"]["result"]["value"]["status"],
        json!(200),
        "inline XHR should complete successfully; messages={messages:#?}"
    );

    // The awaited Runtime response can beat queued Network events to the
    // websocket client under full-suite scheduling, so wait by URL/requestId
    // instead of assuming every XHR event is in the command response batch.
    let request = if let Some(request) = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
    {
        request
    } else {
        let mut more_messages = recv_until_match(&mut socket, |message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .await;
        let request = more_messages
            .iter()
            .find(|message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("XHR")
                    && message["params"]["request"]["url"] == json!(xhr_url)
            })
            .expect("inline XHR requestWillBeSent should be emitted")
            .clone();
        messages.append(&mut more_messages);
        request
    };
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("inline XHR requestId")
        .to_owned();
    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.responseReceived")
            && message["params"]["type"] == json!("XHR")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["url"] == json!(xhr_url)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.responseReceived")
                    && message["params"]["type"] == json!("XHR")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["response"]["url"] == json!(xhr_url)
            })
            .await,
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["response"]["url"] == json!(xhr_url)
        }),
        "inline XHR responseReceived should be emitted; messages={messages:#?}"
    );
    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .await,
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(request_id)
        }),
        "inline XHR loadingFinished should be emitted; messages={messages:#?}"
    );

    let body_messages = send_cdp_command(
        &mut socket,
        60,
        "Network.getResponseBody",
        Some(&target.session_id),
        json!({ "requestId": request_id }),
    )
    .await;
    let body = body_messages
        .iter()
        .find(|message| message["id"] == json!(60_u64))
        .expect("Network.getResponseBody response");
    assert_eq!(body["result"]["base64Encoded"], json!(true));
    assert_eq!(
        body["result"]["body"],
        json!(BASE64_STANDARD.encode([0_u8, 255, b'l', b'm', b'-', b'x', b'h', b'r']))
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_event_source_emits_incremental_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>event source</main></body></html>",
        )
    }

    async fn events() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "text/event-stream; charset=utf-8",
            )],
            "id: 17\nevent: update\ndata: first\ndata: second\n\n",
        )
    }

    let fixture_app = Router::new()
        .route("/page", get(page))
        .route("/events", get(events));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "cdp-event-source");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 10).await;
    let target = cdp_create_attached_target(&mut socket, 20, browser_context_id.as_str()).await;

    for (id, method) in [(30_u64, "Page.enable"), (31_u64, "Network.enable")] {
        let _ =
            send_cdp_command(&mut socket, id, method, Some(&target.session_id), json!({})).await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let events_url = format!("http://{fixture_addr}/events");
    let mut messages = send_cdp_command(
        &mut socket,
        40,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": page_url }),
    )
    .await;
    messages.extend(
        send_cdp_command(
            &mut socket,
            50,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": format!(
                    r#"
                    new Promise(resolve => {{
                        const source = new EventSource({events_url:?});
                        source.addEventListener("update", event => {{
                            source.close();
                            resolve({{
                                type: event.type,
                                eventId: event.lastEventId,
                                data: event.data,
                                readyState: source.readyState,
                            }});
                        }});
                        source.onerror = () => {{
                            if (source.readyState === EventSource.CLOSED) {{
                                resolve({{ error: "closed before message" }});
                            }}
                        }};
                    }})
                    "#
                ),
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await,
    );

    let evaluate = messages
        .iter()
        .find(|message| message["id"] == json!(50_u64))
        .expect("Runtime.evaluate EventSource response");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!({
            "type": "update",
            "eventId": "17",
            "data": "first\nsecond",
            "readyState": 2,
        }),
        "EventSource should dispatch the SSE MessageEvent; messages={messages:#?}",
    );
    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.eventSourceMessageReceived")
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.eventSourceMessageReceived")
            })
            .await,
        );
    }
    let event_message = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.eventSourceMessageReceived")
        })
        .expect("Network.eventSourceMessageReceived should be emitted");
    let request_id = event_message["params"]["requestId"]
        .as_str()
        .expect("EventSource requestId")
        .to_owned();

    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .await,
        );
    }

    let request_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["type"] == json!("EventSource")
                && message["params"]["request"]["url"] == json!(events_url)
        })
        .expect("EventSource requestWillBeSent should be emitted");
    let response_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["type"] == json!("EventSource")
        })
        .expect("EventSource responseReceived should be emitted");
    let data_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.dataReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("EventSource dataReceived should be emitted");
    let event_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.eventSourceMessageReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("EventSource message event should be emitted");
    let finished_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("completed EventSource response should emit loadingFinished");
    let evaluate_index = messages
        .iter()
        .position(|message| message["id"] == json!(50_u64))
        .expect("Runtime.evaluate EventSource response should remain in the wire log");
    assert!(
        request_index < response_index
            && response_index < data_index
            && data_index < event_index
            && event_index < finished_index
            && event_index < evaluate_index,
        "EventSource Network events must preserve Chromium ordering; messages={messages:#?}",
    );

    let request_headers = messages[request_index]["params"]["request"]["headers"]
        .as_object()
        .expect("EventSource request headers");
    assert!(request_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("accept") && value == &json!("text/event-stream")
    }));
    assert!(request_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cache-control") && value == &json!("no-cache")
    }));
    assert_eq!(
        messages[event_index]["params"],
        json!({
            "requestId": request_id,
            "timestamp": messages[event_index]["params"]["timestamp"],
            "eventName": "update",
            "eventId": "17",
            "data": "first\nsecond",
        }),
    );
    assert_eq!(
        messages[finished_index]["params"]["encodedDataLength"],
        json!(47),
        "the finite SSE response should retain its completed byte count",
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_runtime_awaitpromise_external_script_node_keeps_node_subtype() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head></head><body><main>plain</main></body></html>",
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            "window.__moliDeferredScriptLoaded = true;",
        )
    }

    let fixture_app = Router::new()
        .route("/plain", get(page))
        .route("/node-script.js", get(script));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-script-node");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/plain");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;

    let utility_object = send_cdp_command(
        &mut socket,
        9,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "({})" }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();

    let messages = send_cdp_command(
        &mut socket,
        10,
        "Runtime.callFunctionOn",
        Some(&session_id),
        json!({
            "objectId": utility_object_id,
            "functionDeclaration": "() => new Promise((resolve, reject) => { const script = document.createElement('script'); script.src = '/node-script.js'; script.onload = () => resolve(script); script.onerror = () => reject(new Error('script failed')); document.head.appendChild(script); })",
            "awaitPromise": true
        }),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("Runtime.callFunctionOn response");
    assert!(
        response.get("error").is_none(),
        "awaitPromise external script node should resolve successfully: {response:#?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("object"));
    assert_eq!(
        response["result"]["result"]["subtype"],
        json!("node"),
        "deferred Runtime renderer-receiver responses must preserve DOM node subtype: {response:#?}"
    );

    let loaded = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        11,
        "String(window.__moliDeferredScriptLoaded)",
    )
    .await;
    assert_eq!(loaded, "true");

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_awaitpromise_same_owner_turn_style_node_keeps_node_subtype() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head></head><body><main>plain</main></body></html>",
        )
    }

    let fixture_app = Router::new().route("/plain", get(page));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-inline-style-node");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/plain");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;

    let utility_object = send_cdp_command(
        &mut socket,
        12,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "({})" }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();

    let messages = send_cdp_command(
        &mut socket,
        13,
        "Runtime.callFunctionOn",
        Some(&session_id),
        json!({
            "objectId": utility_object_id,
            "functionDeclaration": "() => new Promise(resolve => { const style = document.createElement('style'); style.textContent = 'main { color: rgb(1, 2, 3); }'; document.head.appendChild(style); queueMicrotask(() => resolve(style)); })",
            "awaitPromise": true
        }),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(13_u64))
        .expect("Runtime.callFunctionOn response");
    assert!(
        response.get("error").is_none(),
        "same-turn style node awaitPromise should resolve successfully: {response:#?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("object"));
    assert_eq!(
        response["result"]["result"]["subtype"],
        json!("node"),
        "same-turn deferred Runtime response must preserve DOM node subtype: {response:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_playwright_auto_attach_child_frame_events_precede_main_load_boundary() {
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-ws-playwright-auto-attach</body></html>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 100_u64,
                "method": "Target.setAutoAttach",
                "params": {
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root Target.setAutoAttach");
    let _ = recv_until_id(&mut socket, 100).await;

    socket
        .send(WsMessage::Text(
            json!({ "id": 101_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 101).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(101_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 102_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target_messages = recv_until_id(&mut socket, 102).await;
    let target_id = create_target_messages
        .iter()
        .find(|message| message["id"] == json!(102_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();
    let session_id = create_target_messages
        .iter()
        .find(|message| message["method"] == json!("Target.attachedToTarget"))
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached session id")
        .to_owned();

    for (id, method, params) in [
        (103_u64, "Browser.getWindowForTarget", json!({})),
        (104_u64, "Page.enable", json!({})),
        (105_u64, "Page.getFrameTree", json!({})),
        (106_u64, "Log.enable", json!({})),
        (
            107_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
        (108_u64, "Runtime.enable", json!({})),
        (
            109_u64,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({
                "source": "",
                "worldName": "__playwright_utility_world_page@lm"
            }),
        ),
        (110_u64, "Network.enable", json!({})),
        (
            111_u64,
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true
            }),
        ),
        (
            112_u64,
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        ),
        (
            113_u64,
            "Emulation.setEmulatedMedia",
            json!({
                "media": "",
                "features": [
                    { "name": "prefers-color-scheme", "value": "light" },
                    { "name": "prefers-reduced-motion", "value": "no-preference" },
                    { "name": "forced-colors", "value": "none" },
                    { "name": "prefers-contrast", "value": "no-preference" }
                ]
            }),
        ),
        (114_u64, "Runtime.runIfWaitingForDebugger", json!({})),
    ] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id,
                    "params": params
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send playwright-style page init command");
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 115_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/parent")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let mut navigation_messages = Vec::new();
    let mut saw_navigate_response = false;
    let mut saw_main_load_event = false;
    while !(saw_navigate_response && saw_main_load_event) {
        let message = recv_ws_json(&mut socket).await;
        if message["id"] == json!(115_u64) {
            saw_navigate_response = true;
        }
        if message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_main_load_event = true;
        }
        navigation_messages.push(message);
    }

    let child_frame_id = navigation_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .expect("child frame should emit Page.frameAttached")
        .to_owned();
    let child_context_visible = navigation_messages.iter().any(|message| {
        message["sessionId"] == json!(session_id)
            && message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
            && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
    });
    if !child_context_visible {
        navigation_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            })
            .await,
        );
    }
    let child_attached_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child attach index");
    let child_default_context_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!("child default execution context index: {navigation_messages:?}")
        });
    let main_load_event_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.loadEventFired")
        })
        .expect("main load event index");

    assert!(
        child_attached_index < main_load_event_index,
        "child Page.frameAttached should precede main Page.loadEventFired in playwright auto-attach flow: {navigation_messages:?}"
    );
    assert!(
        child_default_context_index < main_load_event_index,
        "child default Runtime.executionContextCreated should precede main Page.loadEventFired in playwright auto-attach flow: {navigation_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 116_u64,
                "method": "Page.getFrameTree",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.getFrameTree after load boundary");
    let frame_tree_messages = recv_until_id(&mut socket, 116).await;
    let frame_tree = frame_tree_messages
        .iter()
        .find(|message| message["id"] == json!(116_u64))
        .expect("frame tree response");
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(child_frames.len(), 1);
    assert_eq!(child_frames[0]["frame"]["id"], json!(child_frame_id));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}
