use super::*;
use crate::script_vm::{RuntimeEvaluateCodeGenerationPolicy, RuntimeEvaluateResultMode};

fn native_element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
    vm.document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.attribute("id") == Some(id)).then_some(DomHandle::new(index))
        })
        .unwrap_or_else(|| panic!("element #{id} should have a native handle"))
}
use moli_browser_profile::{
    DEFAULT_USER_AGENT, DEFAULT_WINDOW_SURFACE_PROFILE, navigator_app_version,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

use super::service_worker_drain::{
    drain_service_worker_test_turn, drain_service_worker_test_until_eval_equals,
    drain_service_worker_test_until_popup_loads_settle,
    run_service_worker_client_focus_request_task_for_test,
    run_service_worker_client_navigate_request_task_for_test,
    run_service_worker_clients_open_window_request_task_for_test,
};

fn service_worker_window_client_target_for_test(
    client_id: crate::runtime::ServiceWorkerClientId,
    document_owner: crate::native_bridge::WindowDocumentOwner,
) -> crate::types::ServiceWorkerWindowClientTarget {
    crate::types::ServiceWorkerWindowClientTarget {
        client_id,
        document_owner,
    }
}

fn service_worker_csp_report_seen(
    items: &[crate::types::ScriptNetworkOutputItem],
    report_url: &str,
    expected_body: &str,
) -> bool {
    let report_handle = items.iter().find_map(|item| {
        let crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item else {
            return None;
        };
        (request.resource_type() == crate::types::SubresourceResourceType::CspReport
            && request.url().as_str() == report_url)
            .then(|| request.handle())
    });
    let report_body_seen = report_handle.is_some_and(|handle| {
        items.iter().any(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(body)
                    if body.handle() == handle
                        && matches!(
                            body.result(),
                            crate::types::SubresourceBodyFinishedResult::Ready(response_body)
                                if response_body.diagnostic_bytes().as_ref() == expected_body.as_bytes()
                        )
            )
        })
    });
    let report_record_seen = items.iter().any(|item| {
        matches!(
            item,
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                if record.url().as_str() == report_url
                    && record.resource_type() == crate::types::SubresourceResourceType::CspReport
                    && matches!(
                        record.outcome(),
                        crate::types::SubresourceNetworkOutcome::Success {
                            status: 200,
                            final_url,
                            response_body,
                            ..
                        } if final_url.as_str() == report_url
                            && response_body.diagnostic_bytes().as_ref() == expected_body.as_bytes()
                    )
        )
    });
    report_body_seen || report_record_seen
}

async fn eval_font_loading_fixture(url: &str, script: &str) -> String {
    use base64::Engine as _;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page =
        crate::runtime::PageVmTaskExecutorTestHarness::new(url::Url::parse(url).unwrap(), &loader);
    let source = format!(
        "url(data:font/ttf;base64,{})",
        base64::engine::general_purpose::STANDARD.encode(include_bytes!(
            "../../../../../moli-layout/tests/fixtures/moli-ahem.ttf"
        ))
    );
    page.eval(&format!(
        "globalThis.fontFixtureSource = {};",
        serde_json::to_string(&source).unwrap()
    ))
    .unwrap();
    page.eval(&format!("({script}).then(value => globalThis.fontFixtureResult = value, error => globalThis.fontFixtureResult = String(error.stack || error))")).unwrap();
    for _ in 0..64 {
        let result = page.eval("String(globalThis.fontFixtureResult)").unwrap();
        if result != "undefined" {
            return result;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut page, &loader)
            .await
            .unwrap();
    }
    panic!("font loading fixture did not settle");
}

