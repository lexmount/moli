use super::*;
use crate::worker::{WorkerErrorPhase, WorkerScriptResourceKind};
use moli_crypto::sha256_hex;

const WORKER_WASM_IMPORT_PM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7f, 0x00, 0x60,
    0x00, 0x00, 0x02, 0x19, 0x01, 0x12, 0x2e, 0x2f, 0x77, 0x6f, 0x72, 0x6b, 0x65, 0x72, 0x2d, 0x68,
    0x65, 0x6c, 0x70, 0x65, 0x72, 0x2e, 0x6a, 0x73, 0x02, 0x70, 0x6d, 0x00, 0x00, 0x03, 0x02, 0x01,
    0x01, 0x08, 0x01, 0x01, 0x0a, 0x08, 0x01, 0x06, 0x00, 0x41, 0x2a, 0x10, 0x00, 0x0b,
];

const WORKER_WASM_EXPORTED_NAMES: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x04, 0x04, 0x01, 0x6f, 0x00, 0x0a, 0x05, 0x04, 0x01, 0x01, 0x00, 0x0a, 0x06, 0x06,
    0x01, 0x7f, 0x00, 0x41, 0x00, 0x0b, 0x07, 0x1b, 0x04, 0x04, 0x67, 0x6c, 0x6f, 0x62, 0x03, 0x00,
    0x03, 0x6d, 0x65, 0x6d, 0x02, 0x00, 0x03, 0x74, 0x61, 0x62, 0x01, 0x00, 0x04, 0x66, 0x75, 0x6e,
    0x63, 0x00, 0x00, 0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b,
];

const WORKER_MUTABLE_GLOBAL_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x04, 0x04, 0x01, 0x6f, 0x00, 0x0a, 0x05, 0x04, 0x01, 0x01, 0x00, 0x0a, 0x06, 0x06,
    0x01, 0x7f, 0x01, 0x41, 0x00, 0x0b, 0x07, 0x1b, 0x04, 0x04, 0x67, 0x6c, 0x6f, 0x62, 0x03, 0x00,
    0x03, 0x6d, 0x65, 0x6d, 0x02, 0x00, 0x03, 0x74, 0x61, 0x62, 0x01, 0x00, 0x04, 0x66, 0x75, 0x6e,
    0x63, 0x00, 0x00, 0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b,
];

const WORKER_MUTABLE_GLOBAL_LIVE_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x09, 0x02, 0x60, 0x01, 0x7f, 0x00, 0x60,
    0x00, 0x01, 0x7f, 0x03, 0x03, 0x02, 0x00, 0x01, 0x06, 0x06, 0x01, 0x7f, 0x01, 0x41, 0x2a, 0x0b,
    0x07, 0x28, 0x03, 0x0c, 0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x56, 0x61, 0x6c, 0x75, 0x65,
    0x03, 0x00, 0x09, 0x73, 0x65, 0x74, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x00, 0x00, 0x09, 0x67,
    0x65, 0x74, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x00, 0x01, 0x0a, 0x0d, 0x02, 0x06, 0x00, 0x20,
    0x00, 0x24, 0x00, 0x0b, 0x04, 0x00, 0x23, 0x00, 0x0b,
];

const WORKER_MUTABLE_GLOBAL_REEXPORT_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x09, 0x02, 0x60, 0x01, 0x7f, 0x00, 0x60,
    0x00, 0x01, 0x7f, 0x02, 0x2e, 0x01, 0x1c, 0x2e, 0x2f, 0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65,
    0x2d, 0x67, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x2d, 0x65, 0x78, 0x70, 0x6f, 0x72, 0x74, 0x2e, 0x77,
    0x61, 0x73, 0x6d, 0x0c, 0x6d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x56, 0x61, 0x6c, 0x75, 0x65,
    0x03, 0x7f, 0x01, 0x03, 0x03, 0x02, 0x00, 0x01, 0x07, 0x42, 0x03, 0x16, 0x72, 0x65, 0x65, 0x78,
    0x70, 0x6f, 0x72, 0x74, 0x65, 0x64, 0x4d, 0x75, 0x74, 0x61, 0x62, 0x6c, 0x65, 0x56, 0x61, 0x6c,
    0x75, 0x65, 0x03, 0x00, 0x11, 0x73, 0x65, 0x74, 0x49, 0x6d, 0x70, 0x6f, 0x72, 0x74, 0x65, 0x64,
    0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x00, 0x00, 0x11, 0x67, 0x65, 0x74, 0x49, 0x6d, 0x70, 0x6f,
    0x72, 0x74, 0x65, 0x64, 0x47, 0x6c, 0x6f, 0x62, 0x61, 0x6c, 0x00, 0x01, 0x0a, 0x0d, 0x02, 0x06,
    0x00, 0x20, 0x00, 0x24, 0x00, 0x0b, 0x04, 0x00, 0x23, 0x00, 0x0b,
];

const WORKER_INVALID_MODULE_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03,
    0x02, 0x01, 0x00, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x42, 0x00, 0x0b,
];

const WORKER_WASM_IMPORT_CYCLE_JS: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02, 0x10,
    0x01, 0x0a, 0x2e, 0x2f, 0x63, 0x79, 0x63, 0x6c, 0x65, 0x2e, 0x6a, 0x73, 0x01, 0x66, 0x00, 0x00,
    0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00, 0x01, 0x0a, 0x06, 0x01,
    0x04, 0x00, 0x10, 0x00, 0x0b,
];

fn worker_wasm_import_pm_body() -> String {
    String::from_utf8(WORKER_WASM_IMPORT_PM.to_vec()).expect("test wasm should be ASCII bytes")
}

fn worker_wasm_exported_names_body() -> String {
    String::from_utf8(WORKER_WASM_EXPORTED_NAMES.to_vec()).expect("test wasm should be ASCII bytes")
}

fn worker_mutable_global_wasm_body() -> String {
    String::from_utf8(WORKER_MUTABLE_GLOBAL_WASM.to_vec()).expect("test wasm should be ASCII bytes")
}

fn worker_mutable_global_live_wasm_body() -> String {
    String::from_utf8(WORKER_MUTABLE_GLOBAL_LIVE_WASM.to_vec())
        .expect("test wasm should be ASCII bytes")
}

fn worker_mutable_global_reexport_wasm_body() -> String {
    String::from_utf8(WORKER_MUTABLE_GLOBAL_REEXPORT_WASM.to_vec())
        .expect("test wasm should be ASCII bytes")
}

fn worker_invalid_module_wasm_body() -> String {
    String::from_utf8(WORKER_INVALID_MODULE_WASM.to_vec()).expect("test wasm should be ASCII bytes")
}

fn worker_wasm_import_cycle_js_body() -> String {
    String::from_utf8(WORKER_WASM_IMPORT_CYCLE_JS.to_vec())
        .expect("test wasm should be ASCII bytes")
}

#[tokio::test]
async fn worker_importscripts_symbol_throws_type_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            importScripts(Symbol("worker.js"));
            postMessage("unexpected");
        } catch (error) {
            postMessage(error.name);
        }
        "#
        .into(),
        "test://importscripts_symbol".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""TypeError""#);
}

#[tokio::test]
async fn worker_importscripts_invalid_url_throws_syntax_error_before_running_any_script() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            importScripts(
                "data:text/javascript,globalThis.__ran=true",
                "http://foo bar"
            );
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                ran: globalThis.__ran === true,
            });
        }
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"SyntaxError","ran":false}"#
    );
}

#[tokio::test]
async fn worker_importscripts_obeys_response_csp_script_src() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            try {
                importScripts("data:text/javascript,globalThis.__ran=true");
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    events,
                    name: error && error.name,
                    ran: globalThis.__ran === true,
                });
            }
            close();
            "#
            .into(),
            "https://app.test/worker/main.js".into(),
        )
        .with_content_security_policies(vec!["script-src 'none'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"events":[{"type":"securitypolicyviolation","effectiveDirective":"script-src","violatedDirective":"script-src","blockedURI":"data","documentURI":"https://app.test/worker/main.js","originalPolicy":"script-src 'none'","disposition":"enforce","instance":true}],"name":"NetworkError","ran":false}"#
    );
}

#[tokio::test]
async fn worker_csp_violation_event_survives_mutated_event_globals() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            self.Event = null;
            Object.defineProperty(SecurityPolicyViolationEvent.prototype, "blockedURI", {
                value: "prototype-blocked-uri",
                writable: false,
                configurable: true
            });
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            try {
                importScripts("data:text/javascript,globalThis.__ran=true");
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    events,
                    name: error && error.name,
                    ran: globalThis.__ran === true,
                });
            }
            close();
            "#
            .into(),
            "https://app.test/worker/main.js".into(),
        )
        .with_content_security_policies(vec!["script-src 'none'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"events":[{"type":"securitypolicyviolation","blockedURI":"data","effectiveDirective":"script-src","disposition":"enforce","instance":true}],"name":"NetworkError","ran":false}"#
    );
}

#[tokio::test]
async fn shared_worker_importscripts_csp_block_dispatches_securitypolicyviolation_event() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.test".to_owned(),
        "https://app.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                let matched = false;
                addEventListener("securitypolicyviolation", event => {
                    matched = event.type === "securitypolicyviolation" &&
                        event.effectiveDirective === "script-src" &&
                        event.violatedDirective === "script-src" &&
                        event.blockedURI === "data" &&
                        event.documentURI === "https://app.test/shared-worker.js" &&
                        event.originalPolicy === "script-src 'none'" &&
                        event.disposition === "enforce" &&
                        event instanceof SecurityPolicyViolationEvent;
                });
                try {
                    importScripts("data:text/javascript,globalThis.__ran=true");
                } catch (_) {
                    if (matched && globalThis.__ran !== true) {
                        close();
                    }
                }
            };
            "#
            .into(),
            "https://app.test/shared-worker.js".into(),
        )
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        })
        .with_content_security_policies(vec!["script-src 'none'".to_owned()]),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn worker_importscripts_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            importScripts("data:text/javascript,globalThis.__ran=true");
            postMessage({
                events,
                ran: globalThis.__ran === true,
            });
            close();
            "#
            .into(),
            "https://app.test/worker/main.js".into(),
        )
        .with_content_security_report_only_policies(vec!["script-src 'none'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"events":[{"type":"securitypolicyviolation","effectiveDirective":"script-src","violatedDirective":"script-src","blockedURI":"data","documentURI":"https://app.test/worker/main.js","originalPolicy":"script-src 'none'","disposition":"report","instance":true}],"ran":true}"#
    );
}

#[tokio::test]
async fn shared_worker_importscripts_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.test".to_owned(),
        "https://app.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                let matched = false;
                addEventListener("securitypolicyviolation", event => {
                    matched = event.type === "securitypolicyviolation" &&
                        event.effectiveDirective === "script-src" &&
                        event.violatedDirective === "script-src" &&
                        event.blockedURI === "data" &&
                        event.documentURI === "https://app.test/shared-worker.js" &&
                        event.originalPolicy === "script-src 'none'" &&
                        event.disposition === "report" &&
                        event instanceof SecurityPolicyViolationEvent;
                });
                importScripts("data:text/javascript,globalThis.__ran=true");
                if (matched && globalThis.__ran === true) {
                    close();
                }
            };
            "#
            .into(),
            "https://app.test/shared-worker.js".into(),
        )
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        })
        .with_content_security_report_only_policies(vec!["script-src 'none'".to_owned()]),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn worker_importscripts_syntax_error_preserves_prior_side_effects_and_stops_later_scripts() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            importScripts(
                "data:text/javascript,globalThis.__first='ok'",
                "data:text/javascript,globalThis.__broken = ;",
                "data:text/javascript,globalThis.__third='unexpected'"
            );
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                first: globalThis.__first,
                hasThird: Object.prototype.hasOwnProperty.call(globalThis, "__third"),
                name: error && error.name,
                syntax: error instanceof SyntaxError,
            });
        }
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"first":"ok","hasThird":false,"name":"SyntaxError","syntax":true}"#
    );
}

#[tokio::test]
async fn worker_importscripts_runtime_throw_preserves_thrown_value_and_stops_later_scripts() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            importScripts(
                "data:text/javascript,globalThis.__x=1",
                "data:text/javascript,throw 2",
                "data:text/javascript,globalThis.__z=3"
            );
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                x: globalThis.__x,
                thrown: error,
                hasZ: Object.prototype.hasOwnProperty.call(globalThis, "__z"),
            });
        }
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"x":1,"thrown":2,"hasZ":false}"#);
}

#[tokio::test]
async fn worker_importscripts_revoked_blob_url_throws_network_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const scriptUrl = URL.createObjectURL(new Blob([
            "globalThis.__ran = true;"
        ], { type: "text/javascript" }));
        URL.revokeObjectURL(scriptUrl);
        try {
            importScripts(scriptUrl);
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                ran: globalThis.__ran === true,
            });
        }
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"NetworkError","ran":false}"#
    );
}

#[tokio::test]
async fn worker_importscripts_prepared_blob_url_survives_revoke_in_earlier_script() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const runScriptUrl = URL.createObjectURL(new Blob([
            "globalThis.__ran = true;"
        ], { type: "text/javascript" }));
        const revokeScriptUrl = URL.createObjectURL(new Blob([
            `URL.revokeObjectURL(${JSON.stringify(runScriptUrl)});`
        ], { type: "text/javascript" }));
        importScripts(revokeScriptUrl, runScriptUrl);
        postMessage(globalThis.__ran === true);
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "true");
}

