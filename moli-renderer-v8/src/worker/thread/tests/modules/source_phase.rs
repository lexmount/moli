use super::*;
use tokio::net::TcpStream;

// Keep responses under the test's control: a source import must settle even
// while the evaluation graph's dependency response is withheld.
pub(super) struct ModuleSourceServer {
    pub(super) url: String,
    requests: tokio::sync::mpsc::UnboundedReceiver<(String, TcpStream)>,
    task: JoinHandle<()>,
}

impl ModuleSourceServer {
    pub(super) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let head = read_http_request_head(&mut stream).await.unwrap();
                let path = head
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
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

    pub(super) fn worker(&self, source: String, kind: WorkerScriptKind) -> WorkerTestHandle {
        self.worker_at(source, kind, "main.js")
    }

    pub(super) fn worker_at(
        &self,
        source: String,
        kind: WorkerScriptKind,
        name: &str,
    ) -> WorkerTestHandle {
        spawn_worker_with_request_client_and_kind(
            source,
            format!("{}/worker/{name}", self.url),
            worker_test_request_client(),
            kind,
        )
    }

    pub(super) async fn request(&mut self, path: &str) -> TcpStream {
        let (actual, stream) = timeout(TIMEOUT, self.requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual, path);
        stream
    }

    pub(super) async fn respond(&mut self, path: &str, status: &str, mime: &str, body: &str) {
        Self::write(self.request(path).await, status, mime, body).await;
    }

    pub(super) async fn respond_bytes(
        &mut self,
        path: &str,
        status: &str,
        mime: &str,
        body: &[u8],
    ) {
        Self::write_bytes(self.request(path).await, status, mime, body).await;
    }

    async fn write(stream: TcpStream, status: &str, mime: &str, body: &str) {
        Self::write_bytes(stream, status, mime, body.as_bytes()).await;
    }

    async fn write_bytes(mut stream: TcpStream, status: &str, mime: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
    }

    async fn wasm(&mut self) {
        self.respond(
            "/worker/worker.wasm",
            "200 OK",
            "application/wasm",
            &worker_wasm_import_pm_body(),
        )
        .await;
    }

    pub(super) fn assert_no_more_requests(&mut self) {
        assert!(self.requests.try_recv().is_err(), "unexpected module fetch");
    }
}

