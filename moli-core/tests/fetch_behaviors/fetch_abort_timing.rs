use super::*;
use std::{collections::HashSet, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};

#[tokio::test(flavor = "multi_thread")]
async fn fetch_resource_timing_real_abort_covers_window_and_worker_body_phases() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let (seen, _) = watch::channel(HashSet::<String>::new());
    let (released, _) = watch::channel(HashSet::<String>::new());
    let (finished, _) = watch::channel(HashSet::<String>::new());
    let requests = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let captured = Arc::clone(&requests);
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => {
                    let (socket, _) = accepted?;
                    let (seen, released, finished) =
                        (seen.clone(), released.clone(), finished.clone());
                    let captured = Arc::clone(&captured);
                    connections.spawn(serve_abort_timing_request(socket, captured, seen, released, finished));
                }
                Some(result) = connections.join_next(), if !connections.is_empty() => { result??; }
            }
        }
        connections.shutdown().await;
        Ok::<_, anyhow::Error>(())
    });
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{base}/page")).await?;
    let script = format!(
        r#"(async()=>{{
        {}
        const windowRows=await fetchAbortResourceTiming(location.href,'window');
        const workerRows=await new Promise((resolve,reject)=>{{const worker=new Worker('/worker.js');worker.onmessage=e=>resolve(e.data);worker.onerror=reject;}});
        return JSON.stringify({{window:windowRows,worker:workerRows}});
    }})()"#,
        include_str!("../../../moli-renderer-v8/tests/fixtures/fetch-abort-resource-timing.js")
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&script, true),
    )
    .await??;
    let _ = stop.send(());
    server.await??;
    let observed: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("abort timing results")?)?;
    let rows = serde_json::json!([
        {"stage":"normal","outcome":"success","count":1,"observed":1,"types":["fetch"]},
        {"stage":"pre-aborted","outcome":"AbortError","count":0,"observed":0,"types":[]},
        {"stage":"headers","outcome":"AbortError","count":1,"observed":1,"types":["fetch"]},
        {"stage":"body","outcome":"AbortError","count":1,"observed":1,"types":["fetch"]}
    ]);
    assert_eq!(observed, serde_json::json!({"window":rows,"worker":rows}));
    assert!(!requests.lock().iter().any(|path| path == "/never"));
    Ok(())
}

async fn serve_abort_timing_request(
    mut socket: tokio::net::TcpStream,
    captured: Arc<parking_lot::Mutex<Vec<String>>>,
    seen: watch::Sender<HashSet<String>>,
    released: watch::Sender<HashSet<String>>,
    finished: watch::Sender<HashSet<String>>,
) -> Result<()> {
    let mut bytes = Vec::new();
    loop {
        let mut buf = [0; 4096];
        let size = socket.read(&mut buf).await?;
        if size == 0 {
            return Ok::<_, anyhow::Error>(());
        }
        bytes.extend_from_slice(&buf[..size]);
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8(bytes)?;
    let target = request.split_whitespace().nth(1).context("request URL")?;
    let url = url::Url::parse(&format!("http://fixture{target}"))?;
    captured.lock().push(url.path().to_owned());
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let token = params.get("token").cloned().unwrap_or_default();
    let (mime, body) = match url.path() {
        "/worker.js" => (
            "text/javascript",
            format!(
                "{}\nfetchAbortResourceTiming(location.href,'worker').then(rows=>{{postMessage(rows);close();}},error=>{{postMessage({{error:String(error.stack||error)}});close();}});",
                include_str!(
                    "../../../moli-renderer-v8/tests/fixtures/fetch-abort-resource-timing.js"
                )
            ),
        ),
        "/slow" => {
            if params.get("stage").is_some_and(|stage| stage == "body") {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 8\r\nConnection: close\r\n\r\npart").await?;
            }
            seen.send_modify(|set| {
                set.insert(token.clone());
            });
            let mut release = released.subscribe();
            while !release.borrow_and_update().contains(&token) {
                release.changed().await?;
            }
            let completion: &[u8] = if params.get("stage").is_some_and(|stage| stage == "body") {
                b"body"
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody"
            };
            let _ = socket.write_all(completion).await;
            finished.send_modify(|set| {
                set.insert(token);
            });
            return Ok(());
        }
        "/seen" => {
            let mut ready = seen.subscribe();
            while !ready.borrow_and_update().contains(&token) {
                ready.changed().await?;
            }
            ("text/plain", "seen".to_owned())
        }
        "/release" => {
            released.send_modify(|set| {
                set.insert(token.clone());
            });
            let mut complete = finished.subscribe();
            while !complete.borrow_and_update().contains(&token) {
                complete.changed().await?;
            }
            ("text/plain", "released".to_owned())
        }
        "/normal" => ("text/plain", "body".to_owned()),
        _ => ("text/html", "<!doctype html><body>Fetch timing".to_owned()),
    };
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await?;
    Ok(())
}
