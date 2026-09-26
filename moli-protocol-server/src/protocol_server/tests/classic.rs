use super::*;
use axum::http::{HeaderMap, header};
use serde_json::{Map, Value};

async fn classic_capture_layout(app: Router, session_id: &str) {
    let captured = classic_request_json(
        app,
        Method::GET,
        &format!("/session/{session_id}/screenshot"),
    )
    .await;
    assert!(
        captured["value"].is_string(),
        "fixture screenshot: {captured}"
    );
}

#[derive(Clone, Copy, Debug)]
enum WindowPromptCommand {
    GetRect,
    SetRect,
    Maximize,
    Minimize,
    Fullscreen,
}

impl WindowPromptCommand {
    fn label(self) -> &'static str {
        match self {
            Self::GetRect => "get window rect",
            Self::SetRect => "set window rect",
            Self::Maximize => "maximize window",
            Self::Minimize => "minimize window",
            Self::Fullscreen => "fullscreen window",
        }
    }

    fn expected_success_value(self) -> serde_json::Value {
        match self {
            Self::GetRect | Self::Minimize => {
                json!({
                    "x": 7,
                    "y": 9,
                    "width": 640,
                    "height": 480,
                })
            }
            Self::SetRect => {
                json!({
                    "x": 11,
                    "y": 13,
                    "width": 650,
                    "height": 490,
                })
            }
            Self::Maximize => {
                json!({
                    "x": 0,
                    "y": 0,
                    "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                    "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
                })
            }
            Self::Fullscreen => {
                json!({
                    "x": 0,
                    "y": 0,
                    "width": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                    "height": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
                })
            }
        }
    }

    fn expected_success_surface(self) -> serde_json::Value {
        match self {
            Self::Minimize => json!({
                "innerWidth": 640,
                "innerHeight": 480,
                "hidden": true,
                "visibilityState": "hidden",
                "hasFullScreen": false,
                "hasWebkitIsFullScreen": false,
            }),
            Self::Fullscreen => json!({
                "innerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                "innerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_HEIGHT,
                "hidden": false,
                "visibilityState": "visible",
                "hasFullScreen": false,
                "hasWebkitIsFullScreen": false,
            }),
            Self::Maximize => json!({
                "innerWidth": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_SCREEN_WIDTH,
                "innerHeight": moli_protocol_webdriver_classic::CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
                "hidden": false,
                "visibilityState": "visible",
                "hasFullScreen": false,
                "hasWebkitIsFullScreen": false,
            }),
            Self::GetRect | Self::SetRect => json!({
                "innerWidth": self.expected_success_value()["width"],
                "innerHeight": self.expected_success_value()["height"],
                "hidden": false,
                "visibilityState": "visible",
                "hasFullScreen": false,
                "hasWebkitIsFullScreen": false,
            }),
        }
    }
}

