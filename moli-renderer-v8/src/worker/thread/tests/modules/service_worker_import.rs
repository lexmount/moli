use super::source_phase::ModuleSourceServer;
use super::*;

const IMPORT_PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/service-worker-dynamic-import.js"
));

#[tokio::test]
async fn service_worker_dynamic_import_rejects_cached_and_uncached_modules_without_fetching() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let mut server = ModuleSourceServer::start().await;
        let static_import = match kind {
            WorkerScriptKind::Classic => "importScripts('./cached.js');",
            WorkerScriptKind::Module => "import './cached.js';",
        };
        let source = format!(
            r#"{static_import}
{IMPORT_PROBE}
serviceWorkerDynamicImportProbe().then(result => console.log(JSON.stringify(result)));
"#
        );
        let mut handle = spawn_test_worker_with_options(
            service_worker_module_options(source, format!("{}/worker/sw.js", server.url))
                .with_script_kind(kind),
        );
        server
            .respond(
                "/worker/cached.js",
                "200 OK",
                "text/javascript",
                "self.staticScriptRuns = (self.staticScriptRuns || 0) + 1;",
            )
            .await;
        loop {
            match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
                WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
                | WorkerToParentMessage::SubresourceNetwork(_)
                | WorkerToParentMessage::PendingSubresourceFetch(_)
                | WorkerToParentMessage::SubresourceContinue(_) => {}
                WorkerToParentMessage::Console(message) => {
                    let result: serde_json::Value =
                        serde_json::from_str(message.message.strip_prefix("log: ").unwrap())
                            .unwrap();
                    assert_eq!(
                        result["failures"],
                        serde_json::json!([]),
                        "{kind:?}: {result}"
                    );
                    assert_eq!(result["checks"], 37, "{kind:?}: {result}");
                    break;
                }
                other => panic!("expected caught import rejection, got {other:?}"),
            }
        }
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}
