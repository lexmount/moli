use super::*;

async fn request_head(stream: &mut tokio::net::TcpStream) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
        let mut buffer = [0; 4096];
        let count =
            tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer)).await??;
        anyhow::ensure!(count != 0 && bytes.len() < 65536, "incomplete HTTP request");
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(String::from_utf8(bytes)?)
}

#[tokio::test]
async fn response_script_preload_is_consumed_once_in_its_child_document() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for expected in ["/pages/child", "/asset.js", "/asset.js"] {
                let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
                let request = request_head(&mut stream).await?;
                assert!(request.starts_with(&format!("GET {expected} ")), "{request}");
                let (content_type, fields, body) = if expected == "/pages/child" {
                    ("text/html", "Link: <../asset.js#preloaded>; rel=preload; as=script; nonce=az\r\nContent-Security-Policy: script-src 'nonce-az'\r\n", r#"<!doctype html><head><base href='/wrong/'></head><body><script nonce=az>
const url = new URL('/asset.js#preloaded', location).href;
let started = false;
function consume() {
  if (started) return;
  started = true;
  const first = document.createElement('script');
  first.src = url; first.nonce = 'az';
  first.onload = () => {
    const second = document.createElement('script');
    second.src = url; second.nonce = 'az';
    second.onload = () => parent.postMessage(JSON.stringify({
      executions: globalThis.preloadedScriptExecutions,
      links: document.querySelectorAll('link').length,
      timings: performance.getEntriesByName(url).map(entry => entry.initiatorType)
    }), '*');
    document.head.appendChild(second);
  };
  document.head.appendChild(first);
}
new PerformanceObserver(list => {
  if (list.getEntries().some(entry => entry.name === url)) consume();
}).observe({type: 'resource', buffered: true});
</script>"#)
                } else {
                    ("text/javascript", "", "globalThis.preloadedScriptExecutions = (globalThis.preloadedScriptExecutions || 0) + 1;")
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n{fields}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
            }
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent")).unwrap());
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
addEventListener('message', event => { globalThis.preloadResult = JSON.parse(event.data); });
const child = document.createElement('iframe');
child.src = '/pages/child'; document.body.appendChild(child);
"#)?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.preloadResult !== undefined)", "Document response script preload consumption").await?;
            let observed = page_vm.vm_mut().eval("JSON.stringify(preloadResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&observed)?, json!({"executions": 2, "links": 0, "timings": ["link", "script"]}));
            assert_eq!(page_vm.vm_mut().eval("String(performance.getEntriesByType('resource').filter(entry => new URL(entry.name).pathname === '/asset.js').length)")?, "0");
            server.await??;
            anyhow::Ok(())
        }).await.expect("header script preload consumption regression");
    }).await;
}

