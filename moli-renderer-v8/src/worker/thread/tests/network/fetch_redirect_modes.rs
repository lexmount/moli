use super::*;

#[tokio::test]
async fn dedicated_worker_fetch_redirect_modes_do_not_require_location() {
    check_worker_fetch_redirect_modes(false).await;
}

#[tokio::test]
async fn service_worker_fetch_redirect_modes_do_not_require_location() {
    check_worker_fetch_redirect_modes(true).await;
}

async fn check_worker_fetch_redirect_modes(service_worker: bool) {
    ensure_v8();
    let mut cases = Vec::new();
    for status in [301, 302, 303, 307, 308] {
        for mode in ["manual", "error"] {
            for location in [None, Some("http://[")] {
                cases.push((mode, status, location, true));
            }
        }
    }
    cases.extend([
        ("follow", 302, None, true),
        ("manual", 200, Some("http://["), true),
        ("error", 200, Some("http://["), true),
        ("manual", 302, None, false),
    ]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_cases = cases.clone();
    let server = tokio::spawn(async move {
        for (index, (_, status, location, cors_allowed)) in server_cases.into_iter().enumerate() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut socket).await.unwrap();
            assert!(
                request.starts_with(&format!("GET /{index} HTTP/1.1\r\n")),
                "{request}"
            );
            let mut headers = String::new();
            if cors_allowed {
                headers.push_str("Access-Control-Allow-Origin: *\r\n");
            }
            if let Some(location) = location {
                headers.push_str(&format!("Location: {location}\r\n"));
            }
            socket.write_all(format!(
                "HTTP/1.1 {status} Test\r\n{headers}Content-Length: 6\r\nConnection: close\r\n\r\nsecret"
            ).as_bytes()).await.unwrap();
        }
    });
    let modes: Vec<_> = cases.iter().map(|(mode, ..)| mode).collect();
    let probe = format!(
        r#"
        async function probe() {{
            const rows = [];
            const modes = {};
            for (let i = 0; i < modes.length; ++i) {{
                const url = {base:?} + '/' + i;
                try {{
                    const response = await fetch(url, {{redirect: modes[i]}});
                    const clone = response.clone();
                    rows.push({{
                        type: response.type, status: response.status,
                        statusText: response.statusText, redirected: response.redirected,
                        url: response.url, bodyIsNull: response.body === null,
                        headers: [...response.headers], body: await response.text(),
                        cloneType: clone.type, cloneBody: await clone.text()
                    }});
                }} catch (error) {{ rows.push({{error: error.name}}); }}
            }}
            return rows;
        }}
    "#,
        serde_json::to_string(&modes).unwrap()
    );
    let script = if service_worker {
        format!(
            "{probe}\nonfetch = event => event.respondWith(probe().then(rows => new Response(JSON.stringify(rows))));"
        )
    } else {
        format!("{probe}\nprobe().then(rows => {{postMessage(rows); close();}});")
    };
    let mut options =
        WorkerSpawnOptions::new(script, "http://worker.test/app/worker.js".to_owned());
    if service_worker {
        options = options.with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("http://worker.test/app/").unwrap(),
        });
    }
    let mut handle = spawn_test_worker_with_options(options);
    let rows: serde_json::Value = if service_worker {
        let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 1).await;
        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected probe response, got {:?}", completion.result);
        };
        serde_json::from_slice(&response.body).unwrap()
    } else {
        serde_json::from_str(&recv_post_json(&mut handle).await).unwrap()
    };
    timeout(TIMEOUT, server).await.unwrap().unwrap();
    assert_eq!(rows.as_array().unwrap().len(), cases.len());
    for (index, (mode, status, location, cors_allowed)) in cases.into_iter().enumerate() {
        let row = &rows[index];
        if !cors_allowed || (mode == "error" && status != 200) {
            assert_eq!(
                row,
                &serde_json::json!({"error":"TypeError"}),
                "{mode}/{status}/{location:?}"
            );
        } else if mode == "manual" && status != 200 {
            assert_eq!(
                row,
                &serde_json::json!({
                    "type":"opaqueredirect", "status":0, "statusText":"", "redirected":false,
                    "url":format!("{base}/{index}"), "bodyIsNull":true, "headers":[],
                    "body":"", "cloneType":"opaqueredirect", "cloneBody":""
                }),
                "{mode}/{status}/{location:?}"
            );
        } else {
            assert_eq!(row["type"], "cors");
            assert_eq!(row["status"], status);
            assert_eq!(row["body"], "secret");
            assert_eq!(row["cloneBody"], "secret");
            assert_eq!(row["url"], format!("{base}/{index}"));
            assert_eq!(row["redirected"], false);
        }
    }
    handle.terminate_and_join();
}
