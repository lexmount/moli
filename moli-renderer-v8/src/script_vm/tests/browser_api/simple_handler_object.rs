use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

const HANDLER_PROBE: &str = r#"
function exerciseHandler(target, type, w, checkException) {
  const holder = target;
  const property = 'on' + type;
  const trace = [];
  const errors = [];
  const errorListener = event => { errors.push(event.error instanceof w.TypeError); event.preventDefault(); };
  w.addEventListener('error', errorListener);
  const dispatch = () => target.dispatchEvent(new w.Event(type, {cancelable:true}));
  const objectValues = [{}, Object.create(null), new w.Number(42), new w.String('text'), [], /pattern/, Object.create(w.Function.prototype)];
  const objectIdentity = objectValues.map(value => { holder[property] = value; return holder[property] === value; });
  const primitiveNull = [undefined, null, false, 42, 'text', Symbol('value'), 1n].map(value => {
    holder[property] = value; return holder[property] === null;
  });

  const before = () => trace.push('before');
  const middle = () => trace.push('middle');
  const after = () => trace.push('after');
  const late = () => trace.push('late');
  const replacement = () => { trace.push('replacement'); return false; };
  target.addEventListener(type, before);
  holder[property] = {};
  target.addEventListener(type, middle);
  holder[property] = replacement;
  target.addEventListener(type, after);
  const firstResult = dispatch();
  const firstOrder = trace.splice(0);
  let operationReads = 0;
  const listenerObject = {get handleEvent() { operationReads++; return () => trace.push('object-listener'); }};
  holder[property] = listenerObject;
  const silentResult = dispatch();
  const silentOrder = trace.splice(0);
  const handlerReads = operationReads;
  target.addEventListener(type, listenerObject);
  dispatch();
  const interfaceOrder = trace.splice(0);
  const interfaceReads = operationReads;
  target.removeEventListener(type, listenerObject);
  holder[property] = replacement;
  dispatch();
  const preservedOrder = trace.splice(0);
  holder[property] = 42;
  target.addEventListener(type, late);
  holder[property] = replacement;
  dispatch();
  const reactivatedOrder = trace.splice(0);
  holder[property] = null;
  for (const listener of [before, middle, after, late]) target.removeEventListener(type, listener);

  let proxyGets = 0;
  let proxyCalls = 0;
  let receiverOK = false;
  const callable = new Proxy(function() { trace.push('called'); return false; }, {
    get() { proxyGets++; throw new Error('callback property read'); },
    apply(fn, receiver, args) {
      proxyCalls++; receiverOK = receiver === target && args.length === 1 && args[0].currentTarget === target;
      return Reflect.apply(fn, receiver, args);
    }
  });
  holder[property] = callable;
  const proxyIdentity = holder[property] === callable;
  const proxyResult = dispatch();
  const proxyTrace = trace.splice(0);
  const revokedObject = Proxy.revocable({}, {});
  revokedObject.revoke();
  holder[property] = revokedObject.proxy;
  const revokedObjectIdentity = holder[property] === revokedObject.proxy;
  const revokedObjectResult = dispatch();
  let revokedFunctionIdentity, revokedFunctionResult;
  if (checkException) {
    const revokedFunction = Proxy.revocable(function(){}, {});
    holder[property] = revokedFunction.proxy;
    revokedFunction.revoke();
    revokedFunctionIdentity = holder[property] === revokedFunction.proxy;
    revokedFunctionResult = dispatch();
  }
  holder[property] = null;
  w.removeEventListener('error', errorListener);
  return {objectIdentity, primitiveNull, firstResult, firstOrder, silentResult, silentOrder,
    handlerReads, interfaceOrder, interfaceReads, preservedOrder, reactivatedOrder,
    proxyIdentity, proxyResult, proxyTrace, proxyGets, proxyCalls, receiverOK,
    revokedObjectIdentity, revokedObjectResult, revokedFunctionIdentity, revokedFunctionResult, errors: checkException ? errors : undefined};
}
"#;