impl Drop for ModuleSourceServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn worker_source_imports_do_not_load_wasm_dependencies() {
    ensure_v8();
    for (kind, static_import) in [
        (WorkerScriptKind::Module, true),
        (WorkerScriptKind::Module, false),
        (WorkerScriptKind::Classic, false),
    ] {
        let mut server = ModuleSourceServer::start().await;
        let (prefix, source) = if static_import {
            ("import source wasm from './worker.wasm';", "")
        } else {
            ("", "const wasm = await import.source('./worker.wasm');")
        };
        let mut handle = server.worker(format!(r#"
            {prefix}
            (async () => {{
                {source}
                let value;
                new WebAssembly.Instance(wasm, {{ "./worker-helper.js": {{ pm: v => value = v }} }});
                const again = await import.source('./worker.wasm');
                postMessage([wasm instanceof WebAssembly.Module, value, wasm === again]);
            }})().catch(e => postMessage({{error: e.name}}));
        "#), kind);
        server.wasm().await;
        assert_eq!(recv_post_json(&mut handle).await, "[true,42,true]");
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_source_import_allows_later_evaluation_of_cached_wasm() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(r#"
        (async () => {
            const source = await import.source('./worker.wasm');
            const first = await import('./worker.wasm');
            const second = await import('./worker.wasm');
            const again = await import.source('./worker.wasm');
            postMessage([source === again, first === second, self.evaluationValue, self.evaluations]);
        })().catch(e => postMessage({error: e.name}));
    "#.into(), WorkerScriptKind::Module);
    server.wasm().await;
    server.respond("/worker/worker-helper.js", "200 OK", "text/javascript",
        "self.evaluations = (self.evaluations || 0) + 1; export function pm(v) { self.evaluationValue = v; }").await;
    assert_eq!(recv_post_json(&mut handle).await, "[true,true,42,1]");
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_concurrent_source_imports_do_not_wait_for_evaluation_dependencies() {
    ensure_v8();
    for order in ["source-first", "evaluation-first", "late-source"] {
        let mut server = ModuleSourceServer::start().await;
        let start = match order {
            "source-first" => "const source = loadSource(); const evaluation = loadEvaluation();",
            "evaluation-first" => {
                "const evaluation = loadEvaluation(); const source = loadSource();"
            }
            _ => {
                "const evaluation = loadEvaluation(); await new Promise(r => onmessage = r); const source = loadSource();"
            }
        };
        let mut handle = server.worker(format!(r#"
            (async () => {{
                const loadSource = () => Promise.all([
                    import.source('./worker.wasm'), import.source('./worker.wasm')
                ]).then(([a, b]) => {{
                    postMessage(['source', a === b, a instanceof WebAssembly.Module]);
                    return a;
                }});
                const loadEvaluation = () => Promise.all([
                    import('./worker.wasm'), import('./worker.wasm')
                ]).then(([a, b]) => a === b);
                {start}
                const [wasm, sameNamespace] = await Promise.all([source, evaluation]);
                postMessage([sameNamespace, self.evaluationValue, wasm === await import.source('./worker.wasm')]);
            }})().catch(e => postMessage({{error: e.name}}));
        "#), WorkerScriptKind::Module);
        server.wasm().await;
        let dependency = server.request("/worker/worker-helper.js").await;
        if order == "late-source" {
            handle.post_message(serialize_test_string("load source"));
        }
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"["source",true,true]"#,
            "{order}"
        );
        ModuleSourceServer::write(
            dependency,
            "200 OK",
            "text/javascript",
            "export function pm(v) { self.evaluationValue = v; }",
        )
        .await;
        assert_eq!(
            recv_post_json(&mut handle).await,
            "[true,42,true]",
            "{order}"
        );
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_source_import_survives_concurrent_evaluation_graph_failure() {
    ensure_v8();
    for (status, body, expected_error) in [
        ("404 Not Found", "missing", "TypeError"),
        ("200 OK", "export function {", "SyntaxError"),
        (
            "200 OK",
            "export function pm() {} throw new Error('evaluation failed');",
            "Error",
        ),
    ] {
        for source_first in [true, false] {
            let mut server = ModuleSourceServer::start().await;
            let start = if source_first {
                "const source = import.source('./worker.wasm'); const evaluation = loadEvaluation();"
            } else {
                "const evaluation = loadEvaluation(); const source = import.source('./worker.wasm');"
            };
            let mut handle = server.worker(format!(r#"
                (async () => {{
                    const loadEvaluation = () => import('./worker.wasm').then(() => 'unexpected', e => e.name);
                    {start}
                    const [wasm, error] = await Promise.all([source, evaluation]);
                    let value;
                    new WebAssembly.Instance(wasm, {{ './worker-helper.js': {{ pm: v => value = v }} }});
                    postMessage([error, value, wasm === await import.source('./worker.wasm')]);
                }})().catch(e => postMessage({{error: e.name}}));
            "#), WorkerScriptKind::Module);
            server.wasm().await;
            server
                .respond("/worker/worker-helper.js", status, "text/javascript", body)
                .await;
            assert_eq!(
                recv_post_json(&mut handle).await,
                format!(r#"["{expected_error}",42,true]"#)
            );
            handle.terminate_and_join();
            server.assert_no_more_requests();
        }
    }
}

#[tokio::test]
async fn worker_source_import_can_reuse_wasm_after_failed_evaluation_loading() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        (async () => {
            const error = await import('./worker.wasm').then(() => 'unexpected', e => e.name);
            const wasm = await import.source('./worker.wasm');
            let value;
            new WebAssembly.Instance(wasm, { './worker-helper.js': { pm: v => value = v } });
            postMessage([error, value]);
        })().catch(e => postMessage({error: e.name}));
    "#
        .into(),
        WorkerScriptKind::Module,
    );
    server.wasm().await;
    server
        .respond(
            "/worker/worker-helper.js",
            "404 Not Found",
            "text/javascript",
            "missing",
        )
        .await;
    assert_eq!(recv_post_json(&mut handle).await, r#"["TypeError",42]"#);
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_imports_do_not_expand_an_unrelated_failed_graph() {
    ensure_v8();
    let mut server = ModuleSourceServer::start().await;
    let mut handle = server.worker(
        r#"
        (async () => {
            const error = await import('./broken.js').then(() => 'unexpected', e => e.name);
            const wasm = await import.source('./worker.wasm');
            const { value } = await import('./clean.js');
            postMessage([error, wasm instanceof WebAssembly.Module, value]);
        })().catch(e => postMessage({error: e.name}));
    "#
        .into(),
        WorkerScriptKind::Module,
    );
    server
        .respond(
            "/worker/broken.js",
            "200 OK",
            "text/javascript",
            "import './missing.js';",
        )
        .await;
    server
        .respond(
            "/worker/missing.js",
            "404 Not Found",
            "text/javascript",
            "missing",
        )
        .await;
    server.wasm().await;
    server
        .respond(
            "/worker/clean.js",
            "200 OK",
            "text/javascript",
            "export const value = 7;",
        )
        .await;
    assert_eq!(recv_post_json(&mut handle).await, r#"["TypeError",true,7]"#);
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_source_and_evaluation_imports_share_root_fetch_failures() {
    ensure_v8();
    for (status, body) in [("404 Not Found", "missing"), ("200 OK", "invalid wasm")] {
        for source_first in [true, false] {
            let mut server = ModuleSourceServer::start().await;
            let imports = if source_first {
                "[import.source('./worker.wasm'), import('./worker.wasm')]"
            } else {
                "[import('./worker.wasm'), import.source('./worker.wasm')]"
            };
            let mut handle = server.worker(format!(r#"
                (async () => {{
                    const results = await Promise.allSettled({imports});
                    const again = await import.source('./worker.wasm').then(() => 'unexpected', e => e.name);
                    postMessage([results.map(r => [r.status, r.reason.name]), again]);
                }})().catch(e => postMessage({{error: e.name}}));
            "#), WorkerScriptKind::Module);
            server
                .respond("/worker/worker.wasm", status, "application/wasm", body)
                .await;
            assert_eq!(
                recv_post_json(&mut handle).await,
                r#"[[["rejected","TypeError"],["rejected","TypeError"]],"TypeError"]"#
            );
            handle.terminate_and_join();
            server.assert_no_more_requests();
        }
    }
}
