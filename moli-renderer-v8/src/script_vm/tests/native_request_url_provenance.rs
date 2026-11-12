use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerClientId, ServiceWorkerFetchRequest, ServiceWorkerRequestDestination,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "current_thread")]
async fn cache_keys_preserves_native_credential_urls_without_author_constructors() {
    let loader = static_http_loader([]);
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://cache-keys.test/", &loader);
    let urls = [
        "https://aa:bb@cache-keys.test/file.txt",
        "https://cc:dd@cache-keys.test/file.txt",
        "https://cache-keys.test/file.txt",
    ];
    vm.with_default_context_scope(|scope, _| {
        let requests = v8::Array::new(scope, urls.len() as i32);
        for (index, url) in urls.iter().enumerate() {
            let source = ServiceWorkerFetchRequest {
                client_id: ServiceWorkerClientId::from_u64_for_test(7),
                resulting_client_id: None,
                url: Url::parse(url).unwrap(),
                method: "GET".to_owned(),
                headers: vec![("X-Cache-Key".to_owned(), index.to_string())],
                body: None,
                destination: ServiceWorkerRequestDestination::Empty,
                request_mode: moli_fetch::RequestMode::Cors,
                credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                priority: None,
                is_reload: false,
                metadata: Default::default(),
            };
            let (request, _) =
                crate::network_host::build_service_worker_request_object(scope, &source).unwrap();
            assert_eq!(
                requests.set_index(scope, index as u32, request.into()),
                Some(true)
            );
        }
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeCacheRequests").unwrap();
        assert_eq!(global.set(scope, key.into(), requests.into()), Some(true));
        Ok(())
    })
    .unwrap();
    vm.exec(
        r#"
        globalThis.cacheKeysResult = null;
        (async () => {
          const NativeRequest = Request;
          let rejected = false;
          try { new NativeRequest(nativeCacheRequests[0].url); }
          catch (error) { rejected = error instanceof TypeError; }
          const cache = await caches.open('native-keys');
          for (const request of nativeCacheRequests) {
            await cache.put(request, new Response('body'));
          }
          globalThis.Request = function() { throw Error('author constructor'); };
          try {
            const keys = await cache.keys();
            return rejected && keys.length === nativeCacheRequests.length &&
              keys.every((request, index) => request instanceof NativeRequest &&
                request.url === nativeCacheRequests[index].url &&
                request.clone().url === request.url && request.method === 'GET' &&
                request.body === null && request.signal instanceof AbortSignal &&
                request.headers.get('X-Cache-Key') === String(index));
          } finally { globalThis.Request = NativeRequest; }
        })().then(value => cacheKeysResult = String(value),
                  error => cacheKeysResult = String(error.stack || error));
        "#,
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "cacheKeysResult",
        "true",
        "cached native Request keys must retain their URL identity",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn window_fetch_retains_native_request_urls_and_rejects_converted_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (stop_server, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            requests.push(String::from_utf8(head).unwrap());
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
        }
        requests
    });
    let loader = static_http_loader([]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    let mut native_url = base.join("native-request").unwrap();
    native_url.set_username("alice").unwrap();
    native_url.set_password(Some("secret")).unwrap();
    let request = ServiceWorkerFetchRequest {
        client_id: ServiceWorkerClientId::from_u64_for_test(7),
        resulting_client_id: None,
        url: native_url.clone(),
        method: "GET".to_owned(),
        headers: Vec::new(),
        body: None,
        destination: ServiceWorkerRequestDestination::Empty,
        request_mode: moli_fetch::RequestMode::Cors,
        credentials_mode: moli_fetch::RequestCredentialsMode::Include,
        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
        priority: None,
        is_reload: false,
        metadata: Default::default(),
    };
    vm.with_default_context_scope(|scope, _| {
        let (request, _) =
            crate::network_host::build_service_worker_request_object(scope, &request).unwrap();
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeURLRequest").unwrap();
        assert_eq!(global.set(scope, key.into(), request.into()), Some(true));
        Ok(())
    })
    .unwrap();
    vm.exec(r#"
        globalThis.nativeURLResult = null;
        (async () => {
            const original = nativeURLRequest, url = original.url;
            Object.defineProperty(original, 'url', {get() { throw Error('public url getter'); }});
            Object.defineProperty(original, 'toString', {value() { throw Error('public toString'); }});
            const copies = [original, new Request(original), new Request(original, {}), new Request(original, {cache:'no-store'})];
            const accepted = [];
            for (const request of copies) accepted.push([request === original ? url : request.url, await (await fetch(request)).text()]);
            const refused = [];
            for (const input of [url, {url}]) {
                try {
                    await fetch(input);
                    refused.push({allowed:true});
                } catch (error) {
                    refused.push({name:error.name, message:error.message});
                }
            }
            return {accepted, refused};
        })().then(result => nativeURLResult = result, error => nativeURLResult = {error:String(error)});
    "#, None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(nativeURLResult !== null)",
        "true",
        "native Window Request URL forwarding should finish",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(nativeURLResult)").unwrap()).unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "accepted": vec![serde_json::json!([native_url.as_str(), "ok"]); 4],
            "refused": vec![serde_json::json!({
                "name": "TypeError",
                "message": "Request URL must not include credentials",
            }); 2],
        })
    );
    stop_server.send(()).unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests
            .iter()
            .all(|head| head.starts_with("GET /native-request HTTP/1.1\r\n"))
    );
}
