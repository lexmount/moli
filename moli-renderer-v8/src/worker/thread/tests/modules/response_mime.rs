use super::source_phase::ModuleSourceServer;
use super::*;

// Exports answer() -> 42 and a custom section "x" containing the byte 0xff.
const WASM_WITH_NON_UTF8_SECTION: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03,
    0x02, 0x01, 0x00, 0x07, 0x0a, 0x01, 0x06, 0x61, 0x6e, 0x73, 0x77, 0x65, 0x72, 0x00, 0x00, 0x0a,
    0x06, 0x01, 0x04, 0x00, 0x41, 0x2a, 0x0b, 0x00, 0x03, 0x01, 0x78, 0xff,
];

fn module_data_url(mime: &str, bytes: &[u8]) -> String {
    let body = bytes
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    format!("data:{mime},{body}")
}

#[tokio::test]
async fn worker_wasm_imports_use_response_mime_independently_of_url_suffix() {
    ensure_v8();
    for name in ["module.wasm", "module.js", "module"] {
        for phase in ["static-source", "dynamic-source", "evaluation"] {
            let mut server = ModuleSourceServer::start().await;
            let url = serde_json::to_string(&format!("./{name}")).unwrap();
            let source = match phase {
                "static-source" => format!("import source wasm from {url};"),
                "dynamic-source" => format!("const wasm = await import.source({url});"),
                _ => format!(
                    "const namespace = await import({url}); postMessage(namespace.answer());"
                ),
            };
            let source = if phase == "evaluation" {
                source
            } else {
                format!(
                    r#"{source}
                    postMessage([new WebAssembly.Instance(wasm).exports.answer(),
                        new Uint8Array(WebAssembly.Module.customSections(wasm, 'x')[0])[0]]);
                "#
                )
            };
            let mut handle = server.worker(source, WorkerScriptKind::Module);
            server
                .respond_bytes(
                    &format!("/worker/{name}"),
                    "200 OK",
                    "Application/Wasm; charset=UTF-8",
                    WASM_WITH_NON_UTF8_SECTION,
                )
                .await;
            let expected = if phase == "evaluation" {
                "42"
            } else {
                "[42,255]"
            };
            assert_eq!(
                recv_post_json(&mut handle).await,
                expected,
                "{name}: {phase}"
            );
            handle.terminate_and_join();
            server.assert_no_more_requests();
        }
    }
}

