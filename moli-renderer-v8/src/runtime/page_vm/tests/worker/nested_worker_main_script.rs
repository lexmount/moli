use super::*;

const CLIENT: &str = include_str!("../../../../../tests/fixtures/nested-worker-main-client.js");
const PARENT: &str = include_str!("../../../../../tests/fixtures/nested-worker-main-parent.js");
const SERVICE_WORKER: &str = include_str!("../../../../../tests/fixtures/nested-worker-main-sw.js");
const CHILD: &str = r#"postMessage({kind:'child',source:'network',scriptURL:location.href,gateReleased:typeof releasedByClient==='boolean'?releasedByClient:null});onmessage=e=>postMessage({kind:'echo',value:e.data.value,bytes:Array.from(new Uint8Array(e.data.bytes))});if(new URL(location.href).searchParams.get('route')==='rewritten')fetch('/child/probe').then(r=>r.json()).then(data=>postMessage({kind:'probe',data}));"#;

async fn check_nested_worker_main_script(mode: &'static str) {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let server_origin = origin.clone();
        let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
        let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                let gates = std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::<String, std::sync::Arc<tokio::sync::Notify>>::new()));
                let started = std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashSet::<String>::new()));
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    let accepted = tokio::select! {
                        result = listener.accept() => result.ok(),
                        _ = &mut stopped => None,
                        _ = tokio::time::sleep(Duration::from_secs(30)) => None,
                    };
                    let Some((mut stream, _)) = accepted else { break };
                    let server_origin = server_origin.clone();
                    let requests = requests.clone();
                    let gates = gates.clone();
                    let started = started.clone();
                    connections.spawn(async move {
                        let head = read_http_request_head(&mut stream).await.unwrap();
                        let path = head.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                        let url = Url::parse(&format!("{server_origin}{path}")).unwrap();
                        requests.send(path.to_owned()).unwrap();
                        let case = url.query_pairs().find(|(key, _)| key == "case").map(|(_, value)| value.into_owned()).unwrap_or_default();
                        let route = url.query_pairs().find(|(key, _)| key == "route").map(|(_, value)| value.into_owned()).unwrap_or_default();
                        let mut status = "200 OK";
                        let mut extra = String::new();
                        let mut content_type = "text/javascript";
                        let body = match url.path() {
                            "/sw.js" => { extra = "Service-Worker-Allowed: /\r\n".to_owned(); SERVICE_WORKER.to_owned() }
                            "/outer/dedicated.js" => format!("{PARENT}\nconnect(self);"),
                            "/outer/shared.js" => format!("{PARENT}\nonconnect=e=>connect(e.ports[0]);"),
                            "/child/script.js" => CHILD.to_owned(),
                            "/final/script.js" => CHILD.to_owned(),
                            "/redirect/script.js" => {
                                status = "302 Found";
                                extra = format!("Location: /final/script.js?{}\r\n", url.query().unwrap());
                                String::new()
                            }
                            "/slow/script.js" => {
                                let gate = gates.lock().entry(case.clone()).or_default().clone();
                                started.lock().insert(case);
                                let released = tokio::time::timeout(Duration::from_secs(10), gate.notified()).await.is_ok();
                                format!("const releasedByClient = {released};\n{CHILD}")
                            }
                            "/release" => { gates.lock().entry(case).or_default().notify_one(); "released".to_owned() }
                            "/started" => if started.lock().contains(&case) { "yes" } else { "no" }.to_owned(),
                            "/failure/script.js" => {
                                match route.as_str() {
                                    "status" => status = "503 Service Unavailable",
                                    "mime" => content_type = "text/plain",
                                    "syntax" => {},
                                    "redirect" => { status = "302 Found"; extra = format!("Location: http://localhost:{}/redirected.js\r\n", address.port()); }
                                    _ => unreachable!(),
                                }
                                if route == "syntax" { "function {".to_owned() } else { CHILD.to_owned() }
                            }
                            "/barrier" => "barrier".to_owned(),
                            other => panic!("unexpected request: {other}"),
                        };
                        let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{extra}\r\n{body}", body.len());
                        let _ = stream.write_all(response.as_bytes()).await;
                    });
                }
                connections.abort_all();
                while connections.join_next().await.is_some() {}
            });
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        hooks.resource_task_runner = Some(loader.browser_resource_runtime().resource_task_runner());
        let document_url = Url::parse(&format!("{origin}/app/page.html")).unwrap();
        let mut page = test_page_vm_with_loader_document_url_and_hooks(&loader, Vec::new(), document_url, hooks);
        page.retain_standalone_request_client_owner_for_test(loader);
        let local = page.local_executor.clone();
        local.run(async move {
            page.vm_mut().eval(&format!("const MODE = {};\n{CLIENT}", serde_json::to_string(mode).unwrap()))?;
                drive_service_worker_page_vm_until_done_with_poll_interval(&mut page, "String(nestedResult !== null || nestedError !== null)", "nested main script fetching must preserve asynchronous startup, native SW clients and cancellation", Duration::from_millis(20), |page| { page.runtime_hooks.browser_context_runtime.drain_shared_worker_service_lane(); }).await?;
            assert_eq!(page.vm_mut().eval("String(nestedError)")?, "null");
            let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(nestedResult)")?)?;
            assert_eq!(result["windowControlled"], false);
            let rows = result["rows"].as_array().unwrap();
            assert_eq!(rows.len(), if mode == "lifecycle" { 20 } else { 4 });
            for row in rows {
                let events = row["events"].as_array().unwrap();
                let route = row["route"].as_str().unwrap();
                let event = |kind| events.iter().find(|event| event["kind"] == kind);
                assert_eq!(event("transferred").unwrap()["byteLength"], 0);
                if mode == "lifecycle" {
                    assert!(event("child").is_none(), "{row}");
                    assert!(event("echo").is_none(), "{row}");
                    if route == "terminate" {
                        assert_eq!(row["releaseReason"], "terminated");
                        assert!(event("child-error").is_none(), "{row}");
                    } else { assert_eq!(event("child-error").unwrap()["eventType"], "error"); }
                    continue;
                }
                let child = event("child").unwrap();
                let echo = event("echo").unwrap();
                assert_eq!(echo["value"], "queued-before-load");
                assert_eq!(echo["bytes"], serde_json::json!([13, 21, 34]));
                if mode == "nonblocking" {
                    assert_eq!(row["releaseReason"], "pong", "{row}");
                    assert_eq!(child["gateReleased"], true, "{row}");
                    assert_eq!(child["source"], "network");
                    continue;
                }
                if matches!(mode, "redirect" | "rewritten") {
                    let final_url = format!("{origin}/final/script.js?case={}&route={mode}", row["id"].as_str().unwrap());
                    assert_eq!(child["scriptURL"], final_url);
                    let clients = result["clients"].as_array().unwrap();
                    let target = clients.iter().find(|client| client["url"] == final_url).unwrap();
                    assert_eq!(target["type"], "worker");
                    if mode == "rewritten" {
                        let requests = result["mainRequests"].as_array().unwrap();
                        let request_url = format!("{origin}/child/script.js?case={}&route=rewritten", row["id"].as_str().unwrap());
                        let main = requests.iter().find(|request| request["url"] == request_url).unwrap();
                        assert_eq!(main["resultingClientId"], target["id"]);
                        assert_eq!(event("probe").unwrap()["data"]["clientId"], target["id"]);
                        assert_eq!(event("probe").unwrap()["data"]["source"], "serviceworker");
                    }
                    continue;
                }
                assert_eq!(child["source"], "serviceworker", "{row}");
                let meta = &child["meta"];
                let clients = result["clients"].as_array().unwrap();
                let creator_url = format!("{origin}/outer/{}.js?case={}", row["parentKind"].as_str().unwrap(), row["id"].as_str().unwrap());
                let child_url = format!("{origin}/child/script.js?case={}&route=intercept", row["id"].as_str().unwrap());
                let creator = clients.iter().find(|client| client["url"] == creator_url).unwrap();
                let target = clients.iter().find(|client| client["url"] == child_url).unwrap();
                assert_eq!(meta["clientId"], creator["id"]);
                assert_eq!(meta["resultingClientId"], target["id"]);
                assert_ne!(creator["id"], target["id"]);
                assert_eq!(meta["destination"], "worker");
                assert_eq!(meta["mode"], "same-origin");
                assert_eq!(meta["credentials"], if row["type"] == "module" { "include" } else { "same-origin" });
                assert_eq!(meta["referrer"], creator_url);
            }
            anyhow::Ok(())
        }).await.unwrap();
        let _ = stop.send(());
        server.join().unwrap();
        let mut observed = Vec::new();
        while let Ok(request) = received.try_recv() { observed.push(request); }
        if mode == "intercept" {
            assert!(!observed.iter().any(|path| path.starts_with("/child/")), "{observed:?}");
        }
        assert!(!observed.iter().any(|path| path.starts_with("/redirected.js")), "{observed:?}");
    }).await;
}

#[tokio::test]
async fn nested_worker_main_script_interception_preserves_native_client_identity() {
    check_nested_worker_main_script("intercept").await;
}

#[tokio::test]
async fn nested_worker_constructor_does_not_block_creator_tasks() {
    check_nested_worker_main_script("nonblocking").await;
}

#[tokio::test]
async fn nested_worker_pending_main_load_termination_and_failures() {
    check_nested_worker_main_script("lifecycle").await;
}

#[tokio::test]
async fn nested_worker_main_script_redirect_preserves_client_url() {
    check_nested_worker_main_script("redirect").await;
}

#[tokio::test]
async fn nested_worker_main_response_url_preserves_client_id_and_controller() {
    check_nested_worker_main_script("rewritten").await;
}
