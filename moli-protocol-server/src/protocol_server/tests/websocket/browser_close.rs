use super::*;

async fn close_profiled_server(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    server: tokio::task::JoinHandle<()>,
) {
    let messages = send_cdp_command(socket, 42, "Browser.close", None, json!({})).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == 42)
        .expect("Browser.close response");
    assert_eq!(response, &json!({"id": 42, "result": {}}));
    timeout(Duration::from_secs(10), server)
        .await
        .expect("profiled server exits after Browser.close")
        .expect("profiled server returns cleanly");
}

#[tokio::test]
async fn websocket_cdp_browser_close_releases_lock_and_persists_profile() {
    let profile = TempDir::new("browser-close-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let page_url = format!("http://{fixture_addr}/page");

    let (addr, server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    assert!(
        BrowserProfileLock::acquire(&paths).is_err(),
        "profile is locked while serving"
    );
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect to profiled browser");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let write = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        "localStorage.clear(); localStorage.setItem('persisted', 'yes'); 'ok'",
    )
    .await;
    assert_eq!(write, "ok");

    close_profiled_server(&mut socket, server).await;
    wait_for_profile_lock_release(&paths).await;
    assert!(paths.local_storage_path.is_file());
    assert!(!paths.partition_root.join("localstorage.json").exists());
    {
        let partition = StoragePartitionState::open(Some(&profile.path)).unwrap();
        assert_eq!(
            partition
                .shared_storage_handles()
                .web_storage_store()
                .get_item(
                    &moli_core::network::web_storage_partitioned_area_key(
                        &format!("http://{fixture_addr}"),
                        &format!("http://{}", fixture_addr.ip()),
                    ),
                    "persisted",
                )
                .as_deref(),
            Some("yes")
        );
    }

    let (addr, server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("reopen the same profile immediately");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;
    let read = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        6,
        "String(localStorage.getItem('persisted'))",
    )
    .await;
    assert_eq!(read, "yes");

    close_profiled_server(&mut socket, server).await;
    wait_for_profile_lock_release(&paths).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_secure_cookie_with_nonsecure_source_survives_restart() {
    let profile = TempDir::new("secure-cookie-source-profile");
    let paths = BrowserProfilePaths::new(&profile.path);
    let mut expected = Vec::new();
    for round in 0..2 {
        let (addr, server) = spawn_profiled_test_protocol_server(profile.path.clone()).await;
        let (mut socket, _) =
            connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
                .await
                .expect("connect to profiled browser");
        let target = cdp_create_attached_target(&mut socket, 1, "BID-default").await;
        if round == 0 {
            for (id, params) in [
                (
                    3,
                    json!({
                        "url": "https://cookie.test/",
                        "name": "sid",
                        "value": "123",
                        "secure": true,
                        "sourceScheme": "NonSecure"
                    }),
                ),
                (
                    4,
                    json!({
                        "url": "https://other.test/",
                        "name": "other",
                        "value": "kept"
                    }),
                ),
            ] {
                let messages = send_cdp_command(
                    &mut socket,
                    id,
                    "Network.setCookie",
                    Some(&target.session_id),
                    params,
                )
                .await;
                let response = messages.iter().find(|message| message["id"] == id).unwrap();
                assert!(response["error"].is_null(), "{response}");
                assert_eq!(response["result"]["success"], true);
            }
        }
        let messages =
            send_cdp_command(&mut socket, 5, "Storage.getCookies", None, json!({})).await;
        let response = messages.iter().find(|message| message["id"] == 5).unwrap();
        assert!(response["error"].is_null(), "{response}");
        let cookies = response["result"]["cookies"].as_array().unwrap();
        if round == 0 {
            assert_eq!(cookies.len(), 2);
            let cookie = cookies
                .iter()
                .find(|cookie| cookie["name"] == "sid")
                .unwrap();
            assert_eq!(cookie["secure"], true);
            assert_eq!(cookie["sourceScheme"], "NonSecure");
            assert_eq!(cookie["value"], "123");
            expected = cookies.clone();
        } else {
            assert_eq!(
                cookies, &expected,
                "all cookies and metadata survive restart"
            );
        }
        close_profiled_server(&mut socket, server).await;
        wait_for_profile_lock_release(&paths).await;
    }
}
