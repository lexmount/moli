use super::*;

#[tokio::test]
async fn websocket_bidi_hidden_scrollbars_preserve_server_configuration() {
    let config = protocol_server_test_runtime_config(
        protocol_server_test_fetch_config(FetchConfig::default()),
        OptionalResourceFetchMask::NONE,
    )
    .with_scrollbars_hidden(true);
    let (addr, server) = spawn_test_protocol_server_with_runtime_config(config).await;
    let (mut socket, context) = bidi_session_with_context(addr).await;
    let viewport = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.setViewport",
        json!({
            "context":context,"viewport":{"width":1280,"height":720}
        }),
    )
    .await;
    assert_eq!(viewport["type"], "success", "{viewport}");
    let navigated = send_bidi_command(&mut socket,4,"browsingContext.navigate",json!({
        "context":context,"url":"data:text/html,<!doctype html><body style='margin:0'><div style='width:100vw;height:2000px'></div>","wait":"complete"
    })).await;
    assert_eq!(navigated["type"], "success", "{navigated}");
    let captured = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.captureScreenshot",
        json!({"context":context}),
    )
    .await;
    assert_eq!(captured["type"], "success", "{captured}");
    let metrics = send_bidi_command(&mut socket,6,"script.evaluate",json!({
        "target":{"context":context},"awaitPromise":false,
        "expression":"JSON.stringify([innerWidth,document.documentElement.clientWidth,document.documentElement.scrollWidth])"
    })).await;
    assert_eq!(metrics["type"], "success", "{metrics}");
    assert_eq!(metrics["result"]["result"]["value"], "[1280,1280,1280]");
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_bidi_page_and_box_screenshot_publish_real_png() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/capture_screenshot/capture_screenshot.py and
    // webdriver/tests/bidi/browsing_context/capture_screenshot/clip.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let viewport = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.setViewport",
        json!({
            "context": context_id,
            "viewport": {
                "width": 120,
                "height": 80
            },
            "devicePixelRatio": 1.5
        }),
    )
    .await;
    assert_eq!(viewport["type"], json!("success"));

    let navigate = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<div style='width:1000px;height:1000px'>capture</div>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let full = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "format": {
                "type": "image/png"
            }
        }),
    )
    .await;
    assert_eq!(full["type"], "success", "{full:?}");
    let bytes = BASE64_STANDARD
        .decode(full["result"]["data"].as_str().expect("PNG data"))
        .expect("valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 180);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 120);

    let clip = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "clip": {
                "type": "box",
                "x": 5,
                "y": 10,
                "width": 33,
                "height": 17
            }
        }),
    )
    .await;
    assert_eq!(clip["type"], "success", "{clip:?}");
    let bytes = BASE64_STANDARD
        .decode(clip["result"]["data"].as_str().expect("PNG data"))
        .expect("valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 50);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 26);

    let navigate_element = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<input id='clip-target'>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate_element["type"], json!("success"));

    let element = send_bidi_command(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "document.querySelector('#clip-target')",
            "awaitPromise": false,
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_eq!(element["type"], json!("success"));
    assert_eq!(element["result"]["type"], json!("success"));
    let shared_id = element["result"]["result"]["sharedId"]
        .as_str()
        .expect("element remote value should include sharedId")
        .to_owned();

    let element_clip = send_bidi_command(
        &mut socket,
        9,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "clip": {
                "type": "element",
                "element": {
                    "sharedId": shared_id
                }
            }
        }),
    )
    .await;
    assert_eq!(
        element_clip["type"],
        json!("error"),
        "element clip screenshot should fail explicitly: {element_clip:?}"
    );
    assert_eq!(element_clip["error"], json!("unsupported operation"));
    assert_eq!(
        element_clip["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_print_reports_unsupported_without_placeholder_pdf() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/print/page.py and
    // webdriver/tests/bidi/browsing_context/print/orientation.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<main>print</main>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let portrait = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.print",
        json!({
            "context": context_id,
            "orientation": "portrait",
            "page": {
                "width": 10.0,
                "height": 20.0
            },
            "margin": {
                "top": 0,
                "bottom": 0,
                "left": 0,
                "right": 0
            }
        }),
    )
    .await;
    assert_eq!(portrait["type"], json!("error"));
    assert_eq!(portrait["error"], json!("unsupported operation"));
    assert_eq!(
        portrait["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let landscape = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.print",
        json!({
            "context": context_id,
            "orientation": "landscape",
            "page": {
                "width": 10.0,
                "height": 20.0
            },
            "margin": {
                "top": 0,
                "bottom": 0,
                "left": 0,
                "right": 0
            }
        }),
    )
    .await;
    assert_eq!(landscape["type"], json!("error"));
    assert_eq!(landscape["error"], json!("unsupported operation"));
    assert_eq!(
        landscape["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_set_viewport_resets_and_persists_across_navigation() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let initial_url = "data:text/html,<title>Viewport Initial</title><main>viewport-initial</main>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": initial_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial viewport browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));

    let original_surface = bidi_viewport_surface(&mut socket, 4, &context_id).await;
    let override_surface = json!({
        "width": 499_u64,
        "height": 599_u64,
        "dpr": 2_u64
    });
    assert_ne!(original_surface, override_surface);

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": context_id.clone(),
                    "viewport": {
                        "width": 499,
                        "height": 599
                    },
                    "devicePixelRatio": 2.0
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.setViewport override");
    let viewport = recv_ws_json(&mut socket).await;
    assert_eq!(viewport["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 6, &context_id).await,
        override_surface
    );

    let first_url = "data:text/html,<title>Viewport A</title><main>viewport-a</main>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": first_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first viewport browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 8, &context_id).await,
        override_surface
    );

    let second_url = "data:text/html,<title>Viewport B</title><main>viewport-b</main>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": second_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second viewport browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 10, &context_id).await,
        override_surface
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "browsingContext.reload",
                "params": {
                    "context": context_id.clone(),
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send viewport browsingContext.reload");
    let reload = recv_ws_json(&mut socket).await;
    assert_eq!(reload["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 12, &context_id).await,
        override_surface
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": context_id.clone(),
                    "viewport": null,
                    "devicePixelRatio": null
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.setViewport reset");
    let reset = recv_ws_json(&mut socket).await;
    assert_eq!(reset["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 14, &context_id).await,
        original_surface
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_emulation_set_user_agent_override_matches_wpt_precedence() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_user_agent_override/user_agent.py,
    // contexts.py, global.py, and user_contexts.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, default_context_id) = bidi_session_with_context(cdp_addr).await;

    let default_user_agent =
        bidi_string_script_value(&mut socket, 3, &default_context_id, "navigator.userAgent").await;
    let global_user_agent = "Moli-BiDi-Global-UA/1.0";
    let user_context_user_agent = "Moli-BiDi-UserContext-UA/1.0";
    let context_user_agent = "Moli-BiDi-Context-UA/1.0";
    assert_ne!(default_user_agent, global_user_agent);

    let set_global = send_bidi_command(
        &mut socket,
        4,
        "emulation.setUserAgentOverride",
        json!({
            "userAgent": global_user_agent
        }),
    )
    .await;
    assert_eq!(set_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 5, &default_context_id, "navigator.userAgent").await,
        global_user_agent
    );

    let user_context =
        send_bidi_command(&mut socket, 6, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        7,
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
    assert_eq!(
        bidi_string_script_value(&mut socket, 8, &user_context_tab_id, "navigator.userAgent").await,
        global_user_agent,
        "new userContext tab should inherit the global userAgent override"
    );

    let set_user_context = send_bidi_command(
        &mut socket,
        9,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": [user_context_id],
            "userAgent": user_context_user_agent
        }),
    )
    .await;
    assert_eq!(set_user_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 10, &user_context_tab_id, "navigator.userAgent")
            .await,
        user_context_user_agent
    );
    assert_eq!(
        bidi_string_script_value(&mut socket, 11, &default_context_id, "navigator.userAgent").await,
        global_user_agent,
        "non-default userContext override should not affect default contexts"
    );

    let set_context = send_bidi_command(
        &mut socket,
        12,
        "emulation.setUserAgentOverride",
        json!({
            "contexts": [user_context_tab_id],
            "userAgent": context_user_agent
        }),
    )
    .await;
    assert_eq!(set_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 13, &user_context_tab_id, "navigator.userAgent")
            .await,
        context_user_agent
    );

    let reset_context = send_bidi_command(
        &mut socket,
        14,
        "emulation.setUserAgentOverride",
        json!({
            "contexts": [user_context_tab_id],
            "userAgent": null
        }),
    )
    .await;
    assert_eq!(reset_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 15, &user_context_tab_id, "navigator.userAgent")
            .await,
        user_context_user_agent,
        "context reset should reveal userContext userAgent override"
    );

    let reset_user_context = send_bidi_command(
        &mut socket,
        16,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": [user_context_id],
            "userAgent": null
        }),
    )
    .await;
    assert_eq!(reset_user_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 17, &user_context_tab_id, "navigator.userAgent")
            .await,
        global_user_agent,
        "userContext reset should reveal global userAgent override"
    );

    let reset_global = send_bidi_command(
        &mut socket,
        18,
        "emulation.setUserAgentOverride",
        json!({
            "userAgent": null
        }),
    )
    .await;
    assert_eq!(reset_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 19, &default_context_id, "navigator.userAgent").await,
        default_user_agent
    );
    assert_eq!(
        bidi_string_script_value(&mut socket, 20, &user_context_tab_id, "navigator.userAgent")
            .await,
        default_user_agent
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_emulation_set_network_conditions_matches_wpt_precedence() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_network_conditions/{contexts,global,user_contexts}.py
    // plus Selenium's set_network_conditions(offline=True/False) facade smoke.
    async fn index() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>network conditions</title>",
        )
    }
    async fn ping() -> &'static str {
        "pong"
    }

    let fixture_app = Router::new()
        .route("/", get(index))
        .route("/ping", get(ping));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network conditions fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network conditions fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, default_context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command_response(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": default_context_id.clone(),
            "url": fixture_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            4,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );
    assert_eq!(
        bidi_awaited_string_script_value(
            &mut socket,
            5,
            &default_context_id,
            "fetch('/ping').then(response => response.text()).catch(() => 'offline')",
        )
        .await,
        "pong"
    );

    let set_global_offline = send_bidi_command(
        &mut socket,
        6,
        "emulation.setNetworkConditions",
        json!({
            "networkConditions": {
                "type": "offline"
            }
        }),
    )
    .await;
    assert_eq!(set_global_offline["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            7,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "false"
    );
    assert_eq!(
        bidi_awaited_string_script_value(
            &mut socket,
            8,
            &default_context_id,
            "fetch('/ping').then(response => response.text()).catch(() => 'offline')",
        )
        .await,
        "offline"
    );

    let user_context =
        send_bidi_command(&mut socket, 9, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();
    let user_context_tab = send_bidi_command(
        &mut socket,
        10,
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
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            11,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "later userContext tab should inherit the global network conditions"
    );

    let reset_user_context_under_global = send_bidi_command(
        &mut socket,
        12,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": [user_context_id],
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_user_context_under_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            13,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "userContext reset should reveal global network conditions"
    );

    let reset_global = send_bidi_command(
        &mut socket,
        14,
        "emulation.setNetworkConditions",
        json!({
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            15,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            16,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );

    let set_user_context_offline = send_bidi_command(
        &mut socket,
        17,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": [user_context_id],
            "networkConditions": {
                "type": "offline"
            }
        }),
    )
    .await;
    assert_eq!(set_user_context_offline["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            18,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            19,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "true",
        "non-default userContext network conditions should not affect default contexts"
    );

    let later_user_context_tab = send_bidi_command(
        &mut socket,
        20,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(later_user_context_tab["type"], json!("success"));
    let later_user_context_tab_id = later_user_context_tab["result"]["context"]
        .as_str()
        .expect("later created userContext tab")
        .to_owned();
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            21,
            &later_user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "later context should inherit userContext network conditions"
    );

    let set_context_offline = send_bidi_command(
        &mut socket,
        22,
        "emulation.setNetworkConditions",
        json!({
            "contexts": [user_context_tab_id],
            "networkConditions": {
                "type": "offline"
            }
        }),
    )
    .await;
    assert_eq!(set_context_offline["type"], json!("success"));
    let reset_user_context = send_bidi_command(
        &mut socket,
        23,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": [user_context_id],
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_user_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            24,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "context override should survive userContext reset"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            25,
            &later_user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "true",
        "userContext reset should restore sibling contexts without context override"
    );

    let reset_context = send_bidi_command(
        &mut socket,
        26,
        "emulation.setNetworkConditions",
        json!({
            "contexts": [user_context_tab_id],
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            27,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_print_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/print/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 10_u64;

    for params in [
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "background": "foo"}),
        json!({"context": context_id, "background": 42}),
        json!({"context": context_id, "margin": false}),
        json!({"context": context_id, "margin": {"top": "foo"}}),
        json!({"context": context_id, "margin": {"bottom": -0.1}}),
        json!({"context": context_id, "orientation": false}),
        json!({"context": context_id, "orientation": "foo"}),
        json!({"context": context_id, "page": "foo"}),
        json!({"context": context_id, "page": {"height": false}}),
        json!({"context": context_id, "page": {"width": 0.03}}),
        json!({"context": context_id, "pageRanges": false}),
        json!({"context": context_id, "pageRanges": [null]}),
        json!({"context": context_id, "pageRanges": ["3-2"]}),
        json!({"context": context_id, "pageRanges": ["1-2-3"]}),
        json!({"context": context_id, "scale": false}),
        json!({"context": context_id, "scale": 0.09}),
        json!({"context": context_id, "scale": 2.01}),
        json!({"context": context_id, "shrinkToFit": "foo"}),
    ] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.print", params.clone()).await;
        assert_eq!(
            response["type"],
            json!("error"),
            "params should fail: {params}"
        );
        assert_eq!(
            response["error"],
            json!("invalid argument"),
            "params should be invalid argument: {params}; response={response:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_capture_screenshot_invalid_parameters_and_unsupported_boundary() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/capture_screenshot/invalid.py.
    // Element clips remain unsupported; page and box captures use the renderer.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 100_u64;

    for params in [
        json!({"context": null}),
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "clip": false}),
        json!({"context": context_id, "clip": {"type": null}}),
        json!({"context": context_id, "clip": {"type": "foo"}}),
        json!({"context": context_id, "clip": {"type": "box", "x": "foo", "y": 0, "width": 1, "height": 1}}),
        json!({"context": context_id, "clip": {"type": "box", "x": 0, "y": false, "width": 1, "height": 1}}),
        json!({"context": context_id, "clip": {"type": "box", "x": 0, "y": 0, "width": [], "height": 1}}),
        json!({"context": context_id, "clip": {"type": "box", "x": 0, "y": 0, "width": 1, "height": {}}}),
        json!({"context": context_id, "origin": 42}),
        json!({"context": context_id, "origin": "page"}),
        json!({"context": context_id, "format": "foo"}),
        json!({"context": context_id, "format": {}}),
        json!({"context": context_id, "format": {"type": null}}),
        json!({"context": context_id, "format": {"type": "image/jpeg", "quality": "foo"}}),
        json!({"context": context_id, "format": {"type": "image/jpeg", "quality": -0.1}}),
        json!({"context": context_id, "format": {"type": "image/jpeg", "quality": 1.1}}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.captureScreenshot",
            params.clone(),
        )
        .await;
        assert_eq!(
            response["type"],
            json!("error"),
            "params should fail: {params}"
        );
        assert_eq!(
            response["error"],
            json!("invalid argument"),
            "params should be invalid argument: {params}; response={response:?}"
        );
    }

    let unknown_element_clip = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "clip": {
                "type": "element",
                "element": {
                    "sharedId": "foo"
                }
            }
        }),
    )
    .await;
    assert_eq!(unknown_element_clip["type"], json!("error"));
    assert_eq!(
        unknown_element_clip["error"],
        json!("unsupported operation"),
        "element clip should fail at the unsupported screenshot boundary: {unknown_element_clip:?}"
    );
    assert_eq!(
        unknown_element_clip["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_set_viewport_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/set_viewport/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 400_u64;

    for params in [
        json!({"context": false, "viewport": {"width": 100, "height": 200}}),
        json!({"context": 42, "viewport": {"width": 100, "height": 200}}),
        json!({"context": {}, "viewport": {"width": 100, "height": 200}}),
        json!({"context": [], "viewport": {"width": 100, "height": 200}}),
        json!({"context": context_id, "viewport": false}),
        json!({"context": context_id, "viewport": 42}),
        json!({"context": context_id, "viewport": ""}),
        json!({"context": context_id, "viewport": {}}),
        json!({"context": context_id, "viewport": []}),
        json!({"context": context_id, "viewport": {"width": 100}}),
        json!({"context": context_id, "viewport": {"height": 100}}),
        json!({"context": context_id, "viewport": {"width": null, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": false, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": "", "height": 100}}),
        json!({"context": context_id, "viewport": {"width": 42.1, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": {}, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": [], "height": 100}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": null}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": false}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": ""}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": 42.1}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": {}}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": []}}),
        json!({"context": context_id, "viewport": {"width": -1, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": -1}}),
        json!({"context": context_id, "viewport": {"width": -1, "height": -1}}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": false}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": ""}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": {}}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": []}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": 0}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": -1}),
        json!({"userContexts": true, "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": "foo", "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": 42, "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": {}, "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [null], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [false], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [42], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [{}], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [[]], "viewport": {"width": 100, "height": 200}}),
        json!({
            "context": context_id,
            "userContexts": ["default"],
            "viewport": {"width": 100, "height": 200}
        }),
        json!({"viewport": {"width": 100, "height": 200}}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.setViewport",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("setViewport params should be invalid argument: {params}"),
        );
    }

    let response = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.setViewport",
        json!({"context": "_invalid_"}),
    )
    .await;
    assert_bidi_error(
        &response,
        "no such frame",
        "setViewport context should be missing",
    );

    for params in [
        json!({"userContexts": [""], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": ["somestring"], "viewport": {"width": 100, "height": 200}}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.setViewport",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "no such user context",
            &format!("setViewport userContexts should be missing: {params}"),
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_emulation_set_user_agent_override_invalid_parameters_match_wpt_error_shape()
{
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_user_agent_override/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 500_u64;

    for params in [
        json!({}),
        json!({"userAgent": false}),
        json!({"userAgent": 42}),
        json!({"userAgent": {}}),
        json!({"userAgent": []}),
        json!({"contexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({
            "contexts": [context_id.clone()],
            "userContexts": ["default"],
            "userAgent": "Moli-UA/1.0"
        }),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "emulation.setUserAgentOverride",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("setUserAgentOverride params should be invalid argument: {params}"),
        );
    }

    let empty_user_agent = send_bidi_command(
        &mut socket,
        id + 1,
        "emulation.setUserAgentOverride",
        json!({
            "userAgent": ""
        }),
    )
    .await;
    assert_bidi_error(
        &empty_user_agent,
        "unsupported operation",
        "setUserAgentOverride empty userAgent should be unsupported",
    );

    let missing_context = send_bidi_command(
        &mut socket,
        id + 2,
        "emulation.setUserAgentOverride",
        json!({
            "contexts": ["_invalid_"],
            "userAgent": "Moli-UA/1.0"
        }),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "setUserAgentOverride context should be missing",
    );

    let missing_user_context = send_bidi_command(
        &mut socket,
        id + 3,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": ["somestring"],
            "userAgent": "Moli-UA/1.0"
        }),
    )
    .await;
    assert_bidi_error(
        &missing_user_context,
        "no such user context",
        "setUserAgentOverride userContext should be missing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_emulation_locale_timezone_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_locale_override/invalid.py and
    // webdriver/tests/bidi/emulation/set_timezone_override/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 600_u64;

    for (method, params) in [
        ("emulation.setLocaleOverride", json!({})),
        ("emulation.setLocaleOverride", json!({"locale": "fr-FR"})),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()]}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": false}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": 42}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": {}}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": []}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [], "locale": "fr-FR"}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"userContexts": [], "locale": "fr-FR"}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({
                "contexts": [context_id.clone()],
                "userContexts": ["default"],
                "locale": "fr-FR"
            }),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": ""}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": "en_US"}),
        ),
        ("emulation.setTimezoneOverride", json!({})),
        (
            "emulation.setTimezoneOverride",
            json!({"timezone": "Asia/Tokyo"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()]}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": false}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": 42}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": {}}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": []}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [], "timezone": "Asia/Tokyo"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"userContexts": [], "timezone": "Asia/Tokyo"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({
                "contexts": [context_id.clone()],
                "userContexts": ["default"],
                "timezone": "Asia/Tokyo"
            }),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": ""}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "Europe/Bielefeld"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "America/Not_A_Zone"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "Z"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "+1:00"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "GMT+05:00"}),
        ),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, method, params.clone()).await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("{method} params should be invalid argument: {params}"),
        );
    }

    for (method, params) in [
        (
            "emulation.setLocaleOverride",
            json!({
                "contexts": ["_invalid_"],
                "locale": "fr-FR"
            }),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({
                "contexts": ["_invalid_"],
                "timezone": "Asia/Tokyo"
            }),
        ),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, method, params).await;
        assert_bidi_error(
            &response,
            "no such frame",
            &format!("{method} context should be missing"),
        );
    }

    for (method, params) in [
        (
            "emulation.setLocaleOverride",
            json!({
                "userContexts": ["somestring"],
                "locale": "fr-FR"
            }),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({
                "userContexts": ["somestring"],
                "timezone": "Asia/Tokyo"
            }),
        ),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, method, params).await;
        assert_bidi_error(
            &response,
            "no such user context",
            &format!("{method} userContext should be missing"),
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_emulation_set_network_conditions_invalid_parameters_match_wpt_error_shape()
{
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_network_conditions/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 650_u64;

    for params in [
        json!({}),
        json!({"contexts": [context_id.clone()]}),
        json!({"contexts": [], "networkConditions": null}),
        json!({"contexts": [false], "networkConditions": null}),
        json!({"contexts": [42], "networkConditions": null}),
        json!({"contexts": [{}], "networkConditions": null}),
        json!({"contexts": [[]], "networkConditions": null}),
        json!({"userContexts": [], "networkConditions": null}),
        json!({"userContexts": [false], "networkConditions": null}),
        json!({"userContexts": [42], "networkConditions": null}),
        json!({"userContexts": [{}], "networkConditions": null}),
        json!({"userContexts": [[]], "networkConditions": null}),
        json!({
            "contexts": [context_id.clone()],
            "userContexts": ["default"],
            "networkConditions": null
        }),
        json!({"contexts": [context_id.clone()], "networkConditions": false}),
        json!({"contexts": [context_id.clone()], "networkConditions": 42}),
        json!({"contexts": [context_id.clone()], "networkConditions": "offline"}),
        json!({"contexts": [context_id.clone()], "networkConditions": []}),
        json!({"contexts": [context_id.clone()], "networkConditions": {}}),
        json!({
            "contexts": [context_id.clone()],
            "networkConditions": {
                "type": "SOME_INVALID_TYPE"
            }
        }),
        json!({
            "contexts": [context_id.clone()],
            "networkConditions": {
                "type": false
            }
        }),
        json!({
            "contexts": [context_id.clone()],
            "networkConditions": {
                "type": "offline",
                "extra": true
            }
        }),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "emulation.setNetworkConditions",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("setNetworkConditions params should be invalid argument: {params}"),
        );
    }

    let missing_context = send_bidi_command(
        &mut socket,
        id + 1,
        "emulation.setNetworkConditions",
        json!({
            "contexts": ["_invalid_"],
            "networkConditions": null
        }),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "setNetworkConditions context should be missing",
    );

    let missing_user_context = send_bidi_command(
        &mut socket,
        id + 2,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": ["somestring"],
            "networkConditions": null
        }),
    )
    .await;
    assert_bidi_error(
        &missing_user_context,
        "no such user context",
        "setNetworkConditions userContext should be missing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
