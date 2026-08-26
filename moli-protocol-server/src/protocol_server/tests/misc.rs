use super::*;
use moli_protocol::version;

#[test]
fn protocol_server_constructor_preserves_optional_resource_fetch_policy() {
    let partition = Arc::new(StoragePartitionState::open(None).unwrap());
    let mask = OptionalResourceFetchMask::FONT
        | OptionalResourceFetchMask::VIDEO
        | OptionalResourceFetchMask::TEXT_TRACK;
    let server = ProtocolServer::new_with_storage_partition_fetch_config_and_resource_loading(
        ServerConfig::default(),
        partition,
        FetchConfig::default(),
        mask,
        true,
    );

    assert_eq!(
        server
            .navigation_runtime_config
            .optional_resource_fetch_mask(),
        mask
    );
}

#[test]
fn protocol_server_constructor_preserves_subframe_loading_policy() {
    let partition = Arc::new(StoragePartitionState::open(None).unwrap());
    let server = ProtocolServer::new_with_storage_partition_fetch_config_and_resource_loading(
        ServerConfig::default(),
        partition,
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
        false,
    );

    assert!(!server.navigation_runtime_config.subframe_loading_enabled());
}

#[test]
fn merge_cookie_profiles_replaces_existing_cookie_by_storage_key() {
    let mut cookies = vec![stored_cookie("sid", "old"), stored_cookie("theme", "dark")];

    merge_cookie_profiles(
        &mut cookies,
        vec![stored_cookie("sid", "new"), stored_cookie("extra", "1")],
    );

    assert_eq!(cookies.len(), 3);
    assert_eq!(
        cookies
            .iter()
            .find(|cookie| cookie.name == "sid")
            .map(|cookie| cookie.value.as_str()),
        Some("new")
    );
    assert_eq!(
        cookies
            .iter()
            .find(|cookie| cookie.name == "theme")
            .map(|cookie| cookie.value.as_str()),
        Some("dark")
    );
    assert_eq!(
        cookies
            .iter()
            .find(|cookie| cookie.name == "extra")
            .map(|cookie| cookie.value.as_str()),
        Some("1")
    );
}

#[test]
fn commit_cookie_profile_removes_initial_cookie_missing_from_final_snapshot() {
    let deleted = stored_cookie("sid", "old");
    let mut cookies = vec![deleted.clone(), stored_cookie("theme", "dark")];

    commit_cookie_profile(
        &mut cookies,
        CookieProfileCommit::new(vec![deleted], vec![stored_cookie("theme", "dark")]),
    );

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "theme");
}

#[test]
fn commit_cookie_profile_preserves_concurrent_update_when_session_deleted_stale_cookie() {
    let initial = stored_cookie("sid", "old");
    let concurrent = stored_cookie("sid", "newer");
    let mut cookies = vec![concurrent];

    commit_cookie_profile(
        &mut cookies,
        CookieProfileCommit::new(vec![initial], Vec::new()),
    );

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "sid");
    assert_eq!(cookies[0].value, "newer");
}

#[test]
fn commit_cookie_profile_merges_final_updates_and_drops_expired_cookies() {
    let mut expired = stored_cookie("expired", "gone");
    expired.expires = Some(time::OffsetDateTime::now_utc() - time::Duration::days(1));
    let mut cookies = vec![
        stored_cookie("sid", "old"),
        stored_cookie("theme", "dark"),
        expired.clone(),
    ];

    commit_cookie_profile(
        &mut cookies,
        CookieProfileCommit::new(
            vec![stored_cookie("sid", "old")],
            vec![
                stored_cookie("sid", "new"),
                stored_cookie("extra", "1"),
                expired,
            ],
        ),
    );

    assert_eq!(
        cookies
            .iter()
            .find(|cookie| cookie.name == "sid")
            .map(|cookie| cookie.value.as_str()),
        Some("new")
    );
    assert_eq!(
        cookies
            .iter()
            .find(|cookie| cookie.name == "theme")
            .map(|cookie| cookie.value.as_str()),
        Some("dark")
    );
    assert!(cookies.iter().any(|cookie| cookie.name == "extra"));
    assert!(!cookies.iter().any(|cookie| cookie.name == "expired"));
}

