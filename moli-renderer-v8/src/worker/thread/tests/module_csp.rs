use super::*;
use crate::worker::WorkerGlobalKind;
use serde_json::{Value, json};

struct ModuleServer {
    url: String,
    requests: Arc<parking_lot::Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl ModuleServer {
    async fn spawn(response: impl Fn(&str) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                observed.lock().push(request.clone());
                stream
                    .write_all(response(&request).as_bytes())
                    .await
                    .unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    async fn modules() -> Self {
        Self::spawn(|request| {
            let (mime, body) = if request.starts_with("GET /data.json") {
                ("application/json", r#"{"answer":42}"#)
            } else if request.starts_with("GET /data.txt") {
                ("text/plain", "module-text")
            } else {
                ("text/javascript", "export default 42;")
            };
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }).await
    }
}

impl Drop for ModuleServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn global_kinds() -> [WorkerGlobalKind; 2] {
    [
        WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        WorkerGlobalKind::Shared {
            name: "module-csp".into(),
            storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(
                &url::Url::parse("http://127.0.0.1/").unwrap(),
                None,
            ),
        },
    ]
}

fn probe(requests: Value) -> String {
    r#"
const events = [];
addEventListener('securitypolicyviolation', event => events.push({
  directive: event.effectiveDirective, disposition: event.disposition,
  policy: event.originalPolicy, blockedURI: event.blockedURI,
  documentURI: event.documentURI, sourceFile: event.sourceFile,
  line: event.lineNumber, column: event.columnNumber,
  native: event instanceof SecurityPolicyViolationEvent, trusted: event.isTrusted
}));
(async () => {
  const results = [];
  for (const [url, type] of REQUESTS) {
    const result = {};
    try {
      const imported = type === 'javascript' ? import(url) : import(url, {with: {type}});
      const module = await imported;
      result.value = type === 'json' ? module.default.answer : module.default;
    } catch (error) {
      result.error = error.name;
      result.native = error instanceof TypeError;
    }
    result.events = events.splice(0);
    results.push(result);
  }
  console.log(JSON.stringify(results));
})().catch(error => console.error(String(error.stack || error)));
"#
    .replace("REQUESTS", &requests.to_string())
}

async fn result(mut worker: WorkerTestHandle) -> Value {
    let text = timeout(TIMEOUT, async {
        loop {
            match worker
                .recv()
                .await
                .expect("module worker must remain alive")
            {
                WorkerToParentMessage::Console(console) => break console.message,
                WorkerToParentMessage::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("module imports must settle");
    worker.terminate_and_join();
    serde_json::from_str(text.strip_prefix("log: ").expect(&text)).unwrap()
}

#[tokio::test]
async fn worker_typed_module_csp_blocks_before_contact_and_reports_every_policy() {
    ensure_v8();
    let server = ModuleServer::modules().await;
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            let worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(
                    probe(json!([
                        [format!("{}/data.json", server.url), "json"],
                        [format!("{}/data.txt", server.url), "text"]
                    ])),
                    format!("{}/main.js", server.url),
                )
                .with_script_kind(kind)
                .with_global_kind(global)
                .with_module_static_import_content_security_policies(vec!["connect-src *".into()])
                .with_content_security_policies(vec![
                    "connect-src 'none'".into(),
                    "default-src 'none'".into(),
                ])
                .with_content_security_report_only_policies(vec![
                    "connect-src 'none'".into(),
                    "default-src 'none'".into(),
                ]),
            );
            let actual = result(worker).await;
            for (index, extension) in ["json", "txt"].into_iter().enumerate() {
                assert_eq!(actual[index]["error"], "TypeError", "{actual}");
                assert_eq!(actual[index]["native"], true);
                let events = actual[index]["events"].as_array().unwrap();
                assert_eq!(events.len(), 4, "{actual}");
                for (event, (disposition, policy)) in events.iter().zip([
                    ("report", "connect-src 'none'"),
                    ("report", "default-src 'none'"),
                    ("enforce", "connect-src 'none'"),
                    ("enforce", "default-src 'none'"),
                ]) {
                    assert_eq!(event["directive"], "connect-src");
                    assert_eq!(event["disposition"], disposition);
                    assert_eq!(event["policy"], policy);
                    assert_eq!(
                        event["blockedURI"],
                        format!("{}/data.{extension}", server.url)
                    );
                    assert_eq!(event["documentURI"], format!("{}/main.js", server.url));
                    assert_eq!(event["sourceFile"], format!("{}/main.js", server.url));
                    assert!(event["line"].as_i64().unwrap() > 0);
                    assert!(event["column"].as_i64().unwrap() > 0);
                    assert_eq!(event["native"], true);
                    assert_eq!(event["trusted"], true);
                }
            }
        }
    }
    assert!(
        server.requests.lock().is_empty(),
        "CSP must prevent network contact"
    );
}

#[tokio::test]
async fn worker_typed_module_csp_uses_connect_instead_of_script_directives() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            let server = ModuleServer::modules().await;
            let worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(
                    probe(json!([
                        [format!("{}/data.json", server.url), "json"],
                        [format!("{}/data.txt", server.url), "text"],
                        [format!("{}/data.js", server.url), "javascript"]
                    ])),
                    format!("{}/main.js", server.url),
                )
                .with_script_kind(kind)
                .with_global_kind(global)
                .with_content_security_policies(vec![
                    "script-src 'none'; connect-src 'self'".into(),
                ])
                .with_content_security_report_only_policies(vec!["connect-src 'none'".into()]),
            );
            let actual = result(worker).await;
            assert_eq!(actual[0]["value"], 42, "{actual}");
            assert_eq!(actual[1]["value"], "module-text", "{actual}");
            for index in 0..2 {
                assert_eq!(actual[index]["events"].as_array().unwrap().len(), 1);
                assert_eq!(actual[index]["events"][0]["directive"], "connect-src");
                assert_eq!(actual[index]["events"][0]["disposition"], "report");
            }
            assert_eq!(actual[2]["error"], "TypeError", "{actual}");
            assert_eq!(actual[2]["events"][0]["directive"], "script-src-elem");
            let requests = server.requests.lock();
            assert_eq!(requests.len(), 2);
            for (request, destination) in requests.iter().zip(["json", "text"]) {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains(&format!("sec-fetch-dest: {destination}\r\n")),
                    "{request}"
                );
            }
        }
    }
}

