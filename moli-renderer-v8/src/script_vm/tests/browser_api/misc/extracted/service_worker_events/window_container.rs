use super::*;

#[tokio::test]
async fn window_service_worker_notifications_ignore_shadowed_navigator_properties() {
    for mode in [
        "normal",
        "navigator-value",
        "navigator-getter",
        "container-value",
        "container-getter",
        "prototype-getter",
    ] {
        let (base_url, server) = spawn_service_worker_response_server(vec![(
            "/app/worker.js",
            "text/javascript",
            r#"
oninstall = event => event.waitUntil(skipWaiting());
onactivate = event => event.waitUntil(clients.claim());
onmessage = event => {
  const source = event.source;
  if (event.data === 'reply') {
    source.postMessage({kind: 'source-ack'});
    return;
  }
  const channel = new MessageChannel();
  channel.port1.onmessage = message => {
    source.postMessage({kind: 'port-ack', value: message.data});
  };
  source.postMessage({kind: 'message', index: 0, port: channel.port2}, [channel.port2]);
  source.postMessage({kind: 'message', index: 1});
};
            "#,
        )])
        .await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                &format!("{base_url}/app/page.html"),
                &loader,
            );
        vm.eval(&format!(
            r#"
globalThis.intrinsicContainerResult = 'pending';
(async () => {{
  const mode = {mode:?}, savedNavigator = navigator, container = savedNavigator.serviceWorker;
  let getterCalls = 0, globalMessages = 0, controllerChanges = 0, portAck = false, sourceAck = false;
  let controller, resolveMessages, resolveChanged;
  const messages = new Promise(resolve => resolveMessages = resolve);
  const changed = new Promise(resolve => resolveChanged = resolve);
  const rows = [], trace = [];
  function checkDone() {{
    if (rows.length === 2 && portAck && sourceAck) resolveMessages();
  }}
  addEventListener('message', () => globalMessages++);
  container.oncontrollerchange = event => {{
    if (event.target !== container || event.currentTarget !== container) throw Error('wrong controllerchange target');
    controllerChanges++;
    resolveChanged();
  }};
  container.onmessage = event => {{
    if (event.data.kind === 'source-ack') sourceAck = true;
    else if (event.data.kind === 'port-ack') portAck = event.data.value === 'port-reply';
    else {{
      const index = event.data.index;
      rows.push({{
        index,
        target: event.target === container,
        currentTarget: event.currentTarget === container,
        source: event.source === controller && event.source instanceof ServiceWorker,
        sourceURL: event.source.scriptURL === new URL('worker.js', location.href).href,
        origin: event.origin === location.origin,
        lastEventId: event.lastEventId,
        ports: event.ports.length,
        port: index !== 0 || (event.ports[0] === event.data.port && event.ports[0] instanceof MessagePort)
      }});
      trace.push('message:' + index);
      Promise.resolve().then(() => trace.push('microtask:' + index));
      if (index === 0) {{
        event.ports[0].postMessage('port-reply');
        event.source.postMessage('reply');
      }}
    }}
    checkDone();
  }};
  const throwingGetter = () => {{ getterCalls++; throw Error('native notification read an author property'); }};
  if (mode === 'navigator-value') Object.defineProperty(globalThis, 'navigator', {{value: null, configurable: true}});
  if (mode === 'navigator-getter') Object.defineProperty(globalThis, 'navigator', {{get: throwingGetter, configurable: true}});
  if (mode === 'container-value') Object.defineProperty(savedNavigator, 'serviceWorker', {{value: null, configurable: true}});
  if (mode === 'container-getter') Object.defineProperty(savedNavigator, 'serviceWorker', {{get: throwingGetter, configurable: true}});
  if (mode === 'prototype-getter') Object.defineProperty(Object.getPrototypeOf(savedNavigator), 'serviceWorker', {{get: throwingGetter, configurable: true}});
  const registration = await container.register('worker.js', {{scope: './'}});
  await __moliTestActivatedServiceWorker(container);
  await changed;
  controller = container.controller;
  if (controller !== registration.active) throw Error('controller does not share the registration binding');
  controller.postMessage('burst');
  await messages;
  const valid = rows.every((row, index) => row.index === index && row.target && row.currentTarget &&
    row.source && row.sourceURL && row.origin && row.lastEventId === '' && row.ports === (index === 0 ? 1 : 0) && row.port);
  intrinsicContainerResult = JSON.stringify({{valid, getterCalls, globalMessages, controllerChanges, trace, portAck, sourceAck}});
}})().catch(error => intrinsicContainerResult = String(error));
            "#,
        ))
        .unwrap();
        drain_service_worker_test_until_eval_equals(
            &mut vm,
            &browser_context_runtime,
            &loader,
            "String(intrinsicContainerResult !== 'pending')",
            "true",
        )
        .await;
        assert_eq!(
            vm.eval("intrinsicContainerResult").unwrap(),
            r#"{"valid":true,"getterCalls":0,"globalMessages":0,"controllerChanges":1,"trace":["message:0","microtask:0","message:1","microtask:1"],"portAck":true,"sourceAck":true}"#,
            "{mode}"
        );
        server.await.unwrap();
    }
}
