use super::*;

#[tokio::test]
async fn worker_console_namespace_uses_realm_intrinsics_across_global_kinds() {
    let source = format!(
        "const probe={};console.log(probe(globalThis));",
        include_str!("../../../script_vm/tests/browser_api/console_namespace.js")
    );
    assert_worker_console_script(&source, "log: true").await;
}

#[tokio::test]
async fn worker_console_labels_preserve_conversion_and_exceptions_across_global_kinds() {
    let source = format!(
        "const probe={};const rows=probe(globalThis);if(rows.length!==70)throw new Error('incomplete label matrix');console.log(JSON.stringify(rows.filter(row=>!row.passed)));",
        include_str!("../../../script_vm/tests/browser_api/console_labels.js")
    );
    assert_worker_console_script(&source, "log: []").await;
}

async fn assert_worker_console_script(source: &str, expected: &str) {
    ensure_v8();
    for kind in worker_console_global_kinds() {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source.to_owned(),
                "https://console-namespace.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected console result, got {message:?}");
        };
        assert_eq!(console.message, expected);
        handle.terminate_and_join();
    }
}

fn worker_console_global_kinds() -> [super::super::WorkerGlobalKind; 3] {
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://console-namespace.test".to_owned(),
        "https://console-namespace.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    [
        super::super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "console".to_owned(),
            storage_key,
        },
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://console-namespace.test/").unwrap(),
        },
    ]
}

async fn dispatch_console_protocol(
    handle: &WorkerHandle,
    id: i64,
    method: &str,
    params: Option<serde_json::Value>,
) -> Vec<crate::runtime::RendererRuntimeInspectorMessage> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut request = serde_json::json!({"id": id, "method": method});
    if let Some(params) = params {
        request["params"] = params;
    }
    assert!(handle.dispatch_runtime_protocol_message(
        Some("SID-console-native".to_owned()),
        request.to_string(),
        None,
        tx,
    ));
    timeout(TIMEOUT, rx)
        .await
        .expect("worker console command timed out")
        .expect("worker console command channel closed")
        .expect("worker console command failed")
}

fn native_console_packets(
    messages: Vec<crate::runtime::RendererRuntimeInspectorMessage>,
) -> Vec<moli_protocol_cdp::CdpMessage> {
    messages
        .into_iter()
        .map(|message| message.into_v8_inspector_message())
        .filter(|message| message.method() == Some("Runtime.consoleAPICalled"))
        .collect()
}