async fn assert_controlled_response_script_preload(response_kind: &'static str) {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let script = "globalThis.controlledPreloadExecutions = (globalThis.controlledPreloadExecutions || 0) + 1;";
        let quoted_script = serde_json::to_string(script).unwrap();
        let response = match response_kind {
            "fallback" => String::new(),
            "forward" => "event.respondWith(fetch(event.request));".to_owned(),
            "response" => format!("event.respondWith(new Response({quoted_script}, {{headers: {{'Content-Type': 'text/javascript'}}}}));"),
            "stream" => format!("event.respondWith(new Response(new ReadableStream({{start(controller) {{controller.enqueue(new TextEncoder().encode({quoted_script})); controller.close();}}}}), {{headers: {{'Content-Type': 'text/javascript'}}}}));"),
            _ => unreachable!(),
        };
        let worker = format!(r#"
addEventListener('install', event => event.waitUntil(skipWaiting()));
addEventListener('activate', event => event.waitUntil(clients.claim()));
addEventListener('fetch', event => {{
  if (new URL(event.request.url).pathname !== '/asset.js') return;
  event.waitUntil(fetch('/seen?destination=' + event.request.destination + '&referrer=' + encodeURIComponent(event.request.referrer) + '&policy=' + encodeURIComponent(event.request.referrerPolicy)));
  {response}
}});
"#);
        let child = r#"<!doctype html><script>
const url = new URL('/asset.js', location).href;
let started = false;
new PerformanceObserver(list => {
  if (started || !list.getEntries().some(entry => entry.name === url)) return;
  started = true;
  const before = globalThis.controlledPreloadExecutions || 0;
  const script = document.createElement('script'); script.src = url;
  script.onerror = () => parent.postMessage({error: 'consumer failed'}, '*');
  script.onload = () => parent.postMessage({
    before, executions: globalThis.controlledPreloadExecutions,
    controlled: !!navigator.serviceWorker.controller,
    links: document.querySelectorAll('link').length,
    timings: performance.getEntriesByName(url).map(entry => entry.initiatorType)
  }, '*');
  document.head.appendChild(script);
}).observe({type: 'resource', buffered: true});
</script>"#;
        let network_script = matches!(response_kind, "fallback" | "forward");
        let server = tokio::spawn(async move {
            let mut paths = Vec::new();
            for _ in 0..if network_script { 4 } else { 3 } {
                let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
                let request = request_head(&mut stream).await?;
                let path = request.split_ascii_whitespace().nth(1).unwrap().to_owned();
                let (content_type, fields, body) = match path.as_str() {
                    "/sw.js" => ("text/javascript", "", worker.as_str()),
                    "/controlled/child" => ("text/html", "Link: </asset.js>;rel=preload;as=script;referrerpolicy=no-referrer\r\n", child),
                    "/seen?destination=script&referrer=&policy=no-referrer" => ("text/plain", "", "observed"),
                    "/asset.js" if network_script => {
                        let expected_destination = if response_kind == "fallback" { "script" } else { "empty" };
                        assert!(request.to_ascii_lowercase().contains(&format!("sec-fetch-dest: {expected_destination}\r\n")), "{request}");
                        assert!(!request.to_ascii_lowercase().contains("\r\nreferer:"), "{request}");
                        ("text/javascript", "", script)
                    }
                    _ => panic!("unexpected controlled preload request: {request}"),
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n{fields}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
                paths.push(path);
            }
            assert_eq!(paths.iter().filter(|path| path.as_str() == "/seen?destination=script&referrer=&policy=no-referrer").count(), 1);
            assert_eq!(paths.iter().filter(|path| path.as_str() == "/asset.js").count(), usize::from(network_script));
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent")).unwrap());
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
addEventListener('message', event => { globalThis.controlledPreloadResult = event.data; });
(async () => {
  const registration = await navigator.serviceWorker.register('/sw.js', {scope: '/controlled/'});
  const worker = registration.installing || registration.waiting || registration.active;
  if (worker.state !== 'activated') await new Promise(resolve => worker.addEventListener('statechange', () => {
    if (worker.state === 'activated') resolve();
  }));
  const frame = document.createElement('iframe'); frame.src = '/controlled/child';
  document.body.appendChild(frame);
})().catch(error => { globalThis.controlledPreloadResult = {error: String(error)}; });
"#)?;
            drive_page_work_until_done_with_explicit_producer_admission(
                &mut page_vm,
                "String(globalThis.controlledPreloadResult !== undefined)",
                "controlled header preload should complete before its consumer",
                |page_vm| {
                    page_vm.runtime_hooks.browser_context_runtime.drain_service_worker_service_lane();
                },
            ).await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(controlledPreloadResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({"before": 0, "executions": 1, "controlled": true, "links": 0, "timings": ["link"]}));
            assert_eq!(page_vm.vm_mut().eval("String(performance.getEntriesByType('resource').some(entry => entry.name.endsWith('/asset.js')))")?, "false");
            server.await??;
            anyhow::Ok(())
        }).await.expect("controlled response preload regression");
    }).await;
}

#[tokio::test]
async fn response_script_preload_service_worker_network_fallback_does_not_consume_itself() {
    assert_controlled_response_script_preload("fallback").await;
}

#[tokio::test]
async fn response_script_preload_service_worker_forwarding_is_consumed_once() {
    assert_controlled_response_script_preload("forward").await;
}

#[tokio::test]
async fn response_script_preload_service_worker_generated_response_is_consumed_once() {
    assert_controlled_response_script_preload("response").await;
}

