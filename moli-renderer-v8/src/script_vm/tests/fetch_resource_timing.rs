use super::*;
mod dom_requests;
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
    let is_preflight = request.starts_with("OPTIONS ");
    let gate = {
        let mut state = state.lock();
        state.seen.insert(key.clone());
        if is_preflight {
            state.seen.insert(format!("OPTIONS:{key}"));
        }
        state.gates.entry(key).or_default().clone()
    };
    if is_preflight {
        if get("preflight-gate") == "1" {
            gate.notified().await;
        }
        let _ = socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: x-timing\r\nConnection: close\r\n\r\n").await;
        return;
    }
    if url.path() == "/fail" {
        return;
    }
    use base64::Engine;
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jL1kAAAAASUVORK5CYII=";
    let mut body = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    let mut mime = "image/png";
    if url.path() == "/frame-document" {
        body = b"<!doctype html><title>frame timing</title>".to_vec();
        mime = "text/html";
    } else if url.path() == "/release" {
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

fn exec_resource_initiator_script(vm: &mut ScriptVm, origin: &str, name: &str, source: &str) {
    vm.exec_in_enclosing_script_turn_with_provenance(
        source,
        &crate::script_provenance::CompiledStringProvenance::at_url(
            Url::parse(&format!("{origin}/{name}.js")).unwrap(),
        ),
        0,
        None,
        true,
    )
    .map_err(|error| error.into_anyhow())
    .unwrap();
}

fn callback_resource_initiators(vm: &mut ScriptVm) -> serde_json::Value {
    serde_json::from_str(
        &vm.eval(
            r#"JSON.stringify(Object.fromEntries(
        performance.getEntriesByType('resource').map(entry => [
            new URL(entry.name).searchParams.get('case'), entry.initiatorUrl
        ])
    ))"#,
        )
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn resource_timing_callback_timers_retain_each_registration_source() {
    let server = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/page.html", server.origin),
        &loader,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "library",
        r#"
        globalThis.completed = [];
        function requestFromShared(label) {
            return fetch('/probe?case=' + label).then(response => response.arrayBuffer())
                .then(() => completed.push(label));
        }
    "#,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "timer-a",
        "setTimeout(requestFromShared, 0, 'timer-a');",
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "timer-b",
        "setTimeout(requestFromShared, 0, 'timer-b');",
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "timer-c",
        r#"
        const intervalId = setInterval(() => {
            clearInterval(intervalId); requestFromShared('interval');
        }, 0);
        setTimeout("requestFromShared('string')", 0);
    "#,
    );
    page.eval("setTimeout(requestFromShared, 0, 'unknown');")
        .unwrap();
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "later",
        "globalThis.laterScriptExecuted = true;",
    );
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(completed.length)",
        "5",
        "callback initiator timers",
    )
    .await;
    assert_eq!(
        callback_resource_initiators(&mut page),
        serde_json::json!({
            "timer-a": format!("{}/timer-a.js", server.origin),
            "timer-b": format!("{}/timer-b.js", server.origin),
            "interval": format!("{}/timer-c.js", server.origin),
            "string": format!("{}/timer-c.js", server.origin),
            "unknown": "",
        })
    );
}

#[tokio::test]
async fn resource_timing_callback_xhr_handler_reassignment_captures_the_new_registration() {
    let server = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/page.html", server.origin),
        &loader,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "library",
        r#"
        globalThis.completed = false;
        function sharedLoadCallback() {
            Promise.resolve().then(() => fetch('/probe?case=handler'))
                .then(response => response.arrayBuffer()).then(() => completed = true);
        }
    "#,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "request",
        r#"
        globalThis.seed = new XMLHttpRequest();
        seed.onload = sharedLoadCallback;
        seed.open('GET', '/probe?case=seed'); seed.send();
    "#,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "replacement",
        "seed.onload = sharedLoadCallback;",
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "later",
        "globalThis.laterScriptExecuted = true;",
    );
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(completed)",
        "true",
        "XHR handler registration initiator",
    )
    .await;
    assert_eq!(
        callback_resource_initiators(&mut page),
        serde_json::json!({
            "seed": format!("{}/request.js", server.origin),
            "handler": format!("{}/replacement.js", server.origin),
        })
    );
}

#[tokio::test]
async fn resource_timing_callback_synchronous_dispatch_preserves_known_and_unknown_callers() {
    let server = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/page.html", server.origin),
        &loader,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "registration",
        r#"
        globalThis.completed = [];
        globalThis.target = new EventTarget();
        target.addEventListener('probe', () => {
            const label = globalThis.currentLabel;
            fetch('/probe?case=' + label).then(response => response.arrayBuffer())
                .then(() => completed.push(label));
        });
    "#,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "caller",
        "globalThis.currentLabel = 'known'; target.dispatchEvent(new Event('probe'));",
    );
    page.eval("globalThis.currentLabel = 'unknown'; target.dispatchEvent(new Event('probe'));")
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(completed.length)",
        "2",
        "synchronous event caller initiators",
    )
    .await;
    assert_eq!(
        callback_resource_initiators(&mut page),
        serde_json::json!({
            "known": format!("{}/caller.js", server.origin),
            "unknown": "",
        })
    );
}

