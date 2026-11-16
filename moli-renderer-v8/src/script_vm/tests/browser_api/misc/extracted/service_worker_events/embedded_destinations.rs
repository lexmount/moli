use super::*;

#[tokio::test]
async fn embedded_navigation_skips_worker_interception_without_disabling_client_claims() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let client_script = include_str!("../../../../fixtures/service-worker-embedded-client.js");
    let network_html =
        format!("<!doctype html><body data-source='network'><script>{client_script}</script>");
    let worker_html =
        format!("<!doctype html><body data-source='worker'><script>{client_script}</script>");
    let worker_source = format!(
        r#"
const documentSource = {};
onfetch = event => {{
  const url = new URL(event.request.url);
  if (url.pathname === '/control/client.html') {{
    event.respondWith(new Response(documentSource, {{headers: {{'Content-Type': 'text/html'}}}}));
  }} else if (url.pathname === '/control/probe') {{
    event.respondWith(new Response('worker:' + event.request.destination));
  }}
}};
onmessage = event => {{
  if (event.data === 'claim') {{
    event.waitUntil(clients.claim().then(() => event.ports[0].postMessage('claimed')));
  }}
}};
        "#,
        serde_json::to_string(&worker_html).unwrap()
    );
    let server = tokio::spawn(async move {
        let mut navigations = Vec::new();
        for index in 0..7 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_storage_bucket_partition_request_head(&mut stream)
                .await
                .unwrap();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let (status, extra_headers, content_type, body) = if index == 0 {
                assert_eq!(path, "/control/worker.js");
                (200, "", "text/javascript", worker_source.as_str())
            } else {
                navigations.push(request.clone());
                if path == "/control/frame-start" {
                    (
                        302,
                        "Location: /control/frame-client.html?kind=frame&phase=frame\r\n",
                        "text/html",
                        "",
                    )
                } else {
                    assert!(
                        path.starts_with("/control/client.html?")
                            || path == "/control/frame-client.html?kind=frame&phase=frame",
                        "{request}"
                    );
                    (200, "", "text/html", network_html.as_str())
                }
            };
            let response = format!(
                "HTTP/1.1 {status} Response\r\n{extra_headers}Content-Type: {content_type}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        navigations
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/control/root.html"),
            &loader,
        );
    vm.eval(
        r#"
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.destinationRows = [];
globalThis.destinationWindows = new Map();
globalThis.destinationError = null;
globalThis.destinationReady = false;
addEventListener('message', event => {
  if (event.data.error) { destinationError = event.data.error; return; }
  if (!event.data.kind) return;
  destinationRows.push(event.data);
  destinationWindows.set(event.data.kind, event.source);
});
(async () => {
  globalThis.destinationRegistration = await navigator.serviceWorker.register('worker.js', {scope: './'});
  await __moliTestActivatedServiceWorker();
  destinationReady = true;
})().catch(error => destinationError = String(error));
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationReady || destinationError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(destinationError)").unwrap(), "null");
    vm.eval(
        r#"
for (const kind of ['iframe', 'object', 'embed']) {
  const element = document.createElement(kind);
  if (kind !== 'iframe') element.type = 'text/html';
  element.setAttribute(kind === 'object' ? 'data' : 'src',
    '/control/client.html?kind=' + kind + '&phase=initial');
  (document.body || document.documentElement).appendChild(element);
}
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationRows.length >= 3 || destinationError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(destinationError)").unwrap(), "null");
    let inspect_rows = |phase: &str| {
        format!(
            "JSON.stringify(destinationRows.filter(row => row.phase === {phase:?}).sort((a, b) => a.kind < b.kind ? -1 : a.kind > b.kind ? 1 : 0))"
        )
    };
    assert_eq!(
        vm.eval(&inspect_rows("initial")).unwrap(),
        r#"[{"kind":"embed","phase":"initial","probe":null,"source":"network","controlled":false},{"kind":"iframe","phase":"initial","probe":null,"source":"worker","controlled":true},{"kind":"object","phase":"initial","probe":null,"source":"network","controlled":false}]"#
    );
    vm.eval(
        r#"
globalThis.destinationClaimed = false;
const claimChannel = new MessageChannel();
claimChannel.port1.onmessage = () => destinationClaimed = true;
destinationRegistration.active.postMessage('claim', [claimChannel.port2]);
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationClaimed)",
        "true",
    )
    .await;
    vm.eval("for (const child of destinationWindows.values()) child.postMessage({action: 'inspect'}, '*');")
        .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationRows.length >= 6 || destinationError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(destinationError)").unwrap(), "null");
    assert_eq!(
        vm.eval(&inspect_rows("claimed")).unwrap(),
        r#"[{"kind":"embed","phase":"claimed","probe":"worker:","source":"network","controlled":true},{"kind":"iframe","phase":"claimed","probe":"worker:","source":"worker","controlled":true},{"kind":"object","phase":"claimed","probe":"worker:","source":"network","controlled":true}]"#
    );
    vm.eval(
        r#"
for (const [kind, child] of destinationWindows) {
  child.postMessage({action: 'navigate', url: '/control/client.html?kind=' + kind + '&phase=navigated'}, '*');
}
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationRows.length >= 9 || destinationError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(destinationError)").unwrap(), "null");
    assert_eq!(
        vm.eval(&inspect_rows("navigated")).unwrap(),
        r#"[{"kind":"embed","phase":"navigated","probe":null,"source":"network","controlled":false},{"kind":"iframe","phase":"navigated","probe":null,"source":"worker","controlled":true},{"kind":"object","phase":"navigated","probe":null,"source":"network","controlled":false}]"#
    );

    let blob_html = format!(
        "<!doctype html><body data-source='blob' data-kind='blob-object' data-phase='blob'><script>{client_script}</script>"
    );
    vm.eval(&format!(
        r#"
globalThis.destinationBlobURL = URL.createObjectURL(new Blob([{}], {{type: 'text/html'}}));
const blobObject = document.createElement('object');
blobObject.type = 'text/html';
blobObject.data = destinationBlobURL;
(document.body || document.documentElement).appendChild(blobObject);
        "#,
        serde_json::to_string(&blob_html).unwrap()
    ))
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationRows.length >= 10 || destinationError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(destinationError)").unwrap(), "null");
    assert_eq!(
        vm.eval(&inspect_rows("blob")).unwrap(),
        r#"[{"kind":"blob-object","phase":"blob","probe":null,"source":"blob","controlled":true}]"#
    );
    vm.eval("URL.revokeObjectURL(destinationBlobURL)").unwrap();

    vm.eval(
        r#"
const frameset = document.createElement('frameset');
const legacyFrame = document.createElement('frame');
legacyFrame.src = '/control/frame-start';
frameset.appendChild(legacyFrame);
document.body.replaceWith(frameset);
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(destinationRows.length >= 11 || destinationError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(destinationError)").unwrap(), "null");
    assert_eq!(
        vm.eval(&inspect_rows("frame")).unwrap(),
        r#"[{"kind":"frame","phase":"frame","probe":null,"source":"network","controlled":true}]"#
    );

    let requests = server.await.unwrap();
    let mut paths = Vec::new();
    for request in requests {
        let path = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let expected = if path.starts_with("/control/frame-") {
            "frame"
        } else if path.contains("kind=object") {
            "object"
        } else {
            "embed"
        };
        let destinations = request
            .lines()
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("sec-fetch-dest")
                    .then(|| value.trim())
            })
            .collect::<Vec<_>>();
        assert_eq!(destinations, [expected], "{request}");
        paths.push(path.to_owned());
    }
    paths.sort();
    assert_eq!(
        paths,
        [
            "/control/client.html?kind=embed&phase=initial",
            "/control/client.html?kind=embed&phase=navigated",
            "/control/client.html?kind=object&phase=initial",
            "/control/client.html?kind=object&phase=navigated",
            "/control/frame-client.html?kind=frame&phase=frame",
            "/control/frame-start",
        ]
    );
}