#[tokio::test]
async fn worker_importscripts_stringifies_undefined_null_and_number_arguments() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/undefined",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "globalThis.__undefinedLoaded = true;".to_owned(),
            Duration::ZERO,
        ),
        (
            "/worker/null",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "globalThis.__nullLoaded = true;".to_owned(),
            Duration::ZERO,
        ),
        (
            "/worker/1",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "globalThis.__oneLoaded = true;".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        importScripts(undefined, null, 1);
        postMessage({
            undefinedLoaded: globalThis.__undefinedLoaded === true,
            nullLoaded: globalThis.__nullLoaded === true,
            oneLoaded: globalThis.__oneLoaded === true,
        });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"undefinedLoaded":true,"nullLoaded":true,"oneLoaded":true}"#
    );
    server.await.expect("stringification server should finish");
}

#[tokio::test]
async fn worker_trusted_types_importscripts_enforces_script_url_sink() {
    ensure_v8();
    let script_url = worker_data_url(
        r#"
        globalThis.__trustedImportLoaded = true;
        "#,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
                const policy = trustedTypes.createPolicy("p", {{
                    createScriptURL: value => value
                }});
                const blocked = (() => {{
                    try {{
                        importScripts({script_url:?});
                        return "unexpected";
                    }} catch (error) {{
                        return error.name;
                    }}
                }})();
                importScripts(policy.createScriptURL({script_url:?}));
                postMessage({{
                    blocked,
                    loaded: globalThis.__trustedImportLoaded === true,
                    trusted: trustedTypes.isScriptURL(policy.createScriptURL("data:text/javascript,")),
                }});
                close();
                "#
            ),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["require-trusted-types-for 'script'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"blocked":"TypeError","loaded":true,"trusted":true}"#
    );
}

#[tokio::test]
async fn worker_trusted_types_rejects_forged_named_properties() {
    ensure_v8();
    let script_url = worker_data_url("globalThis.__forgedImportLoaded = true;");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
                const forged = {{
                    __moliTrustedTypeKind: "script-url",
                    __moliTrustedTypeValue: {script_url:?},
                }};
                const accepted = (() => {{
                    try {{
                        importScripts(forged);
                        return true;
                    }} catch (error) {{
                        return false;
                    }}
                }})();
                postMessage({{
                    accepted,
                    isScriptURL: trustedTypes.isScriptURL(forged),
                    loaded: globalThis.__forgedImportLoaded === true,
                }});
                close();
                "#
            ),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["require-trusted-types-for 'script'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"accepted":false,"isScriptURL":false,"loaded":false}"#
    );
}

#[tokio::test]
async fn worker_trusted_types_policy_create_survives_global_constructor_override() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const policy = trustedTypes.createPolicy("p", {
                createHTML: value => value,
                createScript: value => value,
                createScriptURL: value => value,
            });
            const originalNames = [
                TrustedHTML.name,
                TrustedScript.name,
                TrustedScriptURL.name,
            ];
            TrustedHTML = function FakeTrustedHTML() {};
            TrustedScript = function FakeTrustedScript() {};
            TrustedScriptURL = function FakeTrustedScriptURL() {};
            const html = policy.createHTML("<p>ok</p>");
            const script = policy.createScript("3 + 4");
            const scriptURL = policy.createScriptURL("data:text/javascript,");
            const evalValue = eval(script);
            postMessage({
                originalNames,
                evalValue,
                isHTML: trustedTypes.isHTML(html),
                isScript: trustedTypes.isScript(script),
                isScriptURL: trustedTypes.isScriptURL(scriptURL),
                values: [String(html), String(script), String(scriptURL)],
                constructorNames: [
                    html.constructor.name,
                    script.constructor.name,
                    scriptURL.constructor.name,
                ],
            });
            close();
            "#
            .to_owned(),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["require-trusted-types-for 'script'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"originalNames":["TrustedHTML","TrustedScript","TrustedScriptURL"],"evalValue":7,"isHTML":true,"isScript":true,"isScriptURL":true,"values":["<p>ok</p>","3 + 4","data:text/javascript,"],"constructorNames":["TrustedHTML","TrustedScript","TrustedScriptURL"]}"#
    );
}

#[tokio::test]
async fn worker_trusted_types_policy_callbacks_follow_webidl_contract() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
        r#"
        const facts = {};
        const extra = {};
        const callback = new Proxy(function() {
            "use strict";
            facts.thisIsUndefined = this === undefined;
            facts.arguments = [
                arguments[0],
                arguments[1],
                arguments[2] === extra,
                arguments.length,
            ];
            return null;
        }, {
            apply(target, receiver, args) {
                facts.proxyApply = (facts.proxyApply || 0) + 1;
                return Reflect.apply(target, receiver, args);
            },
        });
        const policy = trustedTypes.createPolicy("worker-callback", {
            createHTML: callback,
            createScriptURL: () => "\ud800",
        });
        const html = policy.createHTML("input", 7, extra);
        const scriptURL = policy.createScriptURL("url");
        let missingError = "none";
        try {
            policy.createScript("missing");
        } catch (error) {
            missingError = error && error.name;
        }
        const revoked = Proxy.revocable(() => "revoked", {});
        revoked.revoke();
        const revokedPolicy = trustedTypes.createPolicy("worker-revoked", {
            createHTML: revoked.proxy,
        });
        let revokedError = "none";
        try {
            revokedPolicy.createHTML("x");
        } catch (error) {
            revokedError = error && error.name;
        }
        postMessage({
            facts,
            methods: [
                typeof policy.createHTML,
                typeof policy.createScript,
                typeof policy.createScriptURL,
            ],
            html: String(html),
            scriptURLCodePoint: String(scriptURL).codePointAt(0),
            missingError,
            revokedError,
        });
        close();
        "#
        .to_owned(),
        "https://app.test/worker/main.js".to_owned(),
    ));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"facts":{"proxyApply":1,"thisIsUndefined":true,"arguments":["input",7,true,3]},"methods":["function","function","function"],"html":"","scriptURLCodePoint":65533,"missingError":"TypeError","revokedError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_trusted_types_timers_and_eval_use_script_sink() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const policy = trustedTypes.createPolicy("p", {
                createScript: value => value.replace("__VALUE__", "7")
            });
            const blockedTimer = (() => {
                try {
                    setTimeout("postMessage('unexpected')");
                    return "unexpected";
                } catch (error) {
                    return error.name;
                }
            })();
            const blockedEval = (() => {
                try {
                    eval("2");
                    return "unexpected";
                } catch (error) {
                    return `${error.name}:${error instanceof EvalError}`;
                }
            })();
            const evalValue = eval(policy.createScript("__VALUE__"));
            let defaultSink = null;
            trustedTypes.createPolicy("default", {
                createScript: (value, _, sink) => {
                    defaultSink = sink;
                    return value;
                }
            });
            const defaultEvalValue = eval("9");
            setTimeout(policy.createScript("postMessage({blockedTimer, blockedEval, evalValue, defaultEvalValue, defaultSink})"));
            "#
            .to_owned(),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["require-trusted-types-for 'script'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"blockedTimer":"TypeError","blockedEval":"EvalError:true","evalValue":7,"defaultEvalValue":9,"defaultSink":"eval"}"#
    );
}

#[tokio::test]
async fn worker_trusted_script_eval_is_unwrapped_with_trusted_types_eval_keyword() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const policy = trustedTypes.createPolicy("p", {
                createScript: value => value
            });
            postMessage({
                trusted: eval(policy.createScript("3 + 4")),
                string: eval("4 + 5"),
            });
            close();
            "#
            .to_owned(),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec![
            "script-src 'trusted-types-eval'; require-trusted-types-for 'script'".to_owned(),
        ]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"trusted":7,"string":9}"#);
}

#[tokio::test]
async fn worker_trusted_types_eval_keyword_requires_enforced_trusted_types() {
    ensure_v8();
    for report_only_policies in [
        Vec::new(),
        vec!["require-trusted-types-for 'script'".to_owned()],
    ] {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                r#"
                let evalRan = false;
                let errorName = null;
                trustedTypes.createPolicy("default", { createScript: value => value });
                addEventListener("securitypolicyviolation", event => {
                    postMessage({
                        evalRan,
                        errorName,
                        event: {
                            type: event.type,
                            effectiveDirective: event.effectiveDirective,
                            violatedDirective: event.violatedDirective,
                            blockedURI: event.blockedURI,
                            documentURI: event.documentURI,
                            originalPolicy: event.originalPolicy,
                            disposition: event.disposition,
                            instance: event instanceof SecurityPolicyViolationEvent,
                        },
                    });
                    close();
                });
                try {
                    eval("evalRan = true");
                    errorName = "allowed";
                } catch (error) {
                    errorName = `${error.name}:${error instanceof EvalError}`;
                }
                postMessage({ phase: "evaluated", evalRan, errorName });
                "#
                .to_owned(),
                "https://app.test/worker/main.js".to_owned(),
            )
            .with_content_security_policies(vec![
                "script-src 'self' 'trusted-types-eval'".to_owned(),
            ])
            .with_content_security_report_only_policies(report_only_policies),
        );

        let evaluated = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(
            expect_post_json(evaluated),
            r#"{"phase":"evaluated","evalRan":false,"errorName":"EvalError:true"}"#
        );
        let violation = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for violation")
            .expect("channel closed");
        assert_eq!(
            expect_post_json(violation),
            r#"{"evalRan":false,"errorName":"EvalError:true","event":{"type":"securitypolicyviolation","effectiveDirective":"script-src","violatedDirective":"script-src","blockedURI":"eval","documentURI":"https://app.test/worker/main.js","originalPolicy":"script-src 'self' 'trusted-types-eval'","disposition":"enforce","instance":true}}"#
        );
    }
}

#[tokio::test]
async fn worker_trusted_types_eval_keyword_allows_eval_when_trusted_types_are_enforced() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            let violations = 0;
            addEventListener("securitypolicyviolation", () => violations++);
            const value = eval("40 + 2");
            setTimeout(() => {
                postMessage({ value, violations });
                close();
            });
            "#
            .to_owned(),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec![
            "script-src 'self' 'trusted-types-eval'; require-trusted-types-for 'script'".to_owned(),
        ]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"value":42,"violations":0}"#);
}

#[tokio::test]
async fn worker_trusted_script_code_like_brand_drives_function_constructor() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const policy = trustedTypes.createPolicy("function-brand", {
                createScript: value => value
            });
            const source = ["a", "b", "return a + b;"];
            const trusted = source.map(value => policy.createScript(value));
            let mixedBlocked = false;
            try {
                new Function(trusted[0], "b", trusted[2]);
            } catch (error) {
                mixedBlocked = error instanceof EvalError;
            }
            const constructed = new Function(...trusted);
            postMessage({ mixedBlocked, value: constructed(20, 22) });
            "#
            .to_owned(),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["require-trusted-types-for 'script'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"mixedBlocked":true,"value":42}"#);
}

#[tokio::test]
async fn worker_module_static_imports_resolve_http_dependencies_against_module_url() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/dep.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            [
                "export const answer = 42;",
                "export const suffix = 'dep';",
                "export default 'default-value';",
                "export function double(value) { return value * 2; }",
                "export class Box { constructor(value) { this.value = value; } }",
            ]
            .join("\n"),
            Duration::ZERO,
        ),
        (
            "/worker/reexport.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            [
                "export { answer as importedAnswer, default as importedDefault } from './dep.js';",
                "export * as depNamespace from './dep.js';",
            ]
            .join("\n"),
            Duration::ZERO,
        ),
        (
            "/worker/star.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "export * from './dep.js';".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import defaultValue, { answer, suffix as renamed, double, Box } from "./dep.js";
        import * as re from "./reexport.js";
        import { answer as starAnswer } from "./star.js";
        var box = new Box(9);
        postMessage({
            defaultValue,
            answer,
            renamed,
            doubled: double(21),
            boxValue: box.value,
            importedAnswer: re.importedAnswer,
            importedDefault: re.importedDefault,
            namespaceAnswer: re.depNamespace.answer,
            starAnswer,
            importScriptsType: typeof importScripts,
            metaUrl: import.meta.url,
        });
        close();
        "#
        .into(),
        script_url.clone(),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"defaultValue":"default-value","answer":42,"renamed":"dep","doubled":42,"boxValue":9,"importedAnswer":42,"importedDefault":"default-value","namespaceAnswer":42,"starAnswer":42,"importScriptsType":"function","metaUrl":"{script_url}"}}"#
        )
    );
    server
        .await
        .expect("worker module import server should finish");
}

#[tokio::test]
async fn worker_module_static_sibling_dependencies_fetch_in_parallel() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker module sibling server");
    let addr = listener.local_addr().expect("worker module sibling addr");
    let base_url = format!("http://{addr}");
    let server = tokio::spawn(async move {
        let mut first_stream = None;
        let mut first_path = String::new();
        let mut second_stream = None;
        let mut second_path = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker module sibling request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker module sibling request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker module sibling request path")
                .to_owned();
            if first_stream.is_none() {
                first_path = path;
                first_stream = Some(stream);
            } else {
                second_path = path;
                second_stream = Some(stream);
            }
        }
        let mut paths = vec![first_path.clone(), second_path.clone()];
        paths.sort();
        assert_eq!(paths, vec!["/worker/a.js", "/worker/b.js"]);
        for (path, stream) in [
            (first_path, first_stream.expect("first sibling stream")),
            (second_path, second_stream.expect("second sibling stream")),
        ] {
            let body = match path.as_str() {
                "/worker/a.js" => "export const a = 'a';",
                "/worker/b.js" => "export const b = 'b';",
                other => panic!("unexpected worker module sibling path: {other}"),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let mut stream = stream;
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker module sibling response");
        }
    });

    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker module sibling loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import { a } from "./a.js";
        import { b } from "./b.js";
        postMessage({ a, b });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"a":"a","b":"b"}"#);
    server
        .await
        .expect("worker module sibling server should finish");
}

