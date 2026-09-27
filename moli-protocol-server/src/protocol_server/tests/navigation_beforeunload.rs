use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const BEFORE: [&str; 3] = [
    "root:beforeunload",
    "child:beforeunload",
    "grand:beforeunload",
];
const SETUP: &str = r#"(() => {
    const child = document.querySelector('iframe');
    const grand = child.contentDocument.querySelector('iframe');
    const root = document.documentElement;
    const trace = [];
    for (const [win, name] of [[window, 'root'], [child.contentWindow, 'child'],
            [grand.contentWindow, 'grand']]) {
        for (const type of ['beforeunload', 'pagehide', 'unload']) {
            win.addEventListener(type, event => {
                trace.push(name + ':' + type);
                sessionStorage.setItem('browserBeforeUnloadTrace', JSON.stringify(trace));
                reportBeforeUnload(JSON.stringify({phase:'event', event:trace.at(-1),
                    trusted:event.isTrusted, cancelable:event.cancelable}));
            });
        }
    }
    globalThis.beforeUnloadSnapshot = () => ({phase:'pending', trace:trace.slice(),
        url:location.href, documentURL:document.URL, hidden:document.hidden,
        sameRoot:document.documentElement === root, child:child.isConnected,
        grand:grand.isConnected});
    fetch('/navigation-received').then(() => reportBeforeUnload(JSON.stringify(beforeUnloadSnapshot())));
    return 'armed';
})()"#;

