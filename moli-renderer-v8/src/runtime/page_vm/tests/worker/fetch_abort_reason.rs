use super::*;

const COMMON: &str = include_str!("../../../../../tests/fixtures/fetch-abort-reason-common.js");
const CLIENT: &str = include_str!("../../../../../tests/fixtures/fetch-abort-reason-client.js");
const SERVICE_WORKER: &str = include_str!("../../../../../tests/fixtures/fetch-abort-reason-sw.js");
const ROUTING_CLIENT: &str =
    include_str!("../../../../../tests/fixtures/fetch-abort-routing-client.js");
const WORKER: &str = "function connect(port){port.onmessage=e=>runAbort(e.data.kind,e.data.id).then(row=>port.postMessage(row),error=>port.postMessage({error:String(error)}));if(port.start)port.start();}";

async fn check_fetch_abort_reason(routing: bool) {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server_origin = origin.clone();
        let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
        let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                let states = std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::<String, serde_json::Value>::new()));
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
                    let states = states.clone();
                    connections.spawn(async move {
                        let request = read_http_request_with_body(&mut stream).await.unwrap();
                        let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                        let url = Url::parse(&format!("{server_origin}{path}")).unwrap();
                        requests.send(path.to_owned()).unwrap();
                        let case = url.query_pairs().find(|(key, _)| key == "case").map(|(_, value)| value.into_owned()).unwrap_or_default();
                        let mut content_type = "text/javascript";
                        let mut extra = "";
                        let body = match url.path() {
                            "/sw.js" => { extra = "Service-Worker-Allowed: /\r\n"; format!("{COMMON}\n{SERVICE_WORKER}") }
                            "/controlled/dedicated.js" | "/controlled/shared.js" => {
                                let common = if url.query_pairs().any(|(key, _)| key == "instance") {
                                    COMMON.replace("  let thrown = null;", "  while (!(await (await fetch('/state?case=' + id)).json()).release) await new Promise(resolve => setTimeout(resolve, 20));\n  let thrown = null;")
                                } else { COMMON.to_owned() };
                                format!("{common}\n{WORKER}\n{}", if url.path().ends_with("/shared.js") { "onconnect=e=>connect(e.ports[0]);" } else { "connect(self);" })
                            }
                            "/release" => {
                                states.lock().entry(case).or_insert_with(|| serde_json::json!({"started":false,"report":null}))["release"] = serde_json::json!(true);
                                "released".to_owned()
                            }
                            "/started" => {
                                states.lock().entry(case).or_insert_with(|| serde_json::json!({"started":false,"report":null}))["started"] = serde_json::json!(true);
                                "started".to_owned()
                            }
                            "/report" => {
                                let body = request.split_once("\r\n\r\n").unwrap().1;
                                states.lock().entry(case).or_insert_with(|| serde_json::json!({"started":false,"report":null}))["report"] = serde_json::from_str(body).unwrap();
                                "reported".to_owned()
                            }
                            "/state" => {
                                content_type = "application/json";
                                serde_json::to_string(&states.lock().get(&case).cloned().unwrap_or_else(|| serde_json::json!({"started":false,"report":null}))).unwrap()
                            }
                            "/controlled/data" => "network-fallback".to_owned(),
                            other => panic!("unexpected request: {other}"),
                        };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{extra}\r\n{body}", body.len());
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
        let document_url = Url::parse(&format!("{origin}/controlled/index.html")).unwrap();
        let mut page = test_page_vm_with_loader_document_url_and_hooks(&loader, Vec::new(), document_url, hooks);
        page.retain_standalone_request_client_owner_for_test(loader);
        let local = page.local_executor.clone();
        local.run(async move {
            page.vm_mut().eval(&format!("{COMMON}\n{}", if routing { ROUTING_CLIENT } else { CLIENT }))?;
            drive_service_worker_page_vm_until_done_with_poll_interval(&mut page, "String(abortError !== null || globalThis.abortReady === true)", "Fetch abort reasons must be serialized at abort and delivered to the real SW request signal", Duration::from_millis(20), |page| { page.runtime_hooks.browser_context_runtime.drain_shared_worker_service_lane(); }).await?;
            assert_eq!(page.vm_mut().eval("String(abortError)")?, "null");
            let rows: Vec<serde_json::Value> = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(abortRows)")?)?;
            if routing {
                assert_eq!(rows.len(), 16);
                for row in rows {
                    assert_eq!(row["matched"], true, "{row}");
                    let reports = row["reports"].as_array().unwrap();
                    assert_eq!(reports.len(), 1);
                    assert_eq!(reports[0]["id"], row["target"]);
                    let report = &reports[0]["report"];
                    assert_eq!(report["reason"]["primitive"], "caller-reason");
                    assert_eq!(report["clientId"], report["client"]["id"]);
                    let (creator, instance) = row["target"].as_str().unwrap().rsplit_once('-').unwrap();
                    assert_eq!(report["client"]["url"], format!("{origin}/controlled/{creator}.js?instance={instance}"));
                }
                return anyhow::Ok(());
            }
            assert_eq!(rows.len(), 33);
            for row in rows {
                let kind = row["kind"].as_str().unwrap();
                let creator = row["creator"].as_str().unwrap();
                assert_eq!(row["thrown"], serde_json::Value::Null, "{row}");
                assert_eq!(row["local"]["same"], true, "{row}");
                assert_eq!(row["serializationGetsOnAbort"], if matches!(kind, "getter" | "throwing") { 1 } else { 0 }, "{row}");
                let report = &row["observed"];
                assert_eq!(report["aborted"], true);
                assert_eq!(report["clientId"], report["client"]["id"]);
                assert!(!report["clientId"].as_str().unwrap().is_empty());
                assert_eq!(report["client"]["url"], format!("{origin}/controlled/{}", if creator == "window" { "index.html".to_owned() } else { format!("{creator}.js") }));
                assert_eq!(report["client"]["type"], match creator { "window" => "window", "dedicated" => "worker", "shared" => "sharedworker", _ => unreachable!() });
                let reason = &report["reason"];
                if matches!(kind, "default" | "function" | "throwing" | "domexception") {
                    assert_eq!(reason["isDOMException"], true, "{row}");
                    assert_eq!(reason["isError"], true, "{row}");
                    assert_eq!(reason["name"], if kind == "domexception" { "TimeoutError" } else { "AbortError" });
                    if kind == "domexception" { assert_eq!(reason["message"], "caller-message"); }
                } else if matches!(kind, "error" | "typeerror" | "custom") {
                    assert_eq!(reason["isError"], true, "{row}");
                    assert_eq!(reason["isDOMException"], false);
                    assert_eq!(reason["isTypeError"], kind == "typeerror");
                    assert_eq!(reason["name"], if kind == "typeerror" { "TypeError" } else { "Error" });
                    assert_eq!(reason["message"], "original-message");
                } else {
                    assert_eq!(reason["isError"], false, "{row}");
                    match kind {
                        "null" => assert_eq!(reason["tag"], "[object Null]"),
                        "string" => assert_eq!(reason["primitive"], "caller-reason"),
                        "object" => { assert_eq!(reason["marker"], "before"); assert_eq!(reason["nested"], 21); assert_eq!(reason["self"], true); }
                        "getter" => assert_eq!(reason["marker"], "before"),
                        _ => unreachable!(),
                    }
                }
            }
            anyhow::Ok(())
        }).await.unwrap();
        let _ = stop.send(());
        server.join().unwrap();
        let mut observed = Vec::new();
        while let Ok(request) = received.try_recv() { observed.push(request); }
        assert!(!observed.iter().any(|path| path.starts_with("/controlled/data")), "{observed:?}");
    }).await;
}

#[tokio::test]
async fn fetch_abort_reason_reaches_service_worker_from_each_client_realm() {
    check_fetch_abort_reason(false).await;
}

#[tokio::test]
async fn aborting_concurrent_workers_cancels_only_the_actual_fetch() {
    check_fetch_abort_reason(true).await;
}
