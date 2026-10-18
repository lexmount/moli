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
async fn response_stylesheet_preload_is_consumed_in_its_child_document_without_applying_early() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            for expected in ["/pages/child", "/asset.css"] {
                let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
                let request = request_head(&mut stream).await?;
                assert!(request.starts_with(&format!("GET {expected} ")), "{request}");
                let (content_type, fields, body) = if expected == "/pages/child" {
                    ("text/html", "Link: <../asset.css#preloaded>;rel=preload;as=style;nonce=az;referrerpolicy=no-referrer\r\nContent-Security-Policy: script-src 'nonce-az';style-src 'nonce-az'\r\n", r#"<!doctype html><head><base href='/wrong/'></head><body><script nonce=az>
const url = new URL('/asset.css#preloaded', location).href;
let started = false;
new PerformanceObserver(list => {
  if (started || !list.getEntries().some(entry => entry.name === url)) return;
  started = true;
  const before = {links: document.querySelectorAll('link').length, sheets: document.styleSheets.length};
  const link = document.createElement('link');link.rel = 'stylesheet';link.href = url;link.nonce = 'az';
  link.onerror = () => parent.postMessage({error:'stylesheet consumer failed'}, '*');
  link.onload = () => parent.postMessage({before, links:document.querySelectorAll('link').length,
    sheets:document.styleSheets.length, color:link.sheet.cssRules[0].style.color,
    timings:performance.getEntriesByName(url).map(entry => entry.initiatorType)}, '*');
  document.head.appendChild(link);
}).observe({type:'resource',buffered:true});
</script>"#)
                } else {
                    assert!(request.to_ascii_lowercase().contains("sec-fetch-dest: style\r\n"), "{request}");
                    assert!(!request.to_ascii_lowercase().contains("\r\nreferer:"), "{request}");
                    ("text/css", "", "p { color: rgb(1, 2, 3); }")
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n{fields}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
            }
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent"))?);
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
addEventListener('message',event=>{globalThis.stylePreloadResult=event.data;});
const child=document.createElement('iframe');child.src='/pages/child';document.body.appendChild(child);
"#)?;
            drive_websocket_until_done(&mut page_vm,"String(globalThis.stylePreloadResult !== undefined)","response stylesheet preload consumption").await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(stylePreloadResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({"before":{"links":0,"sheets":0},"links":1,"sheets":1,"color":"rgb(1, 2, 3)","timings":["link"]}));
            assert_eq!(page_vm.vm_mut().eval("String(performance.getEntriesByType('resource').some(entry=>new URL(entry.name).pathname==='/asset.css'))")?, "false");
            server.await??;
            anyhow::Ok(())
        }).await.expect("response stylesheet preloads retain their Document owner");
        anyhow::Ok(())
    }).await.expect("response stylesheet preload fixture");
}