async fn assert_popup_consecutive_history_back(from_popup: bool) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/page.html", &loader);
    vm.eval(&format!("globalThis.traverseFromPopup = {from_popup}"))
        .expect("traversal initiator should evaluate");
    vm.eval(
        r#"
        globalThis.popupTraversalLoads = [];
        globalThis.popupTraversalUrls = [0, 1, 2].map(index =>
            URL.createObjectURL(new Blob([`<!doctype html><script>
                onload = () => {
                    opener.popupTraversalLoads.push(${index});
                    if (${index} === 2 && opener.traverseFromPopup) {
                        history.go(-10);
                        history.back();
                        history.back();
                    }
                };
            <\/script>`], {type:'text/html'})));
        "#,
    )
    .expect("popup traversal fixtures should evaluate");
    for index in 0..3 {
        vm.eval(&format!(
            "globalThis.traversalPopup = open(popupTraversalUrls[{index}], 'consecutiveHistoryPopup')"
        ))
        .expect("popup navigation should evaluate");
        let expected = match index {
            0 => "0",
            1 => "0,1",
            _ if from_popup => "0,1,2,1",
            _ => "0,1,2",
        };
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "popupTraversalLoads.join(',')",
            expected,
            "popup documents should load in traversal order",
        )
        .await;
    }
    if !from_popup {
        vm.eval("traversalPopup.history.back(); traversalPopup.history.back()")
            .expect("the opener should request both traversals through the outgoing History");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "popupTraversalLoads.join(',')",
            "0,1,2,1",
            "only the first traversal from the outgoing popup History should commit",
        )
        .await;
    }
    assert_eq!(
        vm.eval("String(traversalPopup.location.href === popupTraversalUrls[1])")
            .expect("popup traversal destination should evaluate"),
        "true"
    );
    vm.eval("traversalPopup.history.back()")
        .expect("the restored Document's History should allow another traversal");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "popupTraversalLoads.join(',')",
        "0,1,2,1,0",
        "completed popup loads must release the pending traversal",
    )
    .await;
}

async fn assert_sandbox_child_about_blank_popup_reloads_self_and_messages_top(
    sandbox: &str,
    expected_popup_origin_prefix: &str,
) {
    let (helper_url, server) = spawn_sandbox_popup_helper_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let helper_url_parsed = Url::parse(&helper_url).expect("helper url");
    let document_url = format!(
        "http://127.0.0.1:{}/parent.html",
        helper_url_parsed
            .port()
            .expect("helper url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let helper_url_literal = serde_json::to_string(&helper_url).expect("serialize helper url");
    let sandbox_literal = serde_json::to_string(sandbox).expect("serialize sandbox");

    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__sandboxPopupMessages = [];
  addEventListener("message", event => {{
    __sandboxPopupMessages.push({{
      origin: event.origin,
      data: event.data,
      sourceIsFrame: event.source === frame.contentWindow
    }});
  }});
  const frame = document.createElement("iframe");
  frame.sandbox = {sandbox_literal};
  frame.src = {helper_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
        ))
        .expect("sandbox popup helper setup should evaluate");
    assert_eq!(setup, "queued");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__sandboxPopupMessages.length",
        "1",
        "sandbox popup message",
    )
    .await;
    server
        .await
        .expect("sandbox popup helper server should finish");

    let result = vm
        .eval("JSON.stringify(__sandboxPopupMessages)")
        .expect("sandbox popup messages should evaluate");
    if expected_popup_origin_prefix == "null" {
        assert_eq!(
            result,
            r#"[{"origin":"null","data":{"origin":"null","initialPopupAccessible":false},"sourceIsFrame":true}]"#
        );
    } else {
        assert!(
            result.contains(r#""origin":"null""#),
            "sandbox child should still message top with opaque origin: {result}"
        );
        assert!(
            result.contains(expected_popup_origin_prefix),
            "escaped popup should report non-opaque opener event origin: {result}"
        );
        assert!(
            result.contains(r#""initialPopupAccessible":true"#),
            "escaped popup's initial about:blank should retain its opener's origin: {result}"
        );
    }
}

async fn lightweight_popup_nested_eval_result(
    opener_policy: &'static str,
    popup_policy_header: &'static str,
    expected: &'static str,
) -> String {
    let (popup_url, server) = spawn_lightweight_popup_response_html_server(
        "popup nested eval policy test server",
        "popup nested eval policy",
        popup_policy_header,
        r#"<!doctype html><script>
trustedTypes.createPolicy("default", {
  createScript: (value, _type, sink) => sink === "Location href" ? value : null
});
onload = () => {
  location.href = `javascript:
    try {
      eval("globalThis.__popupNestedEvalRan = true");
      opener.postMessage("allowed", "*");
    } catch (error) {
      opener.postMessage(error.name, "*");
    }
  `;
};
</script>"#,
    )
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    vm.set_response_content_security_policies(&[opener_policy.to_owned()]);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    vm.eval(&format!(
        r#"
globalThis.__popupNestedEvalResult = "pending";
addEventListener("message", event => {{
  globalThis.__popupNestedEvalResult = String(event.data);
}});
open({popup_url_literal});
"#
    ))
    .expect("popup nested eval setup should evaluate");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup nested eval document completion",
    )
    .await;
    server
        .await
        .expect("popup nested eval policy server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__popupNestedEvalResult)",
        expected,
        "popup nested eval policy result",
    )
    .await;
    vm.eval("String(globalThis.__popupNestedEvalResult)")
        .expect("popup nested eval result should evaluate")
}

async fn spawn_lightweight_popup_204_then_loaded_server()
-> (String, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind popup 204 loaded test server");
    let addr = listener.local_addr().expect("popup 204 loaded server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener.accept().await.expect("accept popup 204 request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .await
            .expect("read popup 204 request");
        let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write popup 204 response");

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept popup loaded request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .await
            .expect("read popup loaded request");
        let body = r#"<!doctype html><script>window.onload = () => window.opener.postMessage("loaded", "*");</script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write popup loaded response");
    });
    (
        format!("http://{addr}/popup-204.html"),
        format!("http://{addr}/base/loaded.html"),
        server,
    )
}

