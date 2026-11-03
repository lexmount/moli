use super::*;
use crate::runtime::ResourceRequestClient;
use std::sync::Arc;
use tokio::sync::watch;
use tokio_rustls::rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};

#[derive(Default)]
struct Wire {
    bytes: Vec<u8>,
    headers: http::HeaderMap,
    ended: bool,
}

#[derive(Clone)]
struct Signals {
    headers: watch::Sender<bool>,
    first: watch::Sender<bool>,
    finished: watch::Sender<bool>,
    release: watch::Sender<bool>,
    stop: watch::Sender<bool>,
}

async fn wait(signal: &watch::Sender<bool>) -> anyhow::Result<()> {
    let mut receiver = signal.subscribe();
    while !*receiver.borrow_and_update() {
        receiver.changed().await?;
    }
    Ok(())
}

struct UploadServer {
    url: Url,
    signals: Signals,
    wire: Arc<parking_lot::Mutex<Wire>>,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

impl UploadServer {
    async fn new(blocked: bool, status: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = Url::parse(&format!("https://{}/", listener.local_addr()?))?;
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])?;
        let mut tls = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.cert.der().to_vec())],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der())),
            )?;
        tls.alpn_protocols = vec![b"h2".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let signals = Signals {
            headers: watch::channel(false).0,
            first: watch::channel(false).0,
            finished: watch::channel(false).0,
            release: watch::channel(!blocked).0,
            stop: watch::channel(false).0,
        };
        let wire = Arc::new(parking_lot::Mutex::new(Wire::default()));
        let shared = wire.clone();
        let events = signals.clone();
        let task = tokio::spawn(async move {
            let mut stop = events.stop.subscribe();
            let mut connections = tokio::task::JoinSet::new();
            while !*stop.borrow_and_update() {
                let socket = tokio::select! {
                    _ = stop.changed() => break,
                    accepted = listener.accept() => accepted?.0,
                };
                let events = events.clone();
                let wire = shared.clone();
                let acceptor = acceptor.clone();
                connections.spawn(async move {
                    let socket = acceptor.accept(socket).await?;
                    let mut release = events.release.subscribe();
                    let mut blocked = !*release.borrow_and_update();
                    let mut builder = h2::server::Builder::new();
                    builder.initial_window_size(if blocked { 0 } else { 65_535 });
                    let mut connection = builder.handshake(socket).await?;
                    let mut stop = events.stop.subscribe();
                    let mut requests = tokio::task::JoinSet::new();
                    while !*stop.borrow_and_update() {
                        let request = tokio::select! {
                            _ = stop.changed() => break,
                            _ = release.changed(), if blocked => {
                                connection.set_initial_window_size(1_048_576)?;
                                blocked = false;
                                continue;
                            }
                            request = connection.accept() => request,
                        };
                        let Some(Ok((request, mut response))) = request else {
                            break;
                        };
                        let events = events.clone();
                        let wire = wire.clone();
                        requests.spawn(async move {
                            let path = request.uri().path();
                            let origin = request
                                .headers()
                                .get("origin")
                                .cloned()
                                .unwrap_or_else(|| http::HeaderValue::from_static("*"));
                            let head = http::Response::builder()
                                .header("cache-control", "no-store")
                                .header("access-control-allow-origin", origin)
                                .header("access-control-allow-credentials", "true")
                                .header("access-control-allow-methods", "POST")
                                .header("access-control-allow-headers", "authorization");
                            if request.method() == http::Method::OPTIONS {
                                response.send_response(head.status(204).body(())?, true)?;
                            } else if path == "/upload" {
                                wire.lock().headers = request.headers().clone();
                                events.headers.send_replace(true);
                                let mut head = head.status(status);
                                if matches!(status, 301 | 302 | 303 | 307 | 308) {
                                    head = head.header("location", "/done");
                                }
                                if status == 401 {
                                    head = match request.uri().query() {
                                        Some("basic") => head
                                            .header("www-authenticate", "Basic realm=\"upload\""),
                                        Some("bearer") => head
                                            .header("www-authenticate", "Bearer realm=\"upload\""),
                                        _ => head,
                                    };
                                }
                                let mut send = response.send_response(head.body(())?, false)?;
                                // End the response before the request ends. The browser
                                // must keep uploading and enforce its half-duplex barrier.
                                send.send_data(bytes::Bytes::from_static(b"uploaded"), true)?;
                                let mut body = request.into_body();
                                while let Some(chunk) = body.data().await {
                                    let chunk = chunk?;
                                    wire.lock().bytes.extend_from_slice(&chunk);
                                    if !chunk.is_empty() {
                                        events.first.send_replace(true);
                                    }
                                    body.flow_control().release_capacity(chunk.len())?;
                                }
                                wire.lock().ended = true;
                                events.finished.send_replace(true);
                            } else {
                                match path {
                                    "/first" => wait(&events.first).await?,
                                    "/headers" => wait(&events.headers).await?,
                                    "/finished" => wait(&events.finished).await?,
                                    "/release" => {
                                        events.release.send_replace(true);
                                    }
                                    _ => {}
                                }
                                let head = head.status(200).body(())?;
                                let mut send = response.send_response(head, false)?;
                                send.send_data(bytes::Bytes::from_static(b"uploaded"), true)?;
                            }
                            Ok::<_, anyhow::Error>(())
                        });
                    }
                    requests.abort_all();
                    while requests.join_next().await.is_some() {}
                    Ok::<_, anyhow::Error>(())
                });
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
            Ok(())
        });
        Ok(Self {
            url,
            signals,
            wire,
            task: Some(task),
        })
    }

    async fn shutdown(mut self) {
        self.signals.stop.send_replace(true);
        self.task.take().unwrap().await.unwrap().unwrap();
    }
}

