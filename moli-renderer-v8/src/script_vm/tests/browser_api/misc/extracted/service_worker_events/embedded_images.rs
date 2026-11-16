use super::*;
use base64::Engine as _;

type EmbeddedImageResponse = (u16, &'static str, Vec<u8>, Option<String>);

async fn embedded_image_test_server(
    route: impl Fn(&str) -> EmbeddedImageResponse + Send + Sync + 'static,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<Vec<String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let route = std::sync::Arc::new(route);
    let server = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        let mut requests = Vec::new();
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.unwrap();
                    let route = route.clone();
                    tasks.spawn(async move {
                        let request = read_storage_bucket_partition_request_head(&mut stream).await.unwrap();
                        let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                        let (status, mime, body, location) = route(path);
                        let location = location.map(|url| format!("Location: {url}\r\n")).unwrap_or_default();
                        let head = format!("HTTP/1.1 {status} Response\r\nContent-Type: {mime}\r\n{location}Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        stream.write_all(head.as_bytes()).await.unwrap();
                        stream.write_all(&body).await.unwrap();
                        request
                    });
                }
                Some(result) = tasks.join_next(), if !tasks.is_empty() => requests.push(result.unwrap()),
            }
        }
        while let Some(result) = tasks.join_next().await {
            requests.push(result.unwrap());
        }
        requests
    });
    (base, stop, server)
}

fn embedded_test_png(width: u32) -> Vec<u8> {
    let pixels =
        moli_image::RgbaImage::try_new(width, 1, [32, 180, 96, 255].repeat(width as usize))
            .unwrap();
    moli_image::encode_png(&pixels).unwrap().bytes
}

fn embedded_request_path(request: &str) -> &str {
    request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
}

fn embedded_request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[tokio::test]
async fn embedded_bitmaps_wait_for_rendered_owners_without_blocking_invisible_images() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let deferred_requests = Arc::new(AtomicUsize::new(0));
    let received_deferred_requests = deferred_requests.clone();
    let png = embedded_test_png(2);
    let (base, stop, server) = embedded_image_test_server(move |path| {
        if path.starts_with("/deferred-") {
            received_deferred_requests.fetch_add(1, Ordering::SeqCst);
        }
        (200, "image/png", png.clone(), None)
    })
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/page.html"),
        &loader,
    );
    vm.eval(r#"
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
const bitmapStyle = document.createElement('style');
bitmapStyle.textContent = '.bitmap-suppressed { display: none } #bitmap-override[hidden] { display: inline-block }';
document.documentElement.appendChild(bitmapStyle);
globalThis.renderedBitmapEvents = [];
function renderedBitmap(kind, id, attributes, parent = document.body) {
  const element = document.createElement(kind);
  element.id = id;
  if (kind !== 'img') element.type = 'image/png';
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
  element.onload = () => renderedBitmapEvents.push(id + ':load');
  element.onerror = () => renderedBitmapEvents.push(id + ':error');
  element.setAttribute(kind === 'object' ? 'data' : 'src', '/' + id + '.png');
  parent.appendChild(element);
  return element;
}
globalThis.deferredObject = renderedBitmap('object', 'deferred-object', {class:'bitmap-suppressed'});
globalThis.deferredParent = document.createElement('div');
deferredParent.className = 'bitmap-suppressed';
document.body.appendChild(deferredParent);
globalThis.deferredEmbed = renderedBitmap('embed', 'deferred-embed', {}, deferredParent);
renderedBitmap('img', 'ordinary-hidden-img', {class:'bitmap-suppressed'});
renderedBitmap('object', 'invisible-object', {style:'visibility:hidden'});
renderedBitmap('embed', 'transparent-embed', {style:'opacity:0'});
renderedBitmap('object', 'bitmap-override', {hidden:''});
"#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(renderedBitmapEvents.length === 4)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(renderedBitmapEvents.slice().sort())")
            .unwrap(),
        r#"["bitmap-override:load","invisible-object:load","ordinary-hidden-img:load","transparent-embed:load"]"#
    );
    assert_eq!(deferred_requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        vm.eval("String(document.getElementById('ordinary-hidden-img').naturalWidth)")
            .unwrap(),
        "2"
    );
    assert_eq!(vm.eval("getComputedStyle(deferredObject).display + ',' + getComputedStyle(deferredParent).display").unwrap(), "none,none");

    vm.exec_runtime_turn("deferredObject.classList.remove('bitmap-suppressed'); deferredParent.classList.remove('bitmap-suppressed');", None).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(renderedBitmapEvents.length === 6)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(renderedBitmapEvents.filter(event => event.startsWith('deferred-')).sort())").unwrap(),
        r#"["deferred-embed:load","deferred-object:load"]"#);
    assert_eq!(deferred_requests.load(Ordering::SeqCst), 2);
    assert_eq!(vm.eval("String(frames.length === 0 && deferredObject.contentWindow === null && deferredEmbed.getSVGDocument() === null)").unwrap(), "true");
    let _ = stop.send(());
    let requests = server.await.unwrap();
    let mut paths = requests
        .iter()
        .map(|request| embedded_request_path(request))
        .collect::<Vec<_>>();
    paths.sort_unstable();
    assert_eq!(
        paths,
        [
            "/bitmap-override.png",
            "/deferred-embed.png",
            "/deferred-object.png",
            "/invisible-object.png",
            "/ordinary-hidden-img.png",
            "/transparent-embed.png"
        ]
    );
}