async fn spawn_lightweight_popup_relative_navigation_server()
-> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind popup relative navigation test server");
    let addr = listener
        .local_addr()
        .expect("popup relative navigation server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut request_paths = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept popup relative navigation request");
            let mut buffer = [0; 2048];
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read popup relative navigation request");
            let request = String::from_utf8_lossy(&buffer[..read]);
            let request_path = request
                .lines()
                .next()
                .and_then(|line| line.split_ascii_whitespace().nth(1))
                .expect("popup relative navigation request target")
                .to_owned();
            request_paths.push(request_path);
            let body = r#"<!doctype html><script>window.onload = () => window.opener.postMessage("loaded", "*");</script>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write popup relative navigation response");
        }
        request_paths
    });
    (format!("http://{addr}"), server)
}

async fn spawn_lightweight_popup_response_csp_server() -> (String, tokio::task::JoinHandle<()>) {
    spawn_lightweight_popup_response_html_server(
        "popup response CSP test server",
        "popup response CSP",
        "Content-Security-Policy: script-src 'none'",
        r#"<!doctype html><script>opener.__popupResponseCspEvents.push("script");</script>"#,
    )
    .await
}

async fn spawn_lightweight_popup_response_html_server(
    bind_label: &'static str,
    io_label: &'static str,
    policy_header: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    spawn_lightweight_popup_html_responses(bind_label, io_label, policy_header, body, 1).await
}

pub(super) async fn spawn_lightweight_popup_html_responses(
    bind_label: &'static str,
    io_label: &'static str,
    policy_header: &'static str,
    body: &'static str,
    response_count: usize,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|_| panic!("bind {bind_label}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|_| panic!("{bind_label} addr"));
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for _ in 0..response_count {
            let (mut stream, _) = listener
                .accept()
                .await
                .unwrap_or_else(|_| panic!("accept {io_label} request"));
            let mut buffer = [0; 1024];
            let _ = stream
                .read(&mut buffer)
                .await
                .unwrap_or_else(|_| panic!("read {io_label} request"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n{policy_header}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .unwrap_or_else(|_| panic!("write {io_label} response"));
        }
    });
    (format!("http://{addr}/popup.html"), server)
}

async fn spawn_lightweight_popup_document_domain_server(
    policy_header: Option<&'static str>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind popup document-domain test server");
    let addr = listener
        .local_addr()
        .expect("popup document-domain server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept popup document-domain request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .await
            .expect("read popup document-domain request");
        let body = "<!doctype html><title>popup</title>";
        let policy_header = policy_header
            .map(|header| format!("{header}\r\n"))
            .unwrap_or_default();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            policy_header,
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write popup document-domain response");
    });
    (format!("http://{addr}/popup.html"), server)
}

