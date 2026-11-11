use super::*;

async fn check_window_service_worker_response_policy(mode: &'static str) {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let cross_origin = format!("http://localhost:{}", address.port());
        let server_origin = origin.clone();
        let service_source = r#"
            const CROSS = __CROSS__;
            self.oninstall = event => event.waitUntil(self.skipWaiting());
            self.onactivate = event => event.waitUntil(self.clients.claim());
            self.onfetch = event => {
                const path = new URL(event.request.url).pathname;
                if (path === '/sw-basic') {
                    event.respondWith(new Response(new ReadableStream({start(controller) {
                        const chunk = new Uint8Array(65536).fill(120);
                        let remaining = 16;
                        function push() {
                            if (!remaining--) { controller.close(); return; }
                            controller.enqueue(chunk.slice()); setTimeout(push, 0);
                        }
                        push();
                    }}), {headers: {'Content-Type': 'text/plain', 'X-Visible': 'visible',
                        'X-Hidden': 'basic-visible', 'Set-Cookie': 'secret=value'}}));
                } else if (path === '/app/cors-proxy' || path === '/app/fetch-proxy') {
                    event.respondWith(fetch(CROSS + '/cors-source'));
                }
            };
        "#.replace("__CROSS__", &serde_json::to_string(&cross_origin).unwrap());
        let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                let expected = if mode == "enforce" { 3 } else { 4 };
                for index in 0..expected + 1 {
                    let wait = if index < expected { Duration::from_secs(30) } else { Duration::from_millis(500) };
                    let Ok(accepted) = tokio::time::timeout(wait, listener.accept()).await else { break };
                    let (mut stream, _) = accepted.unwrap();
                    let request = read_http_request_head(&mut stream).await.unwrap();
                    let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    requests.send(path.to_owned()).unwrap();
                    let body = if path == "/app/sw.js" { service_source.as_str() } else { "cors body" };
                    let content_type = if path == "/app/sw.js" { "text/javascript" } else { "text/plain" };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {server_origin}\r\nAccess-Control-Expose-Headers: X-Visible\r\nX-Visible: visible\r\nX-Hidden: network-hidden\r\nSet-Cookie: secret=value\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len(),
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    let middle = body.len() / 2;
                    stream.write_all(&body.as_bytes()[..middle]).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    stream.write_all(&body.as_bytes()[middle..]).await.unwrap();
                }
            });
        });
        let loader_owner = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        hooks.resource_task_runner = Some(loader_owner.browser_resource_runtime().resource_task_runner());
        let document_url = Url::parse(&format!("{origin}/app/page.html")).unwrap();
        let mut page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader_owner, Vec::new(), document_url.clone(), hooks,
        );
        page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
        if mode != "none" {
            page_vm.vm_mut().set_main_navigation_policy_container(
                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                    &[(if mode == "enforce" { "Content-Security-Policy" } else { "Content-Security-Policy-Report-Only" }.to_owned(), b"connect-src 'self'".to_vec())],
                    &document_url,
                ),
            );
        }
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            let source = r#"
                globalThis.windowPolicyResults = null;
                globalThis.windowPolicyError = null;
                (async () => {
                    await navigator.serviceWorker.register('/app/sw.js', {scope: '/app/'});
                    await navigator.serviceWorker.ready;
                    if (!navigator.serviceWorker.controller) await new Promise(resolve =>
                        navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
                    const violations = [];
                    document.addEventListener('securitypolicyviolation', event => violations.push({
                        directive: event.effectiveDirective, disposition: event.disposition, blocked: event.blockedURI
                    }));
                    function take(url) {
                        return new Promise(resolve => {
                            const xhr = new XMLHttpRequest(), snapshots = [];
                            let error = null;
                            xhr.open('GET', url, true);
                            xhr.onreadystatechange = () => {
                                if (xhr.readyState >= 2) snapshots.push({state: xhr.readyState,
                                    visible: xhr.getResponseHeader('X-Visible'), hidden: xhr.getResponseHeader('X-Hidden'),
                                    cookie: xhr.getResponseHeader('Set-Cookie'), all: xhr.getAllResponseHeaders()});
                            };
                            xhr.onerror = () => error = 'NetworkError';
                            xhr.onloadend = () => resolve({status: xhr.status, error, length: xhr.responseText.length, snapshots});
                            xhr.send();
                        });
                    }
                    const basic = await take(__CROSS__ + '/sw-basic');
                    const cors = await take('/app/cors-proxy');
                    let fetched;
                    try {
                        const response = await fetch('/app/fetch-proxy');
                        fetched = {status: response.status, body: await response.text(),
                            visible: response.headers.get('X-Visible'), hidden: response.headers.get('X-Hidden')};
                    } catch (error) { fetched = {error: error.name}; }
                    const fallback = await take(__CROSS__ + '/fallback');
                    await new Promise(resolve => setTimeout(resolve, 50));
                    windowPolicyResults = {basic, cors, fetched, fallback, violations};
                })().catch(error => windowPolicyError = String(error));
            "#.replace("__CROSS__", &serde_json::to_string(&cross_origin).unwrap());
            page_vm.vm_mut().eval(&source)?;
            drive_service_worker_page_vm_until_done(
                &mut page_vm, "String(windowPolicyResults !== null || windowPolicyError !== null)",
                "Window response policy and headers should retain their accepting Document and native filters",
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("String(windowPolicyError)")?, "null");
            let results: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval("JSON.stringify(windowPolicyResults)")?)?;
            for (name, hidden) in [("basic", "basic-visible"), ("cors", ""), ("fallback", "")] {
                let row = &results[name];
                if mode == "enforce" {
                    assert_eq!(row["status"], 0, "{name}: {row}");
                    assert_eq!(row["error"], "NetworkError");
                    assert_eq!(row["length"], 0);
                    assert!(row["snapshots"].as_array().unwrap().iter().all(|snapshot|
                        snapshot["state"] == 4 && snapshot["visible"].is_null() && snapshot["hidden"].is_null()));
                } else {
                    assert_eq!(row["status"], 200, "{name}: {row}");
                    assert!(row["error"].is_null());
                    assert_eq!(row["length"], if name == "basic" { 1024 * 1024 } else { 9 });
                    let snapshots = row["snapshots"].as_array().unwrap();
                    assert!(snapshots.iter().any(|snapshot| snapshot["state"] == 2));
                    assert!(snapshots.iter().any(|snapshot| snapshot["state"] == 4));
                    for snapshot in snapshots {
                        assert_eq!(snapshot["visible"], "visible", "{name}: {snapshot}");
                        assert!(snapshot["cookie"].is_null());
                        assert!(!snapshot["all"].as_str().unwrap().contains("set-cookie"));
                        if hidden.is_empty() {
                            assert!(snapshot["hidden"].is_null(), "{name}: {snapshot}");
                            assert!(!snapshot["all"].as_str().unwrap().contains("x-hidden"));
                        } else {
                            assert_eq!(snapshot["hidden"], hidden);
                        }
                    }
                }
            }
            if mode == "enforce" {
                assert_eq!(results["fetched"]["error"], "TypeError");
            } else {
                assert_eq!(results["fetched"]["status"], 200);
                assert_eq!(results["fetched"]["body"], "cors body");
                assert_eq!(results["fetched"]["visible"], "visible");
                assert!(results["fetched"]["hidden"].is_null());
            }
            let violations = results["violations"].as_array().unwrap();
            assert_eq!(violations.len(), match mode { "none" => 0, "report" => 5, _ => 4 }, "{mode}: {violations:?}");
            if mode != "none" {
                assert!(violations.iter().all(|v| v["directive"] == "connect-src" && v["disposition"] == if mode == "report" { "report" } else { "enforce" }));
                for url in [format!("{origin}/app/cors-proxy"), format!("{origin}/app/fetch-proxy"), format!("{cross_origin}/fallback")] {
                    assert_eq!(violations.iter().filter(|v| v["blocked"] == url).count(), 1);
                }
                assert_eq!(violations.iter().filter(|v| v["blocked"] == format!("{cross_origin}/sw-basic")).count(), if mode == "report" { 2 } else { 1 });
            }
            anyhow::Ok(())
        }).await.unwrap();
        server.join().unwrap();
        let mut paths = Vec::new();
        while let Some(path) = received.recv().await { paths.push(path); }
        assert_eq!(paths.iter().filter(|path| *path == "/cors-source").count(), 2);
        assert_eq!(paths.iter().filter(|path| *path == "/fallback").count(), usize::from(mode != "enforce"));
        assert!(!paths.iter().any(|path| path == "/sw-basic" || path == "/app/cors-proxy" || path == "/app/fetch-proxy"));
    }).await;
}

#[tokio::test]
async fn window_service_worker_response_preserves_xhr_filters_at_headers_and_done() {
    check_window_service_worker_response_policy("none").await;
}

#[tokio::test]
async fn window_service_worker_response_enforces_accepting_document_csp() {
    check_window_service_worker_response_policy("enforce").await;
}

#[tokio::test]
async fn window_service_worker_response_reports_initial_and_response_csp_without_network_duplicates()
 {
    check_window_service_worker_response_policy("report").await;
}
