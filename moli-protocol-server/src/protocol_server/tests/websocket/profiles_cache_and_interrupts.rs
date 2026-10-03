use super::*;

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
async fn websocket_cdp_cookies_persist_across_browser_close_restart() {
    let profile = TempDir::new("cookie-browser-close-profile");
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

    let set_cookie = send_cdp_command(
        &mut socket,
        10,
        "Network.setCookie",
        Some(&session_id),
        json!({ "name": "c1", "value": "v1", "url": page_url }),
    )
    .await;
    assert!(
        set_cookie
            .iter()
            .any(|message| message["id"] == json!(10_u64) && message.get("result").is_some()),
        "Network.setCookie should succeed: {set_cookie:?}"
    );
    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        11,
        "document.cookie = 'c2=v2; path=/'; document.cookie",
    )
    .await;
    assert!(write.contains("c2=v2"), "document.cookie write: {write}");

    send_cdp_command_without_wait(&mut socket, 42, "Browser.close", None, json!({})).await;
    timeout(Duration::from_secs(10), cdp_server)
        .await
        .expect("profiled server should exit after Browser.close")
        .expect("profiled server task should return cleanly");
    wait_for_profile_lock_release(&paths).await;

    let persisted = wait_for_cookie_profile(&paths.cookies_path, |cookies| {
        cookies
            .iter()
            .any(|cookie| cookie.name == "c1" && cookie.value == "v1")
            && cookies
                .iter()
                .any(|cookie| cookie.name == "c2" && cookie.value == "v2")
    })
    .await;
    assert!(
        persisted
            .iter()
            .any(|cookie| cookie.name == "c1" && cookie.value == "v1")
            && persisted
                .iter()
                .any(|cookie| cookie.name == "c2" && cookie.value == "v2"),
        "both CDP and document.cookie cookies should survive Browser.close: {persisted:?}"
    );

    let (cdp_addr, cdp_server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("reconnect to profiled cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read = cdp_runtime_evaluate_string(&mut socket, &session_id, 6, "document.cookie").await;
    assert!(
        read.contains("c1=v1") && read.contains("c2=v2"),
        "reopened profile should restore both cookies: {read}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(cdp_server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_cookie_writes_persist_without_detach() {
    let profile = TempDir::new("cookie-write-through-profile");
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

    let set_cookie = send_cdp_command(
        &mut socket,
        10,
        "Network.setCookie",
        Some(&session_id),
        json!({ "name": "c1", "value": "v1", "url": page_url }),
    )
    .await;
    assert!(
        set_cookie
            .iter()
            .any(|message| message["id"] == json!(10_u64) && message.get("result").is_some()),
        "Network.setCookie should succeed: {set_cookie:?}"
    );

    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        11,
        "document.cookie = 'c2=v2; path=/'; document.cookie",
    )
    .await;
    assert!(write.contains("c2=v2"), "document.cookie write: {write}");

    let persisted = wait_for_cookie_profile(&paths.cookies_path, |cookies| {
        cookies
            .iter()
            .any(|cookie| cookie.name == "c1" && cookie.value == "v1")
            && cookies
                .iter()
                .any(|cookie| cookie.name == "c2" && cookie.value == "v2")
    })
    .await;
    assert!(
        persisted
            .iter()
            .any(|cookie| cookie.name == "c1" && cookie.value == "v1"),
        "CDP-injected cookie should be written through to the profile: {persisted:?}"
    );
    assert!(
        persisted
            .iter()
            .any(|cookie| cookie.name == "c2" && cookie.value == "v2"),
        "document.cookie write should be written through to the profile: {persisted:?}"
    );

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

    let attached_session_response = send_cdp_command(
        &mut socket,
        6,
        "Target.attachToTarget",
        None,
        json!({ "targetId": primary.target_id, "flatten": true }),
    )
    .await;
    let attached_session_id = attached_session_response
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("attached session id")
        .to_owned();
    for (id, method, session_id) in [
        (7, "Debugger.enable", primary.session_id.as_str()),
        (8, "Runtime.enable", attached_session_id.as_str()),
        (9, "Debugger.enable", attached_session_id.as_str()),
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
        Some(&attached_session_id),
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
        Some(&attached_session_id),
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

    // All three commands use the attached session's IO lane. The two source
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
        send_cdp_command_without_wait(&mut socket, id, method, Some(&attached_session_id), params)
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
            "IO source lookup {id} must reach the live attached Inspector session: \
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
        "IO may overtake Main, but the attached Main lane must remain FIFO: {observed:#?}"
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
async fn websocket_cdp_offline_emulation_navigation_fails_like_network_error() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><div id='probe'>online</div></body></html>",
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

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;

    let network_enable = send_cdp_command(
        &mut socket,
        4,
        "Network.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    assert!(
        network_enable
            .iter()
            .any(|message| message["id"] == json!(4_u64) && message.get("error").is_none()),
        "Network.enable should succeed: {network_enable:?}"
    );

    // Turn offline emulation on. Chromium blocks loopback too, so the
    // navigation must fail as a network error rather than succeed.
    let emulate = send_cdp_command(
        &mut socket,
        5,
        "Network.emulateNetworkConditions",
        Some(&session.session_id),
        json!({
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }),
    )
    .await;
    assert!(
        emulate
            .iter()
            .any(|message| message["id"] == json!(5_u64) && message.get("error").is_none()),
        "offline emulation should be accepted: {emulate:?}"
    );

    let navigate = send_cdp_command(
        &mut socket,
        6,
        "Page.navigate",
        Some(&session.session_id),
        json!({ "url": fixture_url }),
    )
    .await;
    let navigate_response = navigate
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("Page.navigate response");
    assert!(
        navigate_response.get("error").is_none(),
        "an offline navigation must not be a -32000 protocol error; got {navigate_response:#?}"
    );
    assert!(
        navigate_response["result"]["frameId"].is_string(),
        "Page.navigate must still resolve with a frameId; got {navigate_response:#?}"
    );
    assert!(
        navigate.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Network.loadingFailed")
                && message["params"]["errorText"] == json!("net::ERR_INTERNET_DISCONNECTED")
        }),
        "offline navigation must emit Network.loadingFailed with net::ERR_INTERNET_DISCONNECTED: {navigate:#?}"
    );
    if let Some(error_text) = navigate_response["result"]["errorText"].as_str() {
        assert_eq!(
            error_text, "net::ERR_INTERNET_DISCONNECTED",
            "Page.navigate errorText must match the network error"
        );
    }

    // The committed error Document keeps the target responsive.
    let value =
        cdp_runtime_evaluate_string(&mut socket, &session.session_id, 7, "String(1 + 1)").await;
    assert_eq!(value, "2");

    // Restoring offline:false lets the same loopback navigation succeed.
    let restore = send_cdp_command(
        &mut socket,
        8,
        "Network.emulateNetworkConditions",
        Some(&session.session_id),
        json!({
            "offline": false,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }),
    )
    .await;
    assert!(
        restore
            .iter()
            .any(|message| message["id"] == json!(8_u64) && message.get("error").is_none()),
        "offline:false should be accepted: {restore:?}"
    );

    let reloaded = send_cdp_command(
        &mut socket,
        9,
        "Page.navigate",
        Some(&session.session_id),
        json!({ "url": fixture_url }),
    )
    .await;
    let reloaded_response = reloaded
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("recovered Page.navigate response");
    assert!(
        reloaded_response.get("error").is_none(),
        "navigation after offline:false must succeed; got {reloaded_response:#?}"
    );
    assert!(
        reloaded_response["result"]
            .get("errorText")
            .is_none_or(|value| value.is_null()),
        "a recovered navigation must not report an errorText; got {reloaded_response:#?}"
    );
    let body = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        10,
        "document.querySelector('#probe')?.textContent ?? ''",
    )
    .await;
    assert_eq!(body, "online");

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}
