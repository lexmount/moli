use super::*;
use tokio::net::TcpStream;

struct SpecifierServer {
    url: String,
    requests: tokio::sync::mpsc::UnboundedReceiver<(String, TcpStream)>,
    task: JoinHandle<()>,
}

impl SpecifierServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let head = read_http_request_head(&mut stream).await.unwrap();
                let Some(path) = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                else {
                    continue;
                };
                if sender.send((path.to_owned(), stream)).is_err() {
                    break;
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    fn worker(&self, source: String, kind: WorkerScriptKind) -> WorkerTestHandle {
        spawn_worker_with_request_client_and_kind(
            source,
            format!("{}/worker/main.js", self.url),
            worker_test_request_client(),
            kind,
        )
    }

    async fn respond(&mut self, path: &str, status: &str, mime: &str, body: &str) {
        let (actual, mut stream) = timeout(TIMEOUT, self.requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual, path);
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
    }

    fn assert_no_more_requests(&mut self) {
        assert!(self.requests.try_recv().is_err(), "unexpected module fetch");
    }
}

impl Drop for SpecifierServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const SPECIFIER_PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/worker-module-specifiers.js"
));

#[tokio::test]
async fn worker_module_specifiers_reject_bare_urls_without_fetching_or_reusing_cached_modules() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let mut server = SpecifierServer::start().await;
        let resolver = match kind {
            WorkerScriptKind::Classic => "undefined",
            WorkerScriptKind::Module => "import.meta.resolve",
        };
        let mut handle = server.worker(
            format!("{SPECIFIER_PROBE}\nworkerModuleSpecifierProbe({resolver}).then(postMessage);"),
            kind,
        );
        server
            .respond(
                "/worker/cached.js",
                "200 OK",
                "text/javascript",
                "self.moduleRuns = (self.moduleRuns || 0) + 1; export const answer = 42;",
            )
            .await;
        let result: serde_json::Value =
            serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
        assert_eq!(result["completed"], true, "{kind:?}: {result}");
        assert_eq!(
            result["failures"],
            serde_json::json!([]),
            "{kind:?}: {result}"
        );
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_static_module_specifiers_fail_before_fetching_including_service_workers() {
    ensure_v8();
    for service_worker in [false, true] {
        for specifier in ["bare.js", "", "?query", "#fragment", ".\\file.js"] {
            let specifier = serde_json::to_string(specifier).unwrap();
            for source in [
                format!("import {specifier};"),
                format!("export * from {specifier};"),
                format!("import source wasm from {specifier};"),
            ] {
                let mut server = SpecifierServer::start().await;
                let script_url = format!("{}/worker/main.js", server.url);
                let options = if service_worker {
                    service_worker_module_options(source, script_url)
                } else {
                    WorkerSpawnOptions::new(source, script_url)
                        .with_script_kind(WorkerScriptKind::Module)
                };
                let mut handle = spawn_test_worker_with_options(options);
                match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
                    WorkerToParentMessage::Error { message, phase, .. } => {
                        assert!(
                            message.contains("Failed to resolve module specifier"),
                            "{message}"
                        );
                        assert_eq!(phase, WorkerErrorPhase::Bootstrap);
                    }
                    other => panic!("expected specifier resolution failure, got {other:?}"),
                }
                handle.terminate_and_join();
                server.assert_no_more_requests();
            }
        }
    }
}

#[tokio::test]
async fn worker_dynamic_import_rejects_bare_dependencies_as_type_errors_without_fetching() {
    ensure_v8();
    let mut server = SpecifierServer::start().await;
    let mut handle = server.worker(
        r#"import('./outer.js').then(
            () => postMessage('unexpected'),
            error => postMessage(error instanceof TypeError)
        );"#
        .into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/outer.js",
            "200 OK",
            "text/javascript",
            "import 'bare.js';",
        )
        .await;
    assert_eq!(recv_post_json(&mut handle).await, "true");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}
