use super::*;
use crate::worker::WorkerGlobalKind;
use moli_websocket::test_support::{StreamWebSocketFixture, spawn_parked_stream_websocket_server};
use tokio::io::AsyncReadExt;

fn stream_global_kinds() -> [WorkerGlobalKind; 3] {
    [
        WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        WorkerGlobalKind::Shared {
            name: "streams".into(),
            storage_key: moli_storage_key::MoliStorageKey::new(
                "http://127.0.0.1".into(),
                "http://127.0.0.1".into(),
                None,
                moli_storage_key::StoragePartitionRelation::FirstParty,
            ),
        },
        WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("http://127.0.0.1/").unwrap(),
        },
    ]
}

async fn stream_console(worker: &mut WorkerTestHandle) -> String {
    timeout(Duration::from_secs(10), async {
        loop {
            match worker.recv().await.expect("worker must remain alive") {
                WorkerToParentMessage::Console(console) => return console.message,
                WorkerToParentMessage::WebSocketSubresource(_)
                | WorkerToParentMessage::WebSocketLifecycle(_)
                | WorkerToParentMessage::WebSocketFrame(_) => {}
                other => panic!("unexpected stream completion: {other:?}"),
            }
        }
    })
    .await
    .expect("stream operation must complete")
}

#[tokio::test]
async fn worker_websocket_stream_contract_across_global_and_script_kinds() {
    ensure_v8();
    let fixture = StreamWebSocketFixture::start().await;
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global_kind in stream_global_kinds() {
            let source = format!(
                "{}\n{}\nworkerStreamFrontendContract({:?}).then(value=>console.log(JSON.stringify(value)),error=>console.error(String(error.stack||error)));",
                include_str!("../../../script_vm/tests/fixtures/websocket-stream-native-abort.js"),
                include_str!("../../../script_vm/tests/fixtures/worker-websocket-stream.js"),
                fixture.base_url,
            );
            let mut worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(source, "http://127.0.0.1/worker.js".into())
                    .with_script_kind(script_kind)
                    .with_global_kind(global_kind),
            );
            let actual = stream_console(&mut worker).await;
            worker.terminate_and_join();
            let result: serde_json::Value =
                serde_json::from_str(actual.strip_prefix("log: ").expect(&actual)).unwrap();
            assert_eq!(
                result,
                serde_json::json!({
                    "checks":26,
                    "stages":["options","abort","synthetic","io","protocol","explicit-close","abort-cancel"],
                    "binaryKind":"Uint8Array"
                }),
                "{script_kind:?}"
            );
        }
    }
}

fn parked_worker(
    url: &str,
    script_kind: WorkerScriptKind,
    global_kind: WorkerGlobalKind,
) -> WorkerTestHandle {
    let source = format!(
        r#"
      const stream = new WebSocketStream({url:?});
      const opened = stream.opened;
      opened.catch(error=>console.error(error));
      stream.closed.catch(error=>console.error(error));
      onmessage = async event => {{
        try {{
          const {{readable,writable}} = await opened;
          const values=[];
          if(event.data==='resume') {{
            const reader=readable.getReader();
            for(let i=0;i<3;i++) {{const result=await reader.read();if(result.done)throw Error('lost frame');values.push(result.value);}}
            await writable.getWriter().close();
          }} else {{
            if(event.data==='close')stream.close();
            else await (event.data==='abort'?writable.abort(new WebSocketError('',{{closeCode:3333,reason:'abort'}})):readable.cancel(new WebSocketError('',{{closeCode:3333,reason:'cancel'}})));
            // Close must settle independently of further consumer reads.
            await stream.closed;
            const reader=readable.getReader();
            for(;;) {{const result=await reader.read();if(result.done)break;values.push(result.value);}}
          }}
          const closed=await stream.closed;
          console.log(JSON.stringify({{values,closeCode:closed.closeCode,reason:closed.reason}}));
        }}catch(error){{console.error(String(error.stack||error));}}
      }};
    "#
    );
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source, "http://127.0.0.1/parked-worker.js".into())
            .with_script_kind(script_kind)
            .with_global_kind(global_kind),
    )
}