#[tokio::test]
async fn embedded_bitmaps_fetch_and_decode_without_windows_or_controlled_image_reuse() {
    let png = embedded_test_png(2);
    let mut corrupt = png.clone();
    let idat = corrupt
        .windows(4)
        .position(|bytes| bytes == b"IDAT")
        .unwrap();
    corrupt[idat + 4] ^= 0xff;
    assert!(
        moli_image::probe_raster_image(&corrupt).is_ok(),
        "the failure fixture must retain valid image metadata"
    );
    let intercepted = base64::engine::general_purpose::STANDARD.encode(embedded_test_png(7));
    let worker = format!(
        r#"
const image = Uint8Array.from(atob({}), c => c.charCodeAt(0));
onactivate = event => event.waitUntil(clients.claim());
onfetch = event => {{
  if (new URL(event.request.url).pathname.endsWith('.png'))
    event.respondWith(new Response(image, {{headers: {{'Content-Type':'image/png'}}}}));
}};
"#,
        serde_json::to_string(&intercepted).unwrap()
    );
    let (base, stop, server) = embedded_image_test_server(move |path| match path {
        "/worker.js" => (200, "text/javascript", worker.as_bytes().to_vec(), None),
        "/shared.png" => (200, "image/png", png.clone(), None),
        "/corrupt.png" => (200, "image/png", corrupt.clone(), None),
        "/status.png" => (404, "image/png", png.clone(), None),
        "/document.html" => (200, "text/html", br#"<!doctype html><body data-source='network'><script>parent.postMessage('bitmap-document-ready','*')</script>"#.to_vec(), None),
        _ => panic!("unexpected embedded image request {path}"),
    }).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/page.html"),
        &loader,
    );
    vm.eval(r#"
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.bitmapReady = false;
globalThis.bitmapError = null;
globalThis.bitmapEvents = [];
globalThis.bitmapDocumentReady = false;
addEventListener('message', event => { if (event.data === 'bitmap-document-ready') bitmapDocumentReady = true; });
(async () => {
  await navigator.serviceWorker.register('/worker.js', {scope:'/'});
  await __moliTestActivatedServiceWorker();
  if (!navigator.serviceWorker.controller)
    await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
  const image = new Image();
  globalThis.controlledBitmap = image;
  image.onload = () => bitmapReady = true;
  image.onerror = () => bitmapError = 'controlled image failed';
  image.src = '/shared.png';
  document.body.appendChild(image);
})().catch(error => bitmapError = String(error));
function appendBitmap(kind, id, path) {
  const element = document.createElement(kind);
  element.type = 'image/png';
  element.onload = () => bitmapEvents.push(id + ':load');
  element.onerror = () => bitmapEvents.push(id + ':error');
  element.setAttribute(kind === 'object' ? 'data' : 'src', path);
  document.body.appendChild(element);
  return element;
}
document.cookie = 'embedded_bitmap=yes; path=/';
"#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapReady || bitmapError !== null)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(bitmapError)").unwrap(), "null");
    assert_eq!(
        vm.eval("String(controlledBitmap.naturalWidth)").unwrap(),
        "7"
    );
    vm.eval("globalThis.bitmapObject = appendBitmap('object','o','/shared.png'); globalThis.bitmapEmbed = appendBitmap('embed','e','/shared.png');").unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapEvents.length === 2)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(bitmapEvents.slice().sort())")
            .unwrap(),
        r#"["e:load","o:load"]"#
    );
    assert_eq!(vm.eval("String(frames.length === 0 && bitmapObject.contentWindow === null && bitmapObject.contentDocument === null && bitmapEmbed.getSVGDocument() === null)").unwrap(), "true");
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByName(new URL('/shared.png',location.href).href).map(entry => entry.initiatorType).sort())").unwrap(), r#"["embed","img","object"]"#);

    vm.eval("bitmapObject.data = '/document.html'; bitmapObject.type = 'text/html';")
        .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapDocumentReady && bitmapEvents.length === 3)",
        "true",
    )
    .await;
    assert_eq!(vm.eval("String(frames.length === 1 && bitmapObject.contentDocument.body.dataset.source === 'network' && bitmapObject.contentWindow.navigator.serviceWorker.controller === null)").unwrap(), "true");
    vm.eval("bitmapObject.data = '/shared.png'; bitmapObject.type = 'image/png';")
        .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapEvents.length === 4)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(frames.length === 0 && bitmapObject.contentWindow === null)")
            .unwrap(),
        "true"
    );
    vm.eval("bitmapObject.setAttribute('data', bitmapObject.getAttribute('data'))")
        .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapEvents.length === 5)",
        "true",
    )
    .await;
    vm.eval(
        r#"
appendBitmap('object','bad-o','/corrupt.png');
appendBitmap('embed','bad-e','/corrupt.png');
appendBitmap('object','http-o','/status.png');
appendBitmap('embed','http-e','/status.png');
"#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapEvents.length === 9)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(bitmapEvents.slice(5).sort())")
            .unwrap(),
        r#"["bad-e:error","bad-o:error","http-e:load","http-o:error"]"#
    );
    assert_eq!(vm.eval("String(frames.length)").unwrap(), "0");
    stop.send(()).unwrap();
    let requests = server.await.unwrap();
    let shared = requests
        .iter()
        .filter(|request| embedded_request_path(request) == "/shared.png")
        .collect::<Vec<_>>();
    assert_eq!(shared.len(), 4, "{requests:?}");
    let mut destinations = shared
        .iter()
        .map(|request| embedded_request_header(request, "Sec-Fetch-Dest").unwrap())
        .collect::<Vec<_>>();
    destinations.sort();
    assert_eq!(destinations, ["embed", "object", "object", "object"]);
    for request in shared {
        assert_eq!(
            embedded_request_header(request, "Sec-Fetch-Mode"),
            Some("navigate"),
            "{request}"
        );
        assert!(
            embedded_request_header(request, "Cookie")
                .is_some_and(|cookie| cookie.contains("embedded_bitmap=yes")),
            "{request}"
        );
    }
    assert_eq!(
        requests
            .iter()
            .filter(|request| embedded_request_path(request) == "/corrupt.png")
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| embedded_request_path(request) == "/status.png")
            .count(),
        2
    );
}