#[tokio::test]
async fn worker_module_fetches_completed_sibling_descendants_before_slow_sibling_finishes() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker module descendant server");
    let addr = listener
        .local_addr()
        .expect("worker module descendant addr");
    let base_url = format!("http://{addr}");
    let server = tokio::spawn(async move {
        let mut first_stream = None;
        let mut first_path = String::new();
        let mut second_stream = None;
        let mut second_path = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker module sibling request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker module sibling request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker module sibling request path")
                .to_owned();
            if first_stream.is_none() {
                first_path = path;
                first_stream = Some(stream);
            } else {
                second_path = path;
                second_stream = Some(stream);
            }
        }

        let (mut a_stream, mut b_stream) = match (
            first_path.as_str(),
            first_stream.expect("first sibling stream"),
            second_path.as_str(),
            second_stream.expect("second sibling stream"),
        ) {
            ("/worker/a.js", a_stream, "/worker/b.js", b_stream)
            | ("/worker/b.js", b_stream, "/worker/a.js", a_stream) => (a_stream, b_stream),
            (first, _, second, _) => panic!("unexpected sibling paths: {first}, {second}"),
        };

        let a_body = "import { child } from './a-child.js'; export const a = `a${child}`;";
        let a_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            a_body.len(),
            a_body
        );
        a_stream
            .write_all(a_response.as_bytes())
            .await
            .expect("write worker module a response");

        let (mut child_stream, _) = listener
            .accept()
            .await
            .expect("accept worker module child request before b finishes");
        let child_request = read_http_request_head(&mut child_stream)
            .await
            .expect("read worker module child request");
        let child_path = child_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker module child request path");
        assert_eq!(
            child_path, "/worker/a-child.js",
            "completed sibling descendants should start before slow sibling completes"
        );

        let child_body = "export const child = 'child';";
        let child_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            child_body.len(),
            child_body
        );
        child_stream
            .write_all(child_response.as_bytes())
            .await
            .expect("write worker module child response");

        let b_body = "export const b = 'b';";
        let b_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            b_body.len(),
            b_body
        );
        b_stream
            .write_all(b_response.as_bytes())
            .await
            .expect("write worker module b response");
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker module descendant loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import { a } from "./a.js";
        import { b } from "./b.js";
        postMessage({ a, b });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"a":"achild","b":"b"}"#);
    server
        .await
        .expect("worker module descendant server should finish");
}

#[tokio::test]
async fn worker_module_importscripts_is_exposed_but_throws_type_error() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        try {
            importScripts("data:text/javascript,postMessage('unexpected')");
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                importScriptsType: typeof importScripts,
                name: error && error.name,
                message: error && error.message,
            });
        }
        close();
        "#
        .into(),
        "test://worker_module_importscripts".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"importScriptsType":"function","name":"TypeError","message":"Module scripts don't support importScripts()."}"#
    );
}

#[tokio::test]
async fn worker_module_same_origin_dependency_fetch_sends_cookies() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker module cookie server");
    let addr = listener.local_addr().expect("worker module cookie addr");
    let base_url = format!("http://{addr}");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker module dependency request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker module dependency request");
        let cookie_seen = request
            .lines()
            .any(|line| line.eq_ignore_ascii_case("cookie: wpt_worker_module_credentials=fixture"));
        let body = format!(
            "export const credentialCookie = '{}';",
            if cookie_seen { "seen" } else { "missing" }
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker module dependency response");
    });

    let cookie_store = moli_cookie_jar::new_shared_browser_cookie_store();
    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &url::Url::parse(&format!("{base_url}/worker/main.js")).unwrap(),
            &[(
                "set-cookie".to_owned(),
                "wpt_worker_module_credentials=fixture; Path=/worker; SameSite=Lax".to_owned(),
            )],
        );
    }
    let loader = ResourceRequestClient::new_with_cookie_store(
        &FetchConfig::default(),
        Arc::clone(&cookie_store),
    )
    .expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import { credentialCookie } from "./dep.js";
        postMessage(credentialCookie);
        close();
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""seen""#);
    server
        .await
        .expect("worker module cookie server should finish");
}

#[tokio::test]
async fn worker_module_redirected_http_dependency_uses_final_response_url_as_base() {
    ensure_v8();
    let dep_source = [
        "export const leafValue = 'redirect-leaf';",
        "export const leafMetaUrl = import.meta.url;",
    ]
    .join("\n");
    let entry_source = [
        "import { leafMetaUrl, leafValue } from './dep.js';",
        "export { leafMetaUrl, leafValue };",
        "export const entryMetaUrl = import.meta.url;",
    ]
    .join("\n");
    let dep_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        dep_source.len(),
        dep_source
    );
    let entry_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        entry_source.len(),
        entry_source
    );
    let (base_url, script_server) = spawn_raw_path_response_http_server(vec![
        ("/worker/final/dep.js", dep_response, Duration::ZERO),
        ("/worker/final/entry.js", entry_response, Duration::ZERO),
    ])
    .await;
    let redirect_response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {base_url}/worker/final/entry.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (redirect_base_url, redirect_server) = spawn_raw_path_response_http_server(vec![(
        "/worker/redirect-entry.js",
        redirect_response,
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker module redirect loader");
    let script_url = format!("{redirect_base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import { entryMetaUrl, leafMetaUrl, leafValue } from "./redirect-entry.js";
        postMessage({
            entryMetaUrl,
            leafMetaUrl,
            leafValue,
            mainMetaUrl: import.meta.url,
        });
        close();
        "#
        .into(),
        script_url.clone(),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"entryMetaUrl":"{base_url}/worker/final/entry.js","leafMetaUrl":"{base_url}/worker/final/dep.js","leafValue":"redirect-leaf","mainMetaUrl":"{script_url}"}}"#
        )
    );
    redirect_server
        .await
        .expect("worker module redirect server should finish");
    script_server
        .await
        .expect("worker module final-url server should finish");
}

#[tokio::test]
async fn worker_module_redirected_http_dependency_final_url_obeys_script_src() {
    ensure_v8();
    let target_source = "postMessage('unexpected module target');";
    let target_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target_source.len(),
        target_source
    );
    let (target_base_url, target_server) = spawn_raw_path_response_http_server(vec![(
        "/worker/final/entry.js",
        target_response,
        Duration::ZERO,
    )])
    .await;
    let redirect_response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {target_base_url}/worker/final/entry.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (source_base_url, source_server) = spawn_raw_path_response_http_server(vec![(
        "/worker/redirect-entry.js",
        redirect_response,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker module CSP redirect loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        import "./redirect-entry.js";
        postMessage("unexpected main");
        "#
            .into(),
            format!("{source_base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_module_static_import_content_security_policies(vec!["script-src 'self'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, phase, .. } => {
            assert!(
                message.contains("Content Security Policy"),
                "expected CSP error, got {message:?}"
            );
            assert_eq!(phase, WorkerErrorPhase::Bootstrap);
        }
        other => panic!("expected Error, got {other:?}"),
    }
    source_server
        .await
        .expect("worker module CSP redirect source server should finish");
    target_server
        .await
        .expect("worker module CSP redirect target server should finish");
}

#[tokio::test]
async fn worker_module_redirected_request_url_remains_module_key() {
    ensure_v8();
    let (base_url, request_paths_rx, server) =
        spawn_worker_redirected_module_key_reuse_server().await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker module redirect key loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        import { metaUrl as staticMetaUrl, value as staticValue } from "./redirect-entry.js";
        const dynamic = await import("./redirect-entry.js");
        postMessage({
          dynamicMetaUrl: dynamic.metaUrl,
          dynamicValue: dynamic.value,
          staticMetaUrl,
          staticValue,
        });
        close();
        "#
            .into(),
            format!("{base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker redirected key result")
        .expect("channel closed");
    let final_url = format!("{base_url}/worker/final/entry.js");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"dynamicMetaUrl":"{final_url}","dynamicValue":"redirect-key","staticMetaUrl":"{final_url}","staticValue":"redirect-key"}}"#
        )
    );
    let request_paths = request_paths_rx
        .await
        .expect("worker redirect key server should report paths");
    assert_eq!(
        request_paths,
        ["/worker/redirect-entry.js", "/worker/final/entry.js"]
    );
    handle.terminate_and_join();
    server
        .await
        .expect("worker redirect key server should finish");
}

async fn spawn_worker_redirected_module_key_reuse_server()
-> (String, oneshot::Receiver<Vec<String>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker redirect key server");
    let addr = listener
        .local_addr()
        .expect("worker redirect key server addr");
    let base_url = format!("http://{addr}");
    let server_base_url = base_url.clone();
    let (paths_tx, paths_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut request_paths = Vec::new();
        loop {
            let accept_result = if request_paths.len() < 2 {
                listener.accept().await.ok()
            } else {
                tokio::time::timeout(Duration::from_millis(250), listener.accept())
                    .await
                    .ok()
                    .and_then(Result::ok)
            };
            let Some((mut stream, _)) = accept_result else {
                break;
            };
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker redirect key request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker redirect key request path")
                .to_owned();
            request_paths.push(path.clone());
            let response = match path.as_str() {
                "/worker/redirect-entry.js" => format!(
                    "HTTP/1.1 302 Found\r\nLocation: {server_base_url}/worker/final/entry.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                "/worker/final/entry.js" => {
                    let body = r#"export const value = "redirect-key";
export const metaUrl = import.meta.url;"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                }
                _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned(),
            };
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker redirect key response");
            if request_paths.len() >= 4 {
                break;
            }
        }
        let _ = paths_tx.send(request_paths);
    });
    (base_url, paths_rx, server)
}

#[tokio::test]
async fn worker_module_static_import_uses_outside_csp_not_worker_response_csp() {
    ensure_v8();
    let target_source = r#"export const value = "static-ok";"#;
    let target_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target_source.len(),
        target_source
    );
    let (target_base_url, target_server) = spawn_raw_path_response_http_server(vec![(
        "/worker/dep.js",
        target_response,
        Duration::ZERO,
    )])
    .await;
    let dep_specifier = serde_json::to_string(&format!("{target_base_url}/worker/dep.js"))
        .expect("dependency URL should serialize");
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker module static CSP loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        import {{ value }} from {dep_specifier};
        postMessage(value);
        close();
        "#
            ),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_module_static_import_initiator_url(
            url::Url::parse("https://app.test/page.html").unwrap(),
        )
        .with_module_static_import_content_security_policies(vec![
            "worker-src *; script-src 'self'".to_owned(),
        ])
        .with_content_security_policies(vec!["script-src 'self'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""static-ok""#);
    target_server
        .await
        .expect("worker module static CSP server should finish");
}

#[tokio::test]
async fn worker_module_response_referrer_policy_controls_descendant_fetch() {
    ensure_v8();
    let (base_url, child_headers_rx, server) =
        spawn_worker_module_referrer_policy_descendant_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker module referrer loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        import "./a.js";
        "#
            .into(),
            format!("{base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_module_static_import_initiator_url(
            url::Url::parse("http://127.0.0.1:1/page.html").unwrap(),
        )
        .with_referrer_policy(Some("origin".to_owned())),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker module referrer result")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""referrer-policy-ok""#);
    let child_headers = child_headers_rx
        .await
        .expect("worker module referrer server should report child headers");
    assert!(
        !child_headers
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("referer:")),
        "descendant request should inherit parent module response Referrer-Policy: no-referrer; headers were {child_headers:?}"
    );
    handle.terminate_and_join();
    server
        .await
        .expect("worker module referrer server should finish");
}

async fn spawn_worker_module_referrer_policy_descendant_server()
-> (String, oneshot::Receiver<String>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker module referrer policy server");
    let addr = listener
        .local_addr()
        .expect("worker module referrer policy server addr");
    let (headers_tx, headers_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut first_stream, _) = listener
            .accept()
            .await
            .expect("accept worker module referrer parent request");
        let first_request = read_http_request_head(&mut first_stream)
            .await
            .expect("read worker module referrer parent request");
        let first_path = first_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker module referrer parent request path");
        assert_eq!(first_path, "/worker/a.js");
        let parent_body = r#"import { value } from "./a-child.js";
postMessage(value);
close();"#;
        let parent_response = format!(
            "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            parent_body.len(),
            parent_body
        );
        first_stream
            .write_all(parent_response.as_bytes())
            .await
            .expect("write worker module referrer parent response");

        let (mut child_stream, _) = listener
            .accept()
            .await
            .expect("accept worker module referrer child request");
        let child_request = read_http_request_head(&mut child_stream)
            .await
            .expect("read worker module referrer child request");
        let child_path = child_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker module referrer child request path");
        assert_eq!(child_path, "/worker/a-child.js");
        let _ = headers_tx.send(child_request);
        let child_body = r#"export const value = "referrer-policy-ok";"#;
        let child_response = format!(
            "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            child_body.len(),
            child_body
        );
        child_stream
            .write_all(child_response.as_bytes())
            .await
            .expect("write worker module referrer child response");
    });
    (format!("http://{addr}"), headers_rx, server)
}