async fn assert_window_prompt_command_matches_chromium_wpt(command: WindowPromptCommand) {
    let app = build_router(test_state());

    struct WindowPromptCase {
        capability: Option<serde_json::Value>,
        dialog_script: &'static str,
        expect_notify: bool,
        expect_closed: bool,
    }

    let prompt_cases = [
        WindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("accept")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("accept and notify")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: false,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("dismiss and notify")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
        WindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { alert('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        WindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { confirm('cheese'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        WindowPromptCase {
            capability: Some(json!("ignore")),
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: false,
        },
        WindowPromptCase {
            capability: None,
            dialog_script: "setTimeout(() => { prompt('cheese', 'default'); }, 0); return 'opened';",
            expect_notify: true,
            expect_closed: true,
        },
    ];

    async fn window_surface(app: Router, session_id: &str) -> serde_json::Value {
        let response = classic_request_json_with_body(
            app,
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": "return JSON.stringify({ innerWidth, innerHeight, hidden: document.hidden, visibilityState: document.visibilityState, hasFullScreen: 'fullScreen' in window, hasWebkitIsFullScreen: 'webkitIsFullScreen' in document });",
                "args": []
            }),
        )
        .await;
        serde_json::from_str(
            response["value"]
                .as_str()
                .expect("window surface should be JSON string"),
        )
        .expect("window surface JSON")
    }

    async fn run_window_prompt_command(
        app: Router,
        session_id: &str,
        command: WindowPromptCommand,
    ) -> (StatusCode, serde_json::Value) {
        match command {
            WindowPromptCommand::GetRect => {
                classic_request_status_and_json(
                    app,
                    Method::GET,
                    &format!("/session/{session_id}/window/rect"),
                )
                .await
            }
            WindowPromptCommand::SetRect => {
                classic_request_status_and_json_with_body(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/rect"),
                    json!({
                        "x": 11,
                        "y": 13,
                        "width": 650,
                        "height": 490,
                    }),
                )
                .await
            }
            WindowPromptCommand::Maximize => {
                classic_request_status_and_json(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/maximize"),
                )
                .await
            }
            WindowPromptCommand::Minimize => {
                classic_request_status_and_json(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/minimize"),
                )
                .await
            }
            WindowPromptCommand::Fullscreen => {
                classic_request_status_and_json(
                    app,
                    Method::POST,
                    &format!("/session/{session_id}/window/fullscreen"),
                )
                .await
            }
        }
    }

    for case in &prompt_cases {
        let session_body = match &case.capability {
            Some(capability) => json!({
                "capabilities": {
                    "alwaysMatch": {
                        "unhandledPromptBehavior": capability
                    }
                }
            }),
            None => json!({
                "capabilities": {
                    "alwaysMatch": {}
                }
            }),
        };
        let session =
            classic_request_json_with_body(app.clone(), Method::POST, "/session", session_body)
                .await;
        let session_id = session["value"]["sessionId"]
            .as_str()
            .expect("classic session id");
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/url"),
                json!({ "url": classic_data_url("<!doctype html><title>window prompt</title>") }),
            )
            .await,
            json!({ "value": null })
        );
        assert_eq!(
            classic_request_json_with_body(
                app.clone(),
                Method::POST,
                &format!("/session/{session_id}/window/rect"),
                json!({
                    "x": 7,
                    "y": 9,
                    "width": 640,
                    "height": 480,
                }),
            )
            .await,
            json!({ "value": {
                "x": 7,
                "y": 9,
                "width": 640,
                "height": 480,
            }})
        );
        let original_rect = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/window/rect"),
        )
        .await;
        let original_surface = window_surface(app.clone(), session_id).await;

        classic_open_dialog_and_wait(app.clone(), session_id, case.dialog_script, "cheese").await;

        let (status, response) = run_window_prompt_command(app.clone(), session_id, command).await;
        if case.expect_notify {
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "{} capability {:?} response {response:?}",
                command.label(),
                case.capability
            );
            assert_eq!(response["value"]["error"], json!("unexpected alert open"));
            assert_eq!(response["value"]["data"], json!({ "text": "cheese" }));
        } else {
            assert_eq!(
                status,
                StatusCode::OK,
                "{} capability {:?} response {response:?}",
                command.label(),
                case.capability
            );
            assert_eq!(response["value"], command.expected_success_value());
        }

        let alert_text_path = format!("/session/{session_id}/alert/text");
        let (alert_status, alert_text) =
            classic_request_status_and_json(app.clone(), Method::GET, &alert_text_path).await;
        if case.expect_closed {
            assert_eq!(alert_status, StatusCode::NOT_FOUND, "{alert_text:?}");
            assert_eq!(alert_text["value"]["error"], json!("no such alert"));
        } else {
            assert_eq!(alert_status, StatusCode::OK, "{alert_text:?}");
            assert_eq!(alert_text, json!({ "value": "cheese" }));
            assert_eq!(
                classic_request_json(
                    app.clone(),
                    Method::POST,
                    &format!("/session/{session_id}/alert/dismiss"),
                )
                .await,
                json!({ "value": null })
            );
        }

        let final_rect = classic_request_json(
            app.clone(),
            Method::GET,
            &format!("/session/{session_id}/window/rect"),
        )
        .await;
        let final_surface = window_surface(app.clone(), session_id).await;
        if case.expect_notify {
            assert_eq!(
                final_rect,
                original_rect,
                "{} should not run after notify preflight",
                command.label()
            );
            assert_eq!(
                final_surface,
                original_surface,
                "{} should preserve surface after notify preflight",
                command.label()
            );
        } else {
            assert_eq!(final_rect["value"], command.expected_success_value());
            assert_eq!(final_surface, command.expected_success_surface());
        }

        let _ = classic_request_json(
            app.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

fn classic_data_url(html: &str) -> String {
    fn push_hex(encoded: &mut String, byte: u8) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        encoded.push('%');
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }

    let mut encoded = String::with_capacity(html.len());
    for byte in html.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => push_hex(&mut encoded, byte),
        }
    }
    format!("data:text/html;charset=utf-8,{encoded}")
}

