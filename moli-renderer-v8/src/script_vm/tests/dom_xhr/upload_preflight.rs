use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread")]
async fn xhr_upload_listener_preflight_reaches_transport_in_page_realms() {
    for asynchronous in [false, true] {
        for mode in [
            "none",
            "removed",
            "late",
            "clear",
            "custom",
            "get",
            "same",
            "redirect",
            "deny",
            "credentials",
            "credentials-allow",
            "intercept",
        ] {
            if mode == "intercept" && !asynchronous {
                continue;
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let same_origin = matches!(mode, "same" | "redirect");
            let document_url = if same_origin {
                format!("http://{addr}/page")
            } else {
                "http://origin.test/page".to_owned()
            };
            let path = if mode == "redirect" {
                "/redirect"
            } else {
                "/actual"
            };
            let url = format!("http://{addr}{path}");
            let target_url = format!("http://localhost:{}/actual", addr.port());
            let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let mut observed = Vec::new();
                loop {
                    let mut socket = tokio::select! {
                        accepted = listener.accept() => accepted.unwrap().0,
                        _ = &mut stop_rx => break,
                    };
                    let mut head = Vec::new();
                    let mut byte = [0; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8(head).unwrap();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).await.unwrap();
                    observed.push(head.clone());
                    let response = if head.starts_with("POST /redirect ") {
                        format!(
                            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                    } else if head.starts_with("OPTIONS ") && mode == "deny" {
                        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else if mode == "credentials-allow" {
                        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: http://origin.test\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
                    } else {
                        // Deliberately omit Allow-Methods: safelisted methods do not
                        // need to appear there, even when upload listeners force OPTIONS.
                        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
                    };
                    socket.write_all(response.as_bytes()).await.unwrap();
                }
                observed
            });
            let mut config = moli_fetch::FetchConfig::default();
            config.set_http_no_proxy(Some("*".to_owned()));
            let loader = ResourceRequestClient::new(&config).unwrap();
            let (mut vm, mut completions) =
                new_storage_test_vm_with_loader_and_resource_completion_queue(
                    &document_url,
                    &loader,
                );
            if mode == "intercept" {
                vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Xhr),
                );
            }
            vm.eval(&format!(
                r#"
                globalThis.result = null;
                const xhr = new XMLHttpRequest();
                const mode = {mode:?};
                const listener = () => {{}};
                if (mode !== "none" && mode !== "late") {{
                    xhr.upload.addEventListener("custom", listener);
                }}
                if (mode === "removed") xhr.upload.removeEventListener("custom", listener);
                xhr.onloadstart = () => {{
                    if (mode === "late") xhr.upload.onprogress = listener;
                    if (mode === "clear") xhr.upload.removeEventListener("custom", listener);
                }};
                xhr.onloadend = () => {{ result = xhr.status; }};
                xhr.open(mode === "get" ? "GET" : "POST", {url:?}, {asynchronous});
                xhr.withCredentials = mode.startsWith("credentials");
                try {{
                    xhr.send("payload");
                    if (!{asynchronous}) result = xhr.status;
                }} catch (error) {{ result = error.name; }}
            "#
            ))
            .unwrap();
            if mode == "intercept" {
                let pending = vm.take_pending_subresource_fetch_infos();
                assert_eq!(pending.len(), 1);
                vm.eval("xhr.upload.removeEventListener('custom', listener)")
                    .unwrap();
                vm.continue_pending_subresource_fetch(
                    pending[0].internal_id,
                    None,
                    None,
                    None,
                    None,
                    false,
                    false,
                )
                .unwrap();
            }
            if asynchronous {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while vm.eval("result === null").unwrap() == "true" {
                        assert!(completions.wait_for_arrival_without_timeout().await);
                        while let Some(event) = completions.pop_next_async_subresource_event() {
                            let activity = vm
                                .complete_async_subresource_fetch_event_body(event)
                                .unwrap();
                            vm.finish_async_subresource_body_checkpoint_for_test(activity)
                                .unwrap();
                        }
                    }
                })
                .await
                .expect("XHR should finish");
            }
            let rejected = matches!(mode, "deny" | "credentials");
            let expected_result = if rejected && !asynchronous {
                "NetworkError"
            } else if rejected {
                "0"
            } else {
                "200"
            };
            assert_eq!(
                vm.eval("String(result)").unwrap(),
                expected_result,
                "{mode}, async={asynchronous}"
            );
            stop_tx.send(()).unwrap();
            let observed = server.await.unwrap();
            let request_lines: Vec<_> = observed
                .iter()
                .map(|head| head.lines().next().unwrap())
                .collect();
            let expected: &[&str] = match mode {
                "none" | "removed" | "late" | "same" => &["POST /actual HTTP/1.1"],
                "get" => &["OPTIONS /actual HTTP/1.1", "GET /actual HTTP/1.1"],
                "redirect" => &[
                    "POST /redirect HTTP/1.1",
                    "OPTIONS /actual HTTP/1.1",
                    "POST /actual HTTP/1.1",
                ],
                "deny" | "credentials" => &["OPTIONS /actual HTTP/1.1"],
                _ => &["OPTIONS /actual HTTP/1.1", "POST /actual HTTP/1.1"],
            };
            assert_eq!(request_lines, expected, "{mode}, async={asynchronous}");
            for head in observed.iter().filter(|head| head.starts_with("OPTIONS ")) {
                let lower = head.to_ascii_lowercase();
                let method = if mode == "get" { "get" } else { "post" };
                assert!(lower.contains(&format!("access-control-request-method: {method}\r\n")));
                assert!(!lower.contains("access-control-request-headers:"));
                assert!(!lower.contains("cookie:"));
            }
        }
    }
}
