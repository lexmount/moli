use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_upload_request(socket: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    (head, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_upload_completes_before_response_and_preserves_reentrant_event_steps() {
    for mode in [
        "normal",
        "empty",
        "absent",
        "intercept",
        "redirect",
        "abort-complete",
        "reopen-complete",
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (head, body) = read_upload_request(&mut socket).await;
            assert!(head.starts_with("POST /upload HTTP/1.1\r\n"));
            let expected = if matches!(mode, "empty" | "absent") {
                b"".as_slice()
            } else {
                b"payload".as_slice()
            };
            assert_eq!(body, expected);
            // Nonempty uploads finish before response bytes are released.
            // Chromium reports no completion events for an empty body.
            release_rx.await.unwrap();
            if mode == "redirect" {
                socket.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            } else {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
            drop(socket);
            if matches!(mode, "redirect" | "reopen-complete") {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (head, body) = read_upload_request(&mut socket).await;
                if mode == "redirect" {
                    assert!(head.starts_with("POST /final HTTP/1.1\r\n"));
                    assert_eq!(body, b"payload");
                } else {
                    assert!(head.starts_with("GET /replacement HTTP/1.1\r\n"));
                    assert!(body.is_empty());
                }
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            }
        });
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let loader = ResourceRequestClient::new(&config).unwrap();
        let (mut vm, mut completions) =
            new_storage_test_vm_with_loader_and_resource_completion_queue(
                &format!("{origin}/page"),
                &loader,
            );
        if mode == "intercept" {
            vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Xhr),
            );
        }
        vm.eval(&format!(r#"
            globalThis.result = null;
            globalThis.events = [];
            const xhr = new XMLHttpRequest();
            const mode = {mode:?};
            for (const type of ["loadstart", "progress", "load", "loadend", "abort", "error"]) {{
                xhr.upload.addEventListener(type, e => events.push([type, e.loaded, e.total, e.lengthComputable]));
            }}
            xhr.upload.onload = () => {{
                if (mode === "abort-complete") xhr.abort();
                if (mode === "reopen-complete") {{ xhr.open("GET", "/replacement"); xhr.send(); }}
            }};
            xhr.onloadend = () => {{ result = xhr.status; }};
            xhr.open("POST", "/upload");
            if (mode === "absent") xhr.send();
            else xhr.send(mode === "empty" ? "" : "payload");
            globalThis.initialEvents = events.map(e => e[0]);
        "#)).unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(initialEvents)").unwrap(),
            if mode == "absent" {
                "[]"
            } else {
                "[\"loadstart\"]"
            },
            "{mode}: upload completed inside send()"
        );
        if mode == "intercept" {
            let pending = vm.take_pending_subresource_fetch_infos();
            assert_eq!(pending.len(), 1);
            vm.set_fetch_subresource_interception(false, None);
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
        let mut release = Some(release_tx);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if (matches!(mode, "absent" | "empty")
                    || vm.eval("events.some(e => e[0] === 'loadend')").unwrap() == "true")
                    && let Some(release) = release.take()
                {
                    release.send(()).unwrap();
                }
                if vm.eval("result !== null").unwrap() == "true" && release.is_none() {
                    break;
                }
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
        .unwrap_or_else(|_| panic!("upload stalled: {mode}"));
        let reports_upload = !matches!(mode, "absent" | "empty");
        if mode == "empty" {
            assert_eq!(
                vm.eval("JSON.stringify(events)").unwrap(),
                "[[\"loadstart\",0,0,true]]"
            );
        }
        assert_eq!(
            vm.eval("String(result)").unwrap(),
            if mode == "abort-complete" { "0" } else { "200" },
            "{mode}"
        );
        assert_eq!(
            vm.eval("events.filter(e => e[0] === 'load').length")
                .unwrap(),
            if reports_upload { "1" } else { "0" },
            "{mode}"
        );
        assert_eq!(
            vm.eval("events.filter(e => e[0] === 'loadend').length")
                .unwrap(),
            if reports_upload { "1" } else { "0" },
            "{mode}"
        );
        assert_eq!(
            vm.eval("events.some(e => e[0] === 'abort' || e[0] === 'error')")
                .unwrap(),
            "false",
            "{mode}"
        );
        if reports_upload {
            assert_eq!(
                vm.eval("JSON.stringify(events.find(e => e[0] === 'loadend').slice(1))")
                    .unwrap(),
                "[7,7,true]",
                "{mode}"
            );
        }
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_partial_upload_abort_and_reopen_discard_old_network_events() {
    for reopen in [false, true] {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(32 * 1024).unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(8).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let total = 16 * 1024 * 1024;
        let (release_reads, cancelled_upload) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.unwrap());
            }
            assert!(head.starts_with(b"POST /upload HTTP/1.1\r\n"));
            let mut bytes = 0;
            let mut chunk = [0; 65536];
            // Open one bounded receive window after a reporting interval,
            // then keep backpressure until JS cancels.
            tokio::time::sleep(Duration::from_millis(150)).await;
            if let Ok(count) = socket.read(&mut chunk).await {
                bytes += count;
            }
            cancelled_upload.await.unwrap();
            while let Ok(count) = socket.read(&mut chunk).await {
                if count == 0 {
                    break;
                }
                bytes += count;
            }
            if reopen {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (head, body) = read_upload_request(&mut socket).await;
                assert!(head.starts_with("GET /replacement HTTP/1.1\r\n"));
                assert!(body.is_empty());
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            }
            bytes
        });
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let loader = ResourceRequestClient::new(&config).unwrap();
        let (mut vm, mut completions) =
            new_storage_test_vm_with_loader_and_resource_completion_queue(
                &format!("{origin}/page"),
                &loader,
            );
        vm.eval(&format!(r#"
            globalThis.result = null;
            globalThis.partial = null;
            globalThis.events = [];
            const xhr = new XMLHttpRequest();
            for (const type of ["loadstart", "progress", "load", "loadend", "abort", "error"]) {{
                xhr.upload.addEventListener(type, e => events.push([type, e.loaded, e.total, e.lengthComputable]));
            }}
            xhr.upload.onprogress = e => {{
                if (!partial && e.loaded > 0 && e.loaded < e.total) {{
                    partial = [e.loaded, e.total];
                    if ({reopen}) {{ xhr.open("GET", "/replacement"); xhr.send(); }}
                    else xhr.abort();
                }}
            }};
            xhr.onloadend = () => {{ result = xhr.status; }};
            xhr.open("POST", "/upload");
            xhr.send("x".repeat({total}));
        "#)).unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(events.map(e => e[0]))").unwrap(),
            "[\"loadstart\"]"
        );
        let mut release_reads = Some(release_reads);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if vm.eval("partial !== null").unwrap() == "true"
                    && let Some(release) = release_reads.take()
                {
                    release.send(()).unwrap();
                }
                if vm.eval("result === null").unwrap() == "false" {
                    break;
                }
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
        .expect("partial upload should be cancelled");
        assert_eq!(
            vm.eval("String(result)").unwrap(),
            if reopen { "200" } else { "0" }
        );
        assert_eq!(
            vm.eval(&format!(
                "partial[0] > 0 && partial[0] < {total} && partial[1] === {total}"
            ))
            .unwrap(),
            "true"
        );
        assert_eq!(
            vm.eval("events.some(e => e[0] === 'load' || e[0] === 'error')")
                .unwrap(),
            "false"
        );
        assert_eq!(
            vm.eval("JSON.stringify(events.filter(e => e[0] === 'abort' || e[0] === 'loadend'))")
                .unwrap(),
            if reopen {
                "[]"
            } else {
                "[[\"abort\",0,0,false],[\"loadend\",0,0,false]]"
            }
        );
        let received = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
        assert!(
            received < total as usize,
            "{reopen}: received {received} bytes before cancellation"
        );
    }
}