impl Drop for UploadServer {
    fn drop(&mut self) {
        self.signals.stop.send_replace(true);
    }
}

async fn window_probe(server: &UploadServer, script: &str) -> serde_json::Value {
    let mut config = FetchConfig::default();
    // This fixture uses a fresh self-signed loopback certificate.
    config.set_tls_verify_host(false);
    config.set_http_no_proxy(Some("*".to_owned()));
    let owner = ResourceRequestClient::new(&config).unwrap();
    let mut page = test_page_vm_with_loader_and_document_url(
        &owner,
        Vec::new(),
        server.url.join("page").unwrap(),
    );
    page.retain_standalone_request_client_owner_for_test(owner);
    let executor = page.local_executor.clone();
    let source = format!(
        r#"
        (async () => {{ {script} }})().then(
            result => globalThis.__uploadResult = result,
            error => globalThis.__uploadResult = {{failure:String(error)}}
        ).finally(() => globalThis.__uploadDone = true);
    "#
    );
    let result = executor
        .run(async move {
            page.vm_mut().eval(&source)?;
            drive_websocket_until_done(
                &mut page,
                "String(globalThis.__uploadDone === true)",
                "H2 author stream upload should complete",
            )
            .await?;
            page.vm_mut()
                .eval("JSON.stringify(globalThis.__uploadResult)")
        })
        .await
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[tokio::test]
async fn local_responses_do_not_consume_author_streams_in_window_or_worker() {
    run_page_vm_async_test(async {
        let mut page = test_page_vm_with_document_url(Url::parse("https://example.com/").unwrap());
        let executor = page.local_executor.clone();
        let result = executor.run(async move {
            page.vm_mut().eval(r#"
                (async()=>{
                    async function check(){
                        const rows=[];
                        for(const stage of ['start','pull']){
                            const body=new ReadableStream({[stage](c){c.error(new Error('source'));}});
                            const response=await fetch('data:text/plain,local',{method:'POST',body,duplex:'half'});
                            rows.push({stage,status:response.status,text:await response.text(),locked:body.locked});
                        }
                        return rows;
                    }
                    const window=await check();
                    const url=URL.createObjectURL(new Blob(['onmessage=async()=>postMessage(await ('+check.toString()+')());']));
                    const worker=new Worker(url);
                    try {
                        const dedicated=await new Promise((resolve,reject)=>{
                            worker.onmessage=e=>resolve(e.data);
                            worker.onerror=e=>reject(new Error(e.message));
                            worker.postMessage(1);
                        });
                        return {window,dedicated};
                    } finally {
                        worker.terminate();URL.revokeObjectURL(url);
                    }
                })().then(value=>globalThis.__uploadResult=value,
                          error=>globalThis.__uploadResult={failure:String(error)})
                    .finally(()=>globalThis.__uploadDone=true);
            "#)?;
            drive_websocket_until_done(&mut page,"String(globalThis.__uploadDone===true)","local responses should ignore author upload streams").await?;
            page.vm_mut().eval("JSON.stringify(globalThis.__uploadResult)")
        }).await.unwrap();
        let rows = serde_json::json!([
            {"stage":"start","status":200,"text":"local","locked":false},
            {"stage":"pull","status":200,"text":"local","locked":false}
        ]);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&result).unwrap(),serde_json::json!({"window":rows,"dedicated":rows}));
    }).await;
}