#[tokio::test]
async fn embedded_bitmap_policy_uses_object_src_and_blocks_redirects_before_network() {
    let png = embedded_test_png(2);
    let target_png = png.clone();
    let (other, other_stop, other_server) =
        embedded_image_test_server(move |_| (200, "image/png", target_png.clone(), None)).await;
    let (base, stop, server) = embedded_image_test_server(move |path| {
        if path == "/redirect.png" {
            (
                302,
                "image/png",
                Vec::new(),
                Some(format!("{other}/forbidden.png")),
            )
        } else {
            (200, "image/png", png.clone(), None)
        }
    })
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/policy.html"),
        &loader,
    );
    vm.eval(r#"
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
if (!document.head) document.documentElement.insertBefore(document.createElement('head'), document.documentElement.firstChild);
globalThis.bitmapPolicyEvents = [];
globalThis.bitmapViolations = [];
document.addEventListener('securitypolicyviolation', event => bitmapViolations.push(event.effectiveDirective));
const policy = document.createElement('meta');
policy.httpEquiv = 'Content-Security-Policy';
policy.content = "img-src 'none'; object-src 'self'";
document.head.appendChild(policy);
for (const [kind,id,url] of [['img','img','/allowed.png'],['object','object','/allowed.png'],['embed','embed','/allowed.png'],['object','redirect-object','/redirect.png'],['embed','redirect-embed','/redirect.png']]) {
  const element = document.createElement(kind);
  if (kind !== 'img') element.type = 'image/png';
  element.onload = () => bitmapPolicyEvents.push(id + ':load');
  element.onerror = () => bitmapPolicyEvents.push(id + ':error');
  element.setAttribute(kind === 'object' ? 'data' : 'src', url);
  document.body.appendChild(element);
}
"#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapPolicyEvents.length === 5 && bitmapViolations.length === 3)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(bitmapPolicyEvents.slice().sort())")
            .unwrap(),
        r#"["embed:load","img:error","object:load","redirect-embed:error","redirect-object:error"]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(bitmapViolations.slice().sort())")
            .unwrap(),
        r#"["img-src","object-src","object-src"]"#
    );
    vm.eval("for (const element of document.querySelectorAll('object')) { void element.contentWindow; void element.contentDocument; } void frames.length;").unwrap();
    assert_eq!(vm.eval("String(bitmapViolations.length)").unwrap(), "3");
    stop.send(()).unwrap();
    other_stop.send(()).unwrap();
    let requests = server.await.unwrap();
    let forbidden = other_server.await.unwrap();
    assert!(
        forbidden.is_empty(),
        "a CSP-blocked redirect reached the network: {forbidden:?}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| embedded_request_path(request) == "/allowed.png")
            .count(),
        2,
        "{requests:?}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| embedded_request_path(request) == "/redirect.png")
            .count(),
        2,
        "{requests:?}"
    );
}