#[tokio::test]
async fn resource_timing_callback_interface_lookup_and_its_promises_retain_the_registration() {
    let server = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/page.html", server.origin),
        &loader,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "library",
        r#"
        globalThis.completed = [];
        globalThis.listener = {
            get handleEvent() {
                fetch('/probe?case=lookup').then(response => response.arrayBuffer())
                    .then(() => completed.push('lookup'));
                return function() {
                    Promise.resolve().then(() => fetch('/probe?case=body'))
                        .then(response => response.arrayBuffer()).then(() => completed.push('body'));
                };
            }
        };
    "#,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "registration",
        r#"
        globalThis.seed = new XMLHttpRequest();
        seed.addEventListener('load', listener);
        seed.open('GET', '/probe?case=seed'); seed.send();
    "#,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "later",
        "globalThis.laterScriptExecuted = true;",
    );
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(completed.length)",
        "2",
        "callback-interface lookup initiator",
    )
    .await;
    assert_eq!(
        callback_resource_initiators(&mut page),
        serde_json::json!({
            "seed": format!("{}/registration.js", server.origin),
            "lookup": format!("{}/registration.js", server.origin),
            "body": format!("{}/registration.js", server.origin),
        })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn window_fetch_reports_native_resource_timing_at_body_terminal() {
    run_resource_timing_fixture(
        include_str!("../../../tests/fixtures/fetch-resource-timing.js"),
        36,
    )
    .await;
}

#[test]
fn resource_timing_initiator_retains_the_triggering_script_until_publication() {
    let mut vm = new_storage_test_vm("https://resource-initiator.test/page.html");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
    for (url, source) in [
        (
            "https://resource-initiator.test/library.js",
            "function requestFromLibrary() { const xhr = new XMLHttpRequest(); xhr.open('GET', '/data'); xhr.send(); }",
        ),
        (
            "https://resource-initiator.test/caller.js",
            "requestFromLibrary();",
        ),
        (
            "https://resource-initiator.test/later.js",
            "globalThis.laterScriptExecuted = true;",
        ),
    ] {
        vm.exec_in_enclosing_script_turn_with_provenance(
            source,
            &crate::script_provenance::CompiledStringProvenance::at_url(Url::parse(url).unwrap()),
            0,
            None,
            true,
        )
        .map_err(|error| error.into_anyhow())
        .expect("the resource initiator scripts should execute");
    }
    let requests = vm.take_pending_subresource_fetch_infos();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: request.internal_id,
        request_url: request.url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Default::default(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            request.url.clone(),
            200,
            vec![("content-type".to_owned(), b"text/plain".to_vec())],
            "body".to_owned(),
        ))
        .into(),
    })
    .expect("the initiating request should complete");
    assert_eq!(
        vm.eval("String(performance.getEntriesByType('resource')[0].initiatorUrl)")
            .unwrap(),
        "https://resource-initiator.test/caller.js",
        "resource timing must capture the caller, not the callee or publication-time script"
    );
    assert_eq!(
        vm.eval("performance.getEntriesByType('resource')[0].toJSON().initiatorUrl")
            .unwrap(),
        "https://resource-initiator.test/caller.js"
    );
}

#[test]
fn resource_timing_initiator_survives_promises_and_keeps_its_source_url() {
    let mut vm = new_storage_test_vm("https://resource-initiator.test/page.html");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.exec_in_enclosing_script_turn_with_provenance(
        "new Promise(resolve => { globalThis.releaseInitiator = resolve; }).then(async () => { await new Promise(resolve => queueMicrotask(resolve)); return fetch('/data'); });",
        &crate::script_provenance::CompiledStringProvenance::new(
            Url::parse("https://resource-initiator.test/caller.js").unwrap(),
            Url::parse("https://resolution-base.test/different/").unwrap(),
        ),
        0, None, true,
    ).map_err(|error| error.into_anyhow()).unwrap();
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    vm.exec_in_enclosing_script_turn_with_provenance(
        "releaseInitiator();",
        &crate::script_provenance::CompiledStringProvenance::at_url(
            Url::parse("https://resource-initiator.test/later.js").unwrap(),
        ),
        0,
        None,
        true,
    )
    .map_err(|error| error.into_anyhow())
    .unwrap();
    let requests = vm.take_pending_subresource_fetch_infos();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url.as_str(), "https://resource-initiator.test/data");
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: request.internal_id,
        request_url: request.url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Default::default(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            request.url.clone(),
            200,
            vec![("content-type".to_owned(), b"text/plain".to_vec())],
            "body".to_owned(),
        ))
        .into(),
    })
    .unwrap();
    assert_eq!(
        vm.eval("performance.getEntriesByType('resource')[0].initiatorUrl")
            .unwrap(),
        "https://resource-initiator.test/caller.js",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
        const entry = performance.getEntriesByType('resource')[0];
        Object.defineProperty(entry, 'initiatorUrl', {get() { throw Error('author getter'); }});
        return entry.toJSON().initiatorUrl;
    })()"#
        )
        .unwrap(),
        "https://resource-initiator.test/caller.js"
    );
}