#[tokio::test]
async fn classic_worker_dynamic_import_uses_worker_referrer_policy() {
    ensure_v8();
    let (base_url, headers_rx, server) =
        spawn_classic_worker_dynamic_import_referrer_policy_server().await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("classic dynamic referrer loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        import("./dynamic.js").then((ns) => {
          postMessage(ns.value);
          close();
        }).catch((error) => {
          postMessage("rejected:" + String(error && error.message));
          close();
        });
        "#
            .into(),
            format!("{base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_referrer_policy(Some("no-referrer".to_owned())),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for classic dynamic referrer result")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""classic-dynamic-referrer-ok""#);
    let headers = headers_rx
        .await
        .expect("classic dynamic referrer server should report headers");
    assert!(
        !headers
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("referer:")),
        "classic worker dynamic import root should inherit worker no-referrer policy; headers were {headers:?}"
    );
    handle.terminate_and_join();
    server
        .await
        .expect("classic dynamic referrer server should finish");
}

async fn spawn_classic_worker_dynamic_import_referrer_policy_server()
-> (String, oneshot::Receiver<String>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classic dynamic referrer policy server");
    let addr = listener
        .local_addr()
        .expect("classic dynamic referrer policy server addr");
    let (headers_tx, headers_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept classic dynamic referrer request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read classic dynamic referrer request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("classic dynamic referrer request path");
        assert_eq!(path, "/worker/dynamic.js");
        let _ = headers_tx.send(request);
        let body = r#"export const value = "classic-dynamic-referrer-ok";"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write classic dynamic referrer response");
    });
    (format!("http://{addr}"), headers_rx, server)
}

#[tokio::test]
async fn worker_dynamic_import_root_joins_inflight_fetch() {
    ensure_v8();
    let (base_url, request_paths_rx, server) =
        spawn_worker_dynamic_import_duplicate_root_server_with_response(
            "HTTP/1.1 200 OK",
            r#"export const value = "joined-root";"#,
        )
        .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker dynamic join loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        const events = [];
        const first = import("./dynamic.js").then((ns) => events.push("first:" + ns.value));
        const second = import("./dynamic.js").then((ns) => events.push("second:" + ns.value));
        Promise.all([first, second]).then(() => {
          postMessage(events.sort().join("|"));
          close();
        }).catch((error) => {
          postMessage("rejected:" + String(error && error.message));
          close();
        });
        "#
            .into(),
            format!("{base_url}/worker/main.js"),
        )
        .with_request_client(loader),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker dynamic root join result")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""first:joined-root|second:joined-root""#
    );
    let request_paths = request_paths_rx
        .await
        .expect("worker dynamic join server should report paths");
    assert_eq!(request_paths, ["/worker/dynamic.js"]);
    handle.terminate_and_join();
    server
        .await
        .expect("worker dynamic join server should finish");
}

#[tokio::test]
async fn worker_dynamic_import_root_join_waits_for_descendant_graph() {
    ensure_v8();
    let (base_url, request_paths_rx, server) =
        spawn_worker_dynamic_import_duplicate_root_with_dependency_server().await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker dynamic descendant join loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        const events = [];
        const first = import("./dynamic.js").then((ns) => events.push("first:" + ns.value));
        const second = import("./dynamic.js").then((ns) => events.push("second:" + ns.value));
        Promise.all([first, second]).then(() => {
          postMessage(events.sort().join("|"));
          close();
        }).catch((error) => {
          postMessage("rejected:" + String(error && error.message));
          close();
        });
        "#
            .into(),
            format!("{base_url}/worker/main.js"),
        )
        .with_request_client(loader),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker dynamic descendant join result")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""first:root-dep|second:root-dep""#);
    let request_paths = request_paths_rx
        .await
        .expect("worker dynamic descendant join server should report paths");
    assert_eq!(request_paths, ["/worker/dynamic.js", "/worker/dep.js"]);
    handle.terminate_and_join();
    server
        .await
        .expect("worker dynamic descendant join server should finish");
}

#[tokio::test]
async fn worker_dynamic_import_root_failure_fans_out_to_joined_import() {
    ensure_v8();
    let (base_url, request_paths_rx, server) =
        spawn_worker_dynamic_import_duplicate_root_server_with_response(
            "HTTP/1.1 500 Internal Server Error",
            "server-error",
        )
        .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker dynamic failure loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
        const events = [];
        const first = import("./dynamic.js").then(
          () => events.push("first:resolved"),
          (error) => events.push("first:" + String(error && error.message).includes("500"))
        );
        const second = import("./dynamic.js").then(
          () => events.push("second:resolved"),
          (error) => events.push("second:" + String(error && error.message).includes("500"))
        );
        Promise.all([first, second]).then(() => {
          postMessage(events.sort().join("|"));
          close();
        });
        "#
            .into(),
            format!("{base_url}/worker/main.js"),
        )
        .with_request_client(loader),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker dynamic root failure result")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""first:true|second:true""#);
    let request_paths = request_paths_rx
        .await
        .expect("worker dynamic failure server should report paths");
    assert_eq!(request_paths, ["/worker/dynamic.js"]);
    handle.terminate_and_join();
    server
        .await
        .expect("worker dynamic failure server should finish");
}

async fn spawn_worker_dynamic_import_duplicate_root_server_with_response(
    status_line: &'static str,
    body: &'static str,
) -> (String, oneshot::Receiver<Vec<String>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker dynamic duplicate root server");
    let addr = listener
        .local_addr()
        .expect("worker dynamic duplicate root server addr");
    let (paths_tx, paths_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut request_paths = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept first worker dynamic root request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read first worker dynamic root request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("first worker dynamic root request path")
            .to_owned();
        request_paths.push(path);
        let response = format!(
            "{status_line}\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write first worker dynamic root response");

        let duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        tokio::select! {
            accept = listener.accept() => {
                let (mut duplicate_stream, _) =
                    accept.expect("accept duplicate worker dynamic root request");
                let duplicate_request = read_http_request_head(&mut duplicate_stream)
                    .await
                    .expect("read duplicate worker dynamic root request");
                let duplicate_path = duplicate_request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("duplicate worker dynamic root request path")
                    .to_owned();
                request_paths.push(duplicate_path);
                let _ = duplicate_stream.write_all(response.as_bytes()).await;
            }
            _ = tokio::time::sleep_until(duplicate_deadline) => {
            }
        }
        let _ = paths_tx.send(request_paths);
    });
    (format!("http://{addr}"), paths_rx, server)
}

async fn spawn_worker_dynamic_import_duplicate_root_with_dependency_server()
-> (String, oneshot::Receiver<Vec<String>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker dynamic duplicate dependency server");
    let addr = listener
        .local_addr()
        .expect("worker dynamic duplicate dependency server addr");
    let (paths_tx, paths_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut request_paths = Vec::new();
        for expected_path in ["/worker/dynamic.js", "/worker/dep.js"] {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker dynamic dependency request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker dynamic dependency request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker dynamic dependency request path")
                .to_owned();
            request_paths.push(path.clone());
            assert_eq!(path, expected_path);
            let body = match path.as_str() {
                "/worker/dynamic.js" => {
                    r#"import { value as dep } from "./dep.js";
export const value = "root-" + dep;"#
                }
                "/worker/dep.js" => r#"export const value = "dep";"#,
                _ => unreachable!("unexpected worker dynamic dependency path"),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker dynamic dependency response");
        }

        let duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        tokio::select! {
            accept = listener.accept() => {
                let (mut duplicate_stream, _) =
                    accept.expect("accept duplicate worker dynamic dependency request");
                let duplicate_request = read_http_request_head(&mut duplicate_stream)
                    .await
                    .expect("read duplicate worker dynamic dependency request");
                let duplicate_path = duplicate_request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("duplicate worker dynamic dependency request path")
                    .to_owned();
                request_paths.push(duplicate_path);
                let _ = duplicate_stream.write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ).await;
            }
            _ = tokio::time::sleep_until(duplicate_deadline) => {}
        }
        let _ = paths_tx.send(request_paths);
    });
    (format!("http://{addr}"), paths_rx, server)
}

#[tokio::test]
async fn worker_dynamic_import_uses_worker_response_csp_not_outside_static_csp() {
    ensure_v8();
    let dep_specifier = serde_json::to_string("http://127.0.0.1:9/worker/dynamic.js")
        .expect("dependency URL should serialize");
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker module dynamic CSP loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        const events = [];
        addEventListener("securitypolicyviolation", event => {{
            events.push({{
                type: event.type,
                effectiveDirective: event.effectiveDirective,
                violatedDirective: event.violatedDirective,
                blockedURI: event.blockedURI,
                documentURI: event.documentURI,
                originalPolicy: event.originalPolicy,
                disposition: event.disposition,
                instance: event instanceof SecurityPolicyViolationEvent
            }});
        }});
        (async () => {{
            try {{
                const mod = await import({dep_specifier});
                postMessage({{ status: "unexpected", value: mod.value }});
            }} catch (error) {{
                postMessage({{
                    status: "blocked",
                    events,
                    name: error && error.name,
                    csp: String(error && error.message).includes("Content Security Policy"),
                }});
            }}
            close();
        }})();
        "#
            ),
            "https://app.test/worker/main.js".to_owned(),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_module_static_import_initiator_url(
            url::Url::parse("https://app.test/page.html").unwrap(),
        )
        .with_module_static_import_content_security_policies(vec![
            "worker-src *; script-src 'self'".to_owned(),
        ])
        .with_content_security_policies(vec!["script-src-elem 'self'; script-src *".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"status":"blocked","events":[{"type":"securitypolicyviolation","effectiveDirective":"script-src-elem","violatedDirective":"script-src-elem","blockedURI":"http://127.0.0.1:9/worker/dynamic.js","documentURI":"https://app.test/worker/main.js","originalPolicy":"script-src-elem 'self'; script-src *","disposition":"enforce","instance":true}],"name":"TypeError","csp":true}"#
    );
}

#[tokio::test]
async fn shared_worker_data_module_dynamic_import_allows_cors_dependency() {
    ensure_v8();
    let dynamic_source = r#"export const value = "cors-ok";"#;
    let dynamic_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        dynamic_source.len(),
        dynamic_source
    );
    let imported_source = r#"
        const importedModulesPromise =
            import("./dynamic.js")
                .then(module => module.value);

        onconnect = () => {
            importedModulesPromise.then(value => {
                if (value === "cors-ok") {
                    close();
                }
            });
        };
    "#;
    let imported_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        imported_source.len(),
        imported_source
    );
    let (target_base_url, target_server) = spawn_raw_path_response_http_server(vec![
        ("/worker/imported.js", imported_response, Duration::ZERO),
        ("/worker/dynamic.js", dynamic_response, Duration::ZERO),
    ])
    .await;
    let dep_specifier = serde_json::to_string(&format!("{target_base_url}/worker/imported.js"))
        .expect("imported module URL should serialize");
    let source = format!(
        r#"
        import {dep_specifier};
        "#
    );
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "null".to_owned(),
        "https://app.test".to_owned(),
        Some(moli_storage_key::OpaqueOriginNonce::new(1)),
        moli_storage_key::StoragePartitionRelation::Unknown,
    );
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("shared worker dynamic import loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source.clone(), worker_data_url(&source))
            .with_request_client(loader)
            .with_script_kind(WorkerScriptKind::Module)
            .with_global_kind(super::super::WorkerGlobalKind::Shared {
                name: "shared".to_owned(),
                storage_key,
            })
            .with_module_static_import_initiator_url(
                url::Url::parse("https://app.test/page.html").unwrap(),
            ),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match msg {
            WorkerToParentMessage::SharedWorkerClosed => break,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("shared worker reported unexpected error: {message}");
            }
            _ => {}
        }
    }
    target_server.abort();
}

#[tokio::test]
async fn shared_worker_data_module_dynamic_import_allows_data_dependency() {
    ensure_v8();
    let dep_specifier =
        serde_json::to_string(&worker_data_url(r#"export const value = "data-ok";"#))
            .expect("data dependency URL should serialize");
    let source = format!(
        r#"
        onconnect = () => {{
            import({dep_specifier})
                .then(module => {{
                    if (module.value === "data-ok") {{
                        close();
                    }}
                }});
        }};
        "#
    );
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "null".to_owned(),
        "https://app.test".to_owned(),
        Some(moli_storage_key::OpaqueOriginNonce::new(2)),
        moli_storage_key::StoragePartitionRelation::Unknown,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source.clone(), worker_data_url(&source))
            .with_script_kind(WorkerScriptKind::Module)
            .with_global_kind(super::super::WorkerGlobalKind::Shared {
                name: "shared".to_owned(),
                storage_key,
            })
            .with_module_static_import_initiator_url(
                url::Url::parse("https://app.test/page.html").unwrap(),
            ),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match msg {
            WorkerToParentMessage::SharedWorkerClosed => break,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("shared worker reported unexpected error: {message}");
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn shared_worker_dynamic_import_csp_block_dispatches_securitypolicyviolation_event() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.test".to_owned(),
        "https://app.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("shared worker dynamic CSP loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                let matched = false;
                addEventListener("securitypolicyviolation", event => {
                    matched = event.type === "securitypolicyviolation" &&
                        event.effectiveDirective === "script-src-elem" &&
                        event.violatedDirective === "script-src-elem" &&
                        event.blockedURI === "http://127.0.0.1:9/worker/dynamic.js" &&
                        event.documentURI === "https://app.test/shared-worker.js" &&
                        event.originalPolicy === "script-src 'self'" &&
                        event.disposition === "enforce" &&
                        event instanceof SecurityPolicyViolationEvent;
                });
                import("http://127.0.0.1:9/worker/dynamic.js").catch(() => {
                    if (matched) {
                        close();
                    }
                });
            };
            "#
            .into(),
            "https://app.test/shared-worker.js".into(),
        )
        .with_request_client(loader)
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        })
        .with_content_security_policies(vec!["script-src 'self'".to_owned()]),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn worker_module_named_imports_read_live_exports() {
    ensure_v8();
    let dep_url = worker_data_url(
        r#"
        export let counter = 1;
        export { counter as default };
        export function bump() { counter += 1; return counter; }
        export function setCounter(value) { counter = value; }
        "#,
    );
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import current, {{ counter, bump, setCounter }} from {dep_specifier};
        function readCounter() {{ return counter; }}
        const before = counter;
        const defaultBefore = current;
        const bumpReturn = bump();
        const afterBump = readCounter();
        setCounter(9);
        const afterSet = counter;
        const defaultAfterSet = current;
        const secondBump = bump();
        const afterSecondBump = readCounter();
        postMessage({{
            before,
            defaultBefore,
            bumpReturn,
            afterBump,
            afterSet,
            defaultAfterSet,
            secondBump,
            afterSecondBump,
        }});
        close();
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"before":1,"defaultBefore":1,"bumpReturn":2,"afterBump":2,"afterSet":9,"defaultAfterSet":9,"secondBump":10,"afterSecondBump":10}"#
    );
}