async fn assert_controlled_response_stylesheet_preload(response_kind: &'static str) {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let css = "p { color: rgb(1, 2, 3); --label: 水🌍; }";
        let quoted_css = serde_json::to_string(css)?;
        let response = match response_kind {
            "fallback" => String::new(),
            "stream" => format!(r#"event.respondWith(new Response(new ReadableStream({{start(controller) {{
    controller.enqueue(new TextEncoder().encode({quoted_css})); controller.close();
  }}}}), {{headers: {{'Content-Type': 'text/css'}}}}));"#),
            _ => unreachable!(),
        };
        let worker = format!(r#"
addEventListener('install', event => event.waitUntil(skipWaiting()));
addEventListener('activate', event => event.waitUntil(clients.claim()));
addEventListener('fetch', event => {{
  if (new URL(event.request.url).pathname !== '/asset.css') return;
  event.waitUntil(fetch('/seen?destination=' + event.request.destination + '&policy=' + event.request.referrerPolicy));
  {response}
}});
"#);
        let server = tokio::spawn(async move {
            for _ in 0..if response_kind == "fallback" { 4 } else { 3 } {
                let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
                let request = request_head(&mut stream).await?;
                let path = request.split_ascii_whitespace().nth(1).unwrap();
                let (kind, fields, body) = match path {
                    "/sw.js" => ("text/javascript", "", worker.as_str()),
                    "/controlled/child" => ("text/html", "Link: </asset.css>;rel=preload;as=style;referrerpolicy=no-referrer\r\n", r#"<!doctype html><script>
const url = new URL('/asset.css', location).href;
let started = false;
new PerformanceObserver(list => {
  if (started || !list.getEntries().some(entry => entry.name === url)) return;
  started = true;
  const before = {links: document.querySelectorAll('link').length, sheets: document.styleSheets.length};
  const link = document.createElement('link');link.rel = 'stylesheet';link.href = url;link.referrerPolicy = 'no-referrer';
  link.onerror = () => parent.postMessage({error:'streamed stylesheet consumer failed'}, '*');
  link.onload = () => parent.postMessage({before, controlled: !!navigator.serviceWorker.controller,
    color:link.sheet.cssRules[0].style.color, label:link.sheet.cssRules[0].style.getPropertyValue('--label').trim(),
    timings:performance.getEntriesByName(url).map(entry => entry.initiatorType)}, '*');
  document.head.appendChild(link);
}).observe({type:'resource',buffered:true});
</script>"#),
                    "/seen?destination=style&policy=no-referrer" => ("text/plain", "", "observed"),
                    "/asset.css" if response_kind == "fallback" => {
                        assert!(request.to_ascii_lowercase().contains("sec-fetch-dest: style\r\n"), "{request}");
                        assert!(!request.to_ascii_lowercase().contains("\r\nreferer:"), "{request}");
                        ("text/css", "", css)
                    }
                    _ => panic!("unexpected streamed stylesheet preload request: {request}"),
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nCache-Control: no-store\r\n{fields}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
            }
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent"))?);
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
addEventListener('message', event => {globalThis.streamedStylePreloadResult = event.data;});
(async () => {
  const registration = await navigator.serviceWorker.register('/sw.js', {scope:'/controlled/'});
  const worker = registration.installing || registration.waiting || registration.active;
  if (worker.state !== 'activated') await new Promise(resolve => worker.addEventListener('statechange', () => {if (worker.state === 'activated') resolve();}));
  const child = document.createElement('iframe');child.src = '/controlled/child';document.body.appendChild(child);
})().catch(error => {globalThis.streamedStylePreloadResult = {error:String(error)};});
"#)?;
            drive_page_work_until_done_with_explicit_producer_admission(
                &mut page_vm,
                "String(globalThis.streamedStylePreloadResult !== undefined)",
                "controlled response stylesheet preload consumption",
                |page_vm| {
                    page_vm.runtime_hooks.browser_context_runtime.drain_service_worker_service_lane();
                },
            ).await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(streamedStylePreloadResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({"before":{"links":0,"sheets":0},"controlled":true,"color":"rgb(1, 2, 3)","label":"水🌍","timings":["link"]}));
            server.await??;
            anyhow::Ok(())
        }).await.expect("streamed stylesheet preloads preserve their body and ownership");
        anyhow::Ok(())
    }).await.expect("streamed stylesheet preload fixture");
}

#[tokio::test]
async fn response_stylesheet_preload_preserves_streamed_service_worker_text() {
    assert_controlled_response_stylesheet_preload("stream").await;
}

#[tokio::test]
async fn response_stylesheet_preload_preserves_service_worker_network_fallback() {
    assert_controlled_response_stylesheet_preload("fallback").await;
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

#[tokio::test]
async fn response_modulepreload_is_imported_from_its_child_document_module_map() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for path in ["/pages/module-child", "/header-module.mjs"] {
                let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
                let head = request_head(&mut stream).await?;
                assert!(head.starts_with(&format!("GET {path} ")), "{head}");
                let (kind, fields, body) = if path == "/pages/module-child" {
                    ("text/html", "Link: <../header-module.mjs#preloaded>;rel=modulepreload;nonce=az\r\nContent-Security-Policy: script-src 'nonce-az'\r\n", r#"<!doctype html><head><base href='/wrong/'></head><body><script nonce=az>
const url = new URL('/header-module.mjs#preloaded', location).href;
let started = false;
new PerformanceObserver(list => {
  if (started || !list.getEntries().some(entry => entry.name === url)) return;
  started = true;
  const before = globalThis.headerModuleExecutions || 0;
  import(url).then(module => parent.postMessage({
    before, executions: globalThis.headerModuleExecutions,
    value: module.value, links: document.querySelectorAll('link').length,
    timings: performance.getEntriesByName(url).map(entry => entry.initiatorType)
  }, '*'));
}).observe({type: 'resource', buffered: true});
</script>"#)
                } else {
                    ("text/javascript", "", "globalThis.headerModuleExecutions = (globalThis.headerModuleExecutions || 0) + 1; export const value = 42;")
                };
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nCache-Control: no-store\r\n{fields}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
                stream.shutdown().await?;
            }
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent")).unwrap());
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
addEventListener('message', event => { globalThis.headerModuleResult = event.data; });
const frame = document.createElement('iframe'); frame.src = '/pages/module-child';
document.body.appendChild(frame);
"#)?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.headerModuleResult !== undefined)", "header modulepreload module-map consumption").await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(headerModuleResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({"before": 0, "executions": 1, "value": 42, "links": 0, "timings": ["script"]}));
            assert_eq!(page_vm.vm_mut().eval("String(performance.getEntriesByType('resource').some(entry => entry.name.includes('/header-module.mjs')))")?, "false");
            server.await??;
            anyhow::Ok(())
        }).await.expect("header modulepreload child regression");
    }).await;
}

