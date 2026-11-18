use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

struct WorkerTimingServer {
    origin: String,
    task: JoinHandle<()>,
}

impl Drop for WorkerTimingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl WorkerTimingServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16384 {
                                let Ok(byte) = stream.read_u8().await else { return };
                                bytes.push(byte);
                            }
                            let request = String::from_utf8_lossy(&bytes);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            let (status, content_type, body) = if path.starts_with("/page") {
                                ("200 OK", "text/html", "<!doctype html><body>worker resource timing".to_owned())
                            } else if path.starts_with("/missing.js") {
                                ("404 Not Found", "text/javascript", "missing worker".to_owned())
                            } else if path.starts_with("/wrong-mime.js") {
                                ("200 OK", "text/plain", "postMessage('wrong MIME');".to_owned())
                            } else {
                                ("200 OK", "text/javascript", "postMessage({ownEntries:performance.getEntriesByName(location.href, 'resource').length});".to_owned())
                            };
                            let response = format!(
                                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self { origin, task })
    }
}

fn assert_worker_timing(entries: &serde_json::Value, kind: &str, status: u16) {
    let entries = entries
        .as_array()
        .unwrap_or_else(|| panic!("resource entries: {entries}"));
    assert_eq!(entries.len(), 1, "one completed creator fetch: {entries:?}");
    let entry = &entries[0];
    assert_eq!(
        entry["initiatorType"],
        if kind == "module" { "script" } else { "other" }
    );
    assert!(
        entry["encodedBodySize"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "{entry}"
    );
    assert_eq!(entry["responseStatus"], status, "{entry}");
    assert!(
        entry["startTime"]
            .as_f64()
            .is_some_and(|start| start >= 0.0),
        "{entry}"
    );
}

async fn check_worker_creator_timing(kind: &str, iframe: bool) -> Result<()> {
    let server = WorkerTimingServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let expression = format!(
        r#"(async () => {{
        const type = {kind};
        let realm = window;
        if ({iframe}) {{
            const frame = document.createElement('iframe');
            frame.src = '/page?child';
            await new Promise(resolve => {{ frame.onload = resolve; document.body.appendChild(frame); }});
            realm = frame.contentWindow;
        }}
        const url = new URL('/leaf.js?type=' + type, location.href).href;
        const worker = new realm.Worker(url, {{type}});
        const message = await new Promise((resolve, reject) => {{
            worker.onmessage = event => resolve(event.data);
            worker.onerror = event => reject(new Error(event.message || 'worker load error'));
        }});
        worker.terminate();
        const entries = realm.performance.getEntriesByName(url, 'resource').map(e => ({{
            initiatorType:e.initiatorType, encodedBodySize:e.encodedBodySize, startTime:e.startTime, responseStatus:e.responseStatus
        }}));
        return {{entries, message, topEntries:performance.getEntriesByName(url, 'resource').length}};
    }})()"#,
        kind = serde_json::to_string(kind)?,
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({expression}).then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("serialized worker result: {result}")),
    )?;
    assert_worker_timing(&value["entries"], kind, 200);
    assert_eq!(value["topEntries"], if iframe { 0 } else { 1 }, "{result}");
    assert_eq!(value["message"]["ownEntries"], 0, "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn classic_worker_timing_belongs_to_creator() -> Result<()> {
    check_worker_creator_timing("classic", false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn module_worker_timing_belongs_to_creator() -> Result<()> {
    check_worker_creator_timing("module", false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_classic_worker_timing_stays_in_creator_window() -> Result<()> {
    check_worker_creator_timing("classic", true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_module_worker_timing_stays_in_creator_window() -> Result<()> {
    check_worker_creator_timing("module", true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_worker_response_keeps_creator_timing() -> Result<()> {
    let server = WorkerTimingServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let result = tokio::time::timeout(Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(r#"(async () => {
            const results = [];
            for (const type of ['classic', 'module']) {
                for (const path of ['/missing.js', '/wrong-mime.js']) {
                    const url = new URL(path + '?type=' + type, location.href).href;
                    const worker = new Worker(url, {type});
                    await new Promise((resolve, reject) => {
                        worker.onerror = event => { event.preventDefault(); resolve(); };
                        worker.onmessage = () => reject(new Error('invalid response executed'));
                    });
                    worker.terminate();
                    results.push({type, path, entries:performance.getEntriesByName(url, 'resource').map(e => ({
                        initiatorType:e.initiatorType, encodedBodySize:e.encodedBodySize, startTime:e.startTime, responseStatus:e.responseStatus
                    }))});
                }
            }
            return results;
        })().then(JSON.stringify)"#, true)).await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("serialized failed fetch result: {result}")),
    )?;
    let cases = value.as_array().expect("failed fetch results");
    assert_eq!(cases.len(), 4, "{result}");
    for case in cases {
        let status = if case["path"] == "/missing.js" {
            404
        } else {
            200
        };
        assert_worker_timing(&case["entries"], case["type"].as_str().unwrap(), status);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn local_worker_scripts_do_not_invent_http_timings() -> Result<()> {
    let server = WorkerTimingServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let result = tokio::time::timeout(Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(r#"(async () => {
            const source = 'postMessage(true)';
            const blobURL = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
            try {
                for (const url of [blobURL, 'data:text/javascript,' + encodeURIComponent(source)]) {
                    const worker = new Worker(url);
                    await new Promise((resolve, reject) => {
                        worker.onmessage = resolve;
                        worker.onerror = event => reject(new Error(event.message || 'local load error'));
                    });
                    worker.terminate();
                    if (performance.getEntriesByName(url, 'resource').length !== 0)
                        throw new Error('local Worker invented an HTTP resource timing');
                }
            } finally { URL.revokeObjectURL(blobURL); }
            return true;
        })()"#, true)).await??;
    assert_eq!(result["value"], true, "{result}");
    Ok(())
}