#[test]
fn shared_cookie_profile_merge_and_save_writes_cache_file() -> anyhow::Result<()> {
    let path = std::env::temp_dir().join(format!(
        "moli-cdp-cookie-profile-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let profile = SharedCookieProfile::new(vec![stored_cookie("sid", "old")], vec![path.clone()]);

    profile.merge_and_save(vec![stored_cookie("sid", "new")])?;

    let loaded = cookie_cache::load_cookie_cache(&path)?;
    let _ = std::fs::remove_file(&path);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "sid");
    assert_eq!(loaded[0].value, "new");
    Ok(())
}

#[test]
fn shared_cookie_profile_partition_backing_uses_storage_partition() -> anyhow::Result<()> {
    let profile = TempDir::new("shared-cookie-profile-partition");
    let paths = BrowserProfilePaths::new(&profile.path);
    let partition = Arc::new(StoragePartitionState::open(Some(&profile.path))?);
    partition.import_cookies(vec![stored_cookie("sid", "old")])?;
    let profile = SharedCookieProfile::from_storage_partition(partition.clone());

    let initial_cookies = profile.snapshot();
    profile.commit_and_save(CookieProfileCommit::new(
        initial_cookies,
        vec![stored_cookie("sid", "new")],
    ))?;

    let partition_cookies = partition.cookies()?;
    assert_eq!(partition_cookies.len(), 1);
    assert_eq!(partition_cookies[0].value, "new");
    let persisted = cookie_cache::load_cookie_cache(&paths.cookies_path)?;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].value, "new");
    Ok(())
}

#[test]
fn app_state_storage_partition_backing_derives_initial_partition_from_owner() -> anyhow::Result<()>
{
    let partition = Arc::new(StoragePartitionState::open(None)?);
    let state = AppState::new_with_storage_partition(
        "127.0.0.1:9222".parse().expect("test addr"),
        partition.clone(),
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
        true,
    )?;
    let initial_storage_partition =
        state.initial_storage_partition(vec![stored_cookie("owner-cookie", "owner-value")]);
    let mut conn =
        moli_protocol::CdpConnection::new_with_initial_storage_partition(initial_storage_partition);

    assert!(conn.snapshot_profile_backed_cookies().is_none());
    conn.install_default_browser_target();
    let cookies = conn.snapshot_profile_backed_cookies().unwrap();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "owner-cookie");
    assert_eq!(cookies[0].value, "owner-value");
    Ok(())
}