async fn classic_open_dialog_and_wait(
    app: Router,
    session_id: &str,
    script: &str,
    expected_text: &str,
) {
    classic_open_dialog_and_wait_with_timeout(
        app,
        session_id,
        script,
        expected_text,
        std::time::Duration::from_secs(1),
    )
    .await;
}

async fn classic_open_dialog_and_wait_with_timeout(
    app: Router,
    session_id: &str,
    script: &str,
    expected_text: &str,
    timeout: std::time::Duration,
) {
    assert_eq!(
        classic_request_json_with_body(
            app.clone(),
            Method::POST,
            &format!("/session/{session_id}/execute/sync"),
            json!({
                "script": script,
                "args": []
            }),
        )
        .await,
        json!({ "value": "opened" })
    );
    let alert_path = format!("/session/{session_id}/alert/text");
    let alert = tokio::time::timeout(timeout, async {
        loop {
            let (status, response) =
                classic_request_status_and_json(app.clone(), Method::GET, &alert_path).await;
            if status == StatusCode::OK {
                break response;
            }
            assert_eq!(status, StatusCode::NOT_FOUND, "{response:?}");
            assert_eq!(response["value"]["error"], json!("no such alert"));
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("scheduled JavaScript dialog should open: {script}"));
    assert_eq!(alert, json!({ "value": expected_text }));
}

async fn classic_request_json(app: Router, method: Method, path: &str) -> serde_json::Value {
    let (status, value) = classic_request_status_and_json(app, method, path).await;
    assert_eq!(status, StatusCode::OK, "path {path}: {value:?}");
    value
}

async fn spawn_classic_delayed_navigation_fixture_server(
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic delayed navigation fixture server");
    let addr = listener
        .local_addr()
        .expect("classic delayed navigation fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new().route(
            "/slow",
            get(move || async move {
                sleep(delay).await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>slow navigation</body></html>",
                )
            }),
        );
        axum::serve(listener, app)
            .await
            .expect("classic delayed navigation fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_form_navigation_fixture_server(
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic form navigation fixture server");
    let addr = listener
        .local_addr()
        .expect("classic form navigation fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/form",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>Form Source</title></head>\
                         <body><form action='/submitted'><input name='login' value='moli'></form></body></html>",
                    )
                }),
            )
            .route(
                "/submitted",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>Submitted Target</title></head>\
                         <body><main>submitted</main></body></html>",
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("classic form navigation fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_frame_fixture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic frame fixture server");
    let addr = listener.local_addr().expect("classic frame fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body data-context='top'><main id='top-main'>top</main><iframe id='child' src='/frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body data-context='child'><main id='inside-frame'>child</main><button id='remove-current-frame' onclick=\"parent.document.getElementById('child').remove()\">remove</button></body></html>",
                    )
                }),
            )
            .route(
                "/nested",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>top nested</title></head><body data-context='top'><main id='top-nested'>top</main><iframe id='outerById' name='outerByName' src='/outer-frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/outer-frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>outer frame</title></head><body data-context='outer'><main id='outer-main'>outer</main><iframe id='innerById' name='innerByName' src='/inner-frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/inner-frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><title>inner frame</title></head><body data-context='inner'><p id='inner-text'>inner</p></body></html>",
                    )
                }),
            )
            .route(
                "/shadow-page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id='shadow-top'>top</main><iframe id='shadow-child' src='/shadow-frame'></iframe></body></html>",
                    )
                }),
            )
            .route(
                "/shadow-frame",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><div id='child-closed-host'></div><script>const root=document.getElementById('child-closed-host').attachShadow({mode:'closed'});root.innerHTML='<span id=\"child-closed-inside\">child closed text</span>';</script></body></html>",
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("classic frame fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_cross_origin_frame_fixture_servers() -> (
    std::net::SocketAddr,
    std::net::SocketAddr,
    std::net::SocketAddr,
    Vec<tokio::task::JoinHandle<()>>,
) {
    let browser_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic browser-origin frame fixture server");
    let alt_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic alt-origin frame fixture server");
    let www_alt_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic www-alt-origin frame fixture server");

    let browser_addr = browser_listener
        .local_addr()
        .expect("classic browser-origin frame fixture addr");
    let alt_addr = alt_listener
        .local_addr()
        .expect("classic alt-origin frame fixture addr");
    let www_alt_addr = www_alt_listener
        .local_addr()
        .expect("classic www-alt-origin frame fixture addr");

    let alt_child_url = format!("http://{alt_addr}/child");
    let browser_middle_url = format!("http://{browser_addr}/middle");
    let www_alt_leaf_url = format!("http://{www_alt_addr}/leaf");

    let browser_app = Router::new()
        .route(
            "/top",
            get({
                let alt_child_url = alt_child_url.clone();
                move || {
                    let alt_child_url = alt_child_url.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            format!(
                                "<!doctype html><html><body><iframe id='cross' src='{alt_child_url}'></iframe></body></html>"
                            ),
                        )
                    }
                }
            }),
        )
        .route(
            "/middle",
            get({
                let www_alt_leaf_url = www_alt_leaf_url.clone();
                move || {
                    let www_alt_leaf_url = www_alt_leaf_url.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            format!(
                                "<!doctype html><html><body data-context='browser-child'><iframe id='to-www-alt' src='{www_alt_leaf_url}'></iframe></body></html>"
                            ),
                        )
                    }
                }
            }),
        );
    let alt_app = Router::new()
        .route(
            "/child",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body data-context='alt-child'>alt child</body></html>",
                )
            }),
        )
        .route(
            "/nested-top",
            get({
                let browser_middle_url = browser_middle_url.clone();
                move || {
                    let browser_middle_url = browser_middle_url.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            format!(
                                "<!doctype html><html><body data-context='alt-top'><iframe id='to-browser' src='{browser_middle_url}'></iframe></body></html>"
                            ),
                        )
                    }
                }
            }),
        );
    let www_alt_app = Router::new().route(
        "/leaf",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body data-context='www-alt-leaf'>www alt leaf</body></html>",
            )
        }),
    );

    let browser_server = tokio::spawn(async move {
        axum::serve(browser_listener, browser_app)
            .await
            .expect("classic browser-origin frame fixture server should serve");
    });
    let alt_server = tokio::spawn(async move {
        axum::serve(alt_listener, alt_app)
            .await
            .expect("classic alt-origin frame fixture server should serve");
    });
    let www_alt_server = tokio::spawn(async move {
        axum::serve(www_alt_listener, www_alt_app)
            .await
            .expect("classic www-alt-origin frame fixture server should serve");
    });

    (
        browser_addr,
        alt_addr,
        www_alt_addr,
        vec![browser_server, alt_server, www_alt_server],
    )
}