#[tokio::test]
async fn response_script_preload_service_worker_stream_is_consumed_once() {
    assert_controlled_response_script_preload("stream").await;
}

#[tokio::test]
async fn intercepted_response_script_preload_rechecks_csp_before_replacement_contact() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/page")).unwrap());
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().set_fetch_subresource_interception(true, Some(SubresourceResourceType::Script));
            page_vm.vm_mut().eval(&format!(r#"
const policy = document.createElement('meta');
policy.httpEquiv = 'Content-Security-Policy'; policy.content = "script-src {base}/allowed";
document.head.appendChild(policy);
"#))?;
            let links = crate::response_links::DocumentResponseLinks::parse(&Url::parse(&base)?, &[("Link".into(), b"</allowed>;rel=preload;as=script".to_vec())]);
            page_vm.vm_mut().start_main_response_links(&links)?;
            let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
            assert_eq!(pending.len(), 1);
            page_vm.continue_pending_subresource_fetch(pending[0].internal_id, Some(Url::parse(&format!("{base}/blocked"))?), None, None, None, false, false)?;
            drain_page_work_until_no_pending_subresources(&mut page_vm, "intercepted preload CSP replacement").await?;
            assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
            anyhow::Ok(())
        }).await.expect("intercepted header preload CSP regression");
    }).await;
}

#[tokio::test]
async fn removing_child_cancels_response_script_preload_and_releases_waiting_consumer() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let source = b"globalThis.retiredPreloadExecuted = true;";
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut document, _) = listener.accept().await?;
            assert!(request_head(&mut document).await?.starts_with("GET /child "));
            document.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nLink: </asset.js>;rel=preload;as=script\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
            document.shutdown().await?;
            let (mut script, _) = listener.accept().await?;
            assert!(request_head(&mut script).await?.starts_with("GET /asset.js "));
            script.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", source.len()).as_bytes()).await?;
            script.write_all(&source[..11]).await?;
            let _ = started_tx.send(());
            let _ = release_rx.await;
            let _ = script.write_all(&source[11..]).await;
            let _ = script.shutdown().await;
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent")).unwrap());
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
globalThis.preloadFrame = document.createElement('iframe');
preloadFrame.id = 'preload-frame'; preloadFrame.src = '/child';
preloadFrame.onload = () => globalThis.preloadFrameReady = true;
document.body.appendChild(preloadFrame);
"#)?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.preloadFrameReady === true)", "header preload child navigation").await?;
            tokio::time::timeout(Duration::from_secs(5), started_rx).await??;
            let child = page_vm.vm().element_handle_by_id_for_test("preload-frame").unwrap();
            let loader = page_vm.vm_mut().with_default_context_scope_for_test(|_, host_ptr| {
                Ok(unsafe { &*host_ptr }.document_resource_loader_for_dispatch_scope(crate::native_bridge::OwnerDispatchScope::Child(child)).unwrap())
            })?;
            let request = moli_fetch::Request::get_with_url(Url::parse(&format!("{base}/asset.js"))?)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Script)
                .with_request_mode(moli_fetch::RequestMode::NoCors)
                .with_credentials_mode(moli_fetch::RequestCredentialsMode::Include);
            let consumer = loader.request_client().consume_document_preload(&request).expect("in-flight header preload must be available to its Document");
            assert_eq!(loader.diagnostics().active_ordinary_load_count, 1);
            page_vm.vm_mut().eval("preloadFrame.remove()")?;
            assert_ne!(loader.state(), crate::network::context::DocumentResourceLoaderState::Active);
            assert_eq!(loader.diagnostics().active_ordinary_load_count, 0);
            assert!(tokio::time::timeout(Duration::from_secs(5), consumer.response()).await?.is_err());
            assert_eq!(page_vm.vm_mut().eval("String(performance.getEntriesByType('resource').some(entry => entry.name.endsWith('/asset.js')))")?, "false");
            let _ = release_tx.send(());
            server.await??;
            anyhow::Ok(())
        }).await.expect("header preload Document retirement regression");
    }).await;
}