#[tokio::test]
async fn discovery_endpoints_accept_trailing_slashes() {
    let version = request_json("/json/version").await;
    assert_eq!(version, request_json("/json/version/").await);
    assert_eq!(
        version["webSocketDebuggerUrl"],
        json!("ws://127.0.0.1:9222/devtools/browser/moli-browser")
    );
    assert_eq!(version["Browser"], json!(version::PRODUCT));
    assert_eq!(
        version["Protocol-Version"],
        json!(version::PROTOCOL_VERSION)
    );
    assert_eq!(
        version["User-Agent"],
        json!(FetchConfig::DEFAULT_USER_AGENT)
    );
    assert_eq!(version["V8-Version"], json!(version::js_version()));
    assert_eq!(version["WebKit-Version"], json!(version::WEBKIT_VERSION));

    let list = request_json("/json").await;
    assert_eq!(list, request_json("/json/list").await);
    assert_eq!(list, request_json("/json/list/").await);
    assert_eq!(
        list[0]["webSocketDebuggerUrl"],
        json!("ws://127.0.0.1:9222/devtools/page/moli-default")
    );
    assert_eq!(list[0]["description"], json!(""));
    assert_eq!(
        list[0]["devtoolsFrontendUrl"],
        json!("/devtools/inspector.html?ws=127.0.0.1:9222/devtools/page/moli-default")
    );
    assert_eq!(list[0]["id"], json!(DEFAULT_TARGET_ID));
    assert_eq!(list[0]["title"], json!(""));
    assert_eq!(list[0]["type"], json!("page"));
    assert_eq!(list[0]["url"], json!(DEFAULT_TARGET_URL));

    let protocol = request_json("/json/protocol").await;
    assert_eq!(protocol, request_json("/json/protocol/").await);
    assert_eq!(protocol["version"]["major"], json!("1"));
    assert_eq!(protocol["version"]["minor"], json!("3"));
    assert!(protocol["domains"].is_array());

    assert_eq!(
        request_status("/json/new").await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(request_status("/json/").await, StatusCode::NOT_FOUND);
    let new_target = request_json_with_method(
        Method::PUT,
        "/json/new?about%3Ablank%23trailing-slash-target",
    )
    .await;
    assert_ne!(new_target["id"], json!(DEFAULT_TARGET_ID));
    assert!(
        new_target["webSocketDebuggerUrl"]
            .as_str()
            .is_some_and(|url| url.ends_with(new_target["id"].as_str().unwrap_or_default()))
    );
    assert_eq!(
        new_target["url"],
        json!("about:blank#trailing-slash-target")
    );
    assert_eq!(new_target["title"], json!(""));
    assert_eq!(
        request_json_with_method(Method::PUT, "/json/new").await["url"],
        json!(DEFAULT_TARGET_URL)
    );
    assert_eq!(
        request_status("/json/activate/moli-default").await,
        StatusCode::OK
    );
    assert_eq!(
        request_status("/json/activate/missing").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request_status("/json/close/moli-default").await,
        StatusCode::OK
    );
    assert_eq!(
        request_status("/json/close/missing").await,
        StatusCode::NOT_FOUND
    );

    assert_eq!(request_status("/cdp").await, StatusCode::NOT_FOUND);

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    assert_eq!(
        rejected_websocket_status(format!("ws://{cdp_addr}/devtools/browser/missing")).await,
        StatusCode::NOT_FOUND.as_u16()
    );
    assert_eq!(
        rejected_websocket_status(format!("ws://{cdp_addr}/devtools/page/missing")).await,
        StatusCode::NOT_FOUND.as_u16()
    );
    protocol_server.abort();
}

#[tokio::test]
async fn browser_websocket_supports_puppeteer_browser_target_session() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to browser cdp websocket");

    let discover = send_cdp_command(
        &mut socket,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({ "discover": true, "filter": [{}] }),
    )
    .await;
    let browser_target = discover
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["type"] == json!("browser")
        })
        .expect("catch-all discovery should report the browser target");
    let browser_target_id = browser_target["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("browser target id")
        .to_owned();

    let attach = send_cdp_command(
        &mut socket,
        2,
        "Target.attachToTarget",
        None,
        json!({ "targetId": browser_target_id, "flatten": true }),
    )
    .await;
    let browser_session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("browser target session id")
        .to_owned();
    assert!(attach.iter().any(|message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(browser_session_id)
            && message["params"]["targetInfo"]["targetId"] == json!(browser_target_id)
    }));

    let version = send_cdp_command(
        &mut socket,
        3,
        "Browser.getVersion",
        Some(&browser_session_id),
        json!({}),
    )
    .await;
    let version_response = version
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .expect("Browser.getVersion response");
    assert_eq!(version_response["sessionId"], json!(browser_session_id));
    assert!(version_response["result"]["product"].is_string());

    socket.close(None).await.expect("close browser websocket");
    protocol_server.abort();
}

