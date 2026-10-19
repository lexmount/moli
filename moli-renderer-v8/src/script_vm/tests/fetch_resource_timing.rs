use super::*;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct TimingServerState {
    seen: HashSet<String>,
    gates: HashMap<String, Arc<tokio::sync::Notify>>,
}

pub(super) struct TimingServer {
    pub(super) origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TimingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn timing_server() -> TimingServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(TimingServerState::default()));
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let state = state.clone();
            connections.spawn(handle_timing_request(socket, state));
            while let Some(result) = connections.try_join_next() {
                result.unwrap();
            }
        }
    });
    TimingServer { origin, task }
}

async fn handle_timing_request(
    mut socket: tokio::net::TcpStream,
    state: Arc<Mutex<TimingServerState>>,
) {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0; 2048];
        let count = socket.read(&mut buffer).await.unwrap();
        if count == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8(request).unwrap();
    let target = request.split_whitespace().nth(1).unwrap();
    let url = url::Url::parse(&format!("http://fixture.test{target}")).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let get = |name: &str| query.get(name).map(String::as_str).unwrap_or("");
    if url.path() == "/compressed-worker.js" {
        let body = r#"self.onmessage = async event => {
            const probe = (0, eval)('(' + event.data.source + ')');
            postMessage(await probe(event.data.origins, event.data.label, event.data.filter));
            close();
        };"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(body.as_bytes()).await;
        return;
    }
    if url.path() == "/compressed" {
        const GZIP: &[u8] = &[
            31, 139, 8, 0, 0, 0, 0, 0, 2, 3, 75, 76, 74, 6, 0, 194, 65, 36, 53, 3, 0, 0, 0,
        ];
        const EMPTY: &[u8] = &[
            31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        const OPAQUE_PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31,
            0, 3, 3, 2, 0, 239, 163, 47, 89, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        const OPAQUE_GZIP: &[u8] = &[
            31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 235, 12, 240, 115, 231, 229, 146, 226, 98, 96, 96,
            224, 245, 244, 112, 9, 2, 210, 140, 32, 204, 193, 2, 36, 183, 202, 240, 48, 1, 41, 110,
            79, 23, 199, 144, 138, 91, 201, 127, 254, 203, 51, 48, 51, 51, 49, 188, 95, 172, 31, 9,
            20, 102, 240, 116, 245, 115, 89, 231, 148, 208, 4, 0, 122, 254, 219, 225, 68, 0, 0, 0,
        ];
        const ZSTD: &[u8] = &[40, 181, 47, 253, 32, 3, 25, 0, 0, 97, 98, 99];
        const ZSTD_EMPTY: &[u8] = &[40, 181, 47, 253, 32, 0, 1, 0, 0];
        const OPAQUE_ZSTD: &[u8] = &[
            40, 181, 47, 253, 32, 68, 33, 2, 0, 137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73,
            72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68,
            65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2, 0, 239, 163, 47, 89, 0, 0, 0, 0, 73,
            69, 78, 68, 174, 66, 96, 130,
        ];
        let body = match (get("opaque"), get("kind")) {
            ("1", "gzip") => OPAQUE_GZIP,
            ("1", "identity") => OPAQUE_PNG,
            ("1", "zstd") => OPAQUE_ZSTD,
            (_, "zstd") => ZSTD,
            (_, "zstd-empty") => ZSTD_EMPTY,
            (_, "gzip") => GZIP,
            (_, "empty") => EMPTY,
            _ => b"abc",
        };
        let validate =
            get("cache") == "1" && request.to_ascii_lowercase().contains("if-none-match:");
        let status = if validate { 304 } else { 200 };
        let chunked = !validate && get("chunked") == "1";
        let length = if validate { 0 } else { body.len() };
        let framing = if chunked {
            "Transfer-Encoding: chunked".to_owned()
        } else {
            format!("Content-Length: {length}")
        };
        let mime = if get("opaque") == "1" {
            "image/png"
        } else {
            "text/plain"
        };
        let mut headers = format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\n{framing}\r\nConnection: close\r\n"
        );
        if get("kind") != "identity" && !validate {
            let encoding = if get("kind").starts_with("zstd") {
                "zstd"
            } else {
                "gzip"
            };
            headers.push_str(&format!("Content-Encoding: {encoding}\r\n"));
        }
        headers.push_str(if get("cache") == "1" {
            "Cache-Control: max-age=600\r\nETag: \"v1\"\r\n"
        } else {
            "Cache-Control: no-store\r\n"
        });
        for (query, header) in [
            ("cors", "Access-Control-Allow-Origin"),
            ("tao", "Timing-Allow-Origin"),
        ] {
            if !get(query).is_empty() {
                headers.push_str(&format!("{header}: {}\r\n", get(query)));
            }
        }
        headers.push_str("\r\n");
        if socket.write_all(headers.as_bytes()).await.is_err()
            || request.starts_with("HEAD ")
            || validate
        {
            return;
        }
        if chunked {
            if socket
                .write_all(format!("{:x};test=value\r\n", body.len()).as_bytes())
                .await
                .is_err()
            {
                return;
            }
            let _ = socket.write_all(body).await;
            let _ = socket
                .write_all(b"\r\n0\r\nX-Trailer: present\r\n\r\n")
                .await;
        } else {
            let _ = socket.write_all(body).await;
        }
        return;
    }
    let key = get("key").to_owned();
    let gate = {
        let mut state = state.lock();
        state.seen.insert(key.clone());
        state.gates.entry(key).or_default().clone()
    };
    if url.path() == "/fail" {
        return;
    }
    use base64::Engine;
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jL1kAAAAASUVORK5CYII=";
    let mut body = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    let mut mime = "image/png";
    if url.path() == "/release" {
        gate.notify_one();
        body = b"ok".to_vec();
        mime = "text/plain";
    } else if url.path() == "/seen" {
        body = state
            .lock()
            .seen
            .contains(get("target"))
            .to_string()
            .into_bytes();
        mime = "application/json";
    }
    if get("gate") == "before" {
        gate.notified().await;
    }
    let redirect = url.path() == "/redirect";
    if redirect {
        body.clear();
    }
    let status = get("status")
        .parse::<u16>()
        .unwrap_or(if redirect { 302 } else { 200 });
    if matches!(status, 204 | 205) {
        body.clear();
    }
    let mut headers = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if url.path() == "/mime" {
        for (_, value) in url.query_pairs().filter(|(name, _)| name == "value") {
            headers.push_str(&format!("Content-Type: {value}\r\n"));
        }
    } else {
        headers.push_str(&format!("Content-Type: {mime}\r\n"));
    }
    if redirect {
        headers.push_str(&format!("Location: {}\r\n", get("to")));
    }
    for (query_name, header_name) in [
        ("cors", "Access-Control-Allow-Origin"),
        ("tao", "Timing-Allow-Origin"),
        ("tao2", "Timing-Allow-Origin"),
    ] {
        if !get(query_name).is_empty() {
            headers.push_str(&format!("{header_name}: {}\r\n", get(query_name)));
        }
    }
    headers.push_str("\r\n");
    if socket.write_all(headers.as_bytes()).await.is_err() {
        return;
    }
    if request.starts_with("HEAD ") {
        return;
    }
    if get("truncate") == "1" {
        body.truncate(4);
    }
    if get("gate") == "body" {
        if socket.write_all(&body[..4]).await.is_err() {
            return;
        }
        gate.notified().await;
        body.drain(..4);
    }
    let _ = socket.write_all(&body).await;
}