const PAGE_PROBE: &str = r#"
__EXERCISE__
(() => {
  const settings = __CASE__;
  const frame = settings.kind.startsWith('child-') ? document.createElement('iframe') : null;
  if (frame) document.body.appendChild(frame);
  const w = frame ? frame.contentWindow : window;
  let target, holder, type, cleanup = () => {};
  const kind = settings.kind.replace(/^child-/, '');
  switch (kind) {
    case 'xhr': target = new w.XMLHttpRequest(); type = 'load'; break;
    case 'xhr-upload': target = new w.XMLHttpRequest().upload; type = 'load'; break;
    case 'performance': target = w.performance; type = 'resourcetimingbufferfull'; break;
    case 'fonts': target = w.document.fonts; type = 'loadingdone'; break;
    case 'broadcast': target = new w.BroadcastChannel('handler-values'); type = 'message'; cleanup = () => target.close(); break;
    case 'speech': target = new w.SpeechSynthesisUtterance(''); type = 'end'; break;
    case 'synthesis': target = w.speechSynthesis; type = 'voiceschanged'; break;
    case 'devices': target = w.navigator.mediaDevices; type = 'devicechange'; break;
    case 'service-container': target = w.navigator.serviceWorker; type = 'message'; break;
    case 'event-source': target = new w.EventSource('/event-source'); type = 'open'; cleanup = () => target.close(); break;
    case 'websocket': target = new w.WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/socket'); type = 'open'; cleanup = () => target.close(); break;
    case 'worker-host': {
      const url = w.URL.createObjectURL(new w.Blob([''], {type:'text/javascript'}));
      target = new w.Worker(url); type = 'message'; cleanup = () => { target.terminate(); w.URL.revokeObjectURL(url); }; break;
    }
    case 'shared-worker-host': {
      const url = w.URL.createObjectURL(new w.Blob(['onconnect = e => e.ports[0].start();'], {type:'text/javascript'}));
      target = new w.SharedWorker(url); type = 'error'; cleanup = () => { target.port.close(); w.URL.revokeObjectURL(url); }; break;
    }
    default: throw new Error('unknown target ' + kind);
  }
  const value = exerciseHandler(target, type, w, settings.checkException);
  cleanup();
  if (frame) frame.remove();
  return value;
})()
"#;

const WORKER_PARENT_PROBE: &str = r#"
(async () => {
  const config = __CONFIG__;
  const source = __SOURCE__;
  let worker, url, registration, port, timer;
  const result = await new Promise(async (resolve, reject) => {
    timer = setTimeout(() => reject(new Error(config.name + ' timeout')), 8000);
    try {
      if (config.mode === 'service') {
        const listener = event => { navigator.serviceWorker.removeEventListener('message', listener); resolve(event.data); };
        navigator.serviceWorker.addEventListener('message', listener);
        registration = await navigator.serviceWorker.register('/handler-sw.js?case=' + encodeURIComponent(config.name), {scope:'./'});
        const active = registration.active || registration.installing || registration.waiting;
        if (active.state !== 'activated') await new Promise(done => active.addEventListener('statechange', () => { if (active.state === 'activated') done(); }));
        active.postMessage('probe');
      } else {
        url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
        worker = config.mode === 'shared' ? new SharedWorker(url) : new Worker(url);
        port = config.mode === 'shared' ? worker.port : worker;
        port.onmessage = event => resolve(event.data);
        worker.onerror = event => reject(new Error(event.message || 'worker error'));
      }
    } catch (error) { reject(error); }
  }).finally(async () => {
    clearTimeout(timer);
    if (config.mode === 'shared') port?.close();
    else worker?.terminate();
    if (url) URL.revokeObjectURL(url);
    if (registration) await registration.unregister();
  });
  return result;
})()
"#;

fn expected_handler_value(check_exception: bool) -> serde_json::Value {
    let mut expected = serde_json::json!({
        "objectIdentity": vec![true; 7],
        "primitiveNull": vec![true; 7],
        "firstResult": false,
        "firstOrder": ["before", "replacement", "middle", "after"],
        "silentResult": true,
        "silentOrder": ["before", "middle", "after"],
        "handlerReads": 0,
        "interfaceOrder": ["before", "middle", "after", "object-listener"],
        "interfaceReads": 1,
        "preservedOrder": ["before", "replacement", "middle", "after"],
        "reactivatedOrder": ["before", "middle", "after", "late", "replacement"],
        "proxyIdentity": true, "proxyResult": false, "proxyTrace": ["called"],
        "proxyGets": 0, "proxyCalls": 1, "receiverOK": true,
        "revokedObjectIdentity": true, "revokedObjectResult": true,
    });
    if check_exception {
        expected["revokedFunctionIdentity"] = true.into();
        expected["revokedFunctionResult"] = true.into();
        expected["errors"] = serde_json::json!([true]);
    }
    expected
}

#[tokio::test]
async fn simple_handler_object_values_preserve_identity_and_listener_position() {
    for kind in [
        "xhr",
        "xhr-upload",
        "performance",
        "fonts",
        "broadcast",
        "speech",
        "synthesis",
        "devices",
        "service-container",
        "event-source",
        "websocket",
        "worker-host",
        "shared-worker-host",
        "child-xhr",
        "child-xhr-upload",
        "child-performance",
        "child-fonts",
        "child-broadcast",
        "child-speech",
    ] {
        // Child/error callback exception routing is independent of handler conversion.
        let check_exception = !kind.starts_with("child-") && kind != "shared-worker-host";
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, _runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://simple-handler-object.test/",
                &loader,
            );
        vm.eval("if (!document.body) document.documentElement.appendChild(document.createElement('body'))").unwrap();
        let source = PAGE_PROBE.replace("__EXERCISE__", HANDLER_PROBE).replace(
            "__CASE__",
            &serde_json::json!({"kind": kind, "checkException": check_exception}).to_string(),
        );
        let source = source.replacen("\n(() => {", "\nreturn (() => {", 1);
        let value = vm
            .eval(&format!("JSON.stringify((() => {{ {source} }})())"))
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        assert_eq!(value, expected_handler_value(check_exception), "{kind}");
    }
}