async fn assert_controlled_response_module_preload(response_kind: &'static str) {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let script = "globalThis.controlledPreloadExecutions = (globalThis.controlledPreloadExecutions || 0) + 1; export const answer = 42;";
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
  import(url).then(module => parent.postMessage({
    before, executions: globalThis.controlledPreloadExecutions, answer: module.answer,
    controlled: !!navigator.serviceWorker.controller,
    links: document.querySelectorAll('link').length,
    timings: performance.getEntriesByName(url).map(entry => entry.initiatorType)
  }, '*')).catch(error => parent.postMessage({error: String(error)}, '*'));
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
                    "/controlled/child" => ("text/html", "Link: </asset.js>;rel=modulepreload;referrerpolicy=no-referrer\r\n", child),
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
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({"before": 0, "executions": 1, "controlled": true, "links": 0, "timings": ["script"], "answer": 42}));
            assert_eq!(page_vm.vm_mut().eval("String(performance.getEntriesByType('resource').some(entry => entry.name.endsWith('/asset.js')))")?, "false");
            server.await??;
            anyhow::Ok(())
        }).await.expect("controlled response modulepreload regression");
    }).await;
}

#[tokio::test]
async fn response_modulepreload_service_worker_network_fallback_uses_its_document_module_map() {
    assert_controlled_response_module_preload("fallback").await;
}

#[tokio::test]
async fn response_modulepreload_service_worker_forwarding_uses_its_document_module_map() {
    assert_controlled_response_module_preload("forward").await;
}

#[tokio::test]
async fn response_modulepreload_service_worker_generated_response_uses_its_document_module_map() {
    assert_controlled_response_module_preload("response").await;
}

#[tokio::test]
async fn response_modulepreload_service_worker_stream_uses_its_document_module_map() {
    assert_controlled_response_module_preload("stream").await;
}