#[tokio::test(flavor = "current_thread")]
async fn window_fetch_reports_native_resource_timing_at_body_terminal() {
    run_resource_timing_fixture(
        include_str!("../../../tests/fixtures/fetch-resource-timing.js"),
        36,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn resource_timing_minimizes_response_content_type() {
    run_resource_timing_fixture(
        include_str!("../../../tests/fixtures/resource-timing-content-type.js"),
        44,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn window_requests_report_resource_timing_for_success_and_network_errors() {
    run_resource_timing_fixture(
        include_str!("../../../tests/fixtures/window-request-resource-timing.js"),
        38,
    )
    .await;
}

#[test]
fn xhr_resource_timing_reads_internal_headers_before_cors_exposure() {
    for streaming in [false, true] {
        for tao in [false, true] {
            let mut vm = new_storage_test_vm("https://xhr-timing-origin.test/");
            vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Xhr),
            );
            vm.eval(
                "globalThis.xhr = new XMLHttpRequest(); xhr.open('GET', 'https://xhr-timing-target.test/data'); xhr.send();",
            ).unwrap();
            let pending = vm.take_pending_subresource_fetch_infos();
            assert_eq!(pending.len(), 1);
            let request = &pending[0];
            let mut headers = vec![
                ("content-type".to_owned(), b"text/plain".to_vec()),
                ("content-encoding".to_owned(), b"identity".to_vec()),
                ("access-control-allow-origin".to_owned(), b"*".to_vec()),
            ];
            if tao {
                headers.push(("timing-allow-origin".to_owned(), b"*".to_vec()));
            }
            let head = moli_fetch::ResponseHead {
                final_url: request.url.clone(),
                status: 200,
                status_text: None,
                headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
                negotiated_http_version: None,
            };
            if streaming {
                let body_source_id = crate::network_host::new_network_body_source_id();
                vm.start_streaming_async_subresource_fetch(
                    crate::types::AsyncSubresourceStreamingStarted {
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Default::default(),
                        request_body: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        body_source_id,
                        head,
                        network_request_headers: None,
                    },
                )
                .unwrap();
                vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"body".to_vec());
                vm.finish_streaming_async_subresource_fetch(
                    request.internal_id,
                    body_source_id,
                    Ok(()),
                )
                .unwrap();
            } else {
                vm.complete_async_subresource_fetch(
                    crate::types::AsyncSubresourceFetchCompletion {
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Default::default(),
                        request_body: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        response_status_text: None,
                        network_error_text: None,
                        result: Ok(
                            crate::protocol_types::NavigationResponse::from_head_and_body(
                                head,
                                "body".to_owned(),
                                b"body".to_vec(),
                            ),
                        )
                        .into(),
                    },
                )
                .unwrap();
            }
            let actual = vm.eval(r#"JSON.stringify((() => {
                const entries = performance.getEntriesByName(xhr.responseURL, 'resource');
                const entry = entries[0];
                return [entries.length, xhr.status, xhr.responseText,
                    xhr.getResponseHeader('content-encoding'), xhr.getResponseHeader('timing-allow-origin'),
                    entry.initiatorType, entry.responseStatus, entry.contentType, entry.contentEncoding,
                    entry.encodedBodySize, entry.decodedBodySize, entry.transferSize];
            })())"#).unwrap();
            let expected = serde_json::json!([
                1,
                200,
                "body",
                null,
                null,
                "xmlhttprequest",
                200,
                "text/plain",
                "@unknown",
                4,
                4,
                if tao { 304 } else { 0 }
            ]);
            assert_eq!(
                actual,
                expected.to_string(),
                "streaming={streaming}, tao={tao}"
            );
        }
    }
}

