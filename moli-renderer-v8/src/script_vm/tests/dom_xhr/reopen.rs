use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_reopen_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
        assert!(head.len() < 65536);
    }
    String::from_utf8(head).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn window_xhr_open_cancels_transport_before_headers_and_during_body() {
    for body_started in [false, true] {
        for action in ["reopen", "restart", "abort", "invalid-open"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
            let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let (mut old, _) = listener.accept().await.unwrap();
                assert!(
                    read_reopen_request(&mut old)
                        .await
                        .starts_with("GET /old HTTP/1.1\r\n")
                );
                if body_started {
                    old.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Content-Type-Options: nosniff\r\nContent-Length: 4099\r\nConnection: close\r\n\r\n").await.unwrap();
                    old.write_all(&vec![b'o'; 4096]).await.unwrap();
                }
                accepted_tx.send(()).unwrap();
                if action == "invalid-open" {
                    release_rx.await.unwrap();
                    if body_started {
                        old.write_all(b"end").await.unwrap();
                    } else {
                        old.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nold",
                        )
                        .await
                        .unwrap();
                    }
                } else {
                    // The response remains withheld until the client closes.
                    // Dropping the fixture/loader cannot satisfy this check.
                    let mut byte = [0];
                    let result = tokio::time::timeout(Duration::from_secs(5), old.read(&mut byte))
                        .await
                        .expect(
                            "open/abort must cancel the old transport before any server release",
                        );
                    assert!(
                        matches!(result, Ok(0))
                            || result.as_ref().is_err_and(|error| matches!(
                                error.kind(),
                                std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::BrokenPipe
                            ))
                    );
                    closed_tx.send(()).unwrap();
                }
                drop(old);
                if action == "restart" {
                    let (mut replacement, _) = listener.accept().await.unwrap();
                    assert!(
                        read_reopen_request(&mut replacement)
                            .await
                            .starts_with("GET /replacement HTTP/1.1\r\n")
                    );
                    replacement
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
            vm.eval(
                r#"
globalThis.events=[];globalThis.xhr=new XMLHttpRequest();
for (const type of ['load','error','abort','loadend'])
  xhr.addEventListener(type,event=>events.push(type));
xhr.open('GET','/old');xhr.send();
"#,
            )
            .unwrap();
            tokio::time::timeout(Duration::from_secs(5), accepted_rx)
                .await
                .unwrap()
                .unwrap();
            if body_started {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while vm
                        .eval("xhr.readyState===3 && xhr.responseText.length===4096")
                        .unwrap()
                        != "true"
                    {
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
                .expect("a partial body must be visible before the action");
            }
            let result = vm.eval(match action {
                "reopen"=>"xhr.open('GET','/replacement'); 'ok'",
                "restart"=>"xhr.open('GET','/replacement'); xhr.send(); 'ok'",
                "abort"=>"xhr.abort(); 'ok'",
                _=>"(()=>{try{xhr.open('GET','http://[')}catch(error){return error.name+':'+(error instanceof DOMException)}return 'missing'})()",
            }).unwrap();
            if action == "invalid-open" {
                assert_eq!(result, "SyntaxError:true");
                release_tx.send(()).unwrap();
            } else {
                assert_eq!(result, "ok");
                tokio::time::timeout(Duration::from_secs(5), closed_rx)
                    .await
                    .unwrap()
                    .unwrap();
                drop(release_tx);
            }
            if matches!(action, "restart" | "invalid-open") {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while vm.eval("events.includes('loadend')").unwrap() != "true" {
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
                .expect("the retained/replacement request must complete");
            }
            server.await.unwrap();
            while let Some(event) = completions.pop_next_async_subresource_event() {
                let activity = vm
                    .complete_async_subresource_fetch_event_body(event)
                    .unwrap();
                vm.finish_async_subresource_body_checkpoint_for_test(activity)
                    .unwrap();
            }
            assert_eq!(
                vm.eval("JSON.stringify(events)").unwrap(),
                match action {
                    "reopen" => "[]",
                    "abort" => r#"["abort","loadend"]"#,
                    _ => r#"["load","loadend"]"#,
                },
                "body_started={body_started}, action={action}"
            );
            let expected_state = match action {
                "reopen" => "1",
                "abort" => "0",
                _ => "4",
            };
            assert_eq!(vm.eval("String(xhr.readyState)").unwrap(), expected_state);
            let expected_text = match action {
                "restart" => "new".to_owned(),
                "invalid-open" if body_started => format!("{}end", "o".repeat(4096)),
                "invalid-open" => "old".to_owned(),
                _ => String::new(),
            };
            assert_eq!(vm.eval("xhr.responseText").unwrap(), expected_text);
        }
    }
}

#[test]
fn window_xhr_open_retires_intercepted_request_without_abort_events() {
    let mut vm = new_storage_test_vm("https://xhr-reopen-intercept.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
    vm.eval(r#"
globalThis.events=[];globalThis.xhr=new XMLHttpRequest();
for (const type of ['abort','error','load','loadend']) xhr.addEventListener(type,()=>events.push(type));
xhr.open('GET','/old');xhr.send();
"#).unwrap();
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    vm.eval("xhr.open('GET','/replacement')").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([xhr.readyState,xhr.status,events])")
            .unwrap(),
        "[1,0,[]]"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
    // An already in-flight decision for the retired interception cannot start
    // its old request or mutate the now-reopened object.
    let result = vm
        .fail_pending_subresource_fetch_body(pending[0].internal_id, "late old failure".to_owned());
    assert!(result.is_err());
    assert_eq!(
        vm.eval("JSON.stringify([xhr.readyState,xhr.status,events])")
            .unwrap(),
        "[1,0,[]]"
    );
}