#[tokio::test]
async fn browser_websocket_exposes_same_default_target_as_json_list() {
    let list = request_json("/json/list").await;
    assert_eq!(list[0]["id"], json!(DEFAULT_TARGET_ID));
    assert_eq!(list[0]["url"], json!(DEFAULT_TARGET_URL));

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to browser cdp websocket");

    let discover = send_cdp_command(
        &mut socket,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({
            "discover": true,
            "filter": [{ "type": "page" }]
        }),
    )
    .await;
    assert!(
        discover
            .iter()
            .any(|message| message["id"] == json!(1_u64) && message["result"] == json!({})),
        "Target.setDiscoverTargets should succeed: {discover:?}"
    );

    let get_targets = send_cdp_command(&mut socket, 2, "Target.getTargets", None, json!({})).await;
    let response = get_targets
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .expect("Target.getTargets response");
    let target_infos = response["result"]["targetInfos"]
        .as_array()
        .expect("Target.getTargets targetInfos");
    assert_eq!(target_infos.len(), 1);
    assert_eq!(target_infos[0]["targetId"], json!(DEFAULT_TARGET_ID));
    assert_eq!(target_infos[0]["url"], json!(DEFAULT_TARGET_URL));
    assert_eq!(target_infos[0]["attached"], json!(false));

    let target_created = get_targets
        .iter()
        .chain(discover.iter())
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .expect("Target.setDiscoverTargets should report the existing default page target");
    assert_eq!(
        target_created["params"]["targetInfo"]["targetId"],
        json!(DEFAULT_TARGET_ID)
    );

    let attach = send_cdp_command(
        &mut socket,
        3,
        "Target.attachToTarget",
        None,
        json!({
            "targetId": DEFAULT_TARGET_ID,
            "flatten": true
        }),
    )
    .await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("default target sessionId")
        .to_owned();

    let _ = send_cdp_command(
        &mut socket,
        4,
        "Target.setAutoAttach",
        None,
        json!({
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }),
    )
    .await;

    let get_targets_after_auto_attach =
        send_cdp_command(&mut socket, 5, "Target.getTargets", None, json!({})).await;
    let response = get_targets_after_auto_attach
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("Target.getTargets response after auto-attach");
    let target_infos = response["result"]["targetInfos"]
        .as_array()
        .expect("Target.getTargets targetInfos after auto-attach");
    assert_eq!(target_infos.len(), 1);
    assert_eq!(target_infos[0]["targetId"], json!(DEFAULT_TARGET_ID));
    assert_eq!(target_infos[0]["attached"], json!(true));

    assert!(
        !get_targets_after_auto_attach.iter().any(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] != json!(DEFAULT_TARGET_ID)
        }),
        "browser-use startup sequence must not create a second page target: {get_targets_after_auto_attach:?}"
    );

    let _ = send_cdp_command(
        &mut socket,
        6,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn chromium_devtools_json_list_accepts_non_get_methods_and_for_tab() {
    let list = request_json("/json").await;
    let tab_list = request_json("/json/list?for_tab").await;

    // Ported from Chromium DevToolsHttpHandlerTest.TestJsonList: list
    // discovery is not method-gated. Chromium's remote-device discovery asks
    // for tab targets explicitly so its frontend can auto-attach the current
    // page below a stable tab root.
    assert_eq!(request_json_with_method(Method::PUT, "/json").await, list);
    assert_eq!(
        request_json_with_method(Method::PUT, "/json/list").await,
        list
    );
    assert_eq!(
        request_json_with_method(Method::PUT, "/json/list?for_tab").await,
        tab_list
    );
    assert_eq!(list[0]["type"], json!("page"));
    assert_eq!(
        list[0]["webSocketDebuggerUrl"],
        json!("ws://127.0.0.1:9222/devtools/page/moli-default")
    );
    assert_eq!(tab_list.as_array().map(Vec::len), Some(2));
    assert!(tab_list.as_array().is_some_and(|targets| {
        targets.iter().any(|target| {
            target["id"] == json!(DEFAULT_TARGET_ID) && target["type"] == json!("page")
        })
    }));
    let tab = tab_list
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target["id"] == json!(DEFAULT_TAB_TARGET_ID))
        })
        .expect("for_tab discovery should add the stable tab target");
    assert_eq!(tab["type"], json!("tab"));
    assert_eq!(
        tab["devtoolsFrontendUrl"],
        json!("/devtools/inspector.html?ws=127.0.0.1:9222/devtools/page/moli-default-tab")
    );
    assert_eq!(
        tab["webSocketDebuggerUrl"],
        json!("ws://127.0.0.1:9222/devtools/page/moli-default-tab")
    );
}