#[tokio::test]
async fn worker_javascript_at_wasm_url_is_not_poisoned_by_source_import() {
    ensure_v8();
    for source_first in [true, false] {
        let mut server = ModuleSourceServer::start().await;
        let imports = if source_first {
            "[loadSource(), loadEvaluation()]"
        } else {
            "[loadEvaluation(), loadSource()]"
        };
        let mut handle = server.worker(format!(r#"
            const loadSource = () => import.source('./module.wasm').then(() => 'unexpected', e => e.name);
            const loadEvaluation = () => import('./module.wasm').then(m => m.answer);
            const results = await Promise.all({imports});
            postMessage([results, await loadSource(), self.runs]);
        "#), WorkerScriptKind::Module);
        server
            .respond(
                "/worker/module.wasm",
                "200 OK",
                "text/javascript",
                "self.runs = (self.runs || 0) + 1; export const answer = 42;",
            )
            .await;
        let expected = if source_first {
            r#"[["SyntaxError",42],"SyntaxError",1]"#
        } else {
            r#"[[42,"SyntaxError"],"SyntaxError",1]"#
        };
        assert_eq!(recv_post_json(&mut handle).await, expected);
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_static_source_import_rejects_javascript_without_evaluating_it() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        "import source source from './module.wasm'; postMessage('unexpected root');".into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/module.wasm",
            "200 OK",
            "text/javascript",
            "postMessage('unexpected dependency');",
        )
        .await;
    match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(message.contains("WebAssembly"), "{message}")
        }
        other => panic!("expected source-phase rejection before execution, got {other:?}"),
    }
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_data_wasm_imports_preserve_bytes_and_share_source_with_evaluation() {
    ensure_v8();
    let data_url = module_data_url("application/wasm", WASM_WITH_NON_UTF8_SECTION);
    let data_url = serde_json::to_string(&data_url).unwrap();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
            import source wasm from {data_url};
            const again = await import.source({data_url});
            const namespace = await import({data_url});
            postMessage([wasm === again, namespace.answer(),
                new Uint8Array(WebAssembly.Module.customSections(wasm, 'x')[0])[0]]);
        "#
            ),
            "https://example.test/worker/main.js".into(),
        )
        .with_script_kind(WorkerScriptKind::Module),
    );
    assert_eq!(recv_post_json(&mut handle).await, "[true,42,255]");
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_data_javascript_and_json_modules_use_utf8_decoding() {
    ensure_v8();
    let js = module_data_url(
        "text/javascript;charset=windows-1252",
        b"\xef\xbb\xbfexport const text = 'a\xffb';",
    );
    let json = module_data_url(
        "application/json;charset=windows-1252",
        b"\xef\xbb\xbf{\"text\":\"a\xffb\"}",
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
            const js = await import({});
            const json = await import({}, {{with: {{type: 'json'}}}});
            postMessage([js.text, json.default.text]);
        "#,
                serde_json::to_string(&js).unwrap(),
                serde_json::to_string(&json).unwrap()
            ),
            "https://example.test/worker/main.js".into(),
        )
        .with_script_kind(WorkerScriptKind::Module),
    );
    assert_eq!(recv_post_json(&mut handle).await, r#"["a�b","a�b"]"#);
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_data_modules_enforce_mime_and_json_import_attributes() {
    ensure_v8();
    for (mime, json_attribute, body, expected) in [
        ("text/javascript", false, "export const answer = 42;", "42"),
        (
            "text/plain",
            false,
            "export const answer = 42;",
            r#""TypeError""#,
        ),
        ("", false, "export const answer = 42;", r#""TypeError""#),
        ("application/json", true, r#"{"answer":42}"#, "42"),
        ("Text/JSON; charset=UTF-8", true, r#"{"answer":42}"#, "42"),
        ("application/manifest+json", true, r#"{"answer":42}"#, "42"),
        ("text/plain", true, r#"{"answer":42}"#, r#""TypeError""#),
        (
            "text/javascript",
            true,
            r#"{"answer":42}"#,
            r#""TypeError""#,
        ),
        (
            "application/json",
            false,
            r#"{"answer":42}"#,
            r#""TypeError""#,
        ),
    ] {
        let url = serde_json::to_string(&module_data_url(mime, body.as_bytes())).unwrap();
        let options = if json_attribute {
            ", {with: {type: 'json'}}"
        } else {
            ""
        };
        let result = if json_attribute {
            "m.default.answer"
        } else {
            "m.answer"
        };
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(format!("import({url}{options}).then(m => postMessage({result}), e => postMessage(e.name));"),
                "https://example.test/worker/main.js".into()).with_script_kind(WorkerScriptKind::Module),
        );
        assert_eq!(
            recv_post_json(&mut handle).await,
            expected,
            "{mime}, JSON: {json_attribute}"
        );
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_json_imports_accept_text_json_response_mime() {
    ensure_v8();
    for source in [
        "import data from './module.json' with {type: 'json'}; postMessage(data.answer);",
        "const {default: data} = await import('./module.json', {with: {type: 'json'}}); postMessage(data.answer);",
    ] {
        let mut server = ModuleSourceServer::start().await;
        let mut handle = server.worker(source.into(), WorkerScriptKind::Module);
        server
            .respond(
                "/worker/module.json",
                "200 OK",
                "Text/JSON; charset=UTF-8",
                r#"{"answer":42}"#,
            )
            .await;
        assert_eq!(recv_post_json(&mut handle).await, "42");
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_module_map_separates_json_from_javascript_or_wasm() {
    ensure_v8();
    for wasm in [true, false] {
        let mut server = ModuleSourceServer::start().await;
        let (wrong, right, mime, body) = if wasm {
            (
                "import('./module', {with: {type: 'json'}})",
                "import.source('./module').then(m => new WebAssembly.Instance(m).exports.answer())",
                "application/wasm",
                WASM_WITH_NON_UTF8_SECTION,
            )
        } else {
            (
                "import('./module')",
                "import('./module', {with: {type: 'json'}}).then(m => m.default.answer)",
                "application/json",
                br#"{"answer":42}"#.as_slice(),
            )
        };
        let mut handle = server.worker(
            format!(
                r#"
            const error = await {wrong}.then(() => 'unexpected', e => e.name);
            const first = await {right};
            const again = await {right};
            postMessage([error, first, again]);
        "#
            ),
            WorkerScriptKind::Module,
        );
        for _ in 0..2 {
            server
                .respond_bytes("/worker/module", "200 OK", mime, body)
                .await;
        }
        assert_eq!(recv_post_json(&mut handle).await, r#"["TypeError",42,42]"#);
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_javascript_root_at_wasm_url_reuses_the_module_map_entry() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker_at(r#"
        export const answer = 42;
        onmessage = () => import(import.meta.url).then(m => postMessage(m.answer), e => postMessage(e.name));
        postMessage('ready');
    "#.into(), WorkerScriptKind::Module, "main.wasm");
    assert_eq!(recv_post_json(&mut handle).await, r#""ready""#);
    handle.post_message(serialize_test_string("import root"));
    assert_eq!(recv_post_json(&mut handle).await, "42");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_wasm_root_without_wasm_suffix_can_be_imported_as_source() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/worker-helper.js", "HTTP/1.1 200 OK", "text/javascript",
        r#"export function pm(value) {
            import.source('./main.bin').then(source => postMessage([value, source instanceof WebAssembly.Module]),
                error => postMessage({error: error.name}));
        }"#.into(), Duration::ZERO,
    )]).await;
    let mut handle = spawn_worker_with_source_and_kind_and_network_policy(
        WorkerScriptSource::binary(WORKER_WASM_IMPORT_PM.to_vec()),
        format!("{base_url}/worker/main.bin"),
        worker_test_request_client(),
        WorkerScriptKind::Module,
        WorkerNetworkPolicy::default(),
    );
    assert_eq!(recv_post_json(&mut handle).await, "[42,true]");
    handle.terminate_and_join();
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_source_import_records_the_actual_wasm_resource_kind() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = spawn_test_worker_with_options(service_worker_module_options(
        "import source wasm from './module.js'; if (!(wasm instanceof WebAssembly.Module)) throw new Error('not wasm'); skipWaiting();".into(),
        format!("{}/worker/sw.js", server.url),
    ));
    server
        .respond_bytes(
            "/worker/module.js",
            "200 OK",
            "application/wasm",
            WASM_WITH_NON_UTF8_SECTION,
        )
        .await;
    let mut resource = None;
    let mut ready = false;
    while resource.is_none() || !ready {
        match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
            WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
                resource: loaded, ..
            } => resource = Some(loaded),
            WorkerToParentMessage::ServiceWorkerSkipWaiting { .. } => ready = true,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected bootstrap error: {message}")
            }
            _ => {}
        }
    }
    let resource = resource.unwrap();
    assert_eq!(resource.kind, WorkerScriptResourceKind::WebAssemblyModule);
    assert_eq!(resource.body_len, WASM_WITH_NON_UTF8_SECTION.len());
    assert_eq!(resource.body_sha256, sha256_hex(WASM_WITH_NON_UTF8_SECTION));
    handle.terminate_and_join();
    server.assert_no_more_requests();
}
