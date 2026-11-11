use super::*;

const CLIENT: &str = include_str!("../../../../../tests/fixtures/worker-main-redirect-client.js");
const PARENT: &str = include_str!("../../../../../tests/fixtures/worker-main-redirect-parent.js");
const SERVICE_WORKER: &str =
    include_str!("../../../../../tests/fixtures/worker-main-redirect-sw.js");
const CONNECT: &str = "function connect(port){port.postMessage({kind:'child',source:SOURCE,meta:META});port.onmessage=e=>port.postMessage({kind:'echo',payload:e.data});if(port.start)port.start();}";

async fn check_worker_main_redirect(mode: &'static str) {
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
                    connections.spawn(async move {
                        let head = read_http_request_head(&mut stream).await.unwrap();
                        let path = head.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                        let url = Url::parse(&format!("{server_origin}{path}")).unwrap();
                        requests.send(path.to_owned()).unwrap();
                        let mut status = "200 OK";
                        let mut extra = String::new();
                        let body = match url.path() {
                            "/sw.js" => {
                                extra = "Service-Worker-Allowed: /\r\n".to_owned();
                                format!("const CONNECT = {};\n{SERVICE_WORKER}", serde_json::to_string(CONNECT).unwrap())
                            }
                            "/outer/dedicated.js" => format!("{PARENT}\nsetup(self);"),
                            "/outer/shared.js" => format!("{PARENT}\nonconnect=e=>setup(e.ports[0]);"),
                            "/start/one.js" | "/start/two.js" | "/middle/two.js" | "/start/policy.js" | "/start/body.js" => {
                                status = "302 Found";
                                let next = if url.path() == "/start/two.js" { "/middle/two.js" } else { "/child/child.js" };
                                extra = format!("Location: {next}?{}\r\n", url.query().unwrap());
                                if url.path() == "/start/policy.js" { extra.push_str("Referrer-Policy: no-referrer\r\n"); }
                                String::new()
                            }
                            "/child/child.js" | "/other/child.js" | "/outside/child.js" => {
                                let shared = url.query_pairs().any(|(key, value)| key == "shared" && value == "1");
                                format!("const SOURCE='network',META=null;{CONNECT}{}", if shared { "onconnect=e=>connect(e.ports[0]);" } else { "connect(self);" })
                            }
                            other => panic!("unexpected request: {other}"),
                        };
                        // A redirect must not wait for an unused response body.
                        let held = url.path() == "/start/body.js";
                        let length = if held { 100_000 } else { body.len() };
                        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/javascript\r\nContent-Length: {length}\r\nConnection: close\r\nCache-Control: no-store\r\n{extra}\r\n{body}");
                        let _ = stream.write_all(response.as_bytes()).await;
                        if held { std::future::pending::<()>().await; }
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
            drive_service_worker_page_vm_until_done_with_poll_interval(&mut page, "String(redirectResult !== null || redirectError !== null)", "worker main redirects must reselect the controller with the same reserved client", Duration::from_millis(20), |page| { page.runtime_hooks.browser_context_runtime.drain_shared_worker_service_lane(); }).await?;
            assert_eq!(page.vm_mut().eval("String(redirectError)")?, "null");
            let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(redirectResult)")?)?;
            assert_eq!(result["windowControlled"], false);
            let rows = result["rows"].as_array().unwrap();
            assert_eq!(rows.len(), if mode == "network" { 40 } else { 16 });
            let clients = result["clients"].as_array().unwrap();
            let intercepted = result["requests"].as_array().unwrap();
            for row in rows {
                let id = row["id"].as_str().unwrap();
                let route = row["route"].as_str().unwrap();
                let kind = row["parentKind"].as_str().unwrap();
                let events = row["events"].as_array().unwrap();
                let child = events.iter().find(|event| event["kind"] == "child").unwrap();
                let echo = events.iter().find(|event| event["kind"] == "echo").unwrap();
                assert_eq!(echo["payload"], "queued-before-load", "{row}");
                let window_creator = kind.starts_with("window-");
                let creator_url = if window_creator { format!("{origin}/app/page.html") } else { format!("{origin}/outer/{kind}.js?case={id}") };
                let directory = match route { "exit" => "outside", "scope" => "other", _ => "child" };
                let query = format!("case={id}&route={route}&shared={}", if kind == "window-shared" { 1 } else { 0 });
                let target_url = format!("{origin}/{directory}/child.js?{query}");
                let creator = clients.iter().find(|client| client["url"] == creator_url).unwrap();
                let target = clients.iter().find(|client| client["url"] == target_url).unwrap();
                assert_ne!(creator["id"], target["id"]);
                assert_eq!(target["type"], if kind == "window-shared" { "sharedworker" } else { "worker" });
                assert_eq!(child["source"], if route == "exit" { "network" } else { "serviceworker" }, "{row}");
                if route != "exit" {
                    let meta = &child["meta"];
                    assert_eq!(meta["clientId"], creator["id"], "{row}");
                    assert_eq!(meta["resultingClientId"], target["id"], "{row}");
                    assert_eq!(meta["destination"], if kind == "window-shared" { "sharedworker" } else { "worker" });
                    assert_eq!(meta["mode"], "same-origin");
                    assert_eq!(meta["credentials"], if row["type"] == "module" { "include" } else { "same-origin" });
                    assert_eq!(meta["url"], target_url);
                    assert_eq!(meta["referrer"], if route == "policy" { "" } else { &creator_url });
                }
                if matches!(route, "exit" | "scope") {
                    let initial_url = format!("{origin}/child/redirect.js?{query}");
                    let initial = intercepted.iter().find(|request| request["url"] == initial_url).unwrap();
                    assert_eq!(initial["clientId"], creator["id"]);
                    assert_eq!(initial["resultingClientId"], target["id"]);
                }
            }
            anyhow::Ok(())
        }).await.unwrap();
        let _ = stop.send(());
        server.join().unwrap();
        let mut observed = Vec::new();
        while let Ok(request) = received.try_recv() { observed.push(request); }
        assert!(!observed.iter().any(|path| path.starts_with("/child/") || path.starts_with("/other/")), "{observed:?}");
        assert_eq!(observed.iter().filter(|path| path.starts_with("/outside/")).count(), if mode == "network" { 0 } else { 8 });
    }).await;
}

#[tokio::test]
async fn worker_main_network_redirect_reselects_controller_and_preserves_native_clients() {
    check_worker_main_redirect("network").await;
}

#[tokio::test]
async fn worker_main_service_worker_redirect_leaves_and_changes_scope() {
    check_worker_main_redirect("serviceworker").await;
}
