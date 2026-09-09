use super::*;
use std::sync::Arc;

use crate::{
    limits::{
        MAX_PENDING_WEBSOCKET_HANDSHAKES, MAX_WEBSOCKET_CONNECTIONS_PER_RUNTIME,
        acquire_limited_websocket_slot,
    },
    proxy::{append_proxy_connect_header, no_proxy_matches},
    request::build_websocket_request,
    test_support::*,
};
use tokio::{
    sync::{Semaphore, mpsc},
    time::{Duration, timeout},
};
use url::Url;

mod tls;

#[test]
fn websocket_slot_limit_blocks_until_permit_is_dropped() {
    let slots = Arc::new(Semaphore::new(1));
    let first = acquire_limited_websocket_slot(&slots);
    assert!(first.is_some());
    assert!(acquire_limited_websocket_slot(&slots).is_none());

    drop(first);

    assert!(acquire_limited_websocket_slot(&slots).is_some());
}

#[test]
fn websocket_runtime_limits_follow_chromium_process_caps() {
    assert_eq!(MAX_WEBSOCKET_CONNECTIONS_PER_RUNTIME, 255);
    assert_eq!(MAX_PENDING_WEBSOCKET_HANDSHAKES, 255);
}

#[test]
fn websocket_cookie_url_maps_socket_schemes_to_http_cookie_schemes() {
    assert_eq!(
        websocket_cookie_url(&Url::parse("ws://example.com/socket").unwrap()).as_str(),
        "http://example.com/socket"
    );
    assert_eq!(
        websocket_cookie_url(&Url::parse("wss://example.com/socket").unwrap()).as_str(),
        "https://example.com/socket"
    );
}

#[test]
fn websocket_url_normalization_matches_constructor_scheme_rules() {
    let base = Url::parse("https://example.com/base/page.html").unwrap();

    assert_eq!(
        normalize_websocket_url(&base, "ws://[::1"),
        Err(WebSocketUrlError::Invalid)
    );
    assert_eq!(
        normalize_websocket_url(&base, "/socket").unwrap().as_str(),
        "wss://example.com/socket"
    );
    assert_eq!(
        normalize_websocket_url(&base, "http://example.test/socket")
            .unwrap()
            .as_str(),
        "ws://example.test/socket"
    );
    assert_eq!(
        normalize_websocket_url(&base, "https://example.test/socket")
            .unwrap()
            .as_str(),
        "wss://example.test/socket"
    );
    assert_eq!(
        normalize_websocket_url(&base, "ftp://example.test/socket"),
        Err(WebSocketUrlError::DisallowedScheme("ftp".to_owned()))
    );
    assert_eq!(
        normalize_websocket_url(&base, "ws://example.test/socket#frag"),
        Err(WebSocketUrlError::Fragment)
    );
}

#[test]
fn websocket_subprotocol_validation_rejects_invalid_and_case_duplicates() {
    assert!(is_valid_subprotocol("chat"));
    assert!(is_valid_subprotocol("super.chat-1_2"));
    assert!(!is_valid_subprotocol(""));
    assert!(!is_valid_subprotocol("bad protocol"));
    assert!(!is_valid_subprotocol("bad,protocol"));
    assert!(!is_valid_subprotocol("\u{80}echo"));

    assert!(validate_subprotocols(&["chat".to_owned(), "superchat".to_owned()]).is_ok());
    assert_eq!(
        validate_subprotocols(&["chat".to_owned(), "CHAT".to_owned()]),
        Err(WebSocketSubprotocolError::Duplicate("CHAT".to_owned()))
    );
    assert_eq!(
        validate_subprotocols(&["bad/protocol".to_owned()]),
        Err(WebSocketSubprotocolError::Invalid(
            "bad/protocol".to_owned()
        ))
    );
}

#[test]
fn websocket_close_info_validation_matches_web_api_rules() {
    assert!(is_valid_close_code(1000));
    assert!(!is_valid_close_code(1001));
    assert!(!is_valid_close_code(2999));
    assert!(is_valid_close_code(3000));
    assert!(is_valid_close_code(4999));
    assert!(!is_valid_close_code(5000));

    assert!(is_valid_close_reason(&"x".repeat(123)));
    assert!(!is_valid_close_reason(&"x".repeat(124)));
    assert_eq!(default_close_code_for_reason(None, ""), None);
    assert_eq!(default_close_code_for_reason(None, "reason"), Some(1000));
    assert_eq!(
        default_close_code_for_reason(Some(3333), "reason"),
        Some(3333)
    );

    assert_eq!(
        validate_websocket_close_request(Some(3001), "done".to_owned()).unwrap(),
        WebSocketCloseRequest {
            code: Some(3001),
            reason: "done".to_owned(),
        }
    );
    assert_eq!(
        validate_websocket_close_request(None, "done".to_owned()).unwrap(),
        WebSocketCloseRequest {
            code: None,
            reason: "done".to_owned(),
        }
    );
    assert_eq!(
        validate_websocket_close_request(Some(1001), String::new()),
        Err(WebSocketCloseValidationError::InvalidCode)
    );
    assert_eq!(
        validate_websocket_close_request(Some(3000), "x".repeat(124)),
        Err(WebSocketCloseValidationError::ReasonTooLong)
    );

    assert_eq!(close_info_code_from_number(3000.4), Ok(3000));
    assert_eq!(close_info_code_from_number(3000.5), Ok(3001));
    assert_eq!(
        close_info_code_from_number(f64::NAN),
        Err(WebSocketCloseValidationError::InvalidCode)
    );
    assert_eq!(
        normalize_websocket_close_info(None, "reason".to_owned()).unwrap(),
        WebSocketCloseRequest {
            code: Some(1000),
            reason: "reason".to_owned(),
        }
    );
    assert_eq!(
        normalize_websocket_close_info(Some(1001), String::new()),
        Err(WebSocketCloseValidationError::InvalidCode)
    );
}

