use super::*;

const FONT: &[u8] = include_bytes!("../../../../../moli-layout/tests/fixtures/moli-ahem.ttf");

#[tokio::test]
async fn worker_font_loading_exposes_native_fonts_and_settles_tasks() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let source = format!(
            "{}\n{}workerFontsProbe({}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage({{error: String(error.stack || error)}}); close(); }});",
            include_str!("../../../../tests/fixtures/worker-fonts.js"),
            if kind == WorkerScriptKind::Module {
                "await "
            } else {
                ""
            },
            serde_json::to_string(FONT).unwrap(),
        );
        let mut handle = spawn_worker_with_request_client_and_kind(
            source,
            "https://fonts.test/worker.js".to_owned(),
            worker_test_request_client(),
            kind,
        );
        let result: serde_json::Value =
            serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
        assert_eq!(
            result["failures"],
            serde_json::json!([]),
            "{kind:?}: {result}"
        );
        assert_eq!(result["rows"].as_array().unwrap().len(), 33, "{result}");
    }
}

#[tokio::test]
async fn worker_font_loading_fetches_with_font_metadata_cors_and_worker_csp() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let cross = format!("http://localhost:{port}");
    let (stop_tx, mut stop_rx) = oneshot::channel();
    let redirect = format!("{cross}/blocked-final");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let accepted = tokio::select! {
                accepted = listener.accept() => accepted.unwrap(),
                _ = &mut stop_rx => break,
            };
            let mut socket = accepted.0;
            let request = read_http_request_head(&mut socket).await.unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            requests.push(request);
            let (status, extra, body) = match path.as_str() {
                "/bad" => ("200 OK", String::new(), b"\0\x01\0\0".as_slice()),
                "/missing" => ("404 Not Found", String::new(), FONT),
                "/redirect" => (
                    "302 Found",
                    format!("Location: {redirect}\r\n"),
                    b"".as_slice(),
                ),
                "/cors" => (
                    "200 OK",
                    "Access-Control-Allow-Origin: *\r\n".to_owned(),
                    FONT,
                ),
                _ => ("200 OK", String::new(), FONT),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: font/ttf\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        }
        requests
    });
    let probe = r#"
(async () => {
  const load = async (name, source) => new FontFace(name, source).load().then(() => 'loaded', e => e.name);
  const results = [];
  self.fetch = () => { throw new Error('author fetch'); };
  results.push(await load('Relative', 'url(../font.ttf)'));
  results.push(await load('Fallback', 'url(../bad), url(../fallback.ttf)'));
  results.push(await load('Missing', 'url(../missing)'));
  results.push(await load('Cors', 'url(CROSS/cors)'));
  results.push(await load('Denied', 'url(CROSS/denied)'));
  postMessage(results); close();
})().catch(e => {postMessage(String(e.stack || e)); close()});
"#.replace("CROSS", &cross);
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let loader = ResourceRequestClient::new(&config).unwrap();
    loader.set_optional_resource_fetch_enabled(SubresourceResourceType::Font, true);
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(probe, format!("{base}/folder/worker.js"))
            .with_request_client(loader)
            .with_content_security_policies(vec!["font-src *; connect-src 'none'".to_owned()]),
    );
    let result: serde_json::Value =
        serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
    assert_eq!(
        result,
        serde_json::json!(["loaded", "loaded", "NetworkError", "loaded", "NetworkError"])
    );

    let probe = r#"
(async () => {
  const violations = [];
  addEventListener('securitypolicyviolation', e => violations.push(e.effectiveDirective));
  const results = [];
  for (const source of ['url(CROSS/blocked)', 'url(../redirect)']) {
    results.push(await new FontFace('Blocked', source).load().then(() => 'loaded', e => e.name));
  }
  postMessage({results, violations}); close();
})();
"#
    .replace("CROSS", &cross);
    let loader = ResourceRequestClient::new(&config).unwrap();
    loader.set_optional_resource_fetch_enabled(SubresourceResourceType::Font, true);
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(probe, format!("{base}/folder/worker.js"))
            .with_request_client(loader)
            .with_content_security_policies(vec!["font-src 'self'; connect-src *".to_owned()]),
    );
    let result: serde_json::Value =
        serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
    assert_eq!(
        result,
        serde_json::json!({"results":["NetworkError","NetworkError"],"violations":["font-src","font-src"]})
    );
    let _ = stop_tx.send(());
    let requests = timeout(TIMEOUT, server).await.unwrap().unwrap();
    assert_eq!(requests.len(), 7, "{requests:?}");
    for request in requests {
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("sec-fetch-dest: font\r\n"), "{request}");
        assert!(lower.contains("sec-fetch-mode: cors\r\n"), "{request}");
        assert!(!lower.contains("get /blocked"), "{request}");
    }
}

#[tokio::test]
async fn worker_font_loading_uses_font_fetch_interception_and_validates_fulfilled_bytes() {
    ensure_v8();
    let loader = worker_test_request_client();
    loader.set_optional_resource_fetch_enabled(SubresourceResourceType::Font, true);
    let source = r#"
onmessage = async () => {
  const results = [];
  for (const name of ['Valid', 'Invalid']) {
    results.push(await new FontFace(name, 'url(' + name + '.ttf)').load().then(() => 'loaded', e => e.name));
  }
  postMessage(results); close();
};"#;
    let mut handle = spawn_worker_with_request_client(
        source.to_owned(),
        "https://fonts.test/worker.js".to_owned(),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Font));
    handle.post_message(serialize_test_string("go"));
    let mut requests = 0;
    let result = loop {
        match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
            WorkerToParentMessage::PendingSubresourceFetch(pending) => {
                requests += 1;
                assert!(requests <= 2);
                assert_eq!(pending.info.resource_type, SubresourceResourceType::Font);
                assert_eq!(pending.request_mode, moli_fetch::RequestMode::Cors);
                assert_eq!(
                    pending.credentials_mode,
                    moli_fetch::RequestCredentialsMode::SameOrigin
                );
                let request =
                    pending_worker_fetch_continue(pending.fetch_id, requests, &pending.info, false);
                let bytes = if requests == 1 {
                    FONT.to_vec()
                } else {
                    vec![0, 1, 0, 0]
                };
                handle.fulfill_pending_fetch(
                    request,
                    200,
                    Vec::new(),
                    RendererSyntheticResponseBody::from_bytes(bytes),
                );
            }
            WorkerToParentMessage::Post(payload) => break stringify_payload(&payload),
            WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::SubresourceContinue(_) => {}
            other => panic!("unexpected worker response: {other:?}"),
        }
    };
    assert_eq!(requests, 2);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!(["loaded", "NetworkError"])
    );
}