async fn spawn_classic_page_load_strategy_fixture_server(
    delay: Duration,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic page load strategy fixture server");
    let addr = listener
        .local_addr()
        .expect("classic page load strategy fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><head><script>window.__classicLifecycle=[];document.addEventListener('DOMContentLoaded',()=>{window.__classicLifecycle.push('dcl:'+document.readyState);const script=document.createElement('script');script.src='/runtime-script.js';document.head.appendChild(script);});window.addEventListener('load',()=>window.__classicLifecycle.push('load:'+document.readyState));</script></head><body><main>strategy</main></body></html>",
                    )
                }),
            )
            .route(
                "/runtime-script.js",
                get(move || async move {
                    sleep(delay).await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "window.__classicLifecycle.push('external:'+document.readyState);",
                    )
                }),
            );
        axum::serve(listener, app)
            .await
            .expect("classic page load strategy fixture server should serve");
    });
    (addr, server)
}

async fn spawn_classic_cookie_fixture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic cookie fixture server");
    let addr = listener.local_addr().expect("classic cookie fixture addr");
    let server = tokio::spawn(async move {
        let app = Router::new().route(
            "/page",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>classic-cookie</body></html>",
                )
            }),
        );
        axum::serve(listener, app)
            .await
            .expect("classic cookie fixture server should serve");
    });
    (addr, server)
}

