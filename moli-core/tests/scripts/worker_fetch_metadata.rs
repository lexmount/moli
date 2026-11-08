use super::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

struct WorkerMetadataServer {
    origin: String,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    task: JoinHandle<()>,
}

impl Drop for WorkerMetadataServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl WorkerMetadataServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let recorded = Arc::clone(&recorded);
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16384 {
                                let Ok(byte) = stream.read_u8().await else { return };
                                bytes.push(byte);
                            }
                            let request = String::from_utf8_lossy(&bytes);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            let header = |name: &str| request.lines()
                                .filter_map(|line| line.split_once(':'))
                                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                                .map(|(_, value)| value.trim());
                            let metadata = serde_json::json!({
                                "path": path,
                                "mode": header("Sec-Fetch-Mode"),
                                "dest": header("Sec-Fetch-Dest"),
                                "site": header("Sec-Fetch-Site"),
                                "user": header("Sec-Fetch-User"),
                            });
                            recorded.lock().push(metadata.clone());
                            let module = path.contains("type=module");
                            let dependency = if module {
                                "import {mode} from '/dependency.js';"
                            } else { "const mode = null;" };
                            let post = format!("{{...{}, dependencyMode:mode}}", metadata);
                            let (content_type, body) = if path.starts_with("/page") {
                                ("text/html", "<!doctype html><body>worker metadata".to_owned())
                            } else if path.starts_with("/dependency.js") {
                                ("text/javascript", format!("export const mode = {};", serde_json::to_string(&header("Sec-Fetch-Mode")).unwrap()))
                            } else if path.starts_with("/nested.js") {
                                ("text/javascript", format!(
                                    "const child = new Worker('/leaf.js?type={kind}', {{type:'{kind}'}}); child.onmessage = event => {{postMessage(event.data); child.terminate();}}; child.onerror = event => postMessage({{error:event.message || 'nested worker error'}});",
                                    kind = if module { "module" } else { "classic" },
                                ))
                            } else if path.starts_with("/shared.js") {
                                ("text/javascript", format!("{dependency} onconnect = event => event.ports[0].postMessage({post});"))
                            } else {
                                ("text/javascript", format!("{dependency} postMessage({post});"))
                            };
                            let policy = if path.starts_with("/page?policy=no-referrer") {
                                "Referrer-Policy: no-referrer\r\n"
                            } else { "" };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{policy}Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len(),
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            origin,
            requests,
            task,
        })
    }
}

async fn check_worker_metadata(kind: &str) -> Result<()> {
    let server = WorkerMetadataServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for policy in ["default", "no-referrer"] {
        let mut page = browser
            .fetch(&format!("{}/page?policy={policy}", server.origin))
            .await?;
        let expression = format!(
            r#"(async () => {{
            for (const type of ['classic', 'module']) {{
                const kind = {kind};
                const run = {policy};
                const url = '/' + kind + '.js?type=' + type + '&policy=' + run;
                const worker = kind === 'shared'
                    ? (type === 'classic' ? new SharedWorker(url) : new SharedWorker(url, {{type, name: 'metadata-' + type + '-' + run}}))
                    : (type === 'classic' ? new Worker(url) : new Worker(url, {{type}}));
                const result = await new Promise((resolve, reject) => {{
                    const target = kind === 'shared' ? worker.port : worker;
                    target.onmessage = event => resolve(event.data);
                    worker.onerror = event => reject(new Error(event.message || kind + ' load error'));
                }});
                if (kind === 'shared') worker.port.close(); else worker.terminate();
                const destination = kind === 'shared' ? 'sharedworker' : 'worker';
                if (result.mode !== 'same-origin' || result.dest !== destination || result.site !== 'same-origin' || result.user !== null)
                    throw new Error(kind + ' ' + type + ': ' + JSON.stringify(result));
                if (type === 'module' && result.dependencyMode !== 'cors')
                    throw new Error(kind + ' module import mode: ' + JSON.stringify(result));
            }}
            return true;
        }})()"#,
            kind = serde_json::to_string(kind)?,
            policy = serde_json::to_string(policy)?
        );
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        assert_eq!(result["value"], true, "{kind}, {policy}: {result}");
    }
    let requests = server.requests.lock();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["path"]
                .as_str()
                .is_some_and(|path| path.starts_with(&format!("/{kind}.js"))))
            .count(),
        4,
        "both script types and creator policy variants must reach HTTP"
    );
    if kind == "nested" {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("/leaf.js")))
                .count(),
            4
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_main_scripts_send_same_origin_worker_metadata() -> Result<()> {
    check_worker_metadata("dedicated").await
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_worker_main_scripts_send_same_origin_worker_metadata() -> Result<()> {
    check_worker_metadata("nested").await
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_main_scripts_send_same_origin_sharedworker_metadata() -> Result<()> {
    check_worker_metadata("shared").await
}
