use super::*;

#[tokio::test]
async fn worker_fetch_retains_native_request_urls_and_rejects_converted_credentials() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (stop_server, mut stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            requests.push(read_http_request_with_body(&mut socket).await.unwrap());
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
        }
        requests
    });
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let loader = ResourceRequestClient::new(&config).unwrap();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            let attempt = 0;
            addEventListener('fetch', event => event.respondWith((async () => {
                const original = event.request, url = original.url, variant = attempt++;
                Object.defineProperty(original, 'url', {get() { throw Error('public url getter'); }});
                Object.defineProperty(original, 'toString', {value() { throw Error('public toString'); }});
                const request = variant === 0 ? original : variant === 1 ? new Request(original) :
                    new Request(original, variant === 2 ? {} : {cache:'no-store'});
                const response = await fetch(request);
                const accepted = {status:response.status, text:await response.text(), used:request.bodyUsed};
                const refused = [];
                for (const input of [url, {url}]) {
                    try {
                        await fetch(input);
                        refused.push({allowed:true});
                    } catch (error) {
                        refused.push({name:error.name, message:error.message});
                    }
                }
                return new Response(JSON.stringify({accepted, refused}));
            })().catch(error => new Response(JSON.stringify({error:String(error)})))));
            "#
            .to_owned(),
            base.join("app/sw.js").unwrap().to_string(),
        )
        .with_request_client(loader)
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: base.join("app/").unwrap(),
        }),
    );
    let mut native_url = base.join("app/native-request").unwrap();
    native_url.set_username("alice").unwrap();
    native_url.set_password(Some("secret")).unwrap();
    for variant in 0..4 {
        let mut request = service_worker_fetch_request_for_test();
        request.url = native_url.clone();
        request.method = "POST".to_owned();
        request.body = Some(bytes::Bytes::from(format!("native request body {variant}")));
        request.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        let completion = dispatch_service_worker_fetch_event_with_request_for_test(
            &mut handle,
            943 + variant,
            request,
        )
        .await;
        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected Worker fetch URL observations for variant {variant}");
        };
        let observed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            observed,
            serde_json::json!({
                "accepted": {"status":200, "text":"ok", "used":true},
                "refused": vec![serde_json::json!({
                    "name":"TypeError",
                    "message":"Request URL must not include credentials",
                }); 2],
            }),
            "Worker fetch variant {variant}"
        );
    }
    handle.terminate_and_join();
    stop_server.send(()).unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 4);
    for (variant, request) in requests.iter().enumerate() {
        assert!(request.starts_with("POST /app/native-request HTTP/1.1\r\n"));
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        assert_eq!(body, format!("native request body {variant}"));
    }
}

#[tokio::test]
async fn native_request_url_copy_preserves_credentials_and_upload_body() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        let attempt = 0;
        addEventListener('fetch', event => event.respondWith((async () => {
            const original = event.request;
            const url = original.url;
            const variant = attempt++;
            Object.defineProperty(original, 'url', {get() { throw new Error('public url getter'); }});
            Object.defineProperty(original, 'toString', {value() { throw new Error('public toString'); }});
            try {
                const copy = variant === 0 ? new Request(original) :
                    new Request(original, variant === 1 ? {} : {cache:'no-store'});
                const body = await copy.text();
                copy.headers.set('accept', 'changed');
                const rejected = input => {
                    try { new Request(input); return 'allowed'; } catch (e) { return e.name; }
                };
                return new Response(JSON.stringify({
                    url:copy.url, body, used:copy.bodyUsed,
                    mode:copy.mode, credentials:copy.credentials, cache:copy.cache,
                    referrer:copy.referrer, signal:copy.signal instanceof AbortSignal,
                    headers:[original.headers.get('accept'), copy.headers.get('accept')],
                    rejected:[rejected(url), rejected({url})]
                }));
            } catch (e) {
                return new Response(JSON.stringify({error:e.name, message:e.message}));
            }
        })()));
        "#,
    );
    let url = "https://alice:secret@example.test/app/data.txt";
    for (id, changed) in [(940, false), (941, false), (942, true)] {
        let mut request = service_worker_fetch_request_for_test();
        request.url = url::Url::parse(url).unwrap();
        request.method = "POST".into();
        request.body = Some(bytes::Bytes::from_static(b"native request body"));
        request.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        request.metadata.referrer = "https://example.test/from".into();
        let completion =
            dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, id, request)
                .await;
        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected native Request copy observations");
        };
        let observed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            observed,
            serde_json::json!({
                "url":url, "body":"native request body", "used":true,
                "mode":"cors", "credentials":"include",
                "cache":if changed {"no-store"} else {"default"},
                "referrer":if changed {"about:client"} else {"https://example.test/from"},
                "signal":true, "headers":["text/plain","changed"],
                "rejected":["TypeError","TypeError"]
            })
        );
    }
    handle.terminate_and_join();
}