#[tokio::test]
async fn embedded_bitmap_source_changes_and_removal_retire_actual_interception_requests() {
    let png = embedded_test_png(2);
    let response_png = png.clone();
    let (base, stop, server) =
        embedded_image_test_server(move |_| (200, "image/png", response_png.clone(), None)).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/lifetime.html"),
        &loader,
    );
    vm.set_fetch_subresource_interception(
        crate::types::SubresourceInterceptionClasses::for_resource_type(
            crate::types::SubresourceResourceType::Image,
        ),
    );
    vm.eval(
        r#"
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.bitmapLifetimeEvents = [];
globalThis.bitmapLifetimeOwner = document.createElement('object');
bitmapLifetimeOwner.type = 'image/png';
bitmapLifetimeOwner.onload = () => bitmapLifetimeEvents.push('load');
bitmapLifetimeOwner.onerror = () => bitmapLifetimeEvents.push('error');
bitmapLifetimeOwner.data = '/retired.png';
document.body.appendChild(bitmapLifetimeOwner);
"#,
    )
    .unwrap();
    let retired = vm.take_pending_subresource_fetch_infos();
    assert_eq!(retired.len(), 1);
    vm.eval("bitmapLifetimeOwner.data = '/current.png'")
        .unwrap();
    let current = vm.take_pending_subresource_fetch_infos();
    assert_eq!(current.len(), 1);
    assert_ne!(retired[0].internal_id, current[0].internal_id);
    assert!(
        vm.fulfill_pending_subresource_fetch(
            retired[0].internal_id,
            200,
            vec![("Content-Type".to_owned(), b"image/png".to_vec())],
            crate::runtime::RendererSyntheticResponseBody::from_bytes(png.clone()),
        )
        .is_err(),
        "a superseded real request must no longer be fulfillable"
    );
    vm.fulfill_pending_subresource_fetch(
        current[0].internal_id,
        200,
        vec![("Content-Type".to_owned(), b"image/png".to_vec())],
        crate::runtime::RendererSyntheticResponseBody::from_bytes(png.clone()),
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(bitmapLifetimeEvents.length === 1)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(bitmapLifetimeEvents)").unwrap(),
        r#"["load"]"#
    );
    vm.eval("bitmapLifetimeOwner.data = '/removed.png'")
        .unwrap();
    let removed = vm.take_pending_subresource_fetch_infos();
    assert_eq!(removed.len(), 1);
    vm.eval("bitmapLifetimeOwner.remove()").unwrap();
    assert!(
        vm.fulfill_pending_subresource_fetch(
            removed[0].internal_id,
            200,
            vec![("Content-Type".to_owned(), b"image/png".to_vec())],
            crate::runtime::RendererSyntheticResponseBody::from_bytes(png),
        )
        .is_err(),
        "a removed bitmap owner must release its real request"
    );
    assert_eq!(
        vm.eval("JSON.stringify(bitmapLifetimeEvents) + ':' + frames.length")
            .unwrap(),
        r#"["load"]:0"#
    );
    stop.send(()).unwrap();
    assert!(
        server.await.unwrap().is_empty(),
        "all requests in this case were intercepted"
    );
}