async fn spawn_storage_bucket_partition_child_server() -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind storage bucket partition child server");
    let addr = listener
        .local_addr()
        .expect("storage bucket partition child server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept storage bucket partition child request");
        let request = read_storage_bucket_partition_request_head(&mut stream)
            .await
            .expect("read storage bucket partition child request");
        let status = if request.starts_with("GET /storage-bucket-child.html ") {
            "200 OK"
        } else {
            "404 Not Found"
        };
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
(async () => {
  const bucket = await navigator.storageBuckets.open("partitioned-bucket");
  parent.postMessage(JSON.stringify({
    bucketName: bucket.name,
    keys: await navigator.storageBuckets.keys()
  }), "*");
})().catch(error => {
  parent.postMessage("error:" + (error && error.name), "*");
});
</script>"#;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write storage bucket partition child response");
        request
    });
    (
        format!("http://localhost:{}/storage-bucket-child.html", addr.port()),
        server,
    )
}

async fn spawn_service_worker_response_server(
    responses: Vec<(&'static str, &'static str, &'static str)>,
) -> (String, JoinHandle<()>) {
    spawn_service_worker_response_server_with_headers(
        responses
            .into_iter()
            .map(|(path, content_type, body)| (path, content_type, Vec::new(), body))
            .collect(),
    )
    .await
}

async fn spawn_service_worker_response_server_with_headers(
    responses: Vec<(
        &'static str,
        &'static str,
        Vec<(&'static str, &'static str)>,
        &'static str,
    )>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker response server");
    let addr = listener
        .local_addr()
        .expect("service worker response server addr");
    let server = tokio::spawn(async move {
        let mut responses: std::collections::VecDeque<_> = responses.into_iter().collect();
        while let Some((expected_path, content_type, extra_headers, body)) = responses.pop_front() {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept service worker response request");
            let request = read_storage_bucket_partition_request_head(&mut stream)
                .await
                .expect("read service worker response request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker response request path");
            assert_eq!(path, expected_path);
            let extra_headers = extra_headers
                .into_iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write service worker response");
        }
    });
    (format!("http://127.0.0.1:{}", addr.port()), server)
}

async fn spawn_service_worker_redirect_response_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker redirect response server");
    let addr = listener
        .local_addr()
        .expect("service worker redirect response server addr");
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept service worker redirect response request");
            let request = read_storage_bucket_partition_request_head(&mut stream)
                .await
                .expect("read service worker redirect response request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker redirect response request path");
            assert_eq!(path, "/redirect-start");
            let response = concat!(
                "HTTP/1.1 302 Found\r\n",
                "Location: /redirect-target\r\n",
                "Access-Control-Allow-Origin: *\r\n",
                "Content-Length: 0\r\n",
                "Connection: close\r\n",
                "\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write service worker redirect response");
        }
    });
    (
        format!("http://127.0.0.1:{}/redirect-start", addr.port()),
        server,
    )
}

async fn spawn_service_worker_headers_first_body_server()
-> (String, JoinHandle<()>, tokio::sync::oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker headers-first server");
    let addr = listener
        .local_addr()
        .expect("service worker headers-first server addr");
    let (release_body_tx, release_body_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut worker_stream, _) = listener
            .accept()
            .await
            .expect("accept service worker script request");
        let request = read_storage_bucket_partition_request_head(&mut worker_stream)
            .await
            .expect("read service worker script request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("service worker script request path");
        assert_eq!(path, "/app/worker.js");
        let worker_body = r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("fetch", event => {
              event.respondWith(fetch(event.request));
            });
        "#;
        let worker_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            worker_body.len(),
            worker_body
        );
        worker_stream
            .write_all(worker_response.as_bytes())
            .await
            .expect("write service worker script response");

        let (mut api_stream, _) = listener
            .accept()
            .await
            .expect("accept service worker proxied API request");
        let request = read_storage_bucket_partition_request_head(&mut api_stream)
            .await
            .expect("read service worker proxied API request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("service worker proxied API request path");
        assert_eq!(path, "/app/api/headers-first.txt");
        let body = "delayed-body";
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        api_stream
            .write_all(headers.as_bytes())
            .await
            .expect("write service worker proxied API headers");
        api_stream
            .flush()
            .await
            .expect("flush service worker proxied API headers");
        let _ = release_body_rx.await;
        let _ = api_stream.write_all(body.as_bytes()).await;
    });
    (
        format!("http://127.0.0.1:{}", addr.port()),
        server,
        release_body_tx,
    )
}

