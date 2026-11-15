use super::*;

fn client_message_console(message: WorkerToParentMessage) -> String {
    let WorkerToParentMessage::Console(console) = message else {
        panic!("expected the Worker probe result, got {message:?}");
    };
    console.message.strip_prefix("log: ").unwrap().to_owned()
}

async fn worker_client_container_message_probe(shadow: bool) {
    ensure_v8();
    let url = url::Url::parse("https://worker-client-message.test/client.js").unwrap();
    for kind in [
        super::super::WorkerGlobalKind::Dedicated {
            name: "client".into(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "client".into(),
            storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(&url, None),
        },
    ] {
        let script = r#"
        const container = navigator.serviceWorker;
        const trace = [];
        let sender;
        let globalMessages = 0;
        addEventListener('message', event => {
            globalMessages++;
            console.log(JSON.stringify({trace, globalMessages, finished: event.data === 'finish'}));
        });
        container.addEventListener('message', event => {
            if (!(event instanceof MessageEvent) || event.target !== container || event.currentTarget !== container)
                throw Error('container target or event realm');
            const setter = Object.getOwnPropertyDescriptor(event.source, 'onstatechange')?.set;
            if (!setter || event.source.scriptURL !== 'https://worker-client-message.test/sender.js')
                throw Error('sender attributes');
            setter.call(event.source, null);
            let fakeError;
            try { setter.call(Object.create(Object.getPrototypeOf(event.source)), null); }
            catch (error) { fakeError = error; }
            if (!(fakeError instanceof TypeError)) throw Error('native sender brand');
            if (event.origin !== 'https://worker-client-message.test' || event.lastEventId !== '' || event.ports.length !== 0)
                throw Error('sender metadata');
            if (sender && sender !== event.source) throw Error('sender identity');
            if (event.source.state !== (event.data.seq === 0 ? 'activating' : 'activated'))
                throw Error('sender lifecycle snapshot');
            sender = event.source;
            if (Array.from(new Uint8Array(event.data.bytes)).join(',') !== '11,22,33')
                throw Error('cloned buffer');
            trace.push('message:' + event.data.seq);
            Promise.resolve().then(() => trace.push('microtask:' + event.data.seq));
        });
        if (SHADOW) {
            Object.defineProperty(navigator, 'serviceWorker', {get() { throw Error('author container getter'); }});
            Object.defineProperty(globalThis, 'navigator', {get() { throw Error('author navigator getter'); }});
            Object.defineProperty(globalThis, 'ServiceWorker', {get() { throw Error('author constructor getter'); }});
        }
        console.log('ready');
        "#.replace("SHADOW", if shadow {"true"} else {"false"});
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(script, url.to_string()).with_global_kind(kind),
        );
        assert_eq!(
            client_message_console(timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap()),
            "ready"
        );
        for seq in 0..2 {
            handle
                .tx
                .send(crate::worker::WorkerMessage::ServiceWorkerClientMessage(
                    Box::new(crate::types::ServiceWorkerClientMessageData {
                        source_version_id: ServiceWorkerVersionId::from_u64_for_test(42),
                        source_script_url: url::Url::parse(
                            "https://worker-client-message.test/sender.js",
                        )
                        .unwrap(),
                        source_state: if seq == 0 { "activating" } else { "activated" },
                        payload: serialize_test_post_message_value(&format!(
                            "({{seq:{seq},bytes:new Uint8Array([11,22,33]).buffer}})"
                        )),
                    }),
                ))
                .unwrap();
        }
        handle.post_message(serialize_test_string("finish"));
        let result = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        handle.terminate_and_join();
        let actual: serde_json::Value =
            serde_json::from_str(&client_message_console(result)).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({
                "trace": ["message:0", "microtask:0", "message:1", "microtask:1"],
                "globalMessages": 1,
                "finished": true,
            })
        );
    }
}

#[tokio::test]
async fn worker_service_worker_messages_target_the_container_and_checkpoint_each_task() {
    worker_client_container_message_probe(false).await;
}

#[tokio::test]
async fn worker_service_worker_messages_ignore_shadowed_navigator_properties() {
    worker_client_container_message_probe(true).await;
}