#[tokio::test]
async fn worker_console_forwards_original_utf16_arguments_once_in_every_global_kind() {
    ensure_v8();
    for kind in worker_console_global_kinds() {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                "console.log('worker-ready');".to_owned(),
                "https://console-native.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let ready = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert!(
            matches!(ready, WorkerToParentMessage::Console(ref value) if value.message == "log: worker-ready")
        );
        dispatch_console_protocol(
            &handle,
            1,
            "Runtime.evaluate",
            Some(serde_json::json!({"expression": "0"})),
        )
        .await;
        let replay = native_console_packets(
            dispatch_console_protocol(&handle, 2, "Runtime.enable", None).await,
        );
        assert_eq!(
            replay.len(),
            1,
            "native Runtime.enable must replay the original log once"
        );
        assert_eq!(
            replay[0].as_json().unwrap()["params"]["args"][0]["value"],
            "worker-ready"
        );
        let messages = dispatch_console_protocol(&handle, 3, "Runtime.evaluate", Some(serde_json::json!({
            "expression": "for(const method of ['log','info','warn','error','debug','trace'])console[method]('native-utf16',String.fromCharCode(0xd800),String.fromCharCode(0xd801));true",
            "returnByValue": true,
        }))).await;
        let packets = native_console_packets(messages);
        assert_eq!(
            packets.len(),
            6,
            "each console method must produce one native notification"
        );
        for packet in packets {
            let wire = serde_json::to_string(&packet).unwrap().to_ascii_lowercase();
            assert!(wire.contains(r#""value":"\ud800""#), "{wire}");
            assert!(wire.contains(r#""value":"\ud801""#), "{wire}");
        }
        for method in ["log", "info", "warn", "error", "debug", "trace"] {
            let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
            assert!(
                matches!(message, WorkerToParentMessage::Console(ref value) if value.message.starts_with(&format!("{method}: native-utf16"))),
                "native forwarding must preserve existing CLI capture: {message:?}"
            );
        }
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_native_console_does_not_read_page_prepare_stack_trace_getters() {
    ensure_v8();
    for kind in worker_console_global_kinds() {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                "console.log('worker-ready');".to_owned(),
                "https://console-native.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let ready = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert!(matches!(ready, WorkerToParentMessage::Console(_)));
        dispatch_console_protocol(&handle, 1, "Runtime.enable", None).await;
        let messages = dispatch_console_protocol(&handle, 2, "Runtime.evaluate", Some(serde_json::json!({
            "expression": r#"(() => {
                let hits=0;
                const original=Object.getOwnPropertyDescriptor(Error,'prepareStackTrace');
                Object.defineProperty(Error,'prepareStackTrace',{configurable:true,get(){hits++;return()=> 'page stack'}});
                try {
                    console.log(new Error('console'));
                    const afterConsole=hits;
                    void new Error('explicit stack').stack;
                    return `${afterConsole}|${hits}`;
                } finally {
                    delete Error.prepareStackTrace;
                    if(original)Object.defineProperty(Error,'prepareStackTrace',original);
                }
            })()"#,
            "returnByValue": true,
        }))).await;
        let response = messages
            .into_iter()
            .map(|message| message.into_v8_inspector_message())
            .find(|message| message.signed_id() == Some(2))
            .unwrap();
        assert_eq!(
            response.as_json().unwrap()["result"]["result"]["value"],
            "0|1"
        );
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_console_reporting_does_not_add_author_conversions() {
    ensure_v8();
    for kind in worker_console_global_kinds() {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                "console.log('worker-ready');".to_owned(),
                "https://console-native.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let ready = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert!(matches!(ready, WorkerToParentMessage::Console(_)));
        dispatch_console_protocol(&handle, 1, "Runtime.enable", None).await;
        let messages = dispatch_console_protocol(&handle, 2, "Runtime.evaluate", Some(serde_json::json!({
            "expression": r#"(() => {
                const hits=[];
                let functionConversions=0;
                const object={get property(){hits.push('getter');return 1},toJSON(){hits.push('toJSON');return {}},toString(){hits.push('toString');return 'object'},[Symbol.toPrimitive](){hits.push('toPrimitive');return 'object'}};
                const proxy=new Proxy({}, {get(){hits.push('Proxy.get')},ownKeys(){hits.push('Proxy.ownKeys');return []},getOwnPropertyDescriptor(){hits.push('Proxy.getOwnPropertyDescriptor')}});
                const fn=function named(){};
                fn.toString=()=>{functionConversions++;return 'function'};
                const error=new Error('console');
                Object.defineProperty(error,'stack',{enumerable:true,get(){hits.push('stack');return 'stack'}});
                for(const method of ['log','info','warn','error','debug','trace'])console[method](object,proxy,fn,error,Symbol('console'),1n);
                return JSON.stringify([hits,functionConversions]);
            })()"#,
            "returnByValue": true,
        }))).await;
        let response = messages
            .into_iter()
            .map(|message| message.into_v8_inspector_message())
            .find(|message| message.signed_id() == Some(2))
            .unwrap();
        assert_eq!(
            response.as_json().unwrap()["result"]["result"]["value"],
            "[[],6]"
        );
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_native_console_domains_replay_disable_and_discard_independently() {
    ensure_v8();
    for kind in worker_console_global_kinds() {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                "console.log('before-attach');".to_owned(),
                "https://console-native.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let ready = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert!(matches!(ready, WorkerToParentMessage::Console(_)));
        dispatch_console_protocol(
            &handle,
            1,
            "Runtime.evaluate",
            Some(serde_json::json!({"expression": "0"})),
        )
        .await;
        let mut id = 1;
        for (method, expression, expected_console, expected_runtime) in [
            ("Console.enable", None, 1, 0),
            ("Runtime.enable", None, 0, 1),
            ("Runtime.evaluate", Some("console.log('both')"), 1, 1),
            ("Runtime.disable", None, 0, 0),
            (
                "Runtime.evaluate",
                Some("console.log('runtime-disabled')"),
                1,
                0,
            ),
            ("Runtime.enable", None, 0, 3),
            ("Runtime.discardConsoleEntries", None, 0, 0),
            ("Runtime.disable", None, 0, 0),
            ("Runtime.enable", None, 0, 0),
            ("Console.disable", None, 0, 0),
            (
                "Runtime.evaluate",
                Some("console.log('only-runtime')"),
                0,
                1,
            ),
            ("Runtime.disable", None, 0, 0),
            ("Runtime.evaluate", Some("console.log('neither')"), 0, 0),
        ] {
            id += 1;
            let messages = dispatch_console_protocol(
                &handle,
                id,
                method,
                expression.map(|expression| serde_json::json!({"expression": expression})),
            )
            .await
            .into_iter()
            .map(|message| message.into_v8_inspector_message())
            .collect::<Vec<_>>();
            let response = messages
                .iter()
                .find(|message| message.signed_id() == Some(id))
                .expect("native worker console command must reply");
            assert!(!response.has_field("error"), "{method}: {response:?}");
            for (event, expected) in [
                ("Console.messageAdded", expected_console),
                ("Runtime.consoleAPICalled", expected_runtime),
            ] {
                assert_eq!(
                    messages
                        .iter()
                        .filter(|message| message.method() == Some(event))
                        .count(),
                    expected,
                    "{method} {expression:?}: {event}"
                );
            }
        }
        handle.terminate_and_join();
    }
}
