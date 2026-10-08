use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

async fn run_web_storage_partition_probe(
    top_url: &str,
    child_url: &str,
    label: &str,
    loader: &ResourceRequestClient,
    web_storage: &crate::RendererWebStorageHandles,
) -> String {
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(top_url, loader);
    vm.set_web_storage_handles(web_storage);
    let child_url = format!("{child_url}?label={label}");
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");
    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__partitionMessage = null;
  addEventListener("message", event => {{
    globalThis.__partitionMessage = String(event.data);
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
    ))
    .expect("partition probe setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        loader,
        "String(globalThis.__partitionMessage !== null)",
        "true",
        "third-party WebStorage partition result",
    )
    .await;

    let result = vm
        .eval("globalThis.__partitionMessage || 'missing'")
        .expect("partition message should evaluate");
    assert_ne!(
        result, "missing",
        "third-party child frame did not post its WebStorage result"
    );
    result
}

async fn spawn_web_storage_partition_child_server(
    expected_requests: usize,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WebStorage partition child server");
    let addr = listener
        .local_addr()
        .expect("WebStorage partition child server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..expected_requests {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept WebStorage partition child request");
            let request = read_web_storage_partition_request_head(&mut stream)
                .await
                .expect("read WebStorage partition child request");
            let status = if request.starts_with("GET /partition-child.html?") {
                "200 OK"
            } else {
                "404 Not Found"
            };
            let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
const label = new URL(location.href).searchParams.get("label");
const beforeLocal = localStorage.getItem("partitioned-local");
const beforeSession = sessionStorage.getItem("partitioned-session");
localStorage.setItem("partitioned-local", label);
sessionStorage.setItem("partitioned-session", label);
parent.postMessage(JSON.stringify({
  label,
  beforeLocal,
  beforeSession,
  afterLocal: localStorage.getItem("partitioned-local"),
  afterSession: sessionStorage.getItem("partitioned-session")
}), "*");
</script>
"#;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write WebStorage partition child response");
            requests.push(request);
        }
        requests
    });
    (format!("http://{addr}"), server)
}

async fn spawn_wpt_style_web_storage_message_child_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WPT-style WebStorage message child server");
    let addr = listener
        .local_addr()
        .expect("WPT-style WebStorage message child server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept WPT-style WebStorage message child request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read WPT-style WebStorage message child request");
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
window.addEventListener("message", event => {
  if (event.data.command === "create ID") {
    localStorage.setItem(event.data.key, "created");
    event.source.postMessage({
      message: "ID created",
      userID: localStorage.getItem("userID"),
    }, event.source.origin);
  }
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
            .expect("write WPT-style WebStorage message child response");
        requests.push(request);
        requests
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_child_response_csp_sandbox_document_domain_server()
-> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child response CSP sandbox document.domain server");
    let addr = listener
        .local_addr()
        .expect("child response CSP sandbox document.domain server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept child response CSP sandbox document.domain request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read child response CSP sandbox document.domain request");
        let body = "<!doctype html><title>child</title>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: sandbox allow-scripts allow-same-origin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write child response CSP sandbox document.domain response");
        requests.push(request);
        requests
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_pending_child_message_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pending child message server");
    let addr = listener
        .local_addr()
        .expect("pending child message server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept pending child message request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read pending child message request");
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
addEventListener("message", event => {
  parent.postMessage("child:" + typeof event.data + ":" + (event.source === parent), "*");
});
</script>
"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write pending child message response");
        vec![request]
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_nested_same_origin_window_access_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind nested same-origin Window server");
    let addr = listener
        .local_addr()
        .expect("nested same-origin Window server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept nested same-origin Window request");
            let request = read_web_storage_partition_request_head(&mut stream)
                .await
                .expect("read nested same-origin Window request");
            let body = if request.starts_with("GET /root.html ") {
                r#"<!doctype html><body><script>
const nested = document.createElement("iframe");
nested.src = "/nested.html";
nested.addEventListener("load", () => {
  let topDenied = false;
  try {
    void top.document;
  } catch (error) {
    topDenied = error && error.name === "SecurityError";
  }
  let data;
  try {
    const module = new WebAssembly.Module(
      new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
    );
    data = {
      marker: nested.contentWindow.document.body.dataset.marker,
      parentIsRoot: nested.contentWindow.parent === globalThis,
      topDenied,
      wasmModule: module instanceof WebAssembly.Module
    };
  } catch (error) {
    data = { error: error && error.name, topDenied };
  }
  top.postMessage(data, "*");
});
document.body.appendChild(nested);
</script></body>"#
            } else {
                r#"<!doctype html><body data-marker="nested"></body>"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write nested same-origin Window response");
            requests.push(request);
        }
        requests
    });
    (format!("http://{addr}/root.html"), server)
}

async fn spawn_about_blank_popup_storage_child_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind about:blank popup storage child server");
    let addr = listener
        .local_addr()
        .expect("about:blank popup storage child server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept about:blank popup storage child request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read about:blank popup storage child request");
        let status = if request.starts_with("GET /popup-child.html ") {
            "200 OK"
        } else {
            "404 Not Found"
        };
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
localStorage.setItem("popup-scope", "child-partition");
const popup = window.open("about:blank");
const popupBefore = popup.localStorage.getItem("popup-scope");
const isolated = window.open("about:blank", "isolated", "noopener");
popup.localStorage.setItem("popup-scope", "popup-first-party");
parent.postMessage(JSON.stringify({
  popupBefore,
  popupAfter: popup.localStorage.getItem("popup-scope"),
  childAfter: localStorage.getItem("popup-scope"),
  opener: popup.opener === window,
  isolatedReturn: isolated === null
}), "*");
</script>
"#;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write about:blank popup storage child response");
        requests.push(request);
        requests
    });
    (format!("http://{addr}"), server)
}

async fn read_web_storage_partition_request_head(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buf = [0_u8; 1024];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

mod event_classes_ranges_and_selection;
mod event_dispatch_and_targets;
mod postmessage_selection_and_errors;
mod ranges_and_storage;
mod window_security_and_domain;
