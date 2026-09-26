use super::*;

#[tokio::test]
async fn websocket_bidi_classic_session_omits_synthetic_service_worker_runtime() {
    let fixture_app = Router::new()
        .route(
            "/page",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><title>classic bidi service worker</title><main>ready</main>",
                )
            }),
        )
        .route(
            "/sw.js",
            get(|| async move {
                (
                    [(
                        axum::http::header::CONTENT_TYPE.as_str(),
                        "text/javascript; charset=utf-8",
                    )],
                    "console.log('classic-bidi-service-worker-log');\n\
                     self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));\n\
                     self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));",
                )
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "bidi-classic-service-worker-surface");
    let page_url = format!("http://{fixture_addr}/page");
    let worker_url = format!("http://{fixture_addr}/sw.js");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let navigated = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"), "{tree:?}");
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("Classic-owned top-level context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "session.subscribe",
                "params": {
                    "events": [
                        "browsingContext.contextCreated",
                        "script.realmCreated",
                        "log.entryAdded"
                    ]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send service worker surface subscription");
    let mut messages = recv_until_id(&mut socket, 2).await;
    let subscribe = bidi_message_by_id(&messages, 2);
    assert_eq!(subscribe["type"], json!("success"), "{subscribe:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": r#"
(async () => {
  const registration = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
  await navigator.serviceWorker.ready;
  const worker = registration.active || registration.waiting ||
      registration.installing || navigator.serviceWorker.controller;
  return worker ? worker.scriptURL : 'missing-service-worker';
})()
"#,
                    "awaitPromise": true,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send service worker registration script.evaluate");
    messages.extend(recv_until_id(&mut socket, 3).await);
    let evaluate = bidi_message_by_id(&messages, 3);
    assert_eq!(evaluate["type"], json!("success"), "{messages:#?}");
    assert_eq!(
        evaluate["result"]["result"],
        json!({
            "type": "string",
            "value": worker_url
        }),
        "service worker registration should resolve with the active script URL: {messages:#?}"
    );

    let service_worker_context = service_worker_context_created(&messages, &worker_url)
        .and_then(|message| message["params"]["context"].as_str())
        .expect("service worker browsing context should be exposed")
        .to_owned();

    assert!(
        messages.iter().all(|message| {
            message["method"] != json!("script.realmCreated")
                || message["params"]["origin"] != json!(worker_url)
                || message["params"]["type"] == json!("service-worker")
        }),
        "a real Service Worker Runtime context must not be exposed as a generic worker realm: {messages:#?}"
    );

    let service_worker_realm = service_worker_realm_created(&messages);
    if let Some(log) = service_worker_log_entry(&messages, &service_worker_context) {
        let realm = service_worker_realm.unwrap_or_else(|| {
            panic!("a Service Worker log must wait for its real Runtime realm: {messages:#?}")
        });
        assert_eq!(
            log["params"]["source"]["realm"], realm["params"]["realm"],
            "Service Worker logs must use the realm id from Runtime.executionContextCreated: {messages:#?}"
        );
        let realm_index = messages
            .iter()
            .position(|message| std::ptr::eq(message, realm))
            .expect("service worker realm position");
        let log_index = messages
            .iter()
            .position(|message| std::ptr::eq(message, log))
            .expect("service worker log position");
        assert!(
            realm_index < log_index,
            "Service Worker realmCreated must precede its log entry: {messages:#?}"
        );
    }

    let realms = send_bidi_command(
        &mut socket,
        4,
        "script.getRealms",
        json!({
            "context": service_worker_context,
            "type": "service-worker"
        }),
    )
    .await;
    assert_eq!(realms["type"], json!("success"), "{realms:?}");
    let returned_realms = realms["result"]["realms"]
        .as_array()
        .expect("script.getRealms result array");
    assert!(
        returned_realms
            .iter()
            .all(|realm| realm["type"] == json!("service-worker")),
        "script.getRealms must expose only real Service Worker-typed realms for the worker target: {realms:?}"
    );
    if let Some(service_worker_realm) = service_worker_realm {
        assert!(
            returned_realms
                .iter()
                .any(|realm| { realm["realm"] == service_worker_realm["params"]["realm"] }),
            "script.getRealms must retain a Service Worker realm that was already created: {realms:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_browser_get_client_windows_matches_wpt_activation_state() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browser/get_client_windows/get_client_windows.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, initial_context_id) = bidi_session_with_context(cdp_addr).await;

    let initial = send_bidi_command(&mut socket, 3, "browser.getClientWindows", json!({})).await;
    assert_eq!(initial["type"], json!("success"));
    let initial_windows = initial["result"]["clientWindows"]
        .as_array()
        .expect("initial clientWindows array");
    assert_eq!(initial_windows.len(), 1);
    let initial_window_id = initial_windows[0]["clientWindow"]
        .as_str()
        .expect("initial clientWindow id")
        .to_owned();
    assert_eq!(initial_window_id, initial_context_id);
    assert_eq!(initial_windows[0]["active"], json!(true));

    let new_window = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({ "type": "window" }),
    )
    .await;
    assert_eq!(new_window["type"], json!("success"));
    let new_context_id = new_window["result"]["context"]
        .as_str()
        .expect("new window context id")
        .to_owned();

    let updated = send_bidi_command(&mut socket, 5, "browser.getClientWindows", json!({})).await;
    assert_eq!(updated["type"], json!("success"));
    let updated_windows = updated["result"]["clientWindows"]
        .as_array()
        .expect("updated clientWindows array");
    assert_eq!(updated_windows.len(), 2);
    assert_ne!(
        updated_windows[0]["clientWindow"],
        updated_windows[1]["clientWindow"]
    );
    let first_window = updated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(initial_window_id))
        .expect("initial client window");
    let second_window = updated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(new_context_id))
        .expect("new client window");
    assert_eq!(first_window["active"], json!(false));
    assert_eq!(second_window["active"], json!(true));

    let activate_initial = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.activate",
        json!({ "context": initial_context_id }),
    )
    .await;
    assert_eq!(activate_initial["type"], json!("success"));

    let activated = send_bidi_command(&mut socket, 7, "browser.getClientWindows", json!({})).await;
    assert_eq!(activated["type"], json!("success"));
    let activated_windows = activated["result"]["clientWindows"]
        .as_array()
        .expect("activated clientWindows array");
    let first_window = activated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(initial_window_id))
        .expect("activated initial client window");
    let second_window = activated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(new_context_id))
        .expect("activated new client window");
    assert_eq!(first_window["active"], json!(true));
    assert_eq!(second_window["active"], json!(false));

    let close = send_bidi_command(
        &mut socket,
        8,
        "browsingContext.close",
        json!({ "context": new_context_id }),
    )
    .await;
    assert_eq!(close["type"], json!("success"));

    let final_windows =
        send_bidi_command(&mut socket, 9, "browser.getClientWindows", json!({})).await;
    assert_eq!(final_windows["type"], json!("success"));
    assert_eq!(
        final_windows["result"]["clientWindows"],
        json!([{
            "clientWindow": initial_window_id,
            "active": true,
            "state": "normal",
            "width": 0,
            "height": 0,
            "x": 0,
            "y": 0
        }])
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_browser_set_client_window_state_updates_owner_info() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let minimized = send_bidi_command(
        &mut socket,
        3,
        "browser.setClientWindowState",
        json!({
            "clientWindow": context_id,
            "state": "minimized"
        }),
    )
    .await;
    assert_eq!(minimized["type"], json!("success"));
    assert_eq!(minimized["result"]["clientWindow"], json!(context_id));
    assert_eq!(minimized["result"]["state"], json!("minimized"));
    assert_eq!(minimized["result"]["active"], json!(true));

    let after_minimize =
        send_bidi_command(&mut socket, 4, "browser.getClientWindows", json!({})).await;
    assert_eq!(after_minimize["type"], json!("success"));
    assert_eq!(
        after_minimize["result"]["clientWindows"][0]["state"],
        json!("minimized")
    );

    let normal = send_bidi_command(
        &mut socket,
        5,
        "browser.setClientWindowState",
        json!({
            "clientWindow": context_id,
            "state": "normal",
            "width": 1024,
            "height": 768,
            "x": -12,
            "y": 34
        }),
    )
    .await;
    assert_eq!(normal["type"], json!("success"));
    assert_eq!(
        normal["result"],
        json!({
            "clientWindow": context_id,
            "active": true,
            "state": "normal",
            "width": 1024,
            "height": 768,
            "x": -12,
            "y": 34
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_environment_uses_process_claims_instead_of_per_context_precedence() {
    // One Moli renderer process shares native ICU defaults. User contexts
    // cannot create independent Date/Intl environments; conflicting ownership
    // must fail without mutating or staging a later navigation policy.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, default_context_id) = bidi_session_with_context(cdp_addr).await;

    let default_locale = bidi_string_script_value(
        &mut socket,
        3,
        &default_context_id,
        "Intl.DateTimeFormat().resolvedOptions().locale",
    )
    .await;
    let default_timezone = bidi_string_script_value(
        &mut socket,
        4,
        &default_context_id,
        "Intl.DateTimeFormat().resolvedOptions().timeZone",
    )
    .await;
    let user_context_locale = if default_locale == "fr-FR" {
        "de-DE"
    } else {
        "fr-FR"
    };
    let context_locale = if user_context_locale == "fr-FR" {
        "de-DE"
    } else {
        "fr-FR"
    };
    let user_context_timezone = if default_timezone == "Asia/Tokyo" {
        "Europe/Berlin"
    } else {
        "Asia/Tokyo"
    };
    let context_timezone = if user_context_timezone == "Asia/Tokyo" {
        "Europe/Berlin"
    } else {
        "Asia/Tokyo"
    };

    let user_context =
        send_bidi_command(&mut socket, 5, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created userContext tab")
        .to_owned();

    let batch = send_bidi_command(
        &mut socket,
        600,
        "emulation.setLocaleOverride",
        json!({"contexts": [default_context_id, user_context_tab_id], "locale": "fr-FR"}),
    )
    .await;
    assert_bidi_error(
        &batch,
        "unsupported operation",
        "multi-owner claim cannot partially commit",
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            601,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        default_locale
    );

    let set_user_context_locale = send_bidi_command(
        &mut socket,
        7,
        "emulation.setLocaleOverride",
        json!({
            "userContexts": [user_context_id],
            "locale": user_context_locale
        }),
    )
    .await;
    assert_eq!(set_user_context_locale["type"], json!("success"));
    let set_user_context_timezone = send_bidi_command(
        &mut socket,
        8,
        "emulation.setTimezoneOverride",
        json!({
            "userContexts": [user_context_id],
            "timezone": user_context_timezone
        }),
    )
    .await;
    assert_eq!(set_user_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            9,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        user_context_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            10,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        user_context_timezone
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            11,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        user_context_locale,
        "native locale is shared across every context in this process"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            12,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        user_context_timezone,
        "native timezone is shared across every context in this process"
    );

    let set_context_locale = send_bidi_command(
        &mut socket,
        13,
        "emulation.setLocaleOverride",
        json!({
            "contexts": [user_context_tab_id],
            "locale": context_locale
        }),
    )
    .await;
    assert_bidi_error(
        &set_context_locale,
        "invalid argument",
        "another owner holds the process claim",
    );
    let set_context_timezone = send_bidi_command(
        &mut socket,
        14,
        "emulation.setTimezoneOverride",
        json!({
            "contexts": [user_context_tab_id],
            "timezone": context_timezone
        }),
    )
    .await;
    assert_bidi_error(
        &set_context_timezone,
        "invalid argument",
        "another owner holds the process claim",
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            15,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        user_context_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            16,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        user_context_timezone
    );

    let reset_context_locale = send_bidi_command(
        &mut socket,
        17,
        "emulation.setLocaleOverride",
        json!({
            "contexts": [user_context_tab_id],
            "locale": null
        }),
    )
    .await;
    assert_bidi_error(
        &reset_context_locale,
        "invalid argument",
        "another owner holds the process claim",
    );
    let reset_context_timezone = send_bidi_command(
        &mut socket,
        18,
        "emulation.setTimezoneOverride",
        json!({
            "contexts": [user_context_tab_id],
            "timezone": null
        }),
    )
    .await;
    assert_eq!(reset_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            19,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        user_context_locale,
        "a rejected non-owner reset must preserve the process locale"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            20,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        user_context_timezone,
        "a non-owner reset must preserve the process timezone"
    );

    let reset_user_context_locale = send_bidi_command(
        &mut socket,
        21,
        "emulation.setLocaleOverride",
        json!({
            "userContexts": [user_context_id],
            "locale": null
        }),
    )
    .await;
    assert_eq!(reset_user_context_locale["type"], json!("success"));
    let reset_user_context_timezone = send_bidi_command(
        &mut socket,
        22,
        "emulation.setTimezoneOverride",
        json!({
            "userContexts": [user_context_id],
            "timezone": null
        }),
    )
    .await;
    assert_eq!(reset_user_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            23,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        default_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            24,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        default_timezone
    );

    // Disposal must release process claims even while another context stays
    // alive; no explicit reset or navigation is necessary for that peer.
    for (id, method, params) in [
        (
            25,
            "emulation.setLocaleOverride",
            json!({
                "userContexts": [user_context_id], "locale": user_context_locale
            }),
        ),
        (
            26,
            "emulation.setTimezoneOverride",
            json!({
                "userContexts": [user_context_id], "timezone": user_context_timezone
            }),
        ),
    ] {
        let response = send_bidi_command(&mut socket, id, method, params).await;
        assert_eq!(response["type"], json!("success"));
    }
    let removed = send_bidi_command(
        &mut socket,
        27,
        "browser.removeUserContext",
        json!({"userContext": user_context_id}),
    )
    .await;
    assert_eq!(removed["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            28,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().locale",
        )
        .await,
        default_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            29,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone",
        )
        .await,
        default_timezone
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