#[tokio::test]
async fn worker_module_no_import_source_runs_in_strict_mode() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        export const answer = 42;
        const topLevelThisUndefined = this === undefined;
        function sloppyThis() { return this; }
        let implicitGlobalError = "none";
        try {
            accidentalWorkerGlobal = 7;
        } catch (error) {
            implicitGlobalError = error && error.name;
        }
        postMessage({
            answer,
            topLevelThisUndefined,
            functionThisUndefined: sloppyThis() === undefined,
            implicitGlobalError,
            leakedGlobal: Object.prototype.hasOwnProperty.call(self, "accidentalWorkerGlobal"),
        });
        close();
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"answer":42,"topLevelThisUndefined":true,"functionThisUndefined":true,"implicitGlobalError":"ReferenceError","leakedGlobal":false}"#
    );
}

#[tokio::test]
async fn worker_module_top_level_await_fulfillment_completes_startup() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        await new Promise(resolve => setTimeout(resolve, 0));
        postMessage("tla-fulfilled");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""tla-fulfilled""#);
}

#[tokio::test]
async fn worker_module_top_level_await_rejection_reports_parent_error() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        await Promise.reject(new Error("worker tla rejected"));
        postMessage("unexpected");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(message.contains("worker tla rejected"), "{message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_dynamic_import_data_url_resolves_namespace() {
    ensure_v8();
    let dep_url = worker_data_url(
        r#"
        export const answer = 42;
        export default "dynamic-default";
        "#,
    );
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        const dep = await import({dep_specifier});
        postMessage({{
            answer: dep.answer,
            defaultValue: dep.default,
            resolved: import.meta.resolve({dep_specifier}),
        }});
        close();
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(r#"{{"answer":42,"defaultValue":"dynamic-default","resolved":{dep_specifier}}}"#)
    );
}

#[tokio::test]
async fn worker_dynamic_import_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dynamic.js",
        "HTTP/1.1 200 OK",
        "text/javascript; charset=utf-8",
        "export const value = 42;".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("dynamic report-only loader");
    let script_url = format!("{base_url}/worker/main.js");
    let dep_url = format!("{base_url}/worker/dynamic.js");
    let dep_specifier = serde_json::to_string("./dynamic.js").expect("specifier should serialize");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        const events = [];
        addEventListener("securitypolicyviolation", event => {{
            events.push({{
                type: event.type,
                effectiveDirective: event.effectiveDirective,
                violatedDirective: event.violatedDirective,
                blockedURI: event.blockedURI,
                documentURI: event.documentURI,
                originalPolicy: event.originalPolicy,
                disposition: event.disposition,
                instance: event instanceof SecurityPolicyViolationEvent
            }});
        }});
        const mod = await import({dep_specifier});
        postMessage({{ events, value: mod.value }});
        close();
        "#
            ),
            script_url.clone(),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_content_security_report_only_policies(vec!["script-src 'none'".to_owned()]),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"script-src-elem","violatedDirective":"script-src-elem","blockedURI":"{dep_url}","documentURI":"{script_url}","originalPolicy":"script-src 'none'","disposition":"report","instance":true}}],"value":42}}"#
        )
    );
    server
        .await
        .expect("dynamic import report-only server should finish");
}

#[tokio::test]
async fn worker_dynamic_import_sibling_dependencies_fetch_in_parallel() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker dynamic sibling server");
    let addr = listener.local_addr().expect("worker dynamic sibling addr");
    let base_url = format!("http://{addr}");
    let server = tokio::spawn(async move {
        let (mut entry_stream, _) = listener
            .accept()
            .await
            .expect("accept worker dynamic entry request");
        let entry_request = read_http_request_head(&mut entry_stream)
            .await
            .expect("read worker dynamic entry request");
        let entry_path = entry_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker dynamic entry request path");
        assert_eq!(entry_path, "/worker/entry.js");
        let entry_body = [
            "import { a } from './a.js';",
            "import { b } from './b.js';",
            "export const value = `${a}${b}`;",
        ]
        .join("\n");
        let entry_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            entry_body.len(),
            entry_body
        );
        entry_stream
            .write_all(entry_response.as_bytes())
            .await
            .expect("write worker dynamic entry response");

        let mut first_stream = None;
        let mut first_path = String::new();
        let mut second_stream = None;
        let mut second_path = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker dynamic sibling request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker dynamic sibling request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker dynamic sibling request path")
                .to_owned();
            if first_stream.is_none() {
                first_path = path;
                first_stream = Some(stream);
            } else {
                second_path = path;
                second_stream = Some(stream);
            }
        }
        let mut paths = vec![first_path.clone(), second_path.clone()];
        paths.sort();
        assert_eq!(paths, vec!["/worker/a.js", "/worker/b.js"]);
        for (path, stream) in [
            (
                first_path,
                first_stream.expect("first dynamic sibling stream"),
            ),
            (
                second_path,
                second_stream.expect("second dynamic sibling stream"),
            ),
        ] {
            let body = match path.as_str() {
                "/worker/a.js" => "export const a = 'a';",
                "/worker/b.js" => "export const b = 'b';",
                other => panic!("unexpected worker dynamic sibling path: {other}"),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let mut stream = stream;
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker dynamic sibling response");
        }
    });

    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker dynamic sibling loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import("./entry.js").then((mod) => {
            postMessage(mod.value);
            close();
        }, (error) => {
            postMessage({ error: error && error.message ? error.message : String(error) });
            close();
        });
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Classic,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""ab""#);
    server
        .await
        .expect("worker dynamic sibling server should finish");
}

#[tokio::test]
async fn worker_dynamic_import_fetches_completed_sibling_descendants_before_slow_sibling_finishes()
{
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker dynamic descendant server");
    let addr = listener
        .local_addr()
        .expect("worker dynamic descendant addr");
    let base_url = format!("http://{addr}");
    let server = tokio::spawn(async move {
        let (mut entry_stream, _) = listener
            .accept()
            .await
            .expect("accept worker dynamic entry request");
        let entry_request = read_http_request_head(&mut entry_stream)
            .await
            .expect("read worker dynamic entry request");
        let entry_path = entry_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker dynamic entry request path");
        assert_eq!(entry_path, "/worker/entry.js");
        let entry_body = [
            "import { a } from './a.js';",
            "import { b } from './b.js';",
            "export const value = `${a}${b}`;",
        ]
        .join("\n");
        let entry_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            entry_body.len(),
            entry_body
        );
        entry_stream
            .write_all(entry_response.as_bytes())
            .await
            .expect("write worker dynamic entry response");

        let mut first_stream = None;
        let mut first_path = String::new();
        let mut second_stream = None;
        let mut second_path = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker dynamic sibling request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker dynamic sibling request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker dynamic sibling request path")
                .to_owned();
            if first_stream.is_none() {
                first_path = path;
                first_stream = Some(stream);
            } else {
                second_path = path;
                second_stream = Some(stream);
            }
        }

        let (mut a_stream, mut b_stream) = match (
            first_path.as_str(),
            first_stream.expect("first dynamic sibling stream"),
            second_path.as_str(),
            second_stream.expect("second dynamic sibling stream"),
        ) {
            ("/worker/a.js", a_stream, "/worker/b.js", b_stream)
            | ("/worker/b.js", b_stream, "/worker/a.js", a_stream) => (a_stream, b_stream),
            (first, _, second, _) => panic!("unexpected dynamic sibling paths: {first}, {second}"),
        };

        let a_body = "import { child } from './a-child.js'; export const a = `a${child}`;";
        let a_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            a_body.len(),
            a_body
        );
        a_stream
            .write_all(a_response.as_bytes())
            .await
            .expect("write worker dynamic a response");

        let (mut child_stream, _) = listener
            .accept()
            .await
            .expect("accept worker dynamic child request before b finishes");
        let child_request = read_http_request_head(&mut child_stream)
            .await
            .expect("read worker dynamic child request");
        let child_path = child_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker dynamic child request path");
        assert_eq!(
            child_path, "/worker/a-child.js",
            "completed dynamic sibling descendants should start before slow sibling completes"
        );

        let child_body = "export const child = 'child';";
        let child_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            child_body.len(),
            child_body
        );
        child_stream
            .write_all(child_response.as_bytes())
            .await
            .expect("write worker dynamic child response");

        let b_body = "export const b = 'b';";
        let b_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            b_body.len(),
            b_body
        );
        b_stream
            .write_all(b_response.as_bytes())
            .await
            .expect("write worker dynamic b response");
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker dynamic descendant loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import("./entry.js").then((mod) => {
            postMessage(mod.value);
            close();
        }, (error) => {
            postMessage({ error: error && error.message ? error.message : String(error) });
            close();
        });
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Classic,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""achildb""#);
    server
        .await
        .expect("worker dynamic descendant server should finish");
}

#[tokio::test]
async fn worker_classic_dynamic_wasm_import_fetches_namespace() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/exports.wasm",
        "HTTP/1.1 200 OK",
        "application/wasm",
        worker_wasm_exported_names_body(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("classic worker wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import("./exports.wasm").then((mod) => {
            const instance = WebAssembly.namespaceInstance(mod);
            postMessage({
                funcType: typeof mod.func,
                glob: mod.glob,
                memory: mod.mem instanceof WebAssembly.Memory,
                table: mod.tab instanceof WebAssembly.Table,
                instance: instance instanceof WebAssembly.Instance,
            });
            close();
        }, (error) => {
            postMessage({
                error: error && error.message ? error.message : String(error),
            });
            close();
        });
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Classic,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"funcType":"function","glob":0,"memory":true,"table":true,"instance":true}"#
    );
    server
        .await
        .expect("classic worker dynamic wasm server should finish");
}

#[tokio::test]
async fn worker_classic_dynamic_import_preserves_instantiate_exception() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/bad.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            r#"export { missing } from "./empty.js";"#.to_owned(),
            Duration::ZERO,
        ),
        (
            "/worker/empty.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "export const present = 1;".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("classic worker module loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import("./bad.js").then(() => {
            postMessage("unexpected");
            close();
        }, (error) => {
            postMessage({
                name: error && error.name,
                syntax: error instanceof SyntaxError,
            });
            close();
        });
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Classic,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"SyntaxError","syntax":true}"#
    );
    server
        .await
        .expect("classic worker dynamic import syntax server should finish");
}

#[tokio::test]
async fn worker_module_dynamic_import_rejection_can_be_caught() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        try {
            await import("http://[::1");
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                messageIncludesResolve: String(error && error.message).includes("Failed to resolve"),
            });
        }
        close();
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","messageIncludesResolve":true}"#
    );
}

#[tokio::test]
async fn worker_module_dynamic_import_rejects_invalid_attribute_key() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        try {
            await import("data:text/javascript,export%20default%201", { with: { foo: "bar" } });
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                type: error instanceof TypeError,
                message: String(error && error.message),
            });
        }
        close();
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","type":true,"message":"Invalid attribute key \"foo\"."}"#
    );
}

#[tokio::test]
async fn worker_module_dynamic_import_source_rejects_without_hanging() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        const target = "data:text/javascript,export%20default%201";
        try {
            await import.source(target);
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                messageIncludesSourcePhase: String(error && error.message).includes("source-phase"),
            });
        }
        close();
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"SyntaxError","messageIncludesSourcePhase":true}"#
    );
}

#[tokio::test]
async fn worker_module_static_wasm_import_executes_start_function() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/worker.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_wasm_import_pm_body(),
            Duration::ZERO,
        ),
        (
            "/worker/worker-helper.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "export function pm(value) { postMessage(value); }".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm module loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "./worker.wasm";
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "42");
    server
        .await
        .expect("worker wasm import server should finish");
}

#[tokio::test]
async fn worker_module_static_wasm_import_preserves_helper_postmessage_payload() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/worker.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_wasm_import_pm_body(),
            Duration::ZERO,
        ),
        (
            "/worker/worker-helper.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            r#"
            export function pm(value) {
                postMessage({ value, checks: pm.checks });
            }
            "#
            .to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm module loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "./worker.wasm";
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"value":42}"#);
    server
        .await
        .expect("worker wasm import helper payload server should finish");
}

#[tokio::test]
async fn worker_module_static_wasm_import_preserves_v8_compile_exception() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/invalid.wasm",
        "HTTP/1.1 200 OK",
        "application/wasm",
        worker_invalid_module_wasm_body(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker invalid wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "./invalid.wasm";
        postMessage("unexpected");
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let WorkerToParentMessage::Error { message, .. } = msg else {
        panic!("expected worker module compile error, got {msg:?}");
    };
    assert!(message.contains("CompileError"), "{message}");
    assert!(message.contains("WasmModuleObject::Compile"), "{message}");
    assert!(message.contains("expected i32, got i64"), "{message}");
    assert!(
        !message.contains("unknown wasm compile exception"),
        "{message}"
    );
    server
        .await
        .expect("worker invalid wasm server should finish");
}