#[tokio::test]
async fn worker_typed_module_redirect_csp_stops_each_forbidden_hop_once() {
    ensure_v8();
    for global in global_kinds() {
        for (extension, module_type, value) in [
            ("json", "json", json!(42)),
            ("txt", "text", json!("module-text")),
        ] {
            for enforce in [false, true] {
                let target = ModuleServer::modules().await;
                let target_url = format!("{}/data.{extension}", target.url);
                let source = ModuleServer::spawn(move |_| format!(
                    "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )).await;
                let request_url = format!("{}/redirect.{extension}", source.url);
                let worker = spawn_test_worker_with_options(
                    WorkerSpawnOptions::new(
                        probe(json!([[request_url, module_type]])),
                        format!("{}/main.js", source.url),
                    )
                    .with_global_kind(global.clone())
                    .with_content_security_policies(if enforce {
                        vec!["connect-src 'self'".into()]
                    } else {
                        vec![]
                    })
                    .with_content_security_report_only_policies(vec!["connect-src 'self'".into()]),
                );
                let actual = result(worker).await;
                if enforce {
                    assert_eq!(actual[0]["error"], "TypeError", "{actual}");
                } else {
                    assert_eq!(actual[0]["value"], value, "{actual}");
                }
                let events = actual[0]["events"].as_array().unwrap();
                assert_eq!(events.len(), if enforce { 2 } else { 1 }, "{actual}");
                for (index, event) in events.iter().enumerate() {
                    assert_eq!(event["directive"], "connect-src");
                    assert_eq!(
                        event["disposition"],
                        if index == 0 { "report" } else { "enforce" }
                    );
                    assert_eq!(event["blockedURI"], request_url);
                    assert_eq!(event["sourceFile"], "");
                    assert_eq!(event["line"], 0);
                    assert_eq!(event["column"], 0);
                }
                assert_eq!(source.requests.lock().len(), 1);
                assert_eq!(
                    target.requests.lock().len(),
                    usize::from(!enforce),
                    "enforced redirects must never reach the target"
                );
            }
        }
    }
}

#[tokio::test]
async fn worker_typed_module_static_import_uses_creator_connect_policy() {
    ensure_v8();
    for global in global_kinds() {
        for blocked in [false, true] {
            let server = ModuleServer::modules().await;
            let mut worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(
                    "import json from './data.json' with {type: 'json'};\n\
                     import text from './data.txt' with {type: 'text'};\n\
                     console.log(JSON.stringify([json.answer, text]));"
                        .into(),
                    format!("{}/main.js", server.url),
                )
                .with_global_kind(global.clone())
                .with_script_kind(WorkerScriptKind::Module)
                .with_module_static_import_content_security_policies(vec![
                    if blocked {
                        "worker-src *; connect-src 'none'"
                    } else {
                        "connect-src 'self'; script-src 'none'"
                    }
                    .into(),
                ])
                .with_content_security_policies(vec![
                    if blocked {
                        "connect-src *"
                    } else {
                        "connect-src 'none'"
                    }
                    .into(),
                ]),
            );
            if blocked {
                let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
                match message {
                    WorkerToParentMessage::Error { message, phase, .. } => {
                        assert!(message.contains("Content Security Policy"), "{message}");
                        assert_eq!(phase, WorkerErrorPhase::Bootstrap);
                    }
                    other => panic!("expected CSP bootstrap error, got {other:?}"),
                }
                worker.terminate_and_join();
                assert!(server.requests.lock().is_empty());
            } else {
                assert_eq!(result(worker).await, json!([42, "module-text"]));
                assert_eq!(server.requests.lock().len(), 2);
            }
        }
    }
}
