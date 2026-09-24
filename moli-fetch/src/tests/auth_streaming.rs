use super::*;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Finish {
    Complete,
    Truncated,
    Cancelled,
    Rejected,
    RejectedTruncated,
    RejectedTrailers,
    RetryFailed,
    RetryCancelled,
}

macro_rules! auth_stream_tests {
    ($($name:ident: $target:ident, $finish:ident;)*) => {
        $(#[tokio::test]
        async fn $name() {
            digest_stream(RequestAuthTarget::$target, Finish::$finish, "GET").await;
        })*
    };
}

auth_stream_tests! {
    digest_server_streams_final_head_and_body: Server, Complete;
    digest_server_stream_preserves_partial_failure: Server, Truncated;
    digest_server_stream_cancels_after_prefix: Server, Cancelled;
    digest_server_stream_retains_final_rejection: Server, Rejected;
    digest_server_stream_retains_truncated_rejection: Server, RejectedTruncated;
    digest_server_stream_retains_rejection_with_trailers: Server, RejectedTrailers;
    digest_server_retry_fails_before_head: Server, RetryFailed;
    digest_server_retry_cancels_before_head: Server, RetryCancelled;
    digest_proxy_streams_final_head_and_body: Proxy, Complete;
    digest_proxy_stream_preserves_partial_failure: Proxy, Truncated;
    digest_proxy_stream_cancels_after_prefix: Proxy, Cancelled;
    digest_proxy_stream_retains_final_rejection: Proxy, Rejected;
    digest_proxy_stream_retains_truncated_rejection: Proxy, RejectedTruncated;
    digest_proxy_stream_retains_rejection_with_trailers: Proxy, RejectedTrailers;
    digest_proxy_retry_fails_before_head: Proxy, RetryFailed;
    digest_proxy_retry_cancels_before_head: Proxy, RetryCancelled;
}

#[tokio::test]
async fn digest_server_replays_post_body_and_streams_response() {
    digest_stream(RequestAuthTarget::Server, Finish::Complete, "POST").await;
}

#[tokio::test]
async fn digest_proxy_replays_post_body_and_streams_response() {
    digest_stream(RequestAuthTarget::Proxy, Finish::Complete, "POST").await;
}

async fn digest_stream(target: RequestAuthTarget, finish: Finish, method: &'static str) {
    let rejected = matches!(
        finish,
        Finish::Rejected | Finish::RejectedTruncated | Finish::RejectedTrailers
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (authenticated, authentication) = oneshot::channel();
    let (head, release_head) = oneshot::channel();
    let (chunk, release_chunk) = oneshot::channel();
    let (tail, release_tail) = oneshot::channel();
    let (status, authenticate, authorization) = match target {
        RequestAuthTarget::Server => (401, "WWW-Authenticate", "authorization"),
        RequestAuthTarget::Proxy => (407, "Proxy-Authenticate", "proxy-authorization"),
        RequestAuthTarget::ProxyHeader => unreachable!(),
    };
    let challenge = format!(
        "{authenticate}: Digest realm=\"moli\", nonce=\"fixed-nonce\", qop=\"auth\", algorithm=MD5\r\n"
    );
    let server = tokio::spawn(async move {
        let (mut initial, _) = listener.accept().await.unwrap();
        let request = read_request(&mut initial).await;
        read_body(&mut initial, &request).await;
        assert!(!request.lines().any(|line| {
            line.to_ascii_lowercase()
                .starts_with(&format!("{authorization}:"))
        }));
        initial.write_all(format!(
            "HTTP/1.1 {status} Challenge\r\n{challenge}Content-Length: 9\r\nConnection: close\r\n\r\nchallenge"
        ).as_bytes()).await.unwrap();
        drop(initial);
        let (mut response, _) = listener.accept().await.unwrap();
        let request = read_request(&mut response).await;
        let body = read_body(&mut response, &request).await;
        assert_eq!(
            body,
            if method == "POST" {
                vec![0, 255, 1]
            } else {
                Vec::new()
            }
        );
        let credentials = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case(authorization)
                    .then_some(value.trim())
            })
            .expect("libcurl must answer the actual authentication challenge");
        assert!(credentials.starts_with("Digest "));
        assert!(credentials.contains("username=\"user\""));
        assert!(credentials.contains("nonce=\"fixed-nonce\""));
        assert!(credentials.contains("response=\""));
        authenticated.send(()).unwrap();
        if release_head.await.is_err() {
            expect_peer_close(&mut response).await;
            return;
        }
        if finish == Finish::RetryFailed {
            return;
        }
        if rejected {
            let (framing, body) = match finish {
                Finish::RejectedTruncated => ("Content-Length: 6", "den"),
                Finish::RejectedTrailers => (
                    "Transfer-Encoding: chunked",
                    "6\r\ndenied\r\n0\r\nX-Trailer: end\r\n\r\n",
                ),
                _ => ("Content-Length: 6", "denied"),
            };
            response.write_all(format!(
                "HTTP/1.1 {status} Rejected\r\n{challenge}X-Final: yes\r\n{framing}\r\nConnection: close\r\n\r\n{body}"
            ).as_bytes()).await.unwrap();
            return;
        }
        response.write_all(b"HTTP/1.1 200 OK\r\nX-Final: yes\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
        if release_chunk.await.is_err() {
            return;
        }
        response.write_all(b"bo").await.unwrap();
        if release_tail.await.is_err() {
            expect_peer_close(&mut response).await;
            return;
        }
        if finish == Finish::Complete {
            response.write_all(b"dy").await.unwrap();
        }
    });

    let mut config = FetchConfig::default();
    let url = if target == RequestAuthTarget::Proxy {
        config.set_http_proxy(Some(origin));
        "http://example.test/resource".to_owned()
    } else {
        format!("{origin}/resource")
    };
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let handle = client.handle();
    let cancel = FetchCancelHandle::new();
    let transport_cancel = cancel.clone();
    let request = Request::new_bytes(
        method,
        &url,
        (method == "POST").then(|| vec![0, 255, 1]),
        Vec::new(),
    )
    .unwrap()
    .with_auth(RequestAuth {
        target,
        scheme: RequestAuthScheme::Digest,
        username: "user".into(),
        password: "pass".into(),
    });
    let pending = tokio::spawn(async move {
        handle
            .fetch_raw_stream_with_cancel(request, transport_cancel)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), authentication)
        .await
        .expect("Digest retry must reach the server")
        .unwrap();
    if finish == Finish::RetryCancelled {
        cancel.cancel();
        drop(head);
    } else {
        head.send(()).unwrap();
    }
    let result = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("final response head must precede the held body")
        .unwrap();
    if matches!(finish, Finish::RetryFailed | Finish::RetryCancelled) {
        assert!(
            result.is_err(),
            "a failed retry must not resurrect the intermediate challenge"
        );
        server.await.unwrap();
        return;
    }
    let mut response = result.unwrap();
    assert_eq!(response.head().status, if rejected { status } else { 200 });
    assert!(
        response
            .head()
            .headers
            .iter()
            .any(|(name, value)| name == "x-final" && value == b"yes"),
        "an intermediate challenge must not become the final response"
    );

    let mut body = Vec::new();
    if !rejected {
        chunk.send(()).unwrap();
        let prefix = tokio::time::timeout(Duration::from_secs(5), response.next_chunk())
            .await
            .expect("body prefix must precede its tail")
            .unwrap();
        assert_eq!(prefix, b"bo");
        if finish == Finish::Cancelled {
            cancel.cancel();
            drop(tail);
        } else {
            tail.send(()).unwrap();
        }
        body = prefix;
    }
    while let Some(bytes) = tokio::time::timeout(Duration::from_secs(5), response.next_chunk())
        .await
        .expect("stream must settle")
    {
        body.extend_from_slice(&bytes);
    }
    let result = response.finish().await;
    assert_eq!(
        result.is_ok(),
        matches!(
            finish,
            Finish::Complete | Finish::Rejected | Finish::RejectedTrailers
        ),
        "actual transport outcome: {result:?}"
    );
    assert_eq!(
        body,
        match finish {
            Finish::Complete => b"body".as_slice(),
            Finish::Truncated | Finish::Cancelled => b"bo".as_slice(),
            Finish::Rejected | Finish::RejectedTrailers => b"denied".as_slice(),
            Finish::RejectedTruncated => b"den".as_slice(),
            Finish::RetryFailed | Finish::RetryCancelled => unreachable!(),
        }
    );
    server.await.unwrap();
}

