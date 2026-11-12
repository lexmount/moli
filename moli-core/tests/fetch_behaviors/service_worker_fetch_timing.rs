use super::*;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test(flavor = "multi_thread")]
async fn fetch_resource_timing_service_worker_distinguishes_synthetic_and_forwarded_responses()
-> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
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
                    let captured = Arc::clone(&captured);
                    connections.spawn(serve_service_worker_timing_request(socket, captured));
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
        const controlled=navigator.serviceWorker.controller?Promise.resolve():new Promise(resolve=>navigator.serviceWorker.addEventListener('controllerchange',resolve,{{once:true}}));
        await navigator.serviceWorker.register('/sw.js');await navigator.serviceWorker.ready;await controlled;
        const windowRows=await serviceWorkerFetchTiming(location.href);
        const workerRows=await new Promise((resolve,reject)=>{{const worker=new Worker('/child.js');worker.onmessage=e=>resolve(e.data);worker.onerror=reject;}});
        return JSON.stringify({{window:windowRows,worker:workerRows}});
    }})()"#,
        include_str!("../../../moli-renderer-v8/tests/fixtures/service-worker-fetch-timing.js")
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&script, true),
    )
    .await??;
    let _ = stop.send(());
    server.await??;
    let observed: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("SW timing results")?)?;
    let rows: Vec<_> = [
        ("synthetic", "basic", 4),
        ("cross-synthetic", "basic", 4),
        ("stream", "basic", 4),
        ("cross-stream", "basic", 4),
        ("forward-same", "basic", 4),
        ("forward-cors", "cors", 0),
        ("forward-tao", "cors", 4),
        ("cache-same", "basic", 4),
        ("cache-cors", "cors", 0),
        ("cache-tao", "cors", 4),
        ("cache-again-cors", "cors", 0),
        ("cache-again-tao", "cors", 4),
        ("cache-synthetic", "basic", 4),
        ("forward-redirect-tao", "cors", 4),
        ("forward-redirect-denied", "cors", 0),
        ("cache-redirect-tao", "cors", 4),
        ("cache-redirect-denied", "cors", 0),
    ]
    .into_iter()
    .map(|(name, response_type, size)| {
        serde_json::json!({
            "name":name, "body":"body", "type":response_type, "status":200,
            "encoded":size, "decoded":size, "transfer":0, "urlMatchesSource":true,
        })
    })
    .collect();
    assert_eq!(observed, serde_json::json!({"window":rows,"worker":rows}));
    assert!(
        !requests
            .lock()
            .iter()
            .any(|path| path.starts_with("/synthetic")),
        "synthetic responses must not reach the target server"
    );
    Ok(())
}

async fn serve_service_worker_timing_request(
    mut socket: tokio::net::TcpStream,
    captured: Arc<parking_lot::Mutex<Vec<String>>>,
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
    let path = request.split_whitespace().nth(1).context("request URL")?;
    captured.lock().push(path.to_owned());
    let mut headers = String::new();
    let mut status = "200 OK";
    let (mime, body) = match path {
        "/sw.js" => ("text/javascript", r#"
        self.addEventListener('install',event=>event.waitUntil(self.skipWaiting()));
        self.addEventListener('activate',event=>event.waitUntil(self.clients.claim()));
        self.addEventListener('fetch',event=>{
            const url=new URL(event.request.url);
            if(url.pathname==='/synthetic') {
                const body=url.searchParams.has('stream') ? new ReadableStream({start(controller){controller.enqueue(new TextEncoder().encode('body'));controller.close();}}) : 'body';
                event.respondWith(new Response(body,{headers:{'Content-Type':'text/plain'}}));
            } else if(url.pathname==='/forward') {
                event.respondWith(fetch(url.searchParams.get('target')).then(response=>response.clone()));
            } else if(url.pathname==='/cache') {
                event.respondWith((async()=>{
                    const cache=await caches.open('timing');
                    let response=url.searchParams.has('synthetic')
                        ? new Response('body',{headers:{'Content-Type':'text/plain'}})
                        : await fetch(url.searchParams.get('target'));
                    const key=new URL('/cached-entry',location.href);
                    key.search=url.search;
                    const rounds=Number(url.searchParams.get('rounds')||1);
                    for(let i=0;i<rounds;i++) {
                        await cache.put(key,response);
                        response=await cache.match(key);
                    }
                    return response;
                })());
            }
        });
        "#.to_owned()),
        "/child.js" => (
            "text/javascript",
            format!(
                "{}\nserviceWorkerFetchTiming(location.href).then(rows=>{{postMessage(rows);close();}},error=>{{postMessage({{error:String(error.stack||error)}});close();}});",
                include_str!("../../../moli-renderer-v8/tests/fixtures/service-worker-fetch-timing.js")
            ),
        ),
        path if path.starts_with("/redirect?") => {
            let url = url::Url::parse(&format!("http://localhost{path}"))?;
            let target = url.query_pairs().find(|(name, _)| name == "to").context("redirect target")?.1.into_owned();
            headers.push_str(&format!("Access-Control-Allow-Origin: *\r\nLocation: {target}\r\n"));
            if url.query_pairs().any(|(name, value)| name == "tao" && value == "1") {
                headers.push_str("Timing-Allow-Origin: *\r\n");
            }
            status = "302 Found";
            ("text/plain", String::new())
        }
        path if path.starts_with("/raw") => {
            headers.push_str("Access-Control-Allow-Origin: *\r\n");
            if path.contains("tao=1") {
                headers.push_str("Timing-Allow-Origin: *\r\n");
            }
            ("text/plain", "body".to_owned())
        }
        path if path.starts_with("/synthetic") => ("text/plain", "unexpected-network".to_owned()),
        _ => ("text/html", "<!doctype html><body>SW timing".to_owned()),
    };
    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{headers}Connection: close\r\n\r\n{body}",body.len()).as_bytes()).await?;
    Ok(())
}
