use super::*;

const CLIENT: &str =
    include_str!("../../../../../tests/fixtures/window-request-referrer-client.js");
const SERVICE_WORKER: &str =
    include_str!("../../../../../tests/fixtures/window-request-referrer-sw.js");

async fn check_window_request_referrer(policy: &'static str) {
    for controlled in [false, true] {
        run_page_vm_async_test(async move {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let origin = format!("http://{address}");
            let cross = format!("http://localhost:{}", address.port());
            let server_origin = origin.clone();
            let server_cross = cross.clone();
            let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
            let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
            let server = std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                    let listener = TcpListener::from_std(listener).unwrap();
                    loop {
                        let accepted = tokio::select! {
                            result = listener.accept() => result.ok(),
                            _ = &mut stopped => None,
                            _ = tokio::time::sleep(Duration::from_secs(30)) => None,
                        };
                        let Some((mut stream, _)) = accepted else { break };
                        let head = read_http_request_head(&mut stream).await.unwrap();
                        let path = head.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                        let url = Url::parse(&format!("{server_origin}{path}")).unwrap();
                        let referrer = head.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("referer").then(|| value.trim().to_owned())
                        });
                        requests.send((path.to_owned(), referrer.clone())).unwrap();
                        let api = url.query_pairs().find(|(key, _)| key == "api").map(|(_, value)| value.into_owned());
                        let mut status = "200 OK";
                        let mut extra = String::new();
                        let (body, content_type) = if head.starts_with("OPTIONS ") {
                            (String::new(), "text/plain")
                        } else if url.path() == "/app/sw.js" {
                            (format!("const CROSS = {};\n{SERVICE_WORKER}", serde_json::to_string(&server_cross).unwrap()), "text/javascript")
                        } else if url.path() == "/app/network-redirect" {
                            status = "307 Temporary Redirect";
                            extra = format!("Location: {server_cross}/network?api={}\r\nReferrer-Policy: no-referrer\r\n", api.as_deref().unwrap());
                            (String::new(), "text/plain")
                        } else {
                            let value = serde_json::json!({"source":"network","referrer":referrer}).to_string();
                            if api.as_deref() == Some("eventsource") {
                                (format!("data: {value}\n\n"), "text/event-stream")
                            } else { (value, "application/json") }
                        };
                        let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {server_origin}\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: cache-control, last-event-id\r\n{extra}\r\n{body}", body.len());
                        stream.write_all(response.as_bytes()).await.unwrap();
                    }
                });
            });
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
            let document_url = Url::parse(&format!("{origin}/app/page.html?policy={policy}&marker=private#fragment")).unwrap();
            let mut page = test_page_vm_with_loader_document_url_and_hooks(&loader, Vec::new(), document_url.clone(), hooks);
            page.retain_standalone_request_client_owner_for_test(loader);
            let headers = if policy == "default" { Vec::new() } else {
                vec![("Referrer-Policy".to_owned(), if policy == "meta-no-referrer" { b"unsafe-url".to_vec() } else { policy.as_bytes().to_vec() })]
            };
            page.vm_mut().set_main_navigation_policy_container(
                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(&headers, &document_url),
            );
            let mut full_referrer = document_url.clone();
            full_referrer.set_fragment(None);
            let local = page.local_executor.clone();
            local.run(async move {
                let meta = if policy == "meta-no-referrer" { "document.head.appendChild(Object.assign(document.createElement('meta'), {name: 'referrer', content: 'no-referrer'}));" } else { "" };
                let source = format!("const ORIGIN = {}, CROSS = {}, CONTROLLED = {controlled};\ndocument.head.appendChild(Object.assign(document.createElement('base'), {{href: ORIGIN + '/unrelated-base/'}}));\n{meta}\n{CLIENT}", serde_json::to_string(&origin).unwrap(), serde_json::to_string(&cross).unwrap());
                page.vm_mut().eval(&source)?;
                drive_service_worker_page_vm_until_done(&mut page, "String(referrerResult !== null || referrerError !== null)", "Window XHR and EventSource must preserve the requesting Document referrer across SW dispatch and fallback").await?;
                assert_eq!(page.vm_mut().eval("String(referrerError)")?, "null");
                let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(referrerResult)")?)?;
                let rows = result["rows"].as_array().unwrap();
                assert_eq!(rows.len(), 27);
                let effective_policy = match policy { "default" => "strict-origin-when-cross-origin", "meta-no-referrer" => "no-referrer", other => other };
                for row in rows {
                    assert!(row.get("error").is_none(), "{policy}/{controlled}: {row}");
                    let route = row["route"].as_str().unwrap();
                    // Main's synchronous XHR sends directly to the network.
                    let uses_service_worker = controlled && row["kind"] != "syncxhr";
                    let synthetic = uses_service_worker && matches!(route, "synthetic-same" | "synthetic-cross");
                    let is_cross = route.ends_with("-cross") || (uses_service_worker && route == "synthetic-redirect");
                    let expected = if route == "network-policy-redirect" || effective_policy == "no-referrer" || (effective_policy == "same-origin" && is_cross) {
                        None
                    } else if effective_policy == "origin" || (effective_policy == "strict-origin-when-cross-origin" && is_cross) {
                        Some(format!("{origin}/"))
                    } else { Some(full_referrer.to_string()) };
                    let data = &row["data"];
                    if synthetic {
                        assert_eq!(data["source"], "serviceworker", "{policy}/{controlled}: {row}");
                        assert_eq!(data["referrer"], expected.as_deref().unwrap_or(""), "{policy}/{controlled}: {row}");
                        assert_eq!(data["referrerPolicy"], effective_policy);
                        assert_eq!(data["cache"], if row["kind"] == "eventsource" { "no-store" } else { "default" });
                    } else {
                        assert_eq!(data["source"], "network", "{policy}/{controlled}: {row}");
                        assert_eq!(data["referrer"], serde_json::to_value(expected)?, "{policy}/{controlled}: {row}");
                    }
                }
                anyhow::Ok(())
            }).await.unwrap();
            let _ = stop.send(());
            server.join().unwrap();
            let mut observed = Vec::new();
            while let Ok(request) = received.try_recv() { observed.push(request); }
            assert_eq!(observed.iter().filter(|(path, _)| path == "/app/sw.js").count(), usize::from(controlled));
            assert!(observed.iter().any(|(path, _)| path.starts_with("/app/network?api=xhr")));
            assert!(observed.iter().any(|(path, _)| path.starts_with("/app/network?api=eventsource")));
            if matches!(policy, "no-referrer" | "meta-no-referrer") {
                assert!(observed.iter().filter(|(path, _)| path != "/app/sw.js").all(|(_, referrer)| referrer.is_none()));
            }
        }).await;
    }
}

#[tokio::test]
async fn window_request_referrer_default_policy() {
    check_window_request_referrer("default").await;
}

#[tokio::test]
async fn window_request_referrer_no_referrer_policy() {
    check_window_request_referrer("no-referrer").await;
}

#[tokio::test]
async fn window_request_referrer_origin_policy() {
    check_window_request_referrer("origin").await;
}

#[tokio::test]
async fn window_request_referrer_same_origin_policy() {
    check_window_request_referrer("same-origin").await;
}

#[tokio::test]
async fn window_request_referrer_unsafe_url_policy() {
    check_window_request_referrer("unsafe-url").await;
}

#[tokio::test]
async fn window_request_referrer_meta_no_referrer_policy() {
    check_window_request_referrer("meta-no-referrer").await;
}