#[test]
fn websocket_potentially_trustworthy_url_matches_loopback_policy() {
    assert!(websocket_url_is_potentially_trustworthy(
        &Url::parse("ws://localhost/socket").unwrap()
    ));
    assert!(websocket_url_is_potentially_trustworthy(
        &Url::parse("ws://api.localhost/socket").unwrap()
    ));
    assert!(websocket_url_is_potentially_trustworthy(
        &Url::parse("ws://127.0.0.1/socket").unwrap()
    ));
    assert!(websocket_url_is_potentially_trustworthy(
        &Url::parse("ws://[::1]/socket").unwrap()
    ));
    assert!(!websocket_url_is_potentially_trustworthy(
        &Url::parse("ws://example.test/socket").unwrap()
    ));
}

#[test]
fn websocket_request_builder_rejects_invalid_subprotocol_defensively() {
    let context = test_websocket_context();
    let error = build_websocket_request(
        "ws://example.com/socket",
        &["chat".to_owned(), "CHAT".to_owned()],
        &context,
    )
    .expect_err("duplicate subprotocol should fail");

    assert!(error.contains("subprotocol `CHAT` is duplicated"));
}

#[test]
fn websocket_request_builder_rejects_blocked_ports() {
    let context = test_websocket_context();
    let error = build_websocket_request("ws://127.0.0.1:25/socket", &[], &context)
        .expect_err("blocked port should fail");

    assert!(error.contains("port `25` is blocked"));
    assert!(build_websocket_request("ws://127.0.0.1:43210/socket", &[], &context).is_ok());
}

#[test]
fn websocket_no_proxy_matches_hosts_domains_ports_and_wildcard() {
    assert!(no_proxy_matches("example.com", None, Some("example.com")));
    assert!(no_proxy_matches(
        "api.example.com",
        None,
        Some(".example.com")
    ));
    assert!(no_proxy_matches(
        "api.example.com",
        Some(8080),
        Some("example.com:8080")
    ));
    assert!(no_proxy_matches("anything.test", None, Some("*")));
    assert!(!no_proxy_matches(
        "api.example.com",
        Some(8081),
        Some("example.com:8080")
    ));
    assert!(!no_proxy_matches(
        "notexample.com",
        None,
        Some("example.com")
    ));
}

#[test]
fn websocket_proxy_url_uses_env_http_proxy_for_ws_when_unset() {
    let context = test_websocket_context();
    let uri = "ws://target.test/socket".parse().unwrap();
    let proxy = test_websocket_proxy_url_with_env(
        &uri,
        &context,
        &[("http_proxy", "http://127.0.0.1:8080")],
    );

    assert_eq!(proxy.as_deref(), Some("http://127.0.0.1:8080/"));
}

#[test]
fn websocket_proxy_url_uses_env_https_proxy_for_wss_when_unset() {
    let context = test_websocket_context();
    let uri = "wss://target.test/socket".parse().unwrap();
    let proxy = test_websocket_proxy_url_with_env(
        &uri,
        &context,
        &[("HTTPS_PROXY", "http://127.0.0.1:8443")],
    );

    assert_eq!(proxy.as_deref(), Some("http://127.0.0.1:8443/"));
}

#[test]
fn websocket_proxy_url_ignores_uppercase_http_proxy_for_ws() {
    let context = test_websocket_context();
    let uri = "ws://target.test/socket".parse().unwrap();
    let proxy = test_websocket_proxy_url_with_env(
        &uri,
        &context,
        &[("HTTP_PROXY", "http://127.0.0.1:8080")],
    );

    assert_eq!(proxy, None);
}

#[test]
fn websocket_proxy_url_uses_all_proxy_fallback() {
    let context = test_websocket_context();
    let uri = "wss://target.test/socket".parse().unwrap();
    let proxy = test_websocket_proxy_url_with_env(
        &uri,
        &context,
        &[("ALL_PROXY", "http://127.0.0.1:9000")],
    );

    assert_eq!(proxy.as_deref(), Some("http://127.0.0.1:9000/"));
}

#[test]
fn websocket_proxy_url_respects_env_no_proxy() {
    let context = test_websocket_context();
    let uri = "ws://api.example.com/socket".parse().unwrap();
    let proxy = test_websocket_proxy_url_with_env(
        &uri,
        &context,
        &[
            ("http_proxy", "http://127.0.0.1:8080"),
            ("NO_PROXY", ".example.com"),
        ],
    );

    assert_eq!(proxy, None);
}

#[test]
fn websocket_proxy_url_explicit_empty_proxy_disables_env_fallback() {
    let mut context = test_websocket_context();
    context.http_proxy = Some(String::new());
    let uri = "ws://target.test/socket".parse().unwrap();
    let proxy = test_websocket_proxy_url_with_env(
        &uri,
        &context,
        &[("http_proxy", "http://127.0.0.1:8080")],
    );

    assert_eq!(proxy, None);
}