fn spawn_classic_service_worker_fixture_server() -> (std::net::SocketAddr, DedicatedFixtureServer) {
    let app = Router::new()
        .route(
            "/",
            get(|| async move {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><body>classic service worker</body></html>",
                )
            }),
        )
        .route(
            "/service-worker.js",
            get(|| async move {
                (
                    [(header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "console.log('classic-service-worker-log');\
                     self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));\
                     self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));",
                )
            }),
        );
    spawn_dedicated_fixture_server(app, "classic-service-worker")
}

async fn classic_switch_to_child_frame_and_remove_current_frame(
    app: Router,
    session_id: &str,
    page_url: &str,
) {
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let frame_element_id = classic_find_css_element_id(app.clone(), session_id, "#child").await;
    let switched = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: frame_element_id,
            }
        }),
    )
    .await;
    assert_eq!(switched, json!({ "value": null }));

    let removed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const frame = window.frameElement; if (frame) frame.remove(); return frame ? 'removed' : 'missing';",
            "args": []
        }),
    )
    .await;
    assert_eq!(removed, json!({ "value": "removed" }));
}

async fn classic_switch_to_nested_frame_and_remove_parent_frame(
    app: Router,
    session_id: &str,
    page_url: &str,
) {
    let navigated = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let outer_frame_id = classic_find_css_element_id(app.clone(), session_id, "#outerById").await;
    let switched_outer = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: outer_frame_id,
            }
        }),
    )
    .await;
    assert_eq!(switched_outer, json!({ "value": null }));

    let inner_frame_id = classic_find_css_element_id(app.clone(), session_id, "#innerById").await;
    let switched_inner = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/frame"),
        json!({
            "id": {
                CLASSIC_ELEMENT_REFERENCE_KEY: inner_frame_id,
            }
        }),
    )
    .await;
    assert_eq!(switched_inner, json!({ "value": null }));

    let removed = classic_request_json_with_body(
        app.clone(),
        Method::POST,
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "const frame = window.parent.frameElement; if (frame) frame.remove(); return frame ? 'removed' : 'missing';",
            "args": []
        }),
    )
    .await;
    assert_eq!(removed, json!({ "value": "removed" }));
}