#[tokio::test]
async fn basic_credentials_stop_at_cross_origin_redirect() {
    credentials_stay_in_scope(RequestAuthScheme::Basic, true).await;
}

#[tokio::test]
async fn digest_credentials_stop_at_cross_origin_redirect() {
    credentials_stay_in_scope(RequestAuthScheme::Digest, true).await;
}

#[tokio::test]
async fn basic_credentials_respect_omit() {
    credentials_stay_in_scope(RequestAuthScheme::Basic, false).await;
}

#[tokio::test]
async fn digest_credentials_respect_omit() {
    credentials_stay_in_scope(RequestAuthScheme::Digest, false).await;
}

async fn credentials_stay_in_scope(scheme: RequestAuthScheme, redirect: bool) {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_url = format!("http://{}/target", destination.local_addr().unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let initial_url = if redirect {
        format!("http://{}/redirect", origin.local_addr().unwrap())
    } else {
        destination_url.clone()
    };
    let mut request = Request::get(&initial_url).unwrap();
    if !redirect {
        request = request.with_credentials_mode(RequestCredentialsMode::Omit);
    }
    request.set_auth(Some(RequestAuth {
        target: RequestAuthTarget::Server,
        scheme,
        username: "user".into(),
        password: "pass".into(),
    }));
    let server = tokio::spawn(async move {
        if redirect {
            let (mut source, _) = origin.accept().await.unwrap();
            read_request(&mut source).await;
            source.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {destination_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        }
        let (mut target, _) = destination.accept().await.unwrap();
        let request = read_request(&mut target).await;
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "credentials must remain within the request's origin and credentials mode"
        );
        target.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"moli\", nonce=\"other-origin\", qop=\"auth\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        client
            .handle()
            .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        response.head().status,
        401,
        "no automatic credential replay is allowed at this destination"
    );
    response.into_materialized_raw_response().await.unwrap();
    server.await.unwrap();
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

async fn read_body(stream: &mut tokio::net::TcpStream, head: &str) -> Vec<u8> {
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    body
}

async fn expect_peer_close(stream: &mut tokio::net::TcpStream) {
    // Keep the server connection open: an EOF caused by the fixture itself
    // would let a missing transport cancellation pass as a truncated response.
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte))
            .await
            .expect("cancellation must close the physical connection")
            .unwrap(),
        0
    );
}