#[test]
fn websocket_proxy_connect_header_rejects_newline_values() {
    let mut request = String::new();
    assert!(append_proxy_connect_header(&mut request, "User-Agent", "Moli").is_ok());
    assert_eq!(request, "User-Agent: Moli\r\n");
    assert!(
        append_proxy_connect_header(&mut request, "Proxy-Authorization", "Bearer good\nbad")
            .is_err()
    );
}

#[test]
fn websocket_request_builder_applies_context_protocols_and_cookie() {
    let mut context = test_websocket_context();
    context.extra_headers = vec![
        ("X-Moli-Trace".to_owned(), "socket".to_owned()),
        ("Sec-WebSocket-Version".to_owned(), "999".to_owned()),
    ];
    context.cookie_header = Some("sid=server".to_owned());

    let request = build_websocket_request(
        "ws://example.com/socket",
        &["chat".to_owned(), "superchat".to_owned()],
        &context,
    )
    .expect("websocket request should build");

    assert_eq!(request.uri(), "ws://example.com/socket");
    assert_eq!(
        request
            .headers()
            .get(http::header::ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("https://example.com")
    );
    assert_eq!(
        request
            .headers()
            .get(http::header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok()),
        Some("chat, superchat")
    );
    assert_eq!(
        request
            .headers()
            .get(http::header::SEC_WEBSOCKET_VERSION)
            .and_then(|value| value.to_str().ok()),
        Some("13")
    );
    assert_eq!(
        request
            .headers()
            .get(http::header::COOKIE)
            .and_then(|value| value.to_str().ok()),
        Some("sid=server")
    );
    assert_eq!(
        request
            .headers()
            .get("x-moli-trace")
            .and_then(|value| value.to_str().ok()),
        Some("socket")
    );
}

#[test]
fn websocket_request_builder_converts_url_userinfo_to_basic_auth() {
    let context = test_websocket_context();

    let request = build_websocket_request("ws://foo:bar@example.com/socket", &[], &context)
        .expect("websocket request should build");

    assert_eq!(
        request
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Basic Zm9vOmJhcg==")
    );
}

#[test]
fn websocket_request_builder_decodes_percent_encoded_userinfo_for_basic_auth() {
    let context = test_websocket_context();

    let request =
        build_websocket_request("ws://foo%20bar:p%40ss@example.com/socket", &[], &context)
            .expect("websocket request should build");

    assert_eq!(
        request
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Basic Zm9vIGJhcjpwQHNz")
    );
}

#[tokio::test]
async fn websocket_failed_connection_reports_error_then_abnormal_close() {
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let command_tx = spawn_failed_connection(7, "preflight failed".to_owned(), event_tx);

    assert!(
        command_tx
            .send(Command::SendText("ignored".to_owned()))
            .is_err()
    );

    match timeout(Duration::from_secs(3), event_rx.recv())
        .await
        .expect("websocket error should arrive")
        .expect("websocket event channel should stay open")
    {
        Event::Error { socket_id, message } => {
            assert_eq!(socket_id, 7);
            assert_eq!(message, "preflight failed");
        }
        event => panic!("expected websocket error, got {event:?}"),
    }

    match timeout(Duration::from_secs(3), event_rx.recv())
        .await
        .expect("websocket close should arrive")
        .expect("websocket event channel should stay open")
    {
        Event::Close {
            socket_id,
            code,
            reason,
            was_clean,
        } => {
            assert_eq!(socket_id, 7);
            assert_eq!(code, 1006);
            assert!(reason.is_empty());
            assert!(!was_clean);
        }
        event => panic!("expected websocket close, got {event:?}"),
    }
}

#[tokio::test]
async fn websocket_last_handle_drop_releases_pending_handshake() {
    use tokio::io::AsyncReadExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/pending", listener.local_addr().unwrap());
    let (event_tx, _event_rx) = mpsc::channel(1);
    let handle = spawn_connection(61, url, Vec::new(), test_websocket_context(), event_tx);
    let retained = handle.clone();
    let (mut server, _) = timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("connection should reach the server")
        .unwrap();
    drop(handle);
    retained
        .send(Command::SendText("ignored while connecting".to_owned()))
        .expect("another producer still owns the connection");
    drop(retained);
    let result = timeout(Duration::from_secs(3), server.read_to_end(&mut Vec::new()))
        .await
        .expect("dropping the last producer must close a pending handshake");
    if let Err(error) = result {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
}

struct WebSocketSinkDropSignal(mpsc::Sender<()>);

impl Drop for WebSocketSinkDropSignal {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

#[tokio::test]
async fn websocket_rejected_handshake_or_open_event_releases_transport() {
    use futures_util::StreamExt;

    for pause_after_handshake in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/closed-sink", listener.local_addr().unwrap());
        let (command_tx, command_rx) = crate::commands::command_channel();
        let mut context = test_websocket_context();
        context.pause_after_handshake = pause_after_handshake;
        let server = async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let terminal = socket.next().await;
            assert!(terminal.is_none() || terminal.is_some_and(|result| result.is_err()));
        };
        let client = crate::connection::run_websocket_connection(
            64,
            url,
            Vec::new(),
            context,
            command_rx,
            EventSender::closed(),
        );
        let (result, ()) = timeout(Duration::from_secs(3), async {
            tokio::join!(client, server)
        })
        .await
        .expect("a closed sink must release the connection before waiting for commands");
        result.expect_err("delivery should report the unavailable sink");
        assert!(command_tx.is_closed());
    }
}

#[tokio::test]
async fn websocket_rejected_message_event_releases_transport_and_writer() {
    use futures_util::{SinkExt, StreamExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/closed-message-sink",
        listener.local_addr().unwrap()
    );
    let (command_tx, command_rx) = crate::commands::command_channel();
    let sink =
        EventSender::with_async_sink(
            |event| async move { !matches!(event, Event::TextMessage { .. }) },
        );
    let server = async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket.send("reject delivery".into()).await.unwrap();
        let terminal = socket.next().await;
        assert!(terminal.is_none() || terminal.is_some_and(|result| result.is_err()));
    };
    let client = crate::connection::run_websocket_connection(
        65,
        url,
        Vec::new(),
        test_websocket_context(),
        command_rx,
        sink,
    );
    let (result, ()) = timeout(Duration::from_secs(3), async {
        tokio::join!(client, server)
    })
    .await
    .expect("message delivery rejection must stop both transport tasks");
    result.expect_err("message sink is closed");
    assert!(command_tx.is_closed());
}