#[tokio::test]
async fn worker_module_wasm_js_cycle_is_rejected_without_recursive_evaluate() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/cycle.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_wasm_import_cycle_js_body(),
            Duration::ZERO,
        ),
        (
            "/worker/cycle.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            r#"
            import * as wasm from "./cycle.wasm";
            export function f() {
                return wasm.run();
            }
            "#
            .to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm cycle loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "./cycle.wasm";
        postMessage("unexpected");
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let WorkerToParentMessage::Error { message, .. } = msg else {
        panic!("expected worker wasm cycle error, got {msg:?}");
    };
    assert!(
        message.contains(
            "cyclic worker WebAssembly module evaluation through JavaScript dependencies is not supported yet"
        ),
        "{message}"
    );
    server
        .await
        .expect("worker wasm cycle server should finish");
}

#[tokio::test]
#[ignore = "requires worker wasm module records to participate in the V8 module evaluation SCC"]
async fn worker_module_wasm_js_cycle_evaluates_js_dependency_initializers() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/cycle.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_wasm_import_cycle_js_body(),
            Duration::ZERO,
        ),
        (
            "/worker/cycle.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            r#"
            import * as wasm from "./cycle.wasm";
            globalThis.__wasmCycleDependencyInitialized = true;
            globalThis.__wasmCycleCallCount = 0;
            export function f() {
                globalThis.__wasmCycleCallCount++;
            }
            globalThis.__wasmCycleNamespaceHasRun = typeof wasm.run === "function";
            "#
            .to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm cycle loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import * as wasm from "./cycle.wasm";
        wasm.run();
        postMessage({
            dependencyInitialized: globalThis.__wasmCycleDependencyInitialized,
            callCount: globalThis.__wasmCycleCallCount,
            namespaceHasRun: globalThis.__wasmCycleNamespaceHasRun
        });
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"dependencyInitialized":true,"callCount":1,"namespaceHasRun":true}"#
    );
    server
        .await
        .expect("worker wasm cycle acceptance server should finish");
}

#[tokio::test]
async fn worker_module_root_wasm_executes_start_function() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/worker-helper.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        "export function pm(value) { postMessage(value); }".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker root wasm loader");
    let mut handle = spawn_worker_with_source_and_kind_and_network_policy(
        WorkerScriptSource::binary(WORKER_WASM_IMPORT_PM.to_vec()),
        format!("{base_url}/worker/worker.wasm"),
        loader,
        WorkerScriptKind::Module,
        WorkerNetworkPolicy::default(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "42");
    server
        .await
        .expect("worker root wasm helper server should finish");
}

#[tokio::test]
async fn worker_module_source_phase_wasm_import_reuses_module_record_without_evaluation() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/worker.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_wasm_import_pm_body(),
            Duration::ZERO,
        ),
        (
            "/worker/worker-helper.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            r#"
            export function pm(value) {
                postMessage({
                    value,
                    sameSourceObject: pm.sameSourceObject,
                    abstractModuleSourceName: pm.abstractModuleSourceName,
                    abstractModuleSourceHidden: pm.abstractModuleSourceHidden,
                    moduleConstructorExtendsAbstract: pm.moduleConstructorExtendsAbstract,
                    modulePrototypeExtendsAbstract: pm.modulePrototypeExtendsAbstract,
                    staticSourceIsAbstractModuleSource: pm.staticSourceIsAbstractModuleSource
                });
            }
            "#
            .to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm source loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import source staticSource from "./worker.wasm";
        import { pm } from "./worker-helper.js";

        const dynamicSource = await import.source("./worker.wasm");
        const AbstractModuleSource = Object.getPrototypeOf(WebAssembly.Module);
        const AbstractModuleSourceProto =
            Object.getPrototypeOf(WebAssembly.Module.prototype);
        pm.sameSourceObject = dynamicSource === staticSource;
        pm.abstractModuleSourceName = AbstractModuleSource.name;
        pm.abstractModuleSourceHidden = !("AbstractModuleSource" in globalThis);
        pm.moduleConstructorExtendsAbstract = AbstractModuleSource !== Function;
        pm.modulePrototypeExtendsAbstract =
            AbstractModuleSource.prototype === AbstractModuleSourceProto;
        pm.staticSourceIsAbstractModuleSource =
            staticSource instanceof AbstractModuleSource;
        await WebAssembly.instantiate(staticSource, {
            "./worker-helper.js": { pm },
        });
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"value":42,"sameSourceObject":true,"abstractModuleSourceName":"AbstractModuleSource","abstractModuleSourceHidden":true,"moduleConstructorExtendsAbstract":true,"modulePrototypeExtendsAbstract":true,"staticSourceIsAbstractModuleSource":true}"#
    );
    server
        .await
        .expect("worker wasm source server should finish");
}

#[tokio::test]
async fn worker_module_wasm_namespace_instance_returns_cached_instance() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/exports.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_wasm_exported_names_body(),
            Duration::ZERO,
        ),
        (
            "/worker/js-module.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "export const answer = 42;".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm namespace loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import * as staticNamespace from "./exports.wasm";

        try {
            const staticInstance = WebAssembly.namespaceInstance(staticNamespace);
            const dynamicNamespace = await import("./exports.wasm");
            const dynamicInstance =
                WebAssembly.namespaceInstance(dynamicNamespace);
            const dynamicNamespace2 = await import("./exports.wasm");
            const dynamicInstance2 =
                WebAssembly.namespaceInstance(dynamicNamespace2);
            const jsNamespace = await import("./js-module.js");

            let plainObjectRejected = false;
            let jsNamespaceRejected = false;
            try {
                WebAssembly.namespaceInstance({});
            } catch (error) {
                plainObjectRejected = error instanceof TypeError;
            }
            try {
                WebAssembly.namespaceInstance(jsNamespace);
            } catch (error) {
                jsNamespaceRejected = error instanceof TypeError;
            }

            postMessage({
                staticInstance: staticInstance instanceof WebAssembly.Instance,
                shared:
                    staticInstance === dynamicInstance &&
                    dynamicInstance === dynamicInstance2,
                funcType: typeof dynamicInstance.exports.func,
                plainObjectRejected,
                jsNamespaceRejected,
            });
        } catch (error) {
            postMessage({
                error: error && error.message ? error.message : String(error),
            });
        }
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"staticInstance":true,"shared":true,"funcType":"function","plainObjectRejected":true,"jsNamespaceRejected":true}"#
    );
    server
        .await
        .expect("worker wasm namespace server should finish");
}

#[tokio::test]
async fn worker_module_static_and_dynamic_wasm_import_share_namespace() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/exports.wasm",
        "HTTP/1.1 200 OK",
        "application/wasm",
        worker_wasm_exported_names_body(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker wasm namespace loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import * as staticNamespace from "./exports.wasm";

        const dynamicNamespace = await import("./exports.wasm");
        postMessage({
            sameNamespace: dynamicNamespace === staticNamespace,
            sameInstance:
                WebAssembly.namespaceInstance(dynamicNamespace) ===
                WebAssembly.namespaceInstance(staticNamespace),
        });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"sameNamespace":true,"sameInstance":true}"#
    );
    server
        .await
        .expect("worker wasm namespace identity server should finish");
}

#[tokio::test]
async fn worker_module_mutable_wasm_global_initial_value_is_unwrapped() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/mutable-global.wasm",
        "HTTP/1.1 200 OK",
        "application/wasm",
        worker_mutable_global_wasm_body(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker mutable wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import * as mod from "./mutable-global.wasm";
        import { glob as namedGlob } from "./mutable-global.wasm";

        postMessage({
            type: typeof mod.glob,
            isGlobal: mod.glob instanceof WebAssembly.Global,
            initial: mod.glob,
            namedType: typeof namedGlob,
            namedIsGlobal: namedGlob instanceof WebAssembly.Global,
            namedInitial: namedGlob,
        });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"type":"number","isGlobal":false,"initial":0,"namedType":"number","namedIsGlobal":false,"namedInitial":0}"#
    );
    server
        .await
        .expect("worker mutable wasm server should finish");
}

#[tokio::test]
async fn worker_module_wasm_global_unwrap_uses_original_value_getter() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/mutable-global.wasm",
        "HTTP/1.1 200 OK",
        "application/wasm",
        worker_mutable_global_wasm_body(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker mutable wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        const descriptor =
            Object.getOwnPropertyDescriptor(WebAssembly.Global.prototype, "value");
        Object.defineProperty(WebAssembly.Global.prototype, "value", {
            configurable: true,
            get() {
                throw new Error("patched WebAssembly.Global getter was used");
            },
            set: descriptor.set,
        });

        try {
            const mod = await import("./mutable-global.wasm");
            postMessage({
                type: typeof mod.glob,
                isGlobal: mod.glob instanceof WebAssembly.Global,
                value: mod.glob,
            });
        } catch (error) {
            postMessage({
                error: error && error.message ? error.message : String(error),
            });
        }
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"type":"number","isGlobal":false,"value":0}"#
    );
    server
        .await
        .expect("worker mutable wasm getter server should finish");
}

#[tokio::test]
async fn worker_module_static_wasm_global_unwrap_uses_original_value_getter() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/mutable-global.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_mutable_global_wasm_body(),
            Duration::ZERO,
        ),
        (
            "/worker/static-check.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            r#"
            import * as mod from "./mutable-global.wasm";
            import { glob as namedGlob } from "./mutable-global.wasm";

            export const result = {
                type: typeof mod.glob,
                isGlobal: mod.glob instanceof WebAssembly.Global,
                value: mod.glob,
                namedType: typeof namedGlob,
                namedIsGlobal: namedGlob instanceof WebAssembly.Global,
                namedValue: namedGlob,
            };
            "#
            .to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker mutable wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        const descriptor =
            Object.getOwnPropertyDescriptor(WebAssembly.Global.prototype, "value");
        Object.defineProperty(WebAssembly.Global.prototype, "value", {
            configurable: true,
            get() {
                throw new Error("patched WebAssembly.Global getter was used");
            },
            set: descriptor.set,
        });

        try {
            const { result } = await import("./static-check.js");
            postMessage(result);
        } catch (error) {
            postMessage({
                error: error && error.message ? error.message : String(error),
            });
        }
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"type":"number","isGlobal":false,"value":0,"namedType":"number","namedIsGlobal":false,"namedValue":0}"#
    );
    server
        .await
        .expect("worker static wasm getter server should finish");
}

#[tokio::test]
#[ignore = "requires V8 wasm-aware module binding loads for mutable global exports"]
async fn worker_module_mutable_wasm_global_export_is_live_binding() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/mutable-global-live.wasm",
        "HTTP/1.1 200 OK",
        "application/wasm",
        worker_mutable_global_live_wasm_body(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker mutable wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import * as mod from "./mutable-global-live.wasm";
        import {
            getGlobal,
            mutableValue,
            setGlobal,
        } from "./mutable-global-live.wasm";

        const initialNamespace = mod.mutableValue;
        const initialNamed = mutableValue;
        const initialGetter = getGlobal();
        setGlobal(555);
        postMessage({
            initialNamespace,
            initialNamed,
            initialGetter,
            getterAfterSet: getGlobal(),
            namespaceAfterSet: mod.mutableValue,
            namedAfterSet: mutableValue,
            type: typeof mod.mutableValue,
            isGlobal: mod.mutableValue instanceof WebAssembly.Global,
        });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"initialNamespace":42,"initialNamed":42,"initialGetter":42,"getterAfterSet":555,"namespaceAfterSet":555,"namedAfterSet":555,"type":"number","isGlobal":false}"#
    );
    server
        .await
        .expect("worker mutable wasm live binding server should finish");
}

#[tokio::test]
#[ignore = "requires V8 wasm-aware module binding loads for dependency mutable global re-exports"]
async fn worker_module_mutable_wasm_global_dep_reexport_is_live_binding() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/mutable-global-export.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_mutable_global_live_wasm_body(),
            Duration::ZERO,
        ),
        (
            "/worker/mutable-global-reexport.wasm",
            "HTTP/1.1 200 OK",
            "application/wasm",
            worker_mutable_global_reexport_wasm_body(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker mutable wasm loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import * as mod from "./mutable-global-reexport.wasm";
        import {
            getImportedGlobal,
            reexportedMutableValue,
            setImportedGlobal,
        } from "./mutable-global-reexport.wasm";

        const initialNamespace = mod.reexportedMutableValue;
        const initialNamed = reexportedMutableValue;
        const initialGetter = getImportedGlobal();
        setImportedGlobal(777);
        postMessage({
            initialNamespace,
            initialNamed,
            initialGetter,
            getterAfterSet: getImportedGlobal(),
            namespaceAfterSet: mod.reexportedMutableValue,
            namedAfterSet: reexportedMutableValue,
            type: typeof mod.reexportedMutableValue,
            isGlobal: mod.reexportedMutableValue instanceof WebAssembly.Global,
        });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"initialNamespace":42,"initialNamed":42,"initialGetter":42,"getterAfterSet":777,"namespaceAfterSet":777,"namedAfterSet":777,"type":"number","isGlobal":false}"#
    );
    server
        .await
        .expect("worker mutable wasm re-export live binding server should finish");
}

#[tokio::test]
async fn worker_module_dynamic_import_fetches_http_dependency_against_module_url() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dep.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        r#"
        export const answer = 42;
        export const metaUrl = import.meta.url;
        "#
        .to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker module dependency loader");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        const dep = await import("./dep.js");
        postMessage({
            answer: dep.answer,
            metaPath: new URL(dep.metaUrl).pathname,
        });
        close();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"answer":42,"metaPath":"/worker/dep.js"}"#
    );
    server
        .await
        .expect("dynamic import dependency server should finish");
}

