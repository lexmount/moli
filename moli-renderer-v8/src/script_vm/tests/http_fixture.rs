use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use url::Url;

use crate::network::{ResourceRequestClient, ResourceRequestClientOwner};

#[derive(Debug, Eq, PartialEq)]
pub(super) struct CapturedHttpRequest {
    pub(super) method: String,
    pub(super) host: Option<String>,
    pub(super) target: String,
    pub(super) headers: Vec<(String, String)>,
}

impl CapturedHttpRequest {
    pub(super) fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Small real-network fixture for standalone `ScriptVm` tests.
///
/// Resource-loading tests must exercise the same Tokio/libcurl path as
/// production. This server intentionally does not emulate a task runner or
/// completion queue; it only supplies deterministic HTTP responses and records
/// the requests that reached the transport boundary.
pub(super) struct StaticHttpServer {
    address: SocketAddr,
    task: Option<JoinHandle<Vec<CapturedHttpRequest>>>,
}

impl StaticHttpServer {
    pub(super) async fn spawn(expected_requests: usize) -> Self {
        const BODY: &str = "<!doctype html><body>child fixture</body>";
        Self::spawn_with_bodies(vec![BODY.to_owned(); expected_requests]).await
    }

    pub(super) async fn spawn_with_bodies(response_bodies: Vec<String>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind static HTTP test server");
        let address = listener
            .local_addr()
            .expect("read static HTTP test server address");
        let expected_requests = response_bodies.len();
        let task = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(expected_requests);
            for body in response_bodies {
                let (mut socket, _) = listener
                    .accept()
                    .await
                    .expect("accept static HTTP test request");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let count = socket
                        .read(&mut buffer)
                        .await
                        .expect("read static HTTP test request");
                    assert_ne!(
                        count, 0,
                        "static HTTP test request ended before its headers"
                    );
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }

                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .expect("static HTTP request header terminator")
                    + 4;
                let request_head = String::from_utf8_lossy(&request[..header_end]).into_owned();
                let content_length = request_head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while request.len() < header_end + content_length {
                    let count = socket
                        .read(&mut buffer)
                        .await
                        .expect("read static HTTP test request body");
                    assert_ne!(count, 0, "static HTTP test request body ended early");
                    request.extend_from_slice(&buffer[..count]);
                }

                let mut lines = request_head.lines();
                let request_line = lines.next().expect("static HTTP request line");
                let method = request_line
                    .split_ascii_whitespace()
                    .next()
                    .expect("static HTTP request method")
                    .to_owned();
                let target = request_line
                    .split_ascii_whitespace()
                    .nth(1)
                    .expect("static HTTP request target")
                    .to_owned();
                let headers = lines
                    .filter_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        Some((name.to_owned(), value.trim().to_owned()))
                    })
                    .collect::<Vec<_>>();
                let host = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("host"))
                    .map(|(_, value)| value.clone());
                requests.push(CapturedHttpRequest {
                    method,
                    host,
                    target,
                    headers,
                });

                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .expect("write static HTTP test response");
            }
            requests
        });

        Self {
            address,
            task: Some(task),
        }
    }

    pub(super) fn base_url(&self) -> Url {
        Url::parse(&format!("http://{}/", self.address)).expect("static HTTP test server base URL")
    }

    pub(super) fn url_for_host(&self, host: &str, path: &str) -> Url {
        Url::parse(&format!("http://{host}:{}{}", self.address.port(), path))
            .expect("static HTTP test server host URL")
    }

    pub(super) fn resolve_entry(&self, host: &str) -> String {
        format!("{host}:{}:127.0.0.1", self.address.port())
    }

    pub(super) async fn finish(mut self) -> Vec<CapturedHttpRequest> {
        self.task
            .take()
            .expect("static HTTP test server task")
            .await
            .expect("static HTTP test server should finish")
    }

    pub(super) async fn finish_targets(self) -> Vec<String> {
        self.finish()
            .await
            .into_iter()
            .map(|request| request.target)
            .collect()
    }
}

impl Drop for StaticHttpServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(super) fn static_http_loader(
    resolve_entries: impl IntoIterator<Item = String>,
) -> ResourceRequestClientOwner {
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_host_resolve(resolve_entries.into_iter().collect());
    config.set_http_no_proxy(Some("*".to_owned()));
    ResourceRequestClient::new(&config).expect("static HTTP fixture loader")
}