#[tokio::test]
async fn embedded_object_resource_errors_complete_unsupported_schemes_and_keep_blank_documents() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        "https://object-errors.test/page.html",
        &loader,
    );
    vm.exec_runtime_turn(
        r#"
      if (!document.body) document.documentElement.appendChild(document.createElement('body'));
      globalThis.objectEvents = [];
      globalThis.objectEventDetails = [];
      globalThis.objectJavascriptExecuted = false;
      for (const [id, data] of [
        ['file', 'file:\\http://nonexistent.html'],
        ['invalid', 'http://test:test'],
        ['unsupported-about', 'about:unsupported-object-resource'],
        ['javascript', 'javascript:parent.objectJavascriptExecuted = true'],
        ['blank', 'about:blank'],
      ]) {
        const element = document.createElement('object');
        element.id = id;
        element.data = data;
        element.onerror = event => {
          objectEvents.push(id + ':error');
          objectEventDetails.push([
            Object.getPrototypeOf(event).constructor === Event, event.isTrusted,
            !event.bubbles, !event.cancelable, event.target === element,
            element.contentWindow === null, element.contentDocument === null,
          ].every(Boolean));
        };
        element.onload = () => objectEvents.push(id + ':load');
        document.body.append(element);
      }
    "#,
        None,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(objectEvents.length === 5)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(objectEvents.slice().sort())")
            .unwrap(),
        r#"["blank:load","file:error","invalid:error","javascript:error","unsupported-about:error"]"#
    );
    assert_eq!(
        vm.eval("String(objectEventDetails.length === 4 && objectEventDetails.every(Boolean))")
            .unwrap(),
        "true"
    );
    assert_eq!(vm.eval("String(!objectJavascriptExecuted && frames.length === 1 && document.getElementById('blank').contentWindow instanceof document.getElementById('blank').contentWindow.Window)").unwrap(), "true");
    assert!(
        !vm.has_pending_child_document_lifecycle(),
        "failed resources release their navigation and load barriers"
    );
}
