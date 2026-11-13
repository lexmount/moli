use crate::network::ResourceRequestClient;
use crate::{JsRuntime, RendererPageCommand, RendererPageReply};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PARTITION_WORKER: &str = r#"
addEventListener('install', e => e.waitUntil(skipWaiting()));
addEventListener('activate', e => e.waitUntil(clients.claim()));
addEventListener('fetch', e => {
  if (new URL(e.request.url).pathname === '/sw-probe') {
    e.respondWith(new Response(`<!doctype html><script>
      parent.postMessage({marker:location.search.slice(1),kind:'interception',
        controlled:!!navigator.serviceWorker.controller,
        stored:localStorage.getItem('partition-probe')},'*');
    <\/script>`, {headers:{'Content-Type':'text/html'}}));
  }
});
"#;
const PARTITION_NETWORK: &str = r#"<!doctype html><script>
parent.postMessage({marker:location.search.slice(1),kind:'network',
  controlled:!!navigator.serviceWorker.controller,
  stored:localStorage.getItem('partition-probe')},'*');
</script>"#;
const PARTITION_POPUP: &str = r#"<!doctype html><body><script>
addEventListener('message', e => opener.postMessage(e.data,'*'));
const frame=document.createElement('iframe');
frame.src=new URLSearchParams(location.search).get('child');
document.body.appendChild(frame);
</script>"#;

struct PartitionFixture {
    _tls: moli_websocket::test_support::TlsWebSocketFixture,
    ca_file: std::path::PathBuf,
    url: String,
    server: tokio::task::JoinHandle<()>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Drop for PartitionFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn partition_fixture() -> PartitionFixture {
    let tls = moli_websocket::test_support::TlsWebSocketFixture::with_server_names(vec![
        "127.0.0.1".to_owned(),
        "localhost".to_owned(),
    ]);
    let acceptor = tls.server_acceptor(false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    );
    let ca_file = tls.tls_config().ca_cert.expect("fixture CA certificate");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = requests.clone();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            request_log.lock().push("TCP accepted".to_owned());
            let acceptor = acceptor.clone();
            let request_log = request_log.clone();
            tokio::spawn(async move {
                let mut stream = match acceptor.accept(socket).await {
                    Ok(stream) => stream,
                    Err(error) => {
                        request_log.lock().push(format!("TLS error: {error}"));
                        return;
                    }
                };
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let Ok(length) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if length == 0 || request.len() > 8192 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..length]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                request_log.lock().push(path.to_owned());
                let (content_type, body) = match path.split('?').next().unwrap() {
                    "/sw.js" => ("application/javascript", PARTITION_WORKER),
                    "/sw-probe" => ("text/html", PARTITION_NETWORK),
                    "/sw-popup" => ("text/html", PARTITION_POPUP),
                    _ => ("text/plain", "unexpected resource"),
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    PartitionFixture {
        _tls: tls,
        ca_file,
        url,
        server,
        requests,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_frames_keep_top_level_storage_partition_in_popups() {
    let fixture = partition_fixture().await;
    let base = &fixture.url;
    let root_url = url::Url::parse(&format!("{base}/sw-root")).unwrap();
    let mut config = moli_fetch::FetchConfig::default();
    config.set_tls_credentials(Some(fixture.ca_file.clone()), None, None, None);
    config.set_http_host_resolve(vec![format!(
        "localhost:{}:127.0.0.1",
        root_url.port().unwrap()
    )]);
    let loader = ResourceRequestClient::new(&config).unwrap();
    let runtime = JsRuntime::initialize();
    let html = r#"<!doctype html><body><script>
      globalThis.swPartitionMessages=[];
      globalThis.swPartitionError=null;
      addEventListener('message', e=>swPartitionMessages.push(e.data));
      (async()=>{
        localStorage.setItem('partition-probe','first-party');
        const registration=await navigator.serviceWorker.register('/sw.js');
        const worker=registration.installing||registration.waiting||registration.active;
        if(worker.state!=='activated') await new Promise(resolve=>{
          worker.addEventListener('statechange',()=>{
            if(worker.state==='activated') resolve();
          });
        });
        const direct=document.createElement('iframe');
        direct.src='/sw-probe?direct';document.body.appendChild(direct);
        for(const [marker,host] of [['same','127.0.0.1'],['other','localhost']]){
          const child=new URL('/sw-probe?'+marker,location.href);
          const popup=new URL('/sw-popup',location.href);
          popup.hostname=host;popup.searchParams.set('child',child.href);
          open(popup.href);
        }
      })().catch(e=>{swPartitionError=String(e)});
    </script>"#;
    let (mut page, _, _, _artifacts, download) = runtime
        .create_html_page_from_response(
            root_url.clone(),
            root_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            html.to_owned(),
            crate::RendererDocumentOptions::default(),
        )
        .await
        .unwrap();
    assert!(download.is_none());
    let delivered = page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: "swPartitionMessages.length===3 || swPartitionError!==null".to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await;
    let (reply, _)=page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression:"JSON.stringify({error:swPartitionError,messages:swPartitionMessages.sort((a,b)=>a.marker<b.marker?-1:1)})".to_owned(),
        await_promise:false,
    }).await.unwrap();
    let RendererPageReply::RuntimeEvaluationResult(reply) = reply else {
        panic!("expected partition results")
    };
    let payload = reply.into_protocol_payload();
    let result: serde_json::Value =
        serde_json::from_str(payload["value"].as_str().unwrap()).unwrap();
    assert!(
        delivered.is_ok(),
        "wait={:?}; results={result}; requests={:?}",
        delivered.as_ref().err(),
        fixture.requests.lock()
    );
    assert_eq!(
        result,
        serde_json::json!({"error":null,"messages":[
            {"marker":"direct","kind":"interception","controlled":true,"stored":"first-party"},
            {"marker":"other","kind":"network","controlled":false,"stored":null},
            {"marker":"same","kind":"interception","controlled":true,"stored":"first-party"},
        ]})
    );
    page.close_async().await.unwrap();
}