async fn run_resource_timing_fixture(fixture: &str, expected_total: usize) {
    run_resource_timing_fixture_scenario(fixture, expected_total, None).await;
}

async fn run_resource_timing_fixture_scenario(
    fixture: &str,
    expected_total: usize,
    scenario: Option<&serde_json::Value>,
) -> serde_json::Value {
    let first = timing_server().await;
    let second = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", first.origin), &loader);
    page.eval(&format!(
        "globalThis.__fetchOrigins = {}; globalThis.__fetchTimingResult = null;globalThis.__compressedTimingScenario = {};",
        serde_json::to_string(&[&first.origin, &second.origin]).unwrap(),
        serde_json::to_string(&scenario).unwrap(),
    ))
    .unwrap();
    page.eval(&format!(
        "({}).then(value => __fetchTimingResult = value, error => __fetchTimingResult = String(error));",
        fixture.trim().trim_end_matches(';'),
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(__fetchTimingResult !== null)",
        "true",
        "fetch Resource Timing",
    )
    .await;
    let result = page.eval("__fetchTimingResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result)
        .unwrap_or_else(|error| panic!("fetch timing probe returned {result:?}: {error}"));
    assert_eq!(result["total"], expected_total);
    assert_eq!(result["failures"], serde_json::json!([]));
    result
}

#[tokio::test(flavor = "current_thread")]
async fn compressed_resource_timing_sizes_survive_window_child_worker_and_memory_cache() {
    let mut names = std::collections::BTreeSet::new();
    for realm in ["window", "child", "worker"] {
        let apis = if realm == "worker" {
            ["fetch", "xhr", "sync-xhr"].as_slice()
        } else {
            ["fetch", "xhr"].as_slice()
        };
        for api in apis {
            let modes = if *api == "fetch" {
                [
                    "same",
                    "cors",
                    "cors-tao",
                    "denied",
                    "opaque",
                    "opaque-tao",
                    "cache",
                ]
                .as_slice()
            } else {
                ["same", "cors", "cors-tao", "denied", "cache"].as_slice()
            };
            for mode in modes {
                let scenario = serde_json::json!({"realm": realm, "api": api, "mode": mode});
                let total = if *mode == "cache" { 8 } else { 15 };
                let result = run_resource_timing_fixture_scenario(
                    include_str!("../../../tests/fixtures/compressed-request-resource-timing.js"),
                    total,
                    Some(&scenario),
                )
                .await;
                let checks = result["names"].as_array().unwrap();
                assert_eq!(checks.len(), total, "{scenario}");
                for check in checks {
                    let name = check.as_str().unwrap();
                    assert!(
                        name.starts_with(&format!("{realm}: {api}/")),
                        "{scenario}: {name}"
                    );
                    assert!(names.insert(name.to_owned()), "duplicate check: {name}");
                }
            }
        }
    }
    assert_eq!(
        names.len(),
        566,
        "all Window, child, worker, coding, API and cache cases must run"
    );
}