#[test]
fn http_cached_fetches_with_same_url_keep_separate_resource_timing_entries() {
    let mut vm = new_storage_test_vm("https://resource-cache.test/page.html");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    for _ in 0..2 {
        vm.eval("fetch('/cached.txt').then(response => response.text());")
            .unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        let mut response = crate::types::NavigationResponse::from_text_body(
            request.url.clone(),
            200,
            vec![("content-type".into(), b"text/plain".to_vec())],
            "cached".into(),
        );
        response.from_cache = true;
        response.cache_state = moli_fetch::ResponseCacheState::Local;
        vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
            internal_id: request.internal_id,
            request_url: request.url.clone(),
            request_method: "GET".into(),
            request_headers: Default::default(),
            request_body: None,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            result: Ok(response).into(),
        })
        .unwrap();
    }
    assert_eq!(
        vm.eval(
            r#"JSON.stringify(performance.getEntriesByType('resource').map(entry =>
                [entry.name, entry.initiatorType, entry.deliveryType, entry.transferSize]))"#,
        )
        .unwrap(),
        r#"[["https://resource-cache.test/cached.txt","fetch","cache",0],["https://resource-cache.test/cached.txt","fetch","cache",0]]"#
    );
}

#[test]
fn resource_timing_initiator_requires_a_known_source_and_readable_network_response() {
    for streaming in [false, true] {
        for scenario in [
            "same",
            "cors",
            "opaque-tao",
            "cache",
            "synthetic",
            "muted",
            "unknown",
        ] {
            let mut vm = new_storage_test_vm("https://resource-initiator.test/page.html");
            vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Fetch),
            );
            let target = if matches!(scenario, "cors" | "opaque-tao") {
                "https://other-initiator.test/data"
            } else {
                "https://resource-initiator.test/data"
            };
            let mode = if scenario == "opaque-tao" {
                "no-cors"
            } else {
                "cors"
            };
            let source =
                format!("fetch('{target}', {{mode: '{mode}'}}).then(response => response.text());");
            if scenario == "unknown" {
                vm.eval(&source).unwrap();
            } else {
                let metadata = crate::planning::ScriptFetchMetadata {
                    muted_errors: scenario == "muted",
                    ..Default::default()
                };
                vm.exec_in_enclosing_script_turn_with_provenance(
                    &source,
                    &crate::script_provenance::CompiledStringProvenance::at_url(
                        Url::parse("https://resource-initiator.test/caller.js").unwrap(),
                    ),
                    0,
                    Some(&metadata),
                    true,
                )
                .map_err(|error| error.into_anyhow())
                .unwrap();
            }
            let requests = vm.take_pending_subresource_fetch_infos();
            assert_eq!(requests.len(), 1, "{scenario}, streaming={streaming}");
            let request = &requests[0];
            let mut headers = vec![("content-type".to_owned(), b"text/plain".to_vec())];
            if scenario == "cors" {
                headers.push(("access-control-allow-origin".to_owned(), b"*".to_vec()));
            }
            if scenario == "opaque-tao" {
                headers.push(("timing-allow-origin".to_owned(), b"*".to_vec()));
            }
            let mut response = crate::types::NavigationResponse::from_text_body(
                request.url.clone(),
                200,
                headers,
                "body".to_owned(),
            );
            if scenario == "cache" {
                response.from_cache = true;
                response.cache_state = moli_fetch::ResponseCacheState::Local;
            }
            let synthetic = scenario == "synthetic";
            if streaming {
                let body_source_id = crate::network_host::new_network_body_source_id();
                vm.start_streaming_async_subresource_fetch(
                    crate::types::AsyncSubresourceStreamingStarted {
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Default::default(),
                        request_body: None,
                        skip_fetch_security_validation: synthetic,
                        response_filter: None,
                        body_source_id,
                        head: response.head(),
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
                        response_status_text: None,
                        skip_fetch_security_validation: synthetic,
                        response_filter: None,
                        network_error_text: None,
                        result: Ok(response).into(),
                    },
                )
                .unwrap();
            }
            let expected = if matches!(scenario, "same" | "cors") {
                "https://resource-initiator.test/caller.js"
            } else {
                ""
            };
            assert_eq!(
                vm.eval("performance.getEntriesByType('resource')[0].initiatorUrl")
                    .unwrap(),
                expected,
                "{scenario}, streaming={streaming}",
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn resource_connection_phases_survive_lazy_realms_and_native_json() {
    run_resource_timing_fixture(
        include_str!("../../../tests/fixtures/resource-timing-connection-phases.js"),
        38,
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

#[tokio::test(flavor = "current_thread")]
async fn iframe_attribute_navigation_retains_resource_timing_provenance() {
    run_resource_timing_fixture(
        include_str!("../../../tests/fixtures/iframe-attribute-resource-timing.js"),
        58,
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
                request_include_credentials: false,
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