#[tokio::test]
async fn websocket_synthetic_delivery_stops_after_first_rejected_event() {
    let (command_tx, command_rx) = crate::commands::command_channel();
    command_tx
        .send(Command::SendText("hello".to_owned()))
        .unwrap();
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let sink = EventSender::with_async_sink(move |event| {
        let observed_tx = observed_tx.clone();
        async move {
            let accept = matches!(event, Event::Open { .. });
            observed_tx.send(event).unwrap();
            accept
        }
    });
    timeout(
        Duration::from_secs(3),
        crate::synthetic::run_synthetic_websocket_connection(
            66,
            command_rx,
            sink,
            Vec::new(),
            101,
            Vec::new(),
        ),
    )
    .await
    .expect("synthetic producer must stop when FrameSent is rejected")
    .expect_err("synthetic event sink is closed");
    assert!(command_tx.is_closed());
    assert!(matches!(observed_rx.try_recv(), Ok(Event::Open { .. })));
    assert!(matches!(
        observed_rx.try_recv(),
        Ok(Event::FrameSent { .. })
    ));
    assert!(
        observed_rx.try_recv().is_err(),
        "no further delivery attempts after rejection"
    );
}

#[tokio::test]
async fn websocket_cancel_releases_blocked_sink_and_open_writer() {
    use futures_util::{SinkExt, StreamExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/cancel", listener.local_addr().unwrap());
    let entered = Arc::new(tokio::sync::Notify::new());
    let sink_entered = Arc::clone(&entered);
    let (dropped_tx, mut dropped_rx) = mpsc::channel(1);
    let sink = EventSender::with_async_sink(move |event| {
        let entered = Arc::clone(&sink_entered);
        let dropped_tx = dropped_tx.clone();
        async move {
            if matches!(event, Event::TextMessage { .. }) {
                let _guard = WebSocketSinkDropSignal(dropped_tx);
                entered.notify_one();
                return std::future::pending::<bool>().await;
            }
            true
        }
    });
    let handle = spawn_connection(62, url, Vec::new(), test_websocket_context(), sink);
    let retained = handle.clone();
    let (stream, _) = timeout(Duration::from_secs(3), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut server = tokio_tungstenite::accept_async(stream).await.unwrap();
    server.send("block the sink".into()).await.unwrap();
    timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("receive task must enter the blocked sink");
    handle.cancel();
    handle.cancel();
    assert!(retained.is_closed());
    assert!(retained.send(Command::SendBinary(vec![1])).is_err());
    timeout(Duration::from_secs(3), dropped_rx.recv())
        .await
        .expect("cancellation must release the blocked event future")
        .expect("sink drop signal");
    let terminal = timeout(Duration::from_secs(3), server.next())
        .await
        .expect("the writer must release its socket while producer clones remain alive");
    assert!(terminal.is_none() || terminal.is_some_and(|result| result.is_err()));
}

#[tokio::test]
async fn websocket_failed_and_synthetic_delivery_cancel_on_last_handle_drop() {
    for synthetic in [false, true] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let sink_entered = Arc::clone(&entered);
        let (dropped_tx, mut dropped_rx) = mpsc::channel(1);
        let sink = EventSender::with_async_sink(move |_event| {
            let entered = Arc::clone(&sink_entered);
            let dropped_tx = dropped_tx.clone();
            async move {
                let _guard = WebSocketSinkDropSignal(dropped_tx);
                entered.notify_one();
                std::future::pending::<bool>().await
            }
        });
        let handle = if synthetic {
            spawn_synthetic_connection(63, Vec::new(), 101, Vec::new(), sink)
        } else {
            spawn_failed_connection(63, "rejected".to_owned(), sink)
        };
        timeout(Duration::from_secs(3), entered.notified())
            .await
            .expect("connection should reach the async sink");
        drop(handle);
        timeout(Duration::from_secs(3), dropped_rx.recv())
            .await
            .expect("last producer drop must cancel blocked delivery")
            .expect("sink drop signal");
    }
}

#[tokio::test]
async fn websocket_synthetic_connection_opens_accounts_send_and_closes_cleanly() {
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let command_tx = spawn_synthetic_connection(
        92,
        vec![("Origin".to_owned(), "http://example.test".to_owned())],
        101,
        vec![("Sec-WebSocket-Protocol".to_owned(), "chat".to_owned())],
        event_tx,
    );

    let event = timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic websocket open should arrive")
        .expect("synthetic websocket event channel should stay open");
    match event {
        Event::Open {
            socket_id,
            protocol,
            response_status,
            ..
        } => {
            assert_eq!(socket_id, 92);
            assert_eq!(protocol, "chat");
            assert_eq!(response_status, 101);
        }
        event => panic!("expected synthetic websocket open, got {event:?}"),
    }

    command_tx
        .send(Command::SendText("hello".to_owned()))
        .expect("send synthetic websocket text");
    let event = timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic frame sent should arrive")
        .expect("synthetic websocket event channel should stay open");
    match event {
        Event::FrameSent {
            socket_id,
            opcode,
            payload_length,
        } => {
            assert_eq!(socket_id, 92);
            assert_eq!(opcode, FrameOpcode::Text);
            assert_eq!(payload_length, 5);
        }
        event => panic!("expected synthetic websocket frame sent, got {event:?}"),
    }
    let event = timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic buffered amount consumption should arrive")
        .expect("synthetic websocket event channel should stay open");
    match event {
        Event::BufferedAmountConsumed { socket_id, amount } => {
            assert_eq!(socket_id, 92);
            assert_eq!(amount, 5);
        }
        event => panic!("expected synthetic buffered amount event, got {event:?}"),
    }

    command_tx
        .send(Command::Close {
            code: Some(1000),
            reason: "done".to_owned(),
        })
        .expect("close synthetic websocket");
    let event = timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic closing should arrive")
        .expect("synthetic websocket event channel should stay open");
    assert!(matches!(event, Event::Closing { socket_id: 92 }));
    let event = timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic close should arrive")
        .expect("synthetic websocket event channel should stay open");
    match event {
        Event::Close {
            socket_id,
            code,
            reason,
            was_clean,
        } => {
            assert_eq!(socket_id, 92);
            assert_eq!(code, 1000);
            assert_eq!(reason, "done");
            assert!(was_clean);
        }
        event => panic!("expected synthetic close, got {event:?}"),
    }
}

#[tokio::test]
async fn websocket_synthetic_connection_can_receive_frames_and_server_close() {
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let command_tx = spawn_synthetic_connection(
        93,
        vec![("Origin".to_owned(), "http://example.test".to_owned())],
        101,
        Vec::new(),
        event_tx,
    );

    assert!(matches!(
        timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("synthetic websocket open should arrive")
            .expect("synthetic websocket event channel should stay open"),
        Event::Open { socket_id: 93, .. }
    ));

    command_tx
        .send(Command::ReceiveText("server-text".to_owned()))
        .expect("inject synthetic websocket text");
    match timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic text should arrive")
        .expect("synthetic websocket event channel should stay open")
    {
        Event::TextMessage { socket_id, data } => {
            assert_eq!(socket_id, 93);
            assert_eq!(data, "server-text");
        }
        event => panic!("expected synthetic text message, got {event:?}"),
    }

    command_tx
        .send(Command::ReceiveBinary(vec![1, 2, 3, 4]))
        .expect("inject synthetic websocket binary");
    match timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic binary should arrive")
        .expect("synthetic websocket event channel should stay open")
    {
        Event::BinaryMessage { socket_id, data } => {
            assert_eq!(socket_id, 93);
            assert_eq!(data, vec![1, 2, 3, 4]);
        }
        event => panic!("expected synthetic binary message, got {event:?}"),
    }

    command_tx
        .send(Command::ServerClose {
            code: Some(1000),
            reason: "server-done".to_owned(),
        })
        .expect("inject synthetic websocket server close");
    match timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("synthetic server close should arrive")
        .expect("synthetic websocket event channel should stay open")
    {
        Event::Close {
            socket_id,
            code,
            reason,
            was_clean,
        } => {
            assert_eq!(socket_id, 93);
            assert_eq!(code, 1000);
            assert_eq!(reason, "server-done");
            assert!(was_clean);
        }
        event => panic!("expected synthetic server close, got {event:?}"),
    }
}

#[tokio::test]
async fn websocket_transport_close_while_connecting_fails_before_open() {
    let (url, server) = spawn_sleeping_handshake_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let command_tx = spawn_connection(91, url, Vec::new(), test_websocket_context(), event_tx);

    command_tx
        .send(Command::Close {
            code: None,
            reason: String::new(),
        })
        .expect("send connecting close command");

    let error = recv_handshake_failure_events(&mut event_rx).await;
    assert_eq!(error, "WebSocket connection closed before opening");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn websocket_transport_can_pause_after_handshake_before_open() {
    let (url, server) = spawn_text_echo_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let mut context = test_websocket_context();
    context.pause_after_handshake = true;
    let command_tx = spawn_connection(92, url, Vec::new(), context, event_tx);

    match timeout(Duration::from_secs(3), event_rx.recv())
        .await
        .expect("websocket handshake response should arrive")
        .expect("websocket event channel should stay open")
    {
        Event::HandshakeResponse {
            socket_id,
            response_status,
            ..
        } => {
            assert_eq!(socket_id, 92);
            assert_eq!(response_status, 101);
        }
        event => panic!("expected websocket handshake response, got {event:?}"),
    }
    assert!(
        timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err(),
        "websocket open must wait for ContinueOpen"
    );

    command_tx
        .send(Command::ContinueOpen {
            response_status: None,
            response_headers: None,
        })
        .expect("continue paused websocket open");
    let open = recv_open_event(&mut event_rx).await;
    assert_eq!(open.socket_id, 92);
    let _ = command_tx.send(Command::Close {
        code: Some(1000),
        reason: "done".to_owned(),
    });
    server
        .await
        .expect("websocket pause-after-handshake server should finish");
}

#[tokio::test]
async fn websocket_transport_handshake_applies_context_headers_and_preserves_control_headers() {
    let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let mut context = test_websocket_context();
    context.extra_headers = vec![
        ("X-Moli-Trace".to_owned(), "socket".to_owned()),
        // Protocol control headers are generated by tungstenite and should not
        // be overridden by embedding-layer extra headers.
        ("Sec-WebSocket-Version".to_owned(), "999".to_owned()),
    ];
    context.cookie_header = Some("sid=server".to_owned());

    let command_tx = spawn_connection(1, url, vec!["chat".to_owned()], context, event_tx);
    let headers = timeout(Duration::from_secs(3), headers_rx)
        .await
        .expect("websocket headers should arrive")
        .expect("websocket header sender should stay alive");
    let open = recv_open_event(&mut event_rx).await;
    let _ = command_tx.send(Command::Close {
        code: Some(1000),
        reason: "done".to_owned(),
    });
    server.await.expect("websocket header server should finish");

    assert_eq!(open.socket_id, 1);
    assert_eq!(open.protocol, "chat");
    assert_eq!(
        header_value(&headers, "origin").as_deref(),
        Some("https://example.com")
    );
    assert_eq!(
        header_value(&headers, "user-agent").as_deref(),
        Some("Moli-WebSocket-Test/1.0")
    );
    assert_eq!(
        header_value(&headers, "x-moli-trace").as_deref(),
        Some("socket")
    );
    assert_eq!(
        header_value(&headers, "cookie").as_deref(),
        Some("sid=server")
    );
    assert_eq!(
        header_value(&headers, "sec-websocket-protocol").as_deref(),
        Some("chat")
    );
    assert_eq!(
        header_value(&headers, "sec-websocket-version").as_deref(),
        Some("13")
    );
    assert_eq!(header_value(&headers, "referer"), None);
}

#[tokio::test]
async fn websocket_transport_uses_explicit_http_proxy_connect_without_forwarding_proxy_auth() {
    let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
    let (proxy_url, proxy_request_rx, proxy) = spawn_http_connect_proxy().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let mut context = test_websocket_context();
    context.http_proxy = Some(proxy_url);
    context.http_no_proxy = Some(String::new());
    context.proxy_bearer_token = Some("proxy-token".to_owned());

    let command_tx = spawn_connection(2, url.clone(), Vec::new(), context, event_tx);
    let proxy_request = timeout(Duration::from_secs(3), proxy_request_rx)
        .await
        .expect("proxy CONNECT should arrive")
        .expect("proxy request sender should stay alive");
    let headers = timeout(Duration::from_secs(3), headers_rx)
        .await
        .expect("websocket headers should arrive")
        .expect("websocket header sender should stay alive");
    let open = recv_open_event(&mut event_rx).await;
    let _ = command_tx.send(Command::Close {
        code: Some(1000),
        reason: "done".to_owned(),
    });
    server.await.expect("websocket header server should finish");
    proxy.await.expect("websocket proxy should finish");

    let target = Url::parse(&url).expect("websocket target url");
    let expected_connect = format!(
        "CONNECT {}:{} HTTP/1.1",
        target.host_str().expect("target host"),
        target.port_or_known_default().expect("target port")
    );
    assert_eq!(open.socket_id, 2);
    assert!(
        proxy_request.starts_with(&expected_connect),
        "unexpected CONNECT request: {proxy_request:?}"
    );
    assert!(
        proxy_request.contains("\r\nProxy-Authorization: Bearer proxy-token\r\n"),
        "proxy bearer token should be sent only on CONNECT: {proxy_request:?}"
    );
    assert_eq!(
        header_value(&headers, "origin").as_deref(),
        Some("https://example.com")
    );
    assert_eq!(header_value(&headers, "proxy-authorization"), None);
}

#[tokio::test]
async fn websocket_transport_rejects_non_200_http_proxy_connect() {
    let (proxy_url, proxy_request_rx, proxy) =
        spawn_http_connect_proxy_response(b"HTTP/1.1 204 No Content\r\n\r\n").await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let mut context = test_websocket_context();
    context.http_proxy = Some(proxy_url);
    context.http_no_proxy = Some(String::new());

    let _command_tx = spawn_connection(
        3,
        "ws://example.test/socket".to_owned(),
        Vec::new(),
        context,
        event_tx,
    );
    let proxy_request = timeout(Duration::from_secs(3), proxy_request_rx)
        .await
        .expect("proxy CONNECT should arrive")
        .expect("proxy request sender should stay alive");
    let message = recv_handshake_failure_events(&mut event_rx).await;
    proxy.await.expect("websocket proxy should finish");

    assert!(
        proxy_request.starts_with("CONNECT example.test:80 HTTP/1.1"),
        "unexpected CONNECT request: {proxy_request:?}"
    );
    assert!(
        message.contains("HTTP/1.1 204 No Content"),
        "unexpected proxy CONNECT error: {message}"
    );
}

#[tokio::test]
async fn websocket_transport_respects_disabled_tls_verify_for_self_signed_wss() {
    let (url, headers_rx, server) = spawn_tls_header_capture_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let mut context = test_websocket_context();
    context.tls.verify = false;

    let command_tx = spawn_connection(3, url, Vec::new(), context, event_tx);
    let headers = timeout(Duration::from_secs(3), headers_rx)
        .await
        .expect("websocket TLS headers should arrive")
        .expect("websocket TLS header sender should stay alive");
    let open = recv_open_event(&mut event_rx).await;
    let _ = command_tx.send(Command::Close {
        code: Some(1000),
        reason: "done".to_owned(),
    });
    server
        .await
        .expect("websocket TLS header server should finish");

    assert_eq!(open.socket_id, 3);
    assert_eq!(
        header_value(&headers, "origin").as_deref(),
        Some("https://example.com")
    );
}

#[tokio::test]
async fn websocket_transport_wss_allows_server_to_omit_response_subprotocol() {
    let (url, headers_rx, server) = spawn_tls_header_capture_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let mut context = test_websocket_context();
    context.tls.verify = false;

    let _command_tx = spawn_connection(
        4,
        url,
        vec!["chat".to_owned(), "superchat".to_owned()],
        context,
        event_tx,
    );
    let headers = timeout(Duration::from_secs(3), headers_rx)
        .await
        .expect("websocket TLS headers should arrive")
        .expect("websocket TLS header sender should stay alive");
    let open = recv_open_event(&mut event_rx).await;
    assert_close(&mut event_rx, 4, 1005, "", true).await;
    server
        .await
        .expect("websocket TLS no-protocol server should finish");

    assert_eq!(open.socket_id, 4);
    assert_eq!(open.protocol, "");
    assert_eq!(
        header_value(&headers, "sec-websocket-protocol").as_deref(),
        Some("chat, superchat")
    );
}

#[tokio::test]
async fn websocket_transport_rejects_non_switching_and_redirect_statuses() {
    for (path, response) in [
        ("plain-200", b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_slice()),
        (
            "redirect-301",
            b"HTTP/1.1 301 Moved Permanently\r\nLocation: /echo\r\nContent-Length: 0\r\n\r\n"
                .as_slice(),
        ),
        (
            "not-found-404",
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".as_slice(),
        ),
        (
            "unauthorized-401",
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"websocket\"\r\nContent-Length: 0\r\n\r\n"
                .as_slice(),
        ),
    ] {
        let message = websocket_raw_handshake_failure_message(path, response).await;
        assert!(
            message.contains("WebSocket connection failed"),
            "unexpected error for {path}: {message}"
        );
    }
}

#[tokio::test]
async fn websocket_transport_rejects_wrong_accept_key() {
    let message = websocket_raw_handshake_failure_message(
        "wrong-accept",
        b"HTTP/1.1 101 Switching Protocols\r\n\
            Upgrade: websocket\r\n\
            Connection: Upgrade\r\n\
            Sec-WebSocket-Accept: wrongAcceptKey\r\n\r\n",
    )
    .await;

    assert!(
        message.contains("WebSocket connection failed"),
        "unexpected wrong-accept error: {message}"
    );
}

#[tokio::test]
async fn websocket_transport_rejects_missing_or_wrong_upgrade_headers() {
    for (path, headers) in [
        ("missing-upgrade", vec!["Connection: Upgrade"]),
        ("missing-connection", vec!["Upgrade: websocket"]),
        ("wrong-upgrade", vec!["Upgrade: h2c", "Connection: Upgrade"]),
        (
            "wrong-connection",
            vec!["Upgrade: websocket", "Connection: keep-alive"],
        ),
    ] {
        let message =
            websocket_computed_accept_handshake_failure_message(path, headers, Vec::new()).await;
        assert!(
            message.contains("WebSocket connection failed"),
            "unexpected error for {path}: {message}"
        );
    }
}

#[tokio::test]
async fn websocket_transport_rejects_invalid_response_subprotocols() {
    for (path, headers, protocols) in [
        (
            "unrequested-protocol",
            vec![
                "Upgrade: websocket",
                "Connection: Upgrade",
                "Sec-WebSocket-Protocol: other",
            ],
            vec!["chat".to_owned()],
        ),
        (
            "empty-response-protocol",
            vec![
                "Upgrade: websocket",
                "Connection: Upgrade",
                "Sec-WebSocket-Protocol: ",
            ],
            vec!["chat".to_owned()],
        ),
        (
            "multiple-response-protocols",
            vec![
                "Upgrade: websocket",
                "Connection: Upgrade",
                "Sec-WebSocket-Protocol: chat, superchat",
            ],
            vec!["chat".to_owned(), "superchat".to_owned()],
        ),
    ] {
        let message =
            websocket_computed_accept_handshake_failure_message(path, headers, protocols).await;
        assert!(
            message.contains("WebSocket connection failed"),
            "unexpected error for {path}: {message}"
        );
    }
}

#[tokio::test]
async fn websocket_transport_allows_server_to_omit_response_subprotocol() {
    let (url, server) = spawn_text_binary_echo_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);

    let command_tx = spawn_connection(
        4,
        url,
        vec!["chat".to_owned(), "superchat".to_owned()],
        test_websocket_context(),
        event_tx,
    );

    let open = recv_open_event(&mut event_rx).await;
    assert_eq!(open.socket_id, 4);
    assert_eq!(open.protocol, "");
    command_tx
        .send(Command::Close {
            code: Some(1000),
            reason: "no-protocol".to_owned(),
        })
        .expect("send close command");
    assert_closing(&mut event_rx, 4).await;
    assert_close(&mut event_rx, 4, 1000, "no-protocol", true).await;
    server
        .await
        .expect("websocket no-protocol echo server should finish");
}

#[tokio::test]
async fn websocket_transport_sends_text_binary_and_reports_buffered_amount_consumption() {
    let (url, server) = spawn_text_binary_echo_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let command_tx = spawn_connection(4, url, Vec::new(), test_websocket_context(), event_tx);

    let open = recv_open_event(&mut event_rx).await;
    assert_eq!(open.socket_id, 4);

    command_tx
        .send(Command::SendText("hello".to_owned()))
        .expect("send text command");
    assert_frame_sent(&mut event_rx, 4, FrameOpcode::Text, 5).await;
    assert_buffered_amount_consumed(&mut event_rx, 4, 5).await;
    assert_text_message(&mut event_rx, 4, "hello").await;

    command_tx
        .send(Command::SendBinary(vec![1, 2, 3, 4]))
        .expect("send binary command");
    assert_frame_sent(&mut event_rx, 4, FrameOpcode::Binary, 4).await;
    assert_buffered_amount_consumed(&mut event_rx, 4, 4).await;
    assert_binary_message(&mut event_rx, 4, &[1, 2, 3, 4]).await;

    command_tx
        .send(Command::Close {
            code: Some(1000),
            reason: "done".to_owned(),
        })
        .expect("send close command");
    assert_closing(&mut event_rx, 4).await;
    assert_close(&mut event_rx, 4, 1000, "done", true).await;
    server.await.expect("websocket echo server should finish");
}

#[tokio::test]
async fn websocket_transport_reads_while_sending_many_large_messages() {
    let (url, server) = spawn_backpressure_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let command_tx = spawn_connection(40, url, Vec::new(), test_websocket_context(), event_tx);

    let open = recv_open_event(&mut event_rx).await;
    assert_eq!(open.socket_id, 40);

    for _ in 0..50 {
        command_tx
            .send(Command::SendBinary(vec![0; 65_536]))
            .expect("send large binary command");
    }

    let mut replies = 0;
    while replies < 50 {
        match timeout(Duration::from_secs(3), event_rx.recv())
            .await
            .expect("websocket event should arrive")
            .expect("websocket event channel should stay open")
        {
            Event::TextMessage { socket_id, data } => {
                assert_eq!(socket_id, 40);
                assert_eq!(data, "65536");
                replies += 1;
            }
            Event::Error { message, .. } => panic!("unexpected websocket error: {message}"),
            Event::Close { code, reason, .. } => {
                panic!("websocket closed before all replies: {code} {reason}")
            }
            Event::HandshakeResponse { .. }
            | Event::Open { .. }
            | Event::BinaryMessage { .. }
            | Event::FrameSent { .. }
            | Event::BufferedAmountConsumed { .. }
            | Event::Closing { .. } => {}
        }
    }

    command_tx
        .send(Command::Close {
            code: Some(1000),
            reason: "backpressure".to_owned(),
        })
        .expect("send close command");
    assert_closing(&mut event_rx, 40).await;
    assert_close(&mut event_rx, 40, 1000, "backpressure", true).await;
    server
        .await
        .expect("websocket backpressure server should finish");
}

#[tokio::test]
async fn websocket_transport_reports_server_initiated_close_frame() {
    let (url, server) = spawn_server_close_websocket_server().await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let _command_tx = spawn_connection(5, url, Vec::new(), test_websocket_context(), event_tx);

    let open = recv_open_event(&mut event_rx).await;
    assert_eq!(open.socket_id, 5);
    assert_close(&mut event_rx, 5, 3001, "server done", true).await;
    server
        .await
        .expect("websocket server-close server should finish");
}

#[tokio::test]
async fn websocket_transport_handles_close_frame_in_handshake_packet() {
    let (url, server) = spawn_computed_accept_websocket_response_with_body_server(
        "simple-handshake-close",
        vec!["Upgrade: websocket", "Connection: Upgrade"],
        b"\x88\x06\x03\xe9PASS",
    )
    .await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let _command_tx = spawn_connection(6, url, Vec::new(), test_websocket_context(), event_tx);

    let open = recv_open_event(&mut event_rx).await;
    assert_eq!(open.socket_id, 6);
    assert_close(&mut event_rx, 6, 1001, "PASS", true).await;
    server
        .await
        .expect("websocket same-packet close server should finish");
}