#[tokio::test]
async fn worker_module_dynamic_import_fetches_json_with_import_attributes() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/data.json",
        "HTTP/1.1 200 OK",
        "application/json; charset=utf-8",
        r#"{"answer":42,"label":"json-dynamic"}"#.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        const first = await import("./data.json", { with: { type: "json" } });
        const second = await import("./data.json", { with: { type: "json" } });
        postMessage({
            answer: first.default.answer,
            label: first.default.label,
            sameNamespace: first === second,
            sameDefault: first.default === second.default,
        });
        close();
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"answer":42,"label":"json-dynamic","sameNamespace":true,"sameDefault":true}"#
    );
    server
        .await
        .expect("dynamic JSON import server should finish");
}

#[tokio::test]
async fn worker_module_json_import_uses_json_fetch_destination() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker JSON module destination server");
    let addr = listener
        .local_addr()
        .expect("worker JSON module destination addr");
    let base_url = format!("http://{addr}");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker JSON module request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker JSON module request");
        let body = r#"{"answer":42}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker JSON module response");
        let _ = request_tx.send(request);
    });
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import data from "./data.json" with { type: "json" };
        postMessage(data.answer);
        close();
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "42");
    let request = request_rx
        .await
        .expect("worker JSON module request should be captured");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: json\r\n"),
        "worker JSON module fetch must use json destination, request was:\n{request}"
    );
    server
        .await
        .expect("worker JSON module destination server should finish");
}

#[tokio::test]
async fn worker_module_dynamic_css_import_rejects_invalid_module_type() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        try {
            await import("data:text/css,.answer{}", { with: { type: "css" } });
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                type: error instanceof TypeError,
                message: String(error && error.message),
            });
        }
        close();
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","type":true,"message":"module type `css` is not a valid module type for dynamic import `data:text/css,.answer{}`"}"#
    );
}

#[tokio::test]
async fn worker_module_dynamic_text_import_rejects_invalid_module_type() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        try {
            await import("data:text/plain,hello", { with: { type: "text" } });
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                type: error instanceof TypeError,
                message: String(error && error.message),
            });
        }
        close();
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","type":true,"message":"module type `text` is not a valid module type for dynamic import `data:text/plain,hello`"}"#
    );
}

#[tokio::test]
async fn worker_module_static_text_import_rejects_invalid_module_type() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import text from "data:text/plain,hello" with { type: "text" };
        postMessage("unexpected");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("module type `text` is not a valid module type"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_static_css_import_rejects_invalid_module_type() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import sheet from "data:text/css,.answer{}" with { type: "css" };
        postMessage("unexpected");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("module type `css` is not a valid module type"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_namespace_import_source_runs_in_strict_mode() {
    ensure_v8();
    let dep_url = worker_data_url("export const answer = 42;");
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import * as dep from {dep_specifier};
        const topLevelThisUndefined = this === undefined;
        function sloppyThis() {{ return this; }}
        let implicitGlobalError = "none";
        try {{
            namespaceImportGlobal = 7;
        }} catch (error) {{
            implicitGlobalError = error && error.name;
        }}
        postMessage({{
            answer: dep.answer,
            topLevelThisUndefined,
            functionThisUndefined: sloppyThis() === undefined,
            implicitGlobalError,
            leakedGlobal: Object.prototype.hasOwnProperty.call(self, "namespaceImportGlobal"),
        }});
        close();
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"answer":42,"topLevelThisUndefined":true,"functionThisUndefined":true,"implicitGlobalError":"ReferenceError","leakedGlobal":false}"#
    );
}

#[tokio::test]
async fn worker_module_named_import_source_runs_in_strict_mode() {
    ensure_v8();
    let dep_url = worker_data_url(
        r#"
        export let counter = 1;
        export { counter as default };
        export function setCounter(value) { counter = value; }
        "#,
    );
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import current, {{ counter, setCounter }} from {dep_specifier};
        const before = counter;
        const defaultBefore = current;
        const topLevelThisUndefined = this === undefined;
        function sloppyThis() {{ return this; }}
        let implicitGlobalError = "none";
        try {{
            namedImportGlobal = 7;
        }} catch (error) {{
            implicitGlobalError = error && error.name;
        }}
        let namedAssignment = "none";
        try {{
            counter = 11;
        }} catch (error) {{
            namedAssignment = error && error.name;
        }}
        let defaultAssignment = "none";
        try {{
            current = 12;
        }} catch (error) {{
            defaultAssignment = error && error.name;
        }}
        setCounter(5);
        const afterSet = counter;
        const defaultAfterSet = current;
        postMessage({{
            before,
            defaultBefore,
            afterSet,
            defaultAfterSet,
            topLevelThisUndefined,
            functionThisUndefined: sloppyThis() === undefined,
            implicitGlobalError,
            leakedGlobal: Object.prototype.hasOwnProperty.call(self, "namedImportGlobal"),
            namedAssignment,
            defaultAssignment,
        }});
        close();
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"before":1,"defaultBefore":1,"afterSet":5,"defaultAfterSet":5,"topLevelThisUndefined":true,"functionThisUndefined":true,"implicitGlobalError":"ReferenceError","leakedGlobal":false,"namedAssignment":"TypeError","defaultAssignment":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_module_top_level_return_reports_syntax_error() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        return;
        postMessage("unexpected");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error {
            message, filename, ..
        } => {
            assert!(message.contains("return"), "{message}");
            assert_eq!(filename, "data:text/javascript,main");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_with_statement_reports_syntax_error() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        with ({ value: 1 }) {
            postMessage(value);
        }
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, phase, .. } => {
            assert!(
                message.contains("with") || message.contains("strict"),
                "{message}"
            );
            assert_eq!(
                phase,
                WorkerErrorPhase::Bootstrap,
                "dependency parse errors discovered while loading the root module graph are still bootstrap failures"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_dependency_parse_error_reports_syntax_error() {
    ensure_v8();
    let dep_url = worker_data_url(
        r#"
        with ({ value: 1 }) {
            self.__unexpected = value;
        }
        "#,
    );
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import * as dep from {dep_specifier};
        postMessage(dep.__unexpected);
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("with") || message.contains("strict"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_missing_named_import_reports_link_error() {
    ensure_v8();
    let dep_url = worker_data_url("export const present = 1;");
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import {{ absent }} from {dep_specifier};
        postMessage(absent);
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("missing export") || message.contains("SyntaxError"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_missing_reexport_reports_link_error() {
    ensure_v8();
    let dep_url = worker_data_url("export const present = 1;");
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let reexport_url = worker_data_url(&format!(
        "export {{ absent as forwarded }} from {dep_specifier};"
    ));
    let reexport_specifier =
        serde_json::to_string(&reexport_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import * as reexported from {reexport_specifier};
        postMessage(reexported.forwarded);
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("missing export") || message.contains("SyntaxError"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_http_dependency_404_reports_load_error() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/missing.js",
        "HTTP/1.1 404 Not Found",
        "application/javascript",
        "missing module dependency".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "./missing.js";
        postMessage("unexpected");
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(message.contains("404"), "{message}");
            assert!(message.contains("Not Found"), "{message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
    server
        .await
        .expect("worker module 404 server should finish");
}

#[tokio::test]
async fn worker_module_http_dependency_rejects_non_script_mime() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dep.js",
        "HTTP/1.1 200 OK",
        "text/html",
        "export const value = 'html-mime';".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import { value } from "./dep.js";
        postMessage(value);
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("unsupported module script MIME type"),
                "{message}"
            );
            assert!(message.contains("text/html"), "{message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
    server
        .await
        .expect("worker module MIME server should finish");
}

#[tokio::test]
async fn worker_module_http_dependency_accepts_json_suffix_mime_with_attribute() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dep.json",
        "HTTP/1.1 200 OK",
        "application/manifest+json; charset=utf-8",
        r#"{"answer":42}"#.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import data from "./dep.json" with { type: "json" };
        postMessage(data.answer);
        close();
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "42");
    server
        .await
        .expect("worker module JSON server should finish");
}

#[tokio::test]
async fn worker_module_http_dependency_reports_json_attribute_mismatch_before_script_mime() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/plain-json.json",
        "HTTP/1.1 200 OK",
        "text/plain",
        r#"{"answer":42}"#.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import data from "./plain-json.json" with { type: "json" };
        postMessage(data.answer);
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(message.contains("non-JSON module"), "{message}");
            assert!(message.contains("JSON import attribute"), "{message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
    server
        .await
        .expect("worker module JSON MIME mismatch server should finish");
}

#[tokio::test]
async fn worker_module_invalid_static_import_specifier_reports_resolution_error() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "http://[::1";
        postMessage("unexpected");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(message.contains("Failed to resolve"), "{message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_static_import_rejects_invalid_attribute_key() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import "data:text/javascript,export%20default%201" with { foo: "bar" };
        postMessage("unexpected");
        "#
        .into(),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("Invalid attribute key \"foo\"."),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_module_http_dependency_cycle_evaluates_once() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/entry.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            ["import './leaf.js';", "export const entry = 'entry';"].join("\n"),
            Duration::ZERO,
        ),
        (
            "/worker/leaf.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            ["import './entry.js';", "export const leaf = 'leaf';"].join("\n"),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        import { entry } from "./entry.js";
        import { leaf } from "./leaf.js";
        postMessage({ entry, leaf });
        "#
        .into(),
        script_url,
        loader,
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"entry":"entry","leaf":"leaf"}"#);
    server
        .await
        .expect("worker module cycle server should finish");
}

#[tokio::test]
async fn worker_module_http_export_cycles_evaluate_once() {
    ensure_v8();
    let cases = [
        (
            "export-list",
            r#"
            import { entry, leaf } from "./entry.js";
            postMessage({ entry, leaf });
            "#,
            [
                "export { leaf } from './leaf.js';",
                "export const entry = 'entry';",
            ]
            .join("\n"),
            [
                "export { entry } from './entry.js';",
                "export const leaf = 'leaf';",
            ]
            .join("\n"),
        ),
        (
            "export-star",
            r#"
            import { entry, leaf } from "./entry.js";
            postMessage({ entry, leaf });
            "#,
            [
                "export * from './leaf.js';",
                "export const entry = 'entry';",
            ]
            .join("\n"),
            ["export * from './entry.js';", "export const leaf = 'leaf';"].join("\n"),
        ),
    ];

    for (label, main_source, entry_source, leaf_source) in cases {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/worker/entry.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                entry_source,
                Duration::ZERO,
            ),
            (
                "/worker/leaf.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                leaf_source,
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            ResourceRequestClient::new(&FetchConfig::default()).expect("worker module loader");
        let script_url = format!("{base_url}/worker/main.js");
        let mut handle = spawn_worker_with_request_client_and_kind(
            main_source.into(),
            script_url,
            loader,
            WorkerScriptKind::Module,
        );

        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {label} cycle"))
            .expect("channel closed");
        assert_eq!(
            expect_post_json(msg),
            r#"{"entry":"entry","leaf":"leaf"}"#,
            "{label}"
        );
        server
            .await
            .unwrap_or_else(|_| panic!("{label} cycle server should finish"));
    }
}

#[tokio::test]
async fn worker_module_side_effect_imports_evaluate_dependencies_once_in_order() {
    ensure_v8();
    let shared_url = worker_data_url(
        r#"
        self.__moduleOrder = self.__moduleOrder || [];
        self.__moduleOrder.push("shared");
        export const marker = "shared-marker";
        "#,
    );
    let shared_specifier = serde_json::to_string(&shared_url).expect("data URL should serialize");
    let first_url = worker_data_url(&format!(
        r#"
        import {shared_specifier};
        self.__moduleOrder.push("first");
        export const first = "first-export";
        "#
    ));
    let first_specifier = serde_json::to_string(&first_url).expect("data URL should serialize");
    let second_url = worker_data_url(&format!(
        r#"
        import {{ marker }} from {shared_specifier};
        self.__moduleOrder.push("second:" + marker);
        export const second = marker;
        "#
    ));
    let second_specifier = serde_json::to_string(&second_url).expect("data URL should serialize");

    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import {first_specifier};
        import {{ second }} from {second_specifier};
        import {shared_specifier};
        postMessage({{
            order: self.__moduleOrder.join("|"),
            second,
        }});
        close();
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"order":"shared|first|second:shared-marker","second":"shared-marker"}"#
    );
}

#[tokio::test]
async fn worker_module_imported_bindings_reject_assignment() {
    ensure_v8();
    let dep_url = worker_data_url(
        r#"
        export let counter = 1;
        export { counter as default };
        export function setCounter(value) { counter = value; }
        "#,
    );
    let dep_specifier = serde_json::to_string(&dep_url).expect("data URL should serialize");
    let mut handle = spawn_worker_with_request_client_and_kind(
        format!(
            r#"
        import current, {{ counter, setCounter }} from {dep_specifier};
        function assignmentName(callback) {{
            try {{
                callback();
                return "none";
            }} catch (error) {{
                return error && error.name;
            }}
        }}
        const namedAssignment = assignmentName(function () {{ counter = 7; }});
        const defaultAssignment = assignmentName(function () {{ current = 8; }});
        setCounter(5);
        postMessage({{
            namedAssignment,
            defaultAssignment,
            counter,
            current,
        }});
        close();
        "#
        ),
        "data:text/javascript,main".into(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"namedAssignment":"TypeError","defaultAssignment":"TypeError","counter":5,"current":5}"#
    );
}

#[tokio::test]
async fn worker_importscripts_cross_origin_failures_throw_network_error() {
    ensure_v8();
    let (base_url, script_server) = spawn_path_response_http_server(vec![
        (
            "/syntax.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "globalThis.__broken = ;".to_owned(),
            Duration::ZERO,
        ),
        (
            "/throw.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "globalThis.__crossOriginLoaded = true;".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let syntax_url =
        serde_json::to_string(&format!("{base_url}/syntax.js")).expect("serialize syntax url");
    let throw_url =
        serde_json::to_string(&format!("{base_url}/throw.js")).expect("serialize throw url");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
            const results = [];
            for (const url of [{syntax_url}, {throw_url}]) {{
                try {{
                    importScripts(url);
                    results.push({{
                        name: "unexpected",
                        domException: false,
                        loaded: globalThis.__crossOriginLoaded === true,
                    }});
                }} catch (error) {{
                    results.push({{
                        name: error && error.name,
                        domException: error instanceof DOMException,
                        loaded: globalThis.__crossOriginLoaded === true,
                    }});
                }}
                delete globalThis.__crossOriginLoaded;
            }}
            postMessage(results);
            close();
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"[{"name":"NetworkError","domException":true,"loaded":false},{"name":"NetworkError","domException":true,"loaded":false}]"#
    );
    script_server
        .await
        .expect("cross-origin importScripts server should finish");
}

#[tokio::test]
async fn worker_importscripts_redirect_to_cross_origin_failure_throws_network_error() {
    ensure_v8();
    let (cross_origin_base_url, script_server) = spawn_path_response_http_server(vec![(
        "/throw.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        "globalThis.__redirectedCrossOriginLoaded = true;".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let redirect_response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {cross_origin_base_url}/throw.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (same_origin_base_url, redirect_server) = spawn_raw_path_response_http_server(vec![(
        "/worker/redirect-throw.js",
        redirect_response,
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        try {
            importScripts("./redirect-throw.js");
            postMessage({
                name: "unexpected",
                domException: false,
                loaded: globalThis.__redirectedCrossOriginLoaded === true,
            });
        } catch (error) {
            postMessage({
                name: error && error.name,
                domException: error instanceof DOMException,
                loaded: globalThis.__redirectedCrossOriginLoaded === true,
            });
        }
        close();
        "#
        .into(),
        format!("{same_origin_base_url}/worker/main.js"),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"NetworkError","domException":true,"loaded":false}"#
    );
    redirect_server
        .await
        .expect("redirect importScripts server should finish");
    script_server
        .await
        .expect("redirect target importScripts server should finish");
}

#[tokio::test]
async fn worker_importscripts_same_origin_syntax_error_reports_imported_script_location() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/syntax-error.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        "globalThis.__broken = ;".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        addEventListener("error", function(event) {
            postMessage({
                name: event.error && event.error.name,
                messageIncludesSyntaxError: String(event.message).includes("SyntaxError"),
                filename: event.filename,
                lineno: event.lineno
            });
            event.preventDefault();
            close();
        });
        function doImportScripts(url) {
            importScripts(url);
        }
        doImportScripts("./syntax-error.js");
        "#
        .into(),
        format!("{base_url}/worker/report-error-helper.js"),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"name":"SyntaxError","messageIncludesSyntaxError":true,"filename":"{base_url}/worker/syntax-error.js","lineno":1}}"#
        )
    );
    server
        .await
        .expect("same-origin importScripts server should finish");
}

#[tokio::test]
async fn service_worker_importscripts_reports_imported_script_resource() {
    ensure_v8();
    let dep_body = "globalThis.__depLoaded = true;";
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dep.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        dep_body.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            importScripts("./dep.js");
            if (!globalThis.__depLoaded) {
                throw new Error("missing dep");
            }
            skipWaiting();
            "#
            .to_owned(),
            format!("{base_url}/worker/sw.js"),
        )
        .with_request_client(loader)
        .with_global_kind(super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(7),
            version_id: ServiceWorkerVersionId::from_u64_for_test(9),
            scope_url: url::Url::parse(&format!("{base_url}/worker/")).unwrap(),
        }),
    );

    let expected_import_url = format!("{base_url}/worker/dep.js");
    let expected_hash = sha256_hex(dep_body.as_bytes());
    let mut imported_resource = None;
    let mut saw_skip_waiting = false;
    while imported_resource.is_none() || !saw_skip_waiting {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for importScripts resource")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
                registration_id,
                version_id,
                resource,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(7)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(9));
                imported_resource = Some(resource);
            }
            WorkerToParentMessage::ServiceWorkerSkipWaiting {
                registration_id,
                version_id,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(7)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(9));
                saw_skip_waiting = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    }

    let resource = imported_resource.expect("importScripts should report a resource");
    assert_eq!(resource.request_url.as_str(), expected_import_url);
    assert_eq!(resource.final_url.as_str(), expected_import_url);
    assert_eq!(resource.kind, WorkerScriptResourceKind::JavaScript);
    assert_eq!(resource.status, 200);
    assert_eq!(resource.body_len, dep_body.len());
    assert_eq!(resource.body_sha256, expected_hash);
    assert_eq!(
        resource.mime_type.as_deref(),
        Some("application/javascript")
    );
    handle.terminate_and_join();
    server
        .await
        .expect("service worker importScripts resource server should finish");
}