async fn spawn_service_worker_script_server(paths: Vec<&'static str>) -> (String, JoinHandle<()>) {
    spawn_service_worker_response_server(
        paths
            .into_iter()
            .map(|path| {
                (
                    path,
                    "text/javascript; charset=utf-8",
                    "self.addEventListener('install', () => {});",
                )
            })
            .collect(),
    )
    .await
}

fn service_worker_http_cache_test_root(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-renderer-v8-service-worker-http-cache-{label}-{}-{nonce}",
        std::process::id()
    ))
}

async fn read_storage_bucket_partition_request_head(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn spawn_popup_external_child_frame_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind popup external child frame server");
    let addr = listener
        .local_addr()
        .expect("popup external child frame server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept popup external child frame request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .await
            .expect("read popup external child frame request");
        let body = r#"<!doctype html>
<script>
window.onfocus = () => {
  parent.opener.postMessage("child-window-focus", "*");
};
window.onblur = () => {
  parent.opener.postMessage("child-window-blur", "*");
};
window.addEventListener("load", () => {
  parent.opener.postMessage(
    "external:" + (parent !== self) + "|" + (top === parent) + "|" + (typeof parent.opener.postMessage),
    "*"
  );
});
</script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write popup external child frame response");
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_csp_sandbox_storage_bucket_popup_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind CSP sandbox popup test server");
    let addr = listener
        .local_addr()
        .expect("CSP sandbox popup server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept CSP sandbox popup request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .await
            .expect("read CSP sandbox popup request");
        let body = r#"<!doctype html>
<script>
function post_message(data) {
  if (window.parent !== null) {
    window.parent.postMessage(data, { targetOrigin: "*" });
  }
  if (window.opener !== null) {
    window.opener.postMessage(data, { targetOrigin: "*" });
  }
}
navigator.storageBuckets.open("opaque-origin-bucket")
  .then(() => post_message("navigator.storageBuckets.open(): FULFILLED"))
  .catch(error => post_message("navigator.storageBuckets.open(): REJECTED: " + error.name));
navigator.storageBuckets.keys()
  .then(() => post_message("navigator.storageBuckets.keys(): FULFILLED"))
  .catch(error => post_message("navigator.storageBuckets.keys(): REJECTED: " + error.name));
navigator.storageBuckets.delete("opaque-origin-bucket")
  .then(() => post_message("navigator.storageBuckets.delete(): FULFILLED"))
  .catch(error => post_message("navigator.storageBuckets.delete(): REJECTED: " + error.name));
</script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: sandbox allow-scripts\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write CSP sandbox popup response");
    });
    (format!("http://{addr}/popup.html"), server)
}

async fn spawn_sandbox_popup_helper_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind sandbox popup helper server");
    let addr = listener
        .local_addr()
        .expect("sandbox popup helper server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept sandbox popup helper request");
            let mut buffer = [0; 1024];
            let _ = stream
                .read(&mut buffer)
                .await
                .expect("read sandbox popup helper request");
            let body = r#"<!doctype html>
<script>
  if (opener) {
    opener.postMessage(undefined, "*");
    self.close();
  } else {
    var initialPopupAccessible = false;
    onmessage = function (e) {
      parent.postMessage({ data: e.data, origin: e.origin, initialPopupAccessible }, "*");
    };
    var popupWin = window.open();
    try {
      popupWin.origin;
      initialPopupAccessible = true;
    } catch (_) {}
    popupWin.location.href = location.href;
  }
</script>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write sandbox popup helper response");
        }
    });
    (
        format!("http://{addr}/iframe_sandbox_popups_helper-3.html"),
        server,
    )
}