async fn classic_find_css_element_id(app: Router, session_id: &str, selector: &str) -> String {
    classic_request_json_with_body(
        app,
        Method::POST,
        &format!("/session/{session_id}/element"),
        json!({
            "using": "css selector",
            "value": selector
        }),
    )
    .await["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap_or_else(|| panic!("{selector} element reference id"))
        .to_owned()
}

async fn classic_assert_web_element_array_eq(
    app: Router,
    session_id: &str,
    label: &str,
    response: &serde_json::Value,
    expected_element_ids: &[String],
) {
    let actual = response["value"]
        .as_array()
        .unwrap_or_else(|| panic!("{label}: expected WebElement array response: {response:?}"));
    assert_eq!(
        actual.len(),
        expected_element_ids.len(),
        "{label}: unexpected WebElement array length for response {response:?}"
    );
    for (index, (actual, expected_element_id)) in
        actual.iter().zip(expected_element_ids.iter()).enumerate()
    {
        let actual_element_id = actual[CLASSIC_ELEMENT_REFERENCE_KEY]
            .as_str()
            .unwrap_or_else(|| panic!("expected WebElement reference at {index}: {response:?}"));
        let same = classic_request_json(
            app.clone(),
            Method::GET,
            &format!(
                "/session/{session_id}/element/{actual_element_id}/equals/{expected_element_id}"
            ),
        )
        .await;
        assert_eq!(
            same,
            json!({ "value": true }),
            "{label}: returned WebElement at {index} should match expected element"
        );
    }
}

fn classic_temp_file_basename(file: &TempPath) -> String {
    file.path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| {
            panic!(
                "temporary file should have a UTF-8 basename: {:?}",
                file.path
            )
        })
        .to_owned()
}

fn classic_assert_serialized_file_list_names(
    label: &str,
    response: &serde_json::Value,
    expected_names: &[String],
) {
    let actual = response["value"]
        .as_array()
        .unwrap_or_else(|| panic!("{label}: expected FileList array response: {response:?}"));
    assert_eq!(
        actual.len(),
        expected_names.len(),
        "{label}: unexpected FileList length for response {response:?}"
    );
    for (index, (actual, expected_name)) in actual.iter().zip(expected_names.iter()).enumerate() {
        assert!(
            actual.as_object().is_some(),
            "{label}: expected serialized File object at {index}: {response:?}"
        );
        assert!(
            actual["name"].as_str().is_some(),
            "{label}: expected serialized File name string at {index}: {response:?}"
        );
        assert_eq!(
            actual["name"],
            json!(expected_name),
            "{label}: unexpected serialized File name at {index}"
        );
    }
}

async fn classic_assert_no_such_window(app: Router, method: Method, path: &str) {
    let (status, response) = classic_request_status_and_json(app, method, path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "path {path}: {response:?}");
    assert_eq!(
        response["value"]["error"],
        json!("no such window"),
        "path {path}: {response:?}"
    );
}

async fn classic_assert_no_such_window_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) {
    let (status, response) =
        classic_request_status_and_json_with_body(app, method, path, body).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "path {path}: {response:?}");
    assert_eq!(
        response["value"]["error"],
        json!("no such window"),
        "path {path}: {response:?}"
    );
}

async fn classic_request_json_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let (status, value) = classic_request_status_and_json_with_body(app, method, path, body).await;
    assert_eq!(status, StatusCode::OK, "path {path}: {value:?}");
    value
}

async fn classic_request_status_and_json(
    app: Router,
    method: Method,
    path: &str,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&body).expect("json response"),
    )
}

async fn classic_request_status_headers_and_json(
    app: Router,
    method: Method,
    path: &str,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        headers,
        serde_json::from_slice(&body).expect("json response"),
    )
}

async fn classic_request_status_headers_and_text(
    app: Router,
    method: Method,
    path: &str,
) -> (StatusCode, HeaderMap, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        headers,
        String::from_utf8(body.to_vec()).expect("text response"),
    )
}

async fn classic_request_status_and_json_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&body).expect("json response"),
    )
}

async fn classic_request_status_headers_and_json_with_body(
    app: Router,
    method: Method,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        headers,
        serde_json::from_slice(&body).expect("json response"),
    )
}

fn assert_classic_webdriver_json_headers(headers: &HeaderMap) {
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-cache");
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8"
    );
}

fn assert_classic_json_content_type_absent(headers: &HeaderMap) {
    let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
        return;
    };
    let content_type = content_type.to_str().expect("content-type should be ascii");
    assert!(
        !content_type
            .split_once(';')
            .map_or(content_type, |(media_type, _)| media_type)
            .trim()
            .eq_ignore_ascii_case("application/json"),
        "non-Classic router errors must not be relabeled as JSON: {content_type}"
    );
}
mod extracted;
