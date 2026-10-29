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
    let persisted = fs::read_to_string(&paths.local_storage_path)
        .expect("localStorage checkpoint exists before shutdown completes");
    assert!(
        persisted.contains("\"persisted\"") && persisted.contains("\"yes\""),
        "localStorage checkpoint contains the saved entry: {persisted}"
    );

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