async fn spawn_sandbox_child_top_navigation_popup_server() -> (String, tokio::task::JoinHandle<()>)
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind sandbox child top navigation popup server");
    let addr = listener
        .local_addr()
        .expect("sandbox child top navigation popup server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept sandbox child top navigation popup request");
            let mut buffer = [0; 1024];
            let n = stream
                .read(&mut buffer)
                .await
                .expect("read sandbox child top navigation popup request");
            let request = String::from_utf8_lossy(&buffer[..n]);
            let body = if request.starts_with("GET /popup.html ") {
                r#"<!doctype html>
<iframe sandbox="allow-scripts"></iframe>
<script>
  onmessage = event => opener.postMessage(event.data, "*");
  document.querySelector("iframe").src = "/child.html";
</script>"#
            } else {
                r#"<!doctype html>
<script>
  onload = () => {
    try {
      top.location = "/navigated.html";
      top.postMessage("can navigate", "*");
    } catch (error) {
      top.postMessage("cannot navigate", "*");
    }
  };
</script>"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write sandbox child top navigation popup response");
        }
    });
    (format!("http://{addr}/popup.html"), server)
}

async fn spawn_lightweight_popup_response_csp_external_script_server()
-> (String, tokio::task::JoinHandle<bool>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind popup response external CSP test server");
    let addr = listener
        .local_addr()
        .expect("popup response external CSP server addr");
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept popup response external CSP request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .await
            .expect("read popup response external CSP request");
        let body = r#"<!doctype html><script src="/blocked.js"></script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: script-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write popup response external CSP response");

        let Ok(Ok((mut script_stream, _))) =
            tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept()).await
        else {
            return false;
        };
        let _ = script_stream.read(&mut buffer).await;
        let body = r#"opener.__popupResponseCspExternalEvents.push("script");"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = script_stream.write_all(response.as_bytes()).await;
        true
    });
    (format!("http://{addr}/popup.html"), server)
}
async fn spawn_lightweight_popup_response_csp_redirect_external_script_servers() -> (
    String,
    String,
    std::thread::JoinHandle<()>,
    std::thread::JoinHandle<()>,
) {
    let target_listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind popup redirect CSP target server");
    let target_addr = target_listener
        .local_addr()
        .expect("popup redirect CSP target addr");
    let final_script_url = format!("http://{target_addr}/final.js");
    let final_script_url_for_source = final_script_url.clone();
    let target_server = std::thread::spawn(move || {
        use std::io::{Read, Write};

        let mut stream = accept_popup_redirect_test_connection(&target_listener)
            .expect("accept popup redirect CSP final script request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .expect("read popup redirect CSP final script request");
        let body = r#"opener.__popupRedirectCspEvents.push("script");"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write popup redirect CSP final script response");
    });

    let source_listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind popup redirect CSP source server");
    let source_addr = source_listener
        .local_addr()
        .expect("popup redirect CSP source addr");
    let source_server = std::thread::spawn(move || {
        use std::io::{Read, Write};

        let mut stream = accept_popup_redirect_test_connection(&source_listener)
            .expect("accept popup redirect CSP document request");
        let mut buffer = [0; 1024];
        let _ = stream
            .read(&mut buffer)
            .expect("read popup redirect CSP document request");
        let body = r#"<!doctype html><script src="/redirect.js"></script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: script-src 'self'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write popup redirect CSP document response");

        let mut stream = accept_popup_redirect_test_connection(&source_listener)
            .expect("accept popup redirect CSP script request");
        let _ = stream
            .read(&mut buffer)
            .expect("read popup redirect CSP script request");
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {final_script_url_for_source}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .expect("write popup redirect CSP redirect response");
    });
    (
        format!("http://{source_addr}/popup.html"),
        final_script_url,
        source_server,
        target_server,
    )
}

fn accept_popup_redirect_test_connection(
    listener: &std::net::TcpListener,
) -> std::io::Result<std::net::TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets can inherit O_NONBLOCK from the listener on
                // some platforms. The request bytes are allowed to arrive
                // after accept, so restore blocking I/O with a bounded timeout
                // before the fixture reads the request head.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
                stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out waiting for popup redirect test connection",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
}
mod extracted;
