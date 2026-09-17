use super::*;

#[tokio::test]
async fn worker_fetch_request_initializers_use_request_header_guards() {
    ensure_v8();
    let source = format!(
        "{}\nonmessage = async () => {{ const result = await fetchRequestGuardProbe('https://fetch-guard.test', false); postMessage(result); close(); }};",
        include_str!("../../../../tests/fixtures/fetch-request-guard.js"),
    );
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        source,
        "https://fetch-guard.test/worker.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader"),
        WorkerNetworkPolicy::default(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));
    let mut requests = 0;
    let result = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("worker response timeout")
            .expect("worker channel closed");
        match message {
            WorkerToParentMessage::PendingSubresourceFetch(pending) => {
                assert_eq!(pending.info.url.path(), "/echo");
                requests += 1;
                assert!(requests <= 33);
                let body = serde_json::json!({"headers": pending.info.request_headers}).to_string();
                let request =
                    pending_worker_fetch_continue(pending.fetch_id, requests, &pending.info, false);
                handle.fulfill_pending_fetch(
                    request,
                    200,
                    vec![("content-type".to_owned(), "application/json".to_owned())],
                    RendererSyntheticResponseBody::from_bytes(body.into_bytes()),
                );
            }
            WorkerToParentMessage::Post(payload) => break stringify_payload(&payload),
            WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::SubresourceContinue(_) => {}
            other => panic!("unexpected worker response: {other:?}"),
        }
    };
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(requests, 33);
    assert_eq!(result["state"], "pass", "{result}");
    assert_eq!(result["checks"].as_array().unwrap().len(), 1046);
}

#[tokio::test]
async fn worker_fetch_csp_violations_preserve_each_call_location() {
    ensure_v8();
    for enforce in [false, true] {
        let source = r#"
const locations = [];
addEventListener('securitypolicyviolation', e => {
  locations.push([e.sourceFile, e.lineNumber, e.columnNumber]);
  if (locations.length === 6) { postMessage(locations); close(); }
});
for (let i = 0; i < 5; i++) { fetch('data:text/plain,blocked').catch(() => {}); }
fetch('data:text/plain,blocked').catch(() => {});
"#;
        let policies = vec!["connect-src 'none'".to_owned()];
        let options = WorkerSpawnOptions::new(
            source.to_owned(),
            "https://app.test/fetch-worker.js?secret#fragment".into(),
        );
        let mut handle = spawn_test_worker_with_options(if enforce {
            options.with_content_security_policies(policies)
        } else {
            options.with_content_security_report_only_policies(policies)
        });
        let locations: Vec<(String, i32, i32)> =
            serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
        assert_eq!(locations.len(), 6);
        assert!(locations.iter().all(|(url, line, column)| url
            == "https://app.test/fetch-worker.js"
            && *line > 0
            && *column > 0));
        assert!(
            locations[..5]
                .iter()
                .all(|location| location == &locations[0])
        );
        assert_ne!(locations[0].1, locations[5].1);
    }
}

fn assert_initial_worker_auth_network_headers(headers: Option<&[(String, String)]>) {
    let headers = headers.expect("worker auth transport request headers");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("host") && !value.is_empty()),
        "worker auth transport headers should contain Host: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("authorization")),
        "the browser-visible auth request observation must remain the initial unauthenticated exchange: {headers:?}"
    );
}

mod fetch_network;
mod file_and_xhr;
mod opfs_storage;
mod websockets;
mod worker_globals;
mod worker_storage_security;
