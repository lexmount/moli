use super::*;
use crate::cross_origin_isolation::{CrossOriginEmbedderPolicy, DocumentIsolationPolicy};
use crate::types::SubresourcePolicyContext;
use std::collections::BTreeSet;

async fn credentialless_fetch_server(
    listener: TcpListener,
    same_origin: String,
    cross_origin: String,
    requests: usize,
) -> Vec<String> {
    let mut observed = Vec::new();
    for _ in 0..requests {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request_head(&mut socket).await.unwrap();
        let path = request.split_whitespace().nth(1).unwrap();
        let location = match path {
            "/same-to-cross" => Some(format!("{cross_origin}/cross-final")),
            "/cross-to-same" => Some(format!("{same_origin}/same-final")),
            "/round-trip" => Some(format!("{cross_origin}/return")),
            "/return" => Some(format!("{same_origin}/round-final")),
            _ => None,
        };
        let cookie = path.trim_start_matches('/').replace('-', "_");
        let mut headers = format!(
            "HTTP/1.1 {}\r\nContent-Type: image/png\r\nCache-Control: no-store\r\nSet-Cookie: seen_{cookie}=1; Path=/; SameSite=Lax\r\nAccess-Control-Allow-Origin: {same_origin}\r\nAccess-Control-Allow-Credentials: true\r\nConnection: close\r\n",
            if location.is_some() {
                "302 Found"
            } else {
                "200 OK"
            },
        );
        if path != "/no-corp" {
            headers.push_str("Cross-Origin-Resource-Policy: cross-origin\r\n");
        }
        if let Some(location) = location {
            headers.push_str(&format!("Location: {location}\r\n"));
        }
        let body = b"\x89PNG\r\n\x1a\n";
        headers.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        socket.write_all(headers.as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        observed.push(request);
    }
    observed
}

async fn check_worker_credentialless_fetch(intercept: bool) {
    ensure_v8();
    let mut failures = Vec::new();
    for (name, coep, dip, credentialless) in [
        (
            "none",
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::None,
            false,
        ),
        (
            "require-corp",
            CrossOriginEmbedderPolicy::RequireCorp,
            DocumentIsolationPolicy::None,
            false,
        ),
        (
            "credentialless",
            CrossOriginEmbedderPolicy::Credentialless,
            DocumentIsolationPolicy::None,
            true,
        ),
        (
            "isolate-and-credentialless",
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::IsolateAndCredentialless,
            true,
        ),
    ] {
        let same = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cross = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let same_origin = format!("http://{}", same.local_addr().unwrap());
        let cross_origin = format!("http://{}", cross.local_addr().unwrap());
        let same_server = tokio::spawn(credentialless_fetch_server(
            same,
            same_origin.clone(),
            cross_origin.clone(),
            6,
        ));
        let cross_server = tokio::spawn(credentialless_fetch_server(
            cross,
            same_origin.clone(),
            cross_origin.clone(),
            7,
        ));
        let cookie_store = moli_cookie_jar::new_shared_browser_cookie_store();
        cookie_store
            .lock()
            .expect("worker test cookie store should be available")
            .set_document_cookie(
                &url::Url::parse(&same_origin).unwrap(),
                "seed=1; Path=/; SameSite=Lax",
            );
        let mut config = FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let source = format!(
            "{}\nonmessage = async () => {{ postMessage(await workerCredentiallessFetchProbe({same_origin:?}, {cross_origin:?})); close(); }};",
            include_str!("../../../../../tests/fixtures/worker-credentialless-fetch.js"),
        );
        let mut worker = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, format!("{same_origin}/worker.js"))
                .with_request_client(
                    ResourceRequestClient::new_with_cookie_store(&config, cookie_store.clone())
                        .unwrap(),
                )
                .with_policy_context(SubresourcePolicyContext {
                    cross_origin_embedder_policy: coep,
                    document_isolation_policy: dip,
                    // Network policy applies even without an isolated agent cluster.
                    cross_origin_isolated: false,
                }),
        );
        if intercept {
            worker.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
        }
        worker.post_message(serialize_test_string("go"));
        let payload = loop {
            let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
            match message {
                WorkerToParentMessage::PendingSubresourceFetch(pending) => {
                    worker.continue_pending_fetch(pending_worker_fetch_continue(
                        pending.fetch_id,
                        1,
                        &pending.info,
                        false,
                    ));
                }
                WorkerToParentMessage::Post(payload) => break stringify_payload(&payload),
                WorkerToParentMessage::SubresourceNetwork(_) => {}
                WorkerToParentMessage::SubresourceContinue(
                    PendingSubresourceContinueEvent::Completed { .. },
                ) => {}
                other => panic!("unexpected credentialless worker message: {other:?}"),
            }
        };
        worker.terminate_and_join();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["isolated"], false, "{name}");
        let results = payload["results"].as_array().unwrap();
        assert_eq!(results.len(), 9, "{name}: {payload}");
        for (index, result) in results.iter().enumerate() {
            assert_eq!(
                result["credentials"],
                if index < 7 { "include" } else { "omit" }
            );
            assert_eq!(
                result["mode"],
                if matches!(index, 2 | 8) {
                    "cors"
                } else {
                    "no-cors"
                }
            );
            let expected = if index == 6 && coep == CrossOriginEmbedderPolicy::RequireCorp {
                serde_json::json!({"error": "TypeError"})
            } else if matches!(index, 0 | 7) {
                serde_json::json!({"type": "basic", "status": 200})
            } else if matches!(index, 2 | 8) {
                serde_json::json!({"type": "cors", "status": 200})
            } else {
                serde_json::json!({"type": "opaque", "status": 0})
            };
            for (field, expected) in expected.as_object().unwrap() {
                if &result[field] != expected {
                    failures.push(format!("{name}, case {index}, {field}: {result}"));
                }
            }
        }
        let mut requests = timeout(TIMEOUT, same_server).await.unwrap().unwrap();
        requests.extend(timeout(TIMEOUT, cross_server).await.unwrap().unwrap());
        let mut expected_cookies = BTreeSet::from(["seed".to_owned()]);
        for request in &requests {
            let path = request.split_whitespace().nth(1).unwrap();
            let allowed = match path {
                "/same-omit" | "/cors-omit" => false,
                "/same" | "/cors" | "/same-to-cross" | "/round-trip" => true,
                _ => !credentialless,
            };
            let has_cookie = request.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("cookie")
                        && value.split(';').any(|cookie| cookie.trim() == "seed=1")
                })
            });
            if has_cookie != allowed {
                failures.push(format!(
                    "{name}, {path}: expected Cookie={allowed}, got {has_cookie}"
                ));
            }
            if allowed {
                expected_cookies.insert(format!("seen_{}", path[1..].replace('-', "_")));
            }
        }
        let actual_cookies = cookie_store
            .lock()
            .expect("worker test cookie store should remain available")
            .cookies()
            .into_iter()
            .map(|cookie| cookie.name)
            .collect::<BTreeSet<_>>();
        if actual_cookies != expected_cookies {
            failures.push(format!(
                "{name}: cookies {actual_cookies:?}, expected {expected_cookies:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn worker_fetch_credentialless_credentials_follow_redirect_taint() {
    check_worker_credentialless_fetch(false).await;
}

#[tokio::test]
async fn worker_fetch_credentialless_credentials_survive_interception() {
    check_worker_credentialless_fetch(true).await;
}
