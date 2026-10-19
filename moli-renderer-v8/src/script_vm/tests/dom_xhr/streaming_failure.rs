use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread")]
async fn xhr_streaming_body_failure_finishes_only_the_current_request() {
    for chunked in [false, true] {
        for action in ["error", "abort", "reopen", "resend"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(socket.read_u8().await.unwrap());
                }
                assert!(head.starts_with(b"GET /broken HTTP/1.1\r\n"));
                let framing = if chunked {
                    "Transfer-Encoding: chunked"
                } else {
                    "Content-Length: 14"
                };
                socket.write_all(format!(
                    "HTTP/1.1 200 OK\r\n{framing}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n"
                ).as_bytes()).await.unwrap();
                socket
                    .write_all(if chunked {
                        b"7\r\npartial\r\n"
                    } else {
                        b"partial"
                    })
                    .await
                    .unwrap();
                // Expose LOADING and partial response data before the failure.
                release_rx.await.unwrap();
                if chunked {
                    let _ = socket.write_all(b"garbage").await;
                }
                drop(socket);
                if action == "resend" {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        head.push(socket.read_u8().await.unwrap());
                    }
                    assert!(head.starts_with(b"GET /replacement HTTP/1.1\r\n"));
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew",
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
            vm.eval(r#"
                globalThis.events = [];
                globalThis.partial = null;
                globalThis.xhr = new XMLHttpRequest();
                globalThis.snapshot = () => ({
                    state: xhr.readyState, status: xhr.status, text: xhr.responseText,
                    responseURL: xhr.responseURL, headers: xhr.getAllResponseHeaders(),
                    contentType: xhr.getResponseHeader("content-type")
                });
                for (const type of ["error", "abort", "load", "loadend"])
                    xhr.addEventListener(type, e => events.push([type, e.loaded, e.total, e.lengthComputable]));
                xhr.onprogress = () => { if (!partial) partial = snapshot(); };
                xhr.open("GET", "/broken");
                xhr.send();
            "#).unwrap();
            let mut old_id = None;
            tokio::time::timeout(Duration::from_secs(5), async {
                while vm.eval("partial !== null").unwrap() != "true" {
                    assert!(completions.wait_for_arrival_without_timeout().await);
                    while let Some(event) = completions.pop_next_async_subresource_event() {
                        if let crate::types::AsyncSubresourceFetchEvent::StreamingStarted(started) =
                            &event
                        {
                            old_id = Some(started.internal_id);
                        }
                        let activity = vm
                            .complete_async_subresource_fetch_event_body(event)
                            .unwrap();
                        vm.finish_async_subresource_body_checkpoint_for_test(activity)
                            .unwrap();
                    }
                }
            })
            .await
            .expect("partial XHR response should be observable");
            assert_eq!(vm.eval("JSON.stringify([partial.state, partial.status, partial.text, partial.contentType])").unwrap(), "[3,200,\"partial\",\"text/plain\"]");
            let old_id = old_id.expect("stream should start before delivering bytes");
            release_tx.send(()).unwrap();
            let failure =
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        assert!(completions.wait_for_arrival_without_timeout().await);
                        while let Some(event) = completions.pop_next_async_subresource_event() {
                            if let crate::types::AsyncSubresourceFetchEvent::StreamingFinished(
                                finished,
                            ) = &event
                                && finished.internal_id == old_id
                            {
                                assert!(finished.result.is_err());
                                return event;
                            }
                            let activity = vm
                                .complete_async_subresource_fetch_event_body(event)
                                .unwrap();
                            vm.finish_async_subresource_body_checkpoint_for_test(activity)
                                .unwrap();
                        }
                    }
                })
                .await
                .expect("native transport should queue the body failure");
            // The network has finished, but its terminal task has not entered
            // JS yet. Reusing the XHR must invalidate that queued delivery.
            match action {
                "abort" => {
                    vm.eval("xhr.abort()").unwrap();
                }
                "reopen" => {
                    vm.eval("xhr.open('GET', '/replacement')").unwrap();
                }
                "resend" => {
                    vm.eval("xhr.open('GET', '/replacement'); xhr.send()")
                        .unwrap();
                }
                _ => {}
            }
            let activity = vm
                .complete_async_subresource_fetch_event_body(failure)
                .unwrap();
            vm.finish_async_subresource_body_checkpoint_for_test(activity)
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while action == "resend" && vm.eval("xhr.readyState === 4").unwrap() != "true" {
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
            .unwrap_or_else(|_| panic!("stream finish stalled: {chunked}/{action}"));
            let expected_events = match action {
                "error" => "[[\"error\",0,0,false],[\"loadend\",0,0,false]]",
                "abort" => "[[\"abort\",0,0,false],[\"loadend\",0,0,false]]",
                "reopen" => "[]",
                "resend" => "[[\"load\",3,3,true],[\"loadend\",3,3,true]]",
                _ => unreachable!(),
            };
            assert_eq!(
                vm.eval("JSON.stringify(events)").unwrap(),
                expected_events,
                "{chunked}/{action}"
            );
            if action == "resend" {
                assert_eq!(
                    vm.eval("JSON.stringify([xhr.status, xhr.responseText])")
                        .unwrap(),
                    "[200,\"new\"]"
                );
            } else {
                let state = match action {
                    "error" => 4,
                    "reopen" => 1,
                    _ => 0,
                };
                assert_eq!(
                    vm.eval("JSON.stringify(snapshot())").unwrap(),
                    format!(
                        r#"{{"state":{state},"status":0,"text":"","responseURL":"","headers":"","contentType":null}}"#
                    )
                );
            }
            tokio::time::timeout(Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