#[tokio::test]
async fn simple_handler_object_worker_globals_preserve_objects_and_order() {
    for (mode, event) in [
        ("dedicated", "message"),
        ("dedicated", "unhandledrejection"),
        ("dedicated", "online"),
        ("shared", "connect"),
    ] {
        // Synthetic worker exception reporting is independent of handler conversion.
        // This probe exercises legacy handler conversion and callback-interface separation.
        let expression = format!("exerciseHandler(self, {event:?}, self, false)");
        let source = if mode == "shared" {
            format!(
                "{HANDLER_PROBE} addEventListener('connect', e => {{ e.ports[0].postMessage({expression}); close(); }}, {{once:true}});"
            )
        } else {
            format!("{HANDLER_PROBE} postMessage({expression}); close();")
        };
        let probe = WORKER_PARENT_PROBE
            .replace(
                "__CONFIG__",
                &serde_json::json!({"mode": mode, "name": event}).to_string(),
            )
            .replace("__SOURCE__", &serde_json::to_string(&source).unwrap());
        let value = run_async_handler_probe("https://simple-handler-object.test/", &probe).await;
        assert_eq!(value, expected_handler_value(false), "{mode}/{event}");
    }
}

#[tokio::test]
async fn simple_handler_object_service_worker_fetch() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let source = format!(
        "{HANDLER_PROBE} addEventListener('message', event => {{ event.source.postMessage(exerciseHandler(self, 'fetch', self, false)); }}, {{once:true}});"
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0, "service worker request headers");
            request.extend_from_slice(&buffer[..count]);
        }
        assert!(
            String::from_utf8_lossy(&request)
                .starts_with("GET /handler-sw.js?case=service-values ")
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{source}",
            source.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let probe = WORKER_PARENT_PROBE
        .replace(
            "__CONFIG__",
            r#"{"mode":"service","name":"service-values"}"#,
        )
        .replace("__SOURCE__", "''");
    let value = run_async_handler_probe(&format!("http://{address}/"), &probe).await;
    let expected = expected_handler_value(false);
    assert_eq!(value, expected);
    server.await.unwrap();
}

const NATIVE_PROBE: &str = r#"
(async () => {
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  let reads = 0;
  const object = new frame.contentWindow.Object();
  Object.defineProperty(object, 'handleEvent', {get() { reads++; throw new Error('must not read handleEvent'); }});
  const channel = new BroadcastChannel('retired-handler-values');
  channel.onmessage = object;
  const identity = channel.onmessage === object;
  frame.remove();
  const retiredIdentity = channel.onmessage === object;
  const retiredResult = channel.dispatchEvent(new Event('message', {cancelable:true}));
  channel.close();
  const source = `
    const trace = [];
    onmessage = {get handleEvent() { throw new Error('must not read'); }};
    addEventListener('message', () => trace.push('after'));
    onmessage = () => trace.push('handler');
    addEventListener('message', () => setTimeout(() => postMessage(trace), 0));
  `;
  const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
  const worker = new Worker(url);
  const hostOrder = [];
  let timer;
  const workerOrder = await new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error('native worker timeout')), 4000);
    worker.onmessage = {};
    worker.addEventListener('message', event => { hostOrder.push('after'); resolve(event.data); });
    worker.onmessage = () => hostOrder.push('handler');
    worker.onerror = reject;
    worker.postMessage('go');
  }).finally(() => { clearTimeout(timer); worker.terminate(); URL.revokeObjectURL(url); });
  return {identity, retiredIdentity, retiredResult, reads, workerOrder, hostOrder};
})()
"#;

#[tokio::test]
async fn simple_handler_object_retired_realm_and_native_dispatch() {
    let value = run_async_handler_probe("https://simple-handler-object.test/", NATIVE_PROBE).await;
    assert_eq!(
        value,
        serde_json::json!({
            "identity": true, "retiredIdentity": true, "retiredResult": true, "reads": 0,
            "workerOrder": ["handler", "after"], "hostOrder": ["handler", "after"],
        })
    );
}

async fn run_async_handler_probe(url: &str, probe: &str) -> serde_json::Value {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(url, &loader);
    vm.eval(
        "if (!document.body) document.documentElement.appendChild(document.createElement('body'))",
    )
    .unwrap();
    vm.eval(&format!("({probe}).then(value => globalThis.__handlerValue = value, error => globalThis.__handlerValue = String(error));")).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm.eval("globalThis.__handlerValue !== undefined").unwrap() != "true" {
            runtime.drain_shared_worker_service_lane();
            drain_service_worker_test_turn(&mut vm, &runtime).await;
        }
    })
    .await
    .expect("handler probe should settle");
    let value = vm
        .eval("JSON.stringify(globalThis.__handlerValue)")
        .unwrap();
    serde_json::from_str(&value).unwrap()
}