#[tokio::test(flavor = "current_thread")]
async fn response_modulepreload_reports_csp_without_dispatching_into_replacement_realm() {
    run_page_vm_async_test(async move {
        for replace_realm in [false, true] {
            let (base, report_rx, report_server) = spawn_request_capture_http_server().await;
            let document_url = Url::parse(&format!("{base}/source.html"))?;
            let blocked_url = Url::parse(&format!("{base}/blocked.js"))?;
            let mut page_vm = test_page_vm_with_document_url(document_url);
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&[format!(
                    "script-src 'nonce-az' 'unsafe-eval'; report-uri {base}/csp-report"
                )]);
            page_vm.vm_mut().eval(
                "globalThis.modulepreloadCspEvents = 0; \
                 const frame = document.createElement('iframe'); \
                 frame.id = 'modulepreload-csp-realm'; document.body.appendChild(frame);",
            )?;
            let handle = page_vm
                .vm()
                .element_handle_by_id_for_test("modulepreload-csp-realm")
                .expect("child handle");
            materialize_child_realm_through_page_turn_for_test(
                &mut page_vm,
                "modulepreload-csp-realm",
            )?;
            let target = page_vm
                .vm()
                .current_child_document_module_fetch_target(handle)
                .expect("source child realm");
            let context = page_vm
                .vm_mut()
                .capture_module_fetch_context(
                    crate::native_bridge::OwnerDispatchScope::Child(handle),
                    crate::module_runtime::ModuleKind::JavaScript,
                    &crate::module_runtime::ModuleFetchMetadata::default(),
                )
                .map_err(anyhow::Error::msg)?;
            let error = context
                .check_initial(&blocked_url)
                .expect_err("missing preload nonce must be blocked");
            if replace_realm {
                page_vm.vm_mut().retire_child_frame_realm_for_test(handle);
                materialize_child_realm_through_page_turn_for_test(
                    &mut page_vm,
                    "modulepreload-csp-realm",
                )?;
                let replacement = page_vm
                    .vm()
                    .current_child_document_module_fetch_target(handle)
                    .expect("replacement child realm");
                assert_eq!(target.task_owner(), replacement.task_owner());
                assert_ne!(target.realm_id(), replacement.realm_id());
            }
            page_vm.vm_mut().eval(
                "document.getElementById('modulepreload-csp-realm').contentDocument \
                 .addEventListener('securitypolicyviolation', () => modulepreloadCspEvents++);",
            )?;
            let root = page_vm.document_lifecycle.identity().document;
            let completion = test_child_modulepreload_completion_for_target(
                target,
                701,
                "modulepreload-csp-realm",
                Some(&error),
            )
            .with_fetch_context(Some(context));
            let mut source = RendererPageNetworkingSource::new_for_test();
            source.enqueue_local_for_test(
                RendererPageResourceCompletion::child_modulepreload_fetch(root, completion),
            );
            let outcome = page_vm
                .apply_one_page_resource_terminal_owner_admission_for_test(&mut source)?
                .expect("one modulepreload completion");
            assert_eq!(
                matches!(
                    outcome.action.document_effect,
                    PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
                ),
                replace_realm
            );
            let events = page_vm
                .vm_mut()
                .drain_pre_domcontentloaded_content_security_policy_violation_tasks_for_test();
            assert_eq!(events, usize::from(!replace_realm));
            assert_eq!(
                page_vm.vm_mut().eval("String(modulepreloadCspEvents)")?,
                if replace_realm { "0" } else { "1" }
            );
            let report = tokio::time::timeout(Duration::from_secs(5), report_rx).await??;
            assert!(report.starts_with("POST /csp-report HTTP/1.1"), "{report}");
            let body = report.split_once("\r\n\r\n").expect("report body").1;
            let body: serde_json::Value = serde_json::from_str(body)?;
            assert_eq!(body["csp-report"]["blocked-uri"], blocked_url.as_str());
            assert_eq!(body["csp-report"]["effective-directive"], "script-src-elem");
            report_server.await?;
        }
        anyhow::Ok(())
    })
    .await
    .expect("modulepreload CSP source realm regression");
}

#[tokio::test(flavor = "current_thread")]
async fn response_modulepreload_redirect_guard_does_not_retain_its_document_fetch_context() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm_with_document_url(Url::parse(
            "https://modulepreload-redirect-context.test/page.html",
        )?);
        page_vm
            .vm_mut()
            .set_response_content_security_policies(&["script-src 'self'".to_owned()]);
        let context = page_vm
            .vm_mut()
            .capture_module_fetch_context(
                crate::native_bridge::OwnerDispatchScope::Top,
                crate::module_runtime::ModuleKind::JavaScript,
                &crate::module_runtime::ModuleFetchMetadata::default(),
            )
            .map_err(anyhow::Error::msg)?;
        let source = Url::parse("https://modulepreload-redirect-context.test/module.js")?;
        let guard = context
            .redirect_check(&source)
            .expect("captured CSP requires a redirect guard");
        let weak = Arc::downgrade(&context);
        drop(context);
        assert!(
            weak.upgrade().is_none(),
            "a retained SW redirect guard must not keep the Document/browser fetch context alive"
        );
        let request = moli_fetch::Request::get(source.as_str())?.with_redirect_check(guard);
        assert!(
            request
                .check_redirect_target(&Url::parse(
                    "https://modulepreload-redirect-context.test/allowed.js"
                )?)
                .is_ok()
        );
        assert!(
            request
                .check_redirect_target(&Url::parse("https://other.test/blocked.js")?)
                .is_err()
        );
        anyhow::Ok(())
    })
    .await
    .expect("modulepreload redirect guard ownership regression");
}
