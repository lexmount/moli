use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn intercepted_window_fetch_preserves_post_bytes() {
    intercepted_window_post_bytes(false, false).await;
}

#[tokio::test]
async fn intercepted_window_fetch_auth_preserves_post_bytes() {
    intercepted_window_post_bytes(false, true).await;
}

#[tokio::test]
async fn intercepted_window_xhr_preserves_post_bytes() {
    intercepted_window_post_bytes(true, false).await;
}

#[tokio::test]
async fn intercepted_window_xhr_auth_preserves_post_bytes() {
    intercepted_window_post_bytes(true, true).await;
}

async fn intercepted_window_post_bytes(xhr: bool, authenticate: bool) {
    for replacement in [None, Some(Some("changed")), Some(None)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut bodies = Vec::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (head, body) = read_post(&mut stream).await;
                bodies.push(body);
                let authorized = head.lines().any(|line| {
                    line.split_once(':').is_some_and(|(name, value)| {
                        name.eq_ignore_ascii_case("authorization")
                            && value.trim() == "Basic dXNlcjpwYXNz"
                    })
                });
                if authenticate && !authorized {
                    stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    return bodies;
                }
            }
        });
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, mut completions) =
            new_storage_test_vm_with_loader_and_resource_completion_queue(&origin, &loader);
        vm.set_fetch_subresource_interception(true, None);
        vm.exec(if xhr {
            "globalThis.xhr = new XMLHttpRequest(); xhr.open('POST','/probe'); xhr.send(new Uint8Array([0,128,255,65]));"
        } else {
            "fetch('/probe',{method:'POST',body:new Uint8Array([0,128,255,65])}).catch(()=>{});"
        }, None).unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(
            request.request_body_bytes.as_deref(),
            Some([0, 128, 255, 65].as_slice())
        );
        vm.continue_pending_subresource_fetch(
            request.internal_id,
            None,
            None,
            replacement.map(|body| body.map(str::to_owned)),
            None,
            false,
            authenticate,
        )
        .unwrap();
        let mut challenges = 0;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                assert!(completions.wait_for_arrival_without_timeout().await);
                while let Some(event) = completions.pop_next_async_subresource_event() {
                    let _ = vm
                        .complete_async_subresource_fetch_event_body(event)
                        .unwrap();
                }
                for event in vm.take_pending_subresource_continue_events() {
                    match event {
                        crate::types::PendingSubresourceContinueEvent::AuthRequired(info) => {
                            assert!(authenticate);
                            assert_eq!(info.internal_id, request.internal_id);
                            challenges += 1;
                            assert_eq!(challenges, 1, "credentials must settle the challenge");
                            let _ = vm
                                .continue_pending_subresource_auth_body(
                                    request.internal_id,
                                    crate::SubresourceAuthCredentials {
                                        target: crate::types::SubresourceAuthTarget::Server,
                                        scheme: crate::types::SubresourceAuthScheme::Basic,
                                        username: "user".into(),
                                        password: "pass".into(),
                                    },
                                )
                                .unwrap();
                        }
                        crate::types::PendingSubresourceContinueEvent::Completed {
                            internal_id,
                        } => {
                            assert_eq!(internal_id, request.internal_id);
                            return;
                        }
                        event => panic!("unexpected continuation: {event:?}"),
                    }
                }
            }
        })
        .await
        .unwrap();
        let bodies = tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(challenges, usize::from(authenticate));
        if authenticate {
            assert!(
                bodies.len() >= 2,
                "capture both initial request and authenticated replay"
            );
        }
        let expected = match replacement {
            None => vec![0, 128, 255, 65],
            Some(Some(body)) => body.as_bytes().to_vec(),
            Some(None) => Vec::new(),
        };
        for body in bodies {
            assert_eq!(
                body, expected,
                "every physical POST must preserve the selected bytes"
            );
        }
    }
}

#[tokio::test]
async fn intercepted_beacon_preserves_binary_body_and_explicit_override() {
    for replacement in [None, Some("changed")] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (_, body) = read_post(&mut stream).await;
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            body
        });
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, mut completions) =
            new_storage_test_vm_with_loader_and_resource_completion_queue(&origin, &loader);
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Ping),
        );
        assert_eq!(
            vm.eval("String(navigator.sendBeacon('/probe', new Uint8Array([0,128,255,65])))")
                .unwrap(),
            "true"
        );
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(
            request.request_body_bytes.as_deref(),
            Some([0, 128, 255, 65].as_slice())
        );
        vm.continue_pending_subresource_fetch(
            request.internal_id,
            None,
            None,
            replacement.map(|body| Some(body.to_owned())),
            None,
            false,
            false,
        )
        .unwrap();
        let actual = tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            actual,
            replacement.map_or_else(|| vec![0, 128, 255, 65], |body| body.as_bytes().to_vec())
        );
        // Claim the exact transport result without entering the sending realm.
        // Its native terminal must be recorded before that application returns.
        loop {
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    completions.wait_for_arrival_without_timeout()
                )
                .await
                .unwrap()
            );
            let event = completions.pop_next_async_subresource_event().unwrap();
            let terminal = matches!(&event,
                crate::types::AsyncSubresourceFetchEvent::TransportCompletion(completion)
                    if completion.internal_id() == request.internal_id);
            assert!(matches!(vm.complete_async_subresource_fetch_event_body(event).unwrap(),
                crate::script_vm::subresource_fetch::AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered));
            if terminal {
                break;
            }
        }
        assert!(
            vm._context_host
                .borrow()
                .pending_window_beacon_execution_contexts_for_test()
                .is_empty()
        );
        assert_eq!(vm.take_network_output().into_items().filter(|item| matches!(item,
            crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(body) if Some(body.handle()) == request.network_request_handle
        )).count(), 1);
    }
}

async fn read_post(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("POST /probe HTTP/1.1"));
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
    (head, body)
}