#[tokio::test]
async fn window_upload_sends_binary_chunks_before_eof_and_waits_for_transmission() {
    run_page_vm_async_test(async {
        for status in [200, 301, 302, 303, 307, 308] {
            let server = UploadServer::new(false, status).await.unwrap();
            let result = window_probe(&server, r#"
                const backing = new Uint8Array([88,0,255,65,66,99]);
                const view = backing.subarray(1,5);
                let controller;
                const stream = new ReadableStream({
                    start(c){controller=c;c.enqueue(view);},
                    pull(){backing.fill(123);}
                });
                ReadableStream.prototype.getReader = () => {throw new Error('public getReader');};
                ReadableStreamDefaultReader.prototype.read = () => {throw new Error('public read');};
                const request = new Request('/upload',{method:'POST',body:stream,duplex:'half'});
                let settled = false;
                const promise = fetch(request).then(
                    async r => {settled=true;return {ok:true,status:r.status,text:await r.text()};},
                    e => {settled=true;return {ok:false,type:e.name};}
                );
                const locked = stream.locked && request.bodyUsed;
                await (await fetch('/first')).text();
                if(settled) throw new Error('Response settled before upload EOF: '+JSON.stringify(await promise));
                controller.enqueue(new Uint8Array());
                controller.enqueue(new Uint8Array(196613).fill(7));
                controller.close();
                const result = await promise;
                await (await fetch('/finished')).text();
                return {locked,result};
            "#).await;
            let expected = if matches!(status, 200 | 303) {
                serde_json::json!({"locked":true,"result":{"ok":true,"status":200,"text":"uploaded"}})
            } else {
                serde_json::json!({"locked":true,"result":{"ok":false,"type":"TypeError"}})
            };
            assert_eq!(result, expected, "early status {status}");
            {
                let wire = server.wire.lock();
                assert!(wire.ended);
                assert_eq!(&wire.bytes[..4], &[0,255,65,66]);
                assert_eq!(wire.bytes.len(), 4 + 196_613);
                assert!(wire.bytes[4..].iter().all(|byte| *byte == 7));
                assert!(wire.headers.get("content-length").is_none());
                assert!(wire.headers.get("content-type").is_none());
            }
            server.shutdown().await;
        }
    }).await;
}

#[tokio::test]
async fn window_upload_applies_transport_backpressure_to_internal_stream_reads() {
    run_page_vm_async_test(async {
        let server = UploadServer::new(true, 200).await.unwrap();
        let result = window_probe(
            &server,
            r#"
            let pulls=0, settled=false;
            const body = new ReadableStream({pull(c){
                c.enqueue(new Uint8Array(65536).fill(1));
                if(++pulls===64)c.close();
            }},{highWaterMark:0});
            const promise = fetch('/upload',{method:'POST',body,duplex:'half'})
                .then(async r => {settled=true;return r.text();});
            await (await fetch('/headers')).text();
            const bounded = pulls < 64 && !settled;
            await (await fetch('/release')).text();
            const text = await promise;
            await (await fetch('/finished')).text();
            return {bounded,pulls,text};
        "#,
        )
        .await;
        assert_eq!(
            result,
            serde_json::json!({"bounded":true,"pulls":64,"text":"uploaded"})
        );
        assert_eq!(server.wire.lock().bytes.len(), 64 * 65536);
        assert!(server.wire.lock().ended);
        server.shutdown().await;
    })
    .await;
}

#[tokio::test]
async fn window_upload_authentication_depends_on_challenge_and_credentials() {
    run_page_vm_async_test(async {
        let server = UploadServer::new(false, 401).await.unwrap();
        let result = window_probe(&server, r#"
            const failures=[];
            for(const cross of [false,true])for(const challenge of ['basic','bearer','none'])
            for(const credentials of ['omit','same-origin','include'])
            for(const streaming of [false,true])for(const authorization of [false,true]){
                const url=new URL('/upload?'+challenge,location.href);
                if(cross)url.hostname='localhost';
                const bytes=new Uint8Array([0,255,65,66]);
                const body=streaming?new ReadableStream({start(c){c.enqueue(bytes);c.close();}}):bytes;
                const blocked=streaming && challenge==='basic' &&
                    (credentials==='include' || credentials==='same-origin'&&!cross);
                let correct;
                try {
                    const response=await fetch(url,{method:'POST',body,duplex:'half',credentials,
                        headers:authorization?{Authorization:'Basic dXNlcjpwYXNz'}:{}});
                    correct=!blocked && response.status===401 && await response.text()==='uploaded';
                } catch(e){correct=blocked && e instanceof TypeError;}
                if(!correct)failures.push({cross,challenge,credentials,streaming,authorization});
            }
            return failures;
        "#).await;
        assert_eq!(result, serde_json::json!([]));
        server.shutdown().await;
    }).await;
}

#[tokio::test]
async fn window_upload_maps_stream_failures_and_preserves_abort_identity() {
    run_page_vm_async_test(async {
        let server = UploadServer::new(false, 200).await.unwrap();
        let result = window_probe(&server, r#"
            const checks=[];
            for(const stage of ['start','pull'])for(const reason of [new Error('source'),{source:true},null,undefined,new DOMException('source','AbortError')]){
                const stream = new ReadableStream({[stage](c){c.error(reason);}});
                try {await fetch('/upload',{method:'POST',body:stream,duplex:'half'});checks.push(false);}
                catch(e){checks.push(e instanceof TypeError && e!==reason && stream.locked);}
            }
            for(const reason of [{abort:true},'abort marker',null]){
                let cancelled, calls=0;
                const body=new ReadableStream({cancel(value){cancelled=value;calls++;}});
                const controller=new AbortController();
                const promise=fetch('/upload',{method:'POST',body,duplex:'half',signal:controller.signal}).catch(e=>e);
                controller.abort(reason);
                checks.push(await promise===reason && cancelled===reason && calls===1);
            }
            return checks;
        "#).await;
        assert_eq!(result, serde_json::json!(vec![true; 13]));
        server.shutdown().await;
    }).await;
}