struct NavigationGate {
    armed: AtomicBool,
    requests: AtomicUsize,
    received: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

fn fixture(
    response: &'static str,
) -> (
    std::net::SocketAddr,
    DedicatedFixtureServer,
    Arc<NavigationGate>,
) {
    let gate = Arc::new(NavigationGate {
        armed: AtomicBool::new(false),
        requests: AtomicUsize::new(0),
        received: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let serve = gate.clone();
    let app = Router::new().fallback(get(move |uri: axum::http::Uri| {
        let gate = serve.clone();
        async move {
            if uri.path() == "/navigation-received" {
                gate.received.notified().await;
                return "ready".into_response();
            }
            if matches!(uri.path(), "/source" | "/destination" | "/previous")
                && gate.armed.swap(false, Ordering::SeqCst)
            {
                gate.requests.fetch_add(1, Ordering::SeqCst);
                gate.received.notify_one();
                gate.release.acquire().await.unwrap().forget();
                match response {
                    "204" => return StatusCode::NO_CONTENT.into_response(),
                    "205" => return StatusCode::RESET_CONTENT.into_response(),
                    "attachment" => return ([(header::CONTENT_DISPOSITION, "attachment; filename=probe.txt")], "download").into_response(),
                    _ => {}
                }
            }
            let body = match uri.path() {
                "/child" => "<!doctype html><body><iframe src=/grandchild></iframe>",
                "/grandchild" => "<!doctype html><body>grandchild",
                _ => "<!doctype html><script>globalThis.beforeUnloadAtFirstScript = JSON.parse(sessionStorage.getItem('browserBeforeUnloadTrace') || '[]');</script><body><iframe src=/child></iframe>",
            };
            ([(header::CONTENT_TYPE, "text/html"), (header::CACHE_CONTROL, "no-store")], body).into_response()
        }
    }));
    let (address, server) = spawn_dedicated_fixture_server(app, "navigation-beforeunload");
    (address, server, gate)
}

#[tokio::test]
async fn websocket_navigation_beforeunload_precedes_fetch_and_only_commit_unloads() {
    for method in ["navigate", "reload", "history"] {
        for response in ["html", "204", "205", "attachment"] {
            run_beforeunload_navigation(method, response, "normal").await;
        }
    }
}

#[tokio::test]
async fn websocket_navigation_beforeunload_debugger_pause_delays_fetch() {
    run_beforeunload_navigation("navigate", "html", "debugger").await;
}

#[tokio::test]
async fn websocket_navigation_beforeunload_superseded_check_does_not_start_a_request() {
    run_beforeunload_navigation("navigate", "html", "supersede").await;
}

#[tokio::test]
async fn websocket_navigation_beforeunload_precedes_fetch_interception() {
    run_beforeunload_navigation("navigate", "html", "fetch").await;
}

async fn run_beforeunload_navigation(method: &str, response: &'static str, mode: &str) {
    let pause = matches!(mode, "debugger" | "supersede");
    let (address, _fixture, gate) = fixture(response);
    let download_root = std::env::temp_dir().join(format!(
        "moli-beforeunload-download-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (cdp_address, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_address}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    let session_id = Some(session.session_id.as_str());
    for (id, domain) in [(4, "Page"), (5, "Runtime"), (6, "Network")] {
        send_cdp_command(
            &mut socket,
            id,
            &format!("{domain}.enable"),
            session_id,
            json!({}),
        )
        .await;
    }
    send_cdp_command(
        &mut socket,
        7,
        "Runtime.addBinding",
        session_id,
        json!({"name":"reportBeforeUnload"}),
    )
    .await;
    let source = format!("http://{address}/source");
    let previous = format!("http://{address}/previous");
    if method == "history" {
        cdp_navigate_and_wait_for_load(&mut socket, 8, &session.session_id, &previous).await;
    }
    cdp_navigate_and_wait_for_load(&mut socket, 9, &session.session_id, &source).await;
    assert_eq!(
        cdp_runtime_evaluate_string(&mut socket, &session.session_id, 10, SETUP).await,
        "armed"
    );
    if response == "attachment" {
        send_cdp_command(
            &mut socket,
            11,
            "Browser.setDownloadBehavior",
            None,
            json!({"behavior":"allow","downloadPath":download_root,"browserContextId":context}),
        )
        .await;
    }
    if pause {
        send_cdp_command(&mut socket, 12, "Debugger.enable", session_id, json!({})).await;
        send_cdp_command(&mut socket, 13, "Runtime.evaluate", session_id,
            json!({"expression":"addEventListener('beforeunload', () => { debugger; }, {once:true})"})).await;
    }
    if mode == "fetch" {
        send_cdp_command(
            &mut socket,
            15,
            "Fetch.enable",
            session_id,
            json!({"patterns":[{"urlPattern":"*/destination","requestStage":"Request"}]}),
        )
        .await;
    }
    let (command, params) = match method {
        "reload" => ("Page.reload", json!({})),
        "history" => {
            let messages = send_cdp_command(
                &mut socket,
                14,
                "Page.getNavigationHistory",
                session_id,
                json!({}),
            )
            .await;
            let history = &messages.iter().find(|message| message["id"] == 14).unwrap()["result"];
            let entry = history["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["url"] == previous)
                .unwrap();
            (
                "Page.navigateToHistoryEntry",
                json!({"entryId":entry["id"]}),
            )
        }
        _ => (
            "Page.navigate",
            json!({"url":format!("http://{address}/destination")}),
        ),
    };
    gate.armed.store(true, Ordering::SeqCst);
    send_cdp_command_without_wait(&mut socket, 20, command, session_id, params).await;
    let mut messages = Vec::new();
    if pause {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Debugger.paused"
            })
            .await,
        );
        assert_eq!(
            gate.requests.load(Ordering::SeqCst),
            0,
            "fetch must wait for beforeunload"
        );
        if mode == "supersede" {
            send_cdp_command_without_wait(
                &mut socket,
                21,
                "Page.navigate",
                session_id,
                json!({"url":"data:text/html,<title>replacement</title>"}),
            )
            .await;
        }
        messages.extend(
            send_cdp_command(&mut socket, 22, "Debugger.resume", session_id, json!({})).await,
        );
    }
    if mode == "supersede" {
        if !messages.iter().any(|message| message["id"] == 20) {
            messages.extend(recv_until_id(&mut socket, 20).await);
        }
        let result = messages.iter().find(|message| message["id"] == 20).unwrap();
        assert_eq!(
            result["result"]["errorText"], "net::ERR_ABORTED",
            "{messages:?}"
        );
        assert_eq!(
            gate.requests.load(Ordering::SeqCst),
            0,
            "a superseded check must not fetch"
        );
        if !messages
            .iter()
            .any(|message| message["method"] == "Page.loadEventFired")
        {
            messages.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Page.loadEventFired"
                })
                .await,
            );
        }
        assert_eq!(
            cdp_runtime_evaluate_string(&mut socket, &session.session_id, 23, "document.title")
                .await,
            "replacement"
        );
    } else {
        if mode == "fetch" {
            messages.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.requestPaused"
                })
                .await,
            );
            assert_eq!(gate.requests.load(Ordering::SeqCst), 0);
            let paused = messages.last().unwrap();
            let fetch_request_id = paused["params"]["requestId"].clone();
            for event in BEFORE {
                assert!(
                    messages[..messages.len() - 1]
                        .iter()
                        .any(|message| message["method"] == "Runtime.bindingCalled"
                            && message["params"]["payload"]
                                .as_str()
                                .is_some_and(|payload| payload.contains(event))),
                    "{event} must precede Fetch.requestPaused: {messages:?}"
                );
            }
            // Fetch continuation currently owns the foreground load. Release
            // the fixture before continuing; the pause above already proves
            // the lifecycle check precedes interception and any HTTP request.
            gate.release.add_permits(1);
            send_cdp_command_without_wait(
                &mut socket,
                26,
                "Fetch.continueRequest",
                session_id,
                json!({"requestId":fetch_request_id}),
            )
            .await;
        }
        if mode != "fetch" {
            messages.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Runtime.bindingCalled"
                        && message["params"]["payload"]
                            .as_str()
                            .is_some_and(|value| value.contains("\"phase\":\"pending\""))
                })
                .await,
            );
        }
        let pending = messages.iter().find_map(|message| {
            if message["method"] != "Runtime.bindingCalled" {
                return None;
            }
            let value: serde_json::Value =
                serde_json::from_str(message["params"]["payload"].as_str()?).ok()?;
            (value["phase"] == "pending").then_some(value)
        });
        if mode != "fetch" {
            assert_eq!(
                pending,
                Some(
                    json!({"phase":"pending","trace":BEFORE,"url":source,"documentURL":source,
                "hidden":false,"sameRoot":true,"child":true,"grand":true})
                ),
                "{method}/{response}"
            );
        }
        let request = messages
            .iter()
            .position(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["type"] == "Document"
                    && message["params"]["frameId"] == session.target_id
            })
            .expect("main-document network request");
        for event in BEFORE {
            let index = messages
                .iter()
                .position(|message| {
                    message["method"] == "Runtime.bindingCalled"
                        && message["params"]["payload"]
                            .as_str()
                            .is_some_and(|value| value.contains(event))
                })
                .expect(event);
            assert!(
                index < request,
                "beforeunload output must precede the request: {messages:?}"
            );
        }
        gate.release.add_permits(1);
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.frameStoppedLoading"
                    && message["params"]["frameId"] == session.target_id
            })
            .await,
        );
        if response == "html" {
            let trace = cdp_runtime_evaluate_string(
                &mut socket,
                &session.session_id,
                24,
                "JSON.stringify(beforeUnloadAtFirstScript)",
            )
            .await;
            assert_eq!(
                trace,
                serde_json::to_string(&[
                    "root:beforeunload",
                    "child:beforeunload",
                    "grand:beforeunload",
                    "root:pagehide",
                    "root:unload",
                    "child:pagehide",
                    "child:unload",
                    "grand:pagehide",
                    "grand:unload",
                ])
                .unwrap(),
                "{method}/{response}"
            );
        } else {
            let retained = cdp_runtime_evaluate_string(
                &mut socket,
                &session.session_id,
                25,
                "JSON.stringify(beforeUnloadSnapshot())",
            )
            .await;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&retained).unwrap(),
                pending.unwrap(),
                "{method}/{response}"
            );
            assert!(
                messages
                    .iter()
                    .all(|message| message["method"] != "Page.frameNavigated"
                        || message["params"]["frame"]["id"] != session.target_id)
            );
        }
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
    let _ = fs::remove_dir_all(download_root);
}