#[tokio::test]
async fn service_worker_module_static_import_reports_imported_script_resource() {
    ensure_v8();
    let dep_body = "export const depLoaded = true;";
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dep.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        dep_body.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("service worker module loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            import { depLoaded } from "./dep.js";
            if (!depLoaded) {
                throw new Error("missing module dep");
            }
            skipWaiting();
            "#
            .to_owned(),
            format!("{base_url}/worker/sw.js"),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_global_kind(super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(7),
            version_id: ServiceWorkerVersionId::from_u64_for_test(9),
            scope_url: url::Url::parse(&format!("{base_url}/worker/")).unwrap(),
        }),
    );

    let expected_import_url = format!("{base_url}/worker/dep.js");
    let expected_hash = sha256_hex(dep_body.as_bytes());
    let mut imported_resource = None;
    let mut saw_skip_waiting = false;
    while imported_resource.is_none() || !saw_skip_waiting {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker module resource")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
                registration_id,
                version_id,
                resource,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(7)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(9));
                imported_resource = Some(resource);
            }
            WorkerToParentMessage::ServiceWorkerSkipWaiting {
                registration_id,
                version_id,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(7)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(9));
                saw_skip_waiting = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker module error: {message}");
            }
            _ => {}
        }
    }

    let resource = imported_resource.expect("module static import should report a resource");
    assert_eq!(resource.request_url.as_str(), expected_import_url);
    assert_eq!(resource.final_url.as_str(), expected_import_url);
    assert_eq!(resource.status, 200);
    assert_eq!(resource.body_len, dep_body.len());
    assert_eq!(resource.body_sha256, expected_hash);
    assert_eq!(
        resource.mime_type.as_deref(),
        Some("application/javascript")
    );
    handle.terminate_and_join();
    server
        .await
        .expect("service worker module resource server should finish");
}

#[tokio::test]
async fn service_worker_module_static_json_import_reports_json_resource_kind() {
    ensure_v8();
    let dep_body = r#"{"answer":42}"#;
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/dep.json",
        "HTTP/1.1 200 OK",
        "application/manifest+json; charset=utf-8",
        dep_body.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("service worker module loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            import data from "./dep.json" with { type: "json" };
            if (data.answer !== 42) {
                throw new Error("missing JSON module dep");
            }
            skipWaiting();
            "#
            .to_owned(),
            format!("{base_url}/worker/sw.js"),
        )
        .with_request_client(loader)
        .with_script_kind(WorkerScriptKind::Module)
        .with_global_kind(super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(11),
            version_id: ServiceWorkerVersionId::from_u64_for_test(13),
            scope_url: url::Url::parse(&format!("{base_url}/worker/")).unwrap(),
        }),
    );

    let expected_import_url = format!("{base_url}/worker/dep.json");
    let expected_hash = sha256_hex(dep_body.as_bytes());
    let mut imported_resource = None;
    let mut saw_skip_waiting = false;
    while imported_resource.is_none() || !saw_skip_waiting {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker module JSON resource")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
                registration_id,
                version_id,
                resource,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(11)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(13));
                imported_resource = Some(resource);
            }
            WorkerToParentMessage::ServiceWorkerSkipWaiting {
                registration_id,
                version_id,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(11)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(13));
                saw_skip_waiting = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker module JSON error: {message}");
            }
            _ => {}
        }
    }

    let resource = imported_resource.expect("module static JSON import should report a resource");
    assert_eq!(resource.request_url.as_str(), expected_import_url);
    assert_eq!(resource.final_url.as_str(), expected_import_url);
    assert_eq!(resource.kind, WorkerScriptResourceKind::JsonModule);
    assert_eq!(resource.status, 200);
    assert_eq!(resource.body_len, dep_body.len());
    assert_eq!(resource.body_sha256, expected_hash);
    assert_eq!(
        resource.mime_type.as_deref(),
        Some("application/manifest+json; charset=utf-8")
    );
    handle.terminate_and_join();
    server
        .await
        .expect("service worker module JSON resource server should finish");
}

#[tokio::test]
async fn service_worker_module_static_css_import_rejects_invalid_module_type() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            import sheet from "data:text/css,.answer{}" with { type: "css" };
            skipWaiting();
            "#
            .to_owned(),
            "data:text/javascript,main".to_owned(),
        )
        .with_script_kind(WorkerScriptKind::Module)
        .with_global_kind(super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(15),
            version_id: ServiceWorkerVersionId::from_u64_for_test(17),
            scope_url: url::Url::parse("https://service-worker-module.invalid/scope/").unwrap(),
        }),
    );

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for service worker module CSS error")
        .expect("channel closed");
    match message {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("module type `css` is not a valid module type"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_module_static_text_import_rejects_invalid_module_type() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            import text from "data:text/plain,hello" with { type: "text" };
            skipWaiting();
            "#
            .to_owned(),
            "data:text/javascript,main".to_owned(),
        )
        .with_script_kind(WorkerScriptKind::Module)
        .with_global_kind(super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(19),
            version_id: ServiceWorkerVersionId::from_u64_for_test(21),
            scope_url: url::Url::parse("https://service-worker-module.invalid/scope/").unwrap(),
        }),
    );

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for service worker module text error")
        .expect("channel closed");
    match message {
        WorkerToParentMessage::Error { message, .. } => {
            assert!(
                message.contains("module type `text` is not a valid module type"),
                "{message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_importscripts_nested_rethrow_preserves_imported_script_location() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/worker/entry.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "runTest('./syntax-error.js');".to_owned(),
            Duration::ZERO,
        ),
        (
            "/worker/syntax-error.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            "globalThis.__broken = ;".to_owned(),
            Duration::ZERO,
        ),
    ])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        addEventListener("error", function(event) {
            postMessage({
                name: event.error && event.error.name,
                messageIncludesSyntaxError: String(event.message).includes("SyntaxError"),
                filename: event.filename,
                lineno: event.lineno
            });
            event.preventDefault();
            close();
        });
        function doImportScripts(url) {
            importScripts(url);
        }
        function runTest(url) {
            doImportScripts(url);
        }
        importScripts("./entry.js");
        "#
        .into(),
        format!("{base_url}/worker/report-error-helper.js"),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"name":"SyntaxError","messageIncludesSyntaxError":true,"filename":"{base_url}/worker/syntax-error.js","lineno":1}}"#
        )
    );
    server
        .await
        .expect("nested importScripts server should finish");
}

#[tokio::test]
async fn worker_importscripts_same_origin_runtime_error_reports_imported_script_location() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/runtime-error.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        "throw new Error('runtime-boom');".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        addEventListener("error", function(event) {
            postMessage({
                name: event.error && event.error.name,
                errorMessage: event.error && event.error.message,
                filename: event.filename,
                lineno: event.lineno
            });
            event.preventDefault();
            close();
        });
        function doImportScripts(url) {
            importScripts(url);
        }
        doImportScripts("./runtime-error.js");
        "#
        .into(),
        format!("{base_url}/worker/report-error-helper.js"),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"name":"Error","errorMessage":"runtime-boom","filename":"{base_url}/worker/runtime-error.js","lineno":1}}"#
        )
    );
    server
        .await
        .expect("same-origin runtime importScripts server should finish");
}

#[tokio::test]
async fn worker_importscripts_cross_origin_failure_reports_helper_callsite_from_timer() {
    ensure_v8();
    let (cross_origin_base_url, server) = spawn_path_response_http_server(vec![(
        "/syntax-error.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        "globalThis.__broken = ;".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let cross_origin_url =
        serde_json::to_string(&format!("{cross_origin_base_url}/syntax-error.js"))
            .expect("serialize cross-origin url");
    let script_url = "http://127.0.0.1/worker/report-error-helper.js";
    let mut handle = spawn_worker_with_request_client(
        format!(
            "addEventListener(\"error\", function(event) {{\n  postMessage({{name: event.error && event.error.name, domException: event.error instanceof DOMException, messageIsScriptError: event.message === \"Script error.\", filename: event.filename, lineno: event.lineno}});\n  event.preventDefault();\n  close();\n}});\nfunction doImportScripts(url) {{\n  importScripts(url);\n}}\nsetTimeout(function() {{\n  doImportScripts({cross_origin_url});\n}}, 0);\n"
        ),
        script_url.into(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"NetworkError","domException":true,"messageIsScriptError":false,"filename":"http://127.0.0.1/worker/report-error-helper.js","lineno":7}"#
    );
    server
        .await
        .expect("cross-origin importScripts server should finish");
}