#[tokio::test]
async fn worker_websocket_stream_backpressure_retains_frames_and_parent_tasks() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let (url, pong, server) = spawn_parked_stream_websocket_server().await;
        let mut worker = parked_worker(
            &url,
            kind,
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
        );
        timeout(TIMEOUT, pong).await.unwrap().unwrap();
        worker.post_message(serialize_test_string("resume"));
        assert_eq!(
            stream_console(&mut worker).await,
            "log: {\"values\":[\"first\",\"second\",\"third\"],\"closeCode\":1005,\"reason\":\"\"}"
        );
        worker.terminate_and_join();
        timeout(TIMEOUT, server).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn worker_websocket_stream_local_close_releases_parked_frames() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for mode in ["close", "abort", "cancel"] {
            let (url, pong, server) = spawn_parked_stream_websocket_server().await;
            let mut worker = parked_worker(
                &url,
                kind,
                WorkerGlobalKind::Dedicated {
                    name: String::new(),
                },
            );
            timeout(TIMEOUT, pong).await.unwrap().unwrap();
            worker.post_message(serialize_test_string(mode));
            let actual = stream_console(&mut worker).await;
            worker.terminate_and_join();
            let actual: serde_json::Value =
                serde_json::from_str(actual.strip_prefix("log: ").expect(&actual)).unwrap();
            assert_eq!(
                actual,
                serde_json::json!({
                    "values":if mode=="cancel" {Vec::<&str>::new()} else {vec!["first"]},
                    "closeCode":if mode=="close" {1005} else {3333},
                    "reason":if mode=="close" {""} else {mode}
                }),
                "{kind:?}/{mode}"
            );
            timeout(TIMEOUT, server).await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn worker_websocket_stream_termination_releases_parked_transport() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global_kind in stream_global_kinds() {
            let (url, pong, server) = spawn_parked_stream_websocket_server().await;
            let worker = parked_worker(&url, kind, global_kind);
            timeout(TIMEOUT, pong).await.unwrap().unwrap();
            worker.terminate_and_join();
            timeout(TIMEOUT, server)
                .await
                .expect("termination releases physical TCP")
                .unwrap();
        }
    }
}

#[tokio::test]
async fn worker_websocket_stream_csp_denial_precedes_mixed_content() {
    ensure_v8();
    let mut worker = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(r#"
          (async()=>{
            const violation=new Promise(resolve=>addEventListener('securitypolicyviolation',resolve,{once:true}));
            const stream=new WebSocketStream('ws://blocked.invalid/echo');
            const [opened,closed]=await Promise.all([stream.opened.catch(error=>error),stream.closed.catch(error=>error)]);
            const event=await violation;
            console.log(JSON.stringify({native:opened instanceof WebSocketError,same:opened===closed,name:opened.name,blocked:event.blockedURI,directive:event.effectiveDirective}));
          })().catch(error=>console.error(error));
        "#.into(), "https://worker-csp.test/worker.js".into())
            .with_content_security_policies(vec!["connect-src 'none'".into()]),
    );
    assert_eq!(
        stream_console(&mut worker).await,
        "log: {\"native\":true,\"same\":true,\"name\":\"WebSocketError\",\"blocked\":\"ws://blocked.invalid/echo\",\"directive\":\"connect-src\"}"
    );
    worker.terminate_and_join();
}

#[tokio::test]
async fn worker_websocket_stream_uses_worker_network_authority() {
    ensure_v8();
    let (url, headers, server) = spawn_header_capture_websocket_server().await;
    let source = format!(
        r#"
      (async()=>{{
        const stream=new WebSocketStream({url:?},{{protocols:['probe']}});
        const opened=await stream.opened;
        await stream.closed;
        console.log(opened.protocol);
      }})().catch(error=>console.error(error));
    "#
    );
    let mut worker = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source, "http://worker-authority.test/main.js".into())
            .with_network_policy(WorkerNetworkPolicy {
                extra_http_headers: vec![("x-owner".into(), "worker".into())].into(),
                ..WorkerNetworkPolicy::default()
            }),
    );
    let headers = timeout(TIMEOUT, headers).await.unwrap().unwrap();
    assert_eq!(
        moli_websocket::test_support::header_value(&headers, "origin").as_deref(),
        Some("http://worker-authority.test")
    );
    assert_eq!(
        moli_websocket::test_support::header_value(&headers, "x-owner").as_deref(),
        Some("worker")
    );
    assert_eq!(stream_console(&mut worker).await, "log: probe");
    worker.terminate_and_join();
    timeout(TIMEOUT, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn worker_websocket_stream_retirement_cancels_pending_handshake() {
    ensure_v8();
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global_kind in stream_global_kinds() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/pending", listener.local_addr().unwrap());
            let (accepted_tx, accepted_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                accepted_tx.send(()).unwrap();
                let mut buffer = [0; 1024];
                while stream.read(&mut buffer).await.unwrap_or(0) != 0 {}
            });
            let mut worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(
                    format!(
                        r#"
      const controller=new AbortController();
      const stream=new WebSocketStream({url:?},{{signal:controller.signal}});
      stream.opened.catch(()=>{{}});stream.closed.catch(()=>{{}});
      console.log('pending');
    "#
                    ),
                    "http://127.0.0.1/worker.js".into(),
                )
                .with_script_kind(script_kind)
                .with_global_kind(global_kind),
            );
            assert_eq!(stream_console(&mut worker).await, "log: pending");
            timeout(TIMEOUT, accepted_rx).await.unwrap().unwrap();
            worker.terminate_and_join();
            timeout(TIMEOUT, server)
                .await
                .expect("pending native transport must be cancelled")
                .unwrap();
        }
    }
}