#[tokio::test]
async fn chromium_devtools_json_new_uses_put_and_decodes_first_query_component() {
    // Ported from Chromium DevToolsHttpHandlerTest.MutatingActionsiRequireSafeVerb.
    assert_eq!(
        request_status("/json/new").await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        request_status_with_method(Method::POST, "/json/new").await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        request_json_with_method(Method::PUT, "/json/new").await["url"],
        json!(DEFAULT_TARGET_URL)
    );

    // Ported from Chromium DevToolsHttpHandlerTest.TestJsonNew: the first
    // query component is the target URL, and a trailing for_tab flag does
    // not become part of that URL.
    let encoded = "about%3Ablank%3Fq%3D1%26x%3D2";
    let target =
        request_json_with_method(Method::PUT, &format!("/json/new?{encoded}&for_tab")).await;
    assert_eq!(target["url"], json!("about:blank?q=1&x=2"));
    assert_eq!(target["title"], json!(""));
    assert_eq!(target["type"], json!("tab"));
    assert!(
        target["webSocketDebuggerUrl"]
            .as_str()
            .is_some_and(|url| url.contains("/devtools/page/TAB-"))
    );

    let target = request_json_with_method(Method::PUT, "/json/new?url=about%3Ablank%23named").await;
    assert_eq!(target["url"], json!("about:blank#named"));

    let target = request_json_with_method(
        Method::PUT,
        "/json/new?session=abc&url=about%3Ablank%23session",
    )
    .await;
    assert_eq!(target["url"], json!("about:blank#session"));

    let target = request_json_with_method(Method::PUT, "/json/new?data:text/plain,direct").await;
    assert_eq!(target["url"], json!("data:text/plain,direct"));

    let target = request_json_with_method(Method::PUT, "/json/new?not%20a%20url").await;
    assert_eq!(target["url"], json!(DEFAULT_TARGET_URL));
}

#[tokio::test]
async fn chromium_devtools_activate_and_close_return_text_payloads() {
    // Ported from chrome/test/data/devtools/target_list/background.js.
    let (status, body) =
        request_status_and_text_with_method(Method::GET, "/json/activate/moli-default").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "Target activated");

    let (status, body) =
        request_status_and_text_with_method(Method::GET, "/json/close/moli-default").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "Target is closing");

    let (status, body) =
        request_status_and_text_with_method(Method::GET, "/json/activate/missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, "No such target id: missing");
}

#[tokio::test]
async fn closed_default_target_is_not_readvertised_by_discovery() {
    let app = build_router(test_state());
    let first_list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/json/list?for_tab")
                .body(Body::empty())
                .expect("list request"),
        )
        .await
        .expect("first list response");
    let first_list = serde_json::from_slice::<serde_json::Value>(
        &to_bytes(first_list.into_body(), usize::MAX)
            .await
            .expect("first list body"),
    )
    .expect("first list JSON");
    assert!(first_list.as_array().is_some_and(|targets| {
        targets
            .iter()
            .any(|target| target["id"] == json!(DEFAULT_TARGET_ID))
            && targets
                .iter()
                .any(|target| target["id"] == json!(DEFAULT_TAB_TARGET_ID))
    }));

    let close = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/json/close/moli-default")
                .body(Body::empty())
                .expect("close request"),
        )
        .await
        .expect("close response");
    assert_eq!(close.status(), StatusCode::OK);

    let second_list = app
        .oneshot(
            Request::builder()
                .uri("/json/list?for_tab")
                .body(Body::empty())
                .expect("second list request"),
        )
        .await
        .expect("second list response");
    let second_list = serde_json::from_slice::<serde_json::Value>(
        &to_bytes(second_list.into_body(), usize::MAX)
            .await
            .expect("second list body"),
    )
    .expect("second list JSON");
    assert_eq!(second_list, json!([]));
}
