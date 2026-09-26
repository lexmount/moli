use super::*;

#[tokio::test]
async fn navigator_service_worker_child_frame_controller_uses_child_client() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              const source = event.source;
              event.data.port.postMessage([
                "child-controller-reply",
                event.constructor && event.constructor.name,
                event instanceof ExtendableMessageEvent,
                event instanceof ExtendableEvent,
                event instanceof Event,
                Object.prototype.toString.call(event),
                event.origin,
                event.lastEventId,
                event.ports.length,
                event.ports[0] instanceof MessagePort,
                source && source.constructor && source.constructor.name,
                source instanceof WindowClient,
                source instanceof Client,
                source && source.type,
                source && source.frameType,
                source && source.url,
                source && source.visibilityState,
                source && source.focused
              ].join("|"));
            });
            "#,
        ),
        (
            "/app/frame.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>frame</title>",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerChildControllerProbe = "pending";
              (async () => {
                await navigator.serviceWorker.register("worker.js", { scope: "./" });
                await navigator.serviceWorker.ready;
                const frame = document.createElement("iframe");
                const loaded = new Promise((resolve, reject) => {
                  frame.onload = resolve;
                  frame.onerror = () => reject(new Error("frame load failed"));
                });
                frame.src = "frame.html";
                (document.body || document.documentElement || document).appendChild(frame);
                await loaded;
                const controller = frame.contentWindow.navigator.serviceWorker.controller;
                const channel = new MessageChannel();
                const reply = new Promise(resolve => {
                  channel.port1.onmessage = event => resolve(event.data);
                });
                globalThis.__serviceWorkerChildControllerProbe = [
                  "before-post",
                  controller === null,
                  controller instanceof ServiceWorker,
                  controller instanceof frame.contentWindow.ServiceWorker,
                  controller && controller.scriptURL
                ].join("|");
                controller.postMessage({ port: channel.port2 }, [channel.port2]);
                globalThis.__serviceWorkerChildControllerProbe = [
                  "after-reply",
                  controller === null,
                  controller instanceof ServiceWorker,
                  controller instanceof frame.contentWindow.ServiceWorker,
                  controller && controller.scriptURL,
                  await reply
                ].join("|");
              })().catch(error => {
                globalThis.__serviceWorkerChildControllerProbe =
                  "error:" + String(error && error.name) + ":" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker child frame controller probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected_frame_url = format!("{base_url}/app/frame.html");
    let expected = format!(
        "after-reply|false|false|true|{expected_worker_url}|child-controller-reply|ExtendableMessageEvent|true|true|true|[object ExtendableMessageEvent]|{base_url}||1|true|WindowClient|true|true|window|nested|{expected_frame_url}|visible|false"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerChildControllerProbe)",
        &expected,
    )
    .await;

    server
        .await
        .expect("service worker child controller server should finish");
}
#[tokio::test]
async fn navigator_service_worker_post_message_routes_to_waiting_and_active_versions() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.source.postMessage(event.data);
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("message", event => {
              event.source.postMessage(event.data);
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerWaitingPostMessageProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            const updated = await sw.register("worker.js", { scope: "./" });
            let lookup = await sw.getRegistration();
            for (let i = 0; i < 20 && !lookup.waiting; i++) {
              await new Promise(resolve => setTimeout(resolve, 0));
              lookup = await sw.getRegistration();
            }
            const waiting = lookup.waiting;
            const active = lookup.active;
            if (!waiting || !active) {
              throw new Error("expected waiting and active workers");
            }

            async function echo(worker, data) {
              return await new Promise(resolve => {
                function onMessage(event) {
                  sw.removeEventListener("message", onMessage);
                  resolve(event);
                }
                sw.addEventListener("message", onMessage);
                worker.postMessage(data);
              });
            }

            const waitingEvent = await echo(waiting, "waiting");
            const activeEvent = await echo(active, "active");
            globalThis.__serviceWorkerWaitingPostMessageProbe = [
              waitingEvent.data,
              waitingEvent.source === waiting,
              waitingEvent.source && waitingEvent.source.scriptURL,
              waitingEvent.source && waitingEvent.source.state,
              activeEvent.data,
              activeEvent.source === active,
              activeEvent.source && activeEvent.source.scriptURL,
              activeEvent.source && activeEvent.source.state,
              waiting !== active
            ].join("|");
          })().catch(error => {
            globalThis.__serviceWorkerWaitingPostMessageProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker waiting postMessage probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "waiting|true|{expected_worker_url}|installed|active|true|{expected_worker_url}|activated|true"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerWaitingPostMessageProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker waiting postMessage script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_post_message_routes_to_installing_version() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            let finishInstall;
            self.addEventListener("install", event => {
              event.waitUntil(new Promise(resolve => {
                finishInstall = resolve;
              }));
            });
            self.addEventListener("message", event => {
              event.source.postMessage([
                event.data,
                self.registration.installing && self.registration.installing.scriptURL,
                self.registration.installing && self.registration.installing.state
              ].join("|"));
              setTimeout(() => finishInstall(), 0);
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerInstallingPostMessageProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            let installingWorker = null;
            const messagePromise = new Promise(resolve => {
              registration.addEventListener("updatefound", () => {
                installingWorker = registration.installing;
                if (!installingWorker) {
                  resolve("missing-installing");
                  return;
                }
                function onMessage(event) {
                  sw.removeEventListener("message", onMessage);
                  resolve([
                    event.data,
                    event.source === installingWorker,
                    event.source && event.source.scriptURL,
                    event.source && event.source.state
                  ].join("|"));
                }
                sw.addEventListener("message", onMessage);
                installingWorker.postMessage("installing");
              }, { once: true });
            });
            const updatePromise = sw.register("worker.js", { scope: "./" });
            const result = await messagePromise;
            await updatePromise;
            globalThis.__serviceWorkerInstallingPostMessageProbe = result;
          })().catch(error => {
            globalThis.__serviceWorkerInstallingPostMessageProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker installing postMessage probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "installing|{expected_worker_url}|installing|true|{expected_worker_url}|installing"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerInstallingPostMessageProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker installing postMessage script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_installing_post_message_uses_transferred_message_port() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            let finishInstall;
            let port;
            self.addEventListener("install", event => {
              event.waitUntil(new Promise(resolve => {
                finishInstall = resolve;
              }));
            });
            self.onmessage = event => {
              const message = event.data;
              if (message && "port" in message) {
                port = message.port;
                port.postMessage([
                  "port-ready",
                  port instanceof MessagePort,
                  event.ports.length,
                  event.source && event.source.type,
                  self.registration.installing && self.registration.installing.state
                ].join("|"));
              }
            };
            self.addEventListener("message", event => {
              const message = event.data;
              if (message && "value" in message) {
                port.postMessage("Acking value: " + message.value);
                return;
              }
              if (message && "done" in message) {
                port.postMessage("quit");
                setTimeout(() => finishInstall(), 0);
              }
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerInstallingMessagePortProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;

            const resultPromise = new Promise(resolve => {
              registration.addEventListener("updatefound", () => {
                const installing = registration.installing;
                if (!installing) {
                  resolve("missing-installing");
                  return;
                }
                const stateWhenDiscovered = installing.state;
                const channel = new MessageChannel();
                const replies = [];
                channel.port1.onmessage = event => {
                  replies.push(event.data);
                  if (event.data === "quit") {
                    resolve([
                      stateWhenDiscovered,
                      replies.join(";")
                    ].join("|"));
                  }
                };
                installing.postMessage({ port: channel.port2 }, [channel.port2]);
                installing.postMessage({ value: 1 });
                installing.postMessage({ value: 2 });
                installing.postMessage({ done: true });
              }, { once: true });
            });
            const updatePromise = sw.register("worker.js", { scope: "./" });
            const result = await resultPromise;
            await updatePromise;
            globalThis.__serviceWorkerInstallingMessagePortProbe = result;
          })().catch(error => {
            globalThis.__serviceWorkerInstallingMessagePortProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker installing MessagePort probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerInstallingMessagePortProbe)",
        "installing|port-ready|true|1|window|installing;Acking value: 1;Acking value: 2;quit",
    )
    .await;
    server
        .await
        .expect("service worker installing MessagePort script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_installing_post_message_transfers_array_buffer() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            let finishInstall;
            const decoder = new TextDecoder();
            self.addEventListener("install", event => {
              event.waitUntil(new Promise(resolve => {
                finishInstall = resolve;
              }));
            });
            self.addEventListener("message", event => {
              if (event.data === "done") {
                setTimeout(() => finishInstall(), 0);
                return;
              }
              const text = decoder.decode(event.data);
              event.source.postMessage({
                content: text,
                byteLength: event.data.byteLength
              });
              if (text === "Hello dictionary") {
                event.source.postMessage(event.data, { transfer: [event.data.buffer] });
              } else {
                event.source.postMessage(event.data, [event.data.buffer]);
              }
              event.source.postMessage({
                content: decoder.decode(event.data),
                byteLength: event.data.byteLength
              });
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerInstallingArrayBufferProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;

            const resultPromise = new Promise(resolve => {
              registration.addEventListener("updatefound", () => {
                const installing = registration.installing;
                if (!installing) {
                  resolve("missing-installing");
                  return;
                }
                const encoder = new TextEncoder();
                const decoder = new TextDecoder();
                let activeResolve = null;
                let activeRecords = [];
                sw.onmessage = event => {
                  const data = event.data;
                  if (data && typeof data.content === "string") {
                    activeRecords.push(data.content + ":" + data.byteLength);
                  } else {
                    activeRecords.push(decoder.decode(data) + ":" + data.byteLength);
                  }
                  if (activeRecords.length === 3 && activeResolve) {
                    const resolveRecords = activeResolve;
                    const records = activeRecords;
                    activeResolve = null;
                    activeRecords = [];
                    resolveRecords(records);
                  }
                };
                async function transfer(label, message, transferArgument) {
                  const bytes = encoder.encode(message);
                  const before = bytes.byteLength;
                  const recordsPromise = new Promise(resolve => {
                    activeResolve = resolve;
                  });
                  installing.postMessage(bytes, transferArgument(bytes));
                  const detached = decoder.decode(bytes) === "" && bytes.byteLength === 0;
                  const records = await recordsPromise;
                  return [
                    label,
                    before,
                    detached,
                    records.join(",")
                  ].join("|");
                }
                (async () => {
                  const list = await transfer(
                    "list",
                    "Hello list",
                    bytes => [bytes.buffer]
                  );
                  const dictionary = await transfer(
                    "dictionary",
                    "Hello dictionary",
                    bytes => ({ transfer: [bytes.buffer] })
                  );
                  installing.postMessage("done");
                  resolve([installing.state, list, dictionary].join(";"));
                })().catch(error => resolve("error:" + String(error)));
              }, { once: true });
            });
            const updatePromise = sw.register("worker.js", { scope: "./" });
            const result = await resultPromise;
            await updatePromise;
            globalThis.__serviceWorkerInstallingArrayBufferProbe = result;
          })().catch(error => {
            globalThis.__serviceWorkerInstallingArrayBufferProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker installing ArrayBuffer transfer probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerInstallingArrayBufferProbe)",
        "installing;list|10|true|Hello list:10,Hello list:10,:0;dictionary|16|true|Hello dictionary:16,Hello dictionary:16,:0",
    )
    .await;
    server
        .await
        .expect("service worker installing ArrayBuffer transfer script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_installing_message_port_transfers_array_buffer() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            let finishInstall;
            const decoder = new TextDecoder();
            self.addEventListener("install", event => {
              event.waitUntil(new Promise(resolve => {
                finishInstall = resolve;
              }));
            });
            self.addEventListener("message", event => {
              const port = event.ports[0];
              if (!port) {
                event.source.postMessage("missing-port");
                return;
              }
              port.onmessage = portEvent => {
                if (portEvent.data === "done") {
                  setTimeout(() => finishInstall(), 0);
                  return;
                }
                if (portEvent.data === "queued-before-ready") {
                  port.postMessage("queued-before-ready:received");
                  return;
                }
                port.postMessage({
                  content: decoder.decode(portEvent.data),
                  byteLength: portEvent.data.byteLength
                });
                port.postMessage(portEvent.data, [portEvent.data.buffer]);
                port.postMessage({
                  content: decoder.decode(portEvent.data),
                  byteLength: portEvent.data.byteLength
                });
              };
              port.postMessage([
                "port-ready",
                port instanceof MessagePort,
                event.ports.length,
                event.source && event.source.type,
                self.registration.installing && self.registration.installing.state
              ].join("|"));
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerMessagePortArrayBufferProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;

            const resultPromise = new Promise(resolve => {
              registration.addEventListener("updatefound", () => {
                const installing = registration.installing;
                if (!installing) {
                  resolve("missing-installing");
                  return;
                }
                const encoder = new TextEncoder();
                const decoder = new TextDecoder();
                const channel = new MessageChannel();
                const records = [];
                channel.port1.onmessage = event => {
                  const data = event.data;
                  if (typeof data === "string") {
                    records.push(data);
                    if (data.startsWith("port-ready")) {
                      const bytes = encoder.encode("Hello port");
                      records.push("page-before:" + bytes.byteLength);
                      channel.port1.postMessage(bytes, [bytes.buffer]);
                      records.push([
                        "page-detached",
                        decoder.decode(bytes) === "",
                        bytes.byteLength
                      ].join(":"));
                    }
                    return;
                  }
                  if (data && typeof data.content === "string") {
                    records.push(data.content + ":" + data.byteLength);
                  } else {
                    records.push(decoder.decode(data) + ":" + data.byteLength);
                  }
                  if (records.length === 7) {
                    channel.port1.postMessage("done");
                    resolve([installing.state, records.join(";")].join("|"));
                  }
                };
                installing.postMessage(undefined, [channel.port2]);
                channel.port1.postMessage("queued-before-ready");
              }, { once: true });
            });
            const updatePromise = sw.register("worker.js", { scope: "./" });
            const result = await resultPromise;
            await updatePromise;
            globalThis.__serviceWorkerMessagePortArrayBufferProbe = result;
          })().catch(error => {
            globalThis.__serviceWorkerMessagePortArrayBufferProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker MessagePort ArrayBuffer probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerMessagePortArrayBufferProbe)",
        "installing|port-ready|true|1|window|installing;page-before:10;page-detached:true:0;queued-before-ready:received;Hello port:10;Hello port:10;:0",
    )
    .await;
    server
        .await
        .expect("service worker MessagePort ArrayBuffer script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_installing_post_message_transfers_dataview() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            let finishInstall;
            self.addEventListener("install", event => {
              event.waitUntil(new Promise(resolve => {
                finishInstall = resolve;
              }));
            });
            self.addEventListener("message", event => {
              const view = event.data;
              event.source.postMessage([
                "worker-before",
                view.constructor.name,
                view.byteOffset,
                view.byteLength,
                view.getUint16(0),
                view.getUint16(2),
                self.registration.installing && self.registration.installing.state
              ].join("|"));
              event.source.postMessage(view, [view.buffer]);
              event.source.postMessage([
                "worker-after",
                view.buffer.byteLength
              ].join("|"));
              setTimeout(() => finishInstall(), 0);
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerDataViewTransferProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;

            const resultPromise = new Promise(resolve => {
              registration.addEventListener("updatefound", () => {
                const installing = registration.installing;
                if (!installing) {
                  resolve("missing-installing");
                  return;
                }
                const records = [];
                sw.onmessage = event => {
                  const data = event.data;
                  if (data instanceof DataView) {
                    records.push([
                      "page-return",
                      data.constructor.name,
                      data.byteOffset,
                      data.byteLength,
                      data.getUint16(0),
                      data.getUint16(2),
                      data.buffer.byteLength
                    ].join("|"));
                  } else {
                    records.push(String(data));
                  }
                  if (records.length === 4) {
                    resolve(records.join(";"));
                  }
                };
                const buffer = new ArrayBuffer(8);
                const view = new DataView(buffer, 2, 4);
                view.setUint16(0, 0x1234);
                view.setUint16(2, 0x5678);
                installing.postMessage(view, [buffer]);
                records.push([
                  "page-after",
                  buffer.byteLength
                ].join("|"));
              }, { once: true });
            });
            const updatePromise = sw.register("worker.js", { scope: "./" });
            const result = await resultPromise;
            await updatePromise;
            globalThis.__serviceWorkerDataViewTransferProbe = result;
          })().catch(error => {
            globalThis.__serviceWorkerDataViewTransferProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker DataView transfer probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerDataViewTransferProbe)",
        "page-after|0;worker-before|DataView|2|4|4660|22136|installing;page-return|DataView|2|4|4660|22136|8;worker-after|0",
    )
    .await;
    server
        .await
        .expect("service worker DataView transfer script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_global_post_message_routes_between_worker_versions() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker-v1.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              if (event.data === "loopback") {
                const pagePort = event.ports[0];
                const channel = new MessageChannel();
                channel.port1.onmessage = reply => pagePort.postMessage(reply.data);
                self.registration.active.postMessage({ ping: channel.port2 }, [channel.port2]);
                return;
              }
              if (event.data === "waiting") {
                const pagePort = event.ports[0];
                const channel = new MessageChannel();
                channel.port1.onmessage = reply => pagePort.postMessage(reply.data);
                self.registration.waiting.postMessage({ ping: channel.port2 }, [channel.port2]);
                return;
              }
              if (event.data === "worker-arraybuffer") {
                const pagePort = event.ports[0];
                const channel = new MessageChannel();
                const bytes = new Uint8Array([4, 5, 6]);
                const replies = [];
                channel.port1.onmessage = reply => {
                  if (reply.data instanceof Uint8Array) {
                    replies.push([
                      "active-return",
                      reply.data.constructor.name,
                      reply.data.byteLength,
                      Array.from(reply.data).join("-")
                    ].join(":"));
                  } else {
                    replies.push(String(reply.data));
                  }
                  if (replies.length === 2) {
                    pagePort.postMessage([
                      "worker-arraybuffer",
                      bytes.byteLength,
                      replies.join(";")
                    ].join("|"));
                  }
                };
                self.registration.active.postMessage({
                  kind: "worker-arraybuffer-target",
                  bytes,
                  reply: channel.port2
                }, { transfer: [bytes.buffer, channel.port2] });
                return;
              }
              if (event.data === "worker-transfer-failures") {
                const pagePort = event.ports[0];
                const results = [];
                const record = (label, callback, byteLength = () => "n/a") => {
                  let errorName = "no-error";
                  let isDataCloneError = false;
                  try {
                    callback();
                  } catch (error) {
                    errorName = error && error.name;
                    isDataCloneError =
                      error instanceof DOMException && error.name === "DataCloneError";
                  }
                  results.push([
                    label,
                    errorName,
                    isDataCloneError,
                    byteLength()
                  ].join(":"));
                };
                record("invalid-entry", () => {
                  self.registration.active.postMessage(
                    "transfer-failure-should-not-arrive",
                    [new Uint8Array([9])]
                  );
                });
                const duplicateBuffer = new ArrayBuffer(1);
                record("duplicate-arraybuffer", () => {
                  self.registration.active.postMessage(
                    "transfer-failure-should-not-arrive",
                    [duplicateBuffer, duplicateBuffer]
                  );
                }, () => duplicateBuffer.byteLength);
                const detachedBuffer = new ArrayBuffer(1);
                const channel = new MessageChannel();
                channel.port1.postMessage(null, [detachedBuffer]);
                record("detached-arraybuffer", () => {
                  self.registration.active.postMessage(
                    "transfer-failure-should-not-arrive",
                    [detachedBuffer]
                  );
                }, () => detachedBuffer.byteLength);
                const duplicatePortChannel = new MessageChannel();
                record("duplicate-messageport", () => {
                  self.registration.active.postMessage(
                    "transfer-failure-should-not-arrive",
                    [duplicatePortChannel.port2, duplicatePortChannel.port2]
                  );
                });
                const detachedPortChannel = new MessageChannel();
                channel.port1.postMessage(null, [detachedPortChannel.port2]);
                record("detached-messageport", () => {
                  self.registration.active.postMessage(
                    "transfer-failure-should-not-arrive",
                    [detachedPortChannel.port2]
                  );
                });
                record("source-messageport", () => {
                  pagePort.postMessage(
                    "transfer-failure-should-not-arrive",
                    [pagePort]
                  );
                });
                try {
                  self.registration.active.postMessage(
                    { kind: "transfer-failure-dictionary" },
                    { transfer: [new Uint8Array([8])] }
                  );
                } catch (error) {
                  results.push([
                    "dictionary-invalid-entry",
                    error && error.name,
                    error instanceof DOMException && error.name === "DataCloneError",
                    "n/a"
                  ].join(":"));
                }
                pagePort.postMessage([
                  "worker-transfer-failures",
                  results.join("|")
                ].join("|"));
                return;
              }
              if (event.data && event.data.ping) {
                event.data.ping.postMessage([
                  "active-loopback",
                  event.source instanceof ServiceWorker,
                  event.source && event.source.scriptURL,
                  event.source && event.source.state
                ].join("|"));
                return;
              }
              if (event.data === "transfer-failure-should-not-arrive" ||
                  event.data && event.data.kind === "transfer-failure-dictionary") {
                throw new Error("failed transfer should not dispatch");
              }
              if (event.data && event.data.kind === "worker-arraybuffer-target") {
                const bytes = event.data.bytes;
                event.data.reply.postMessage(bytes, [bytes.buffer]);
                event.data.reply.postMessage([
                  "active-target-after",
                  bytes.byteLength,
                  event.source instanceof ServiceWorker,
                  event.source && event.source.state
                ].join(":"));
              }
            });
            "#,
        ),
        (
            "/app/worker-v2.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("message", event => {
              if (event.data && event.data.ping) {
                event.data.ping.postMessage([
                  "waiting-pong",
                  event.source instanceof ServiceWorker,
                  event.source && event.source.scriptURL,
                  event.source && event.source.state,
                  self.registration.waiting && self.registration.waiting.scriptURL,
                  self.registration.waiting && self.registration.waiting.state
                ].join("|"));
              }
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerGlobalPostMessageProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker-v1.js", { scope: "./" });
            await sw.ready;
            await sw.register("worker-v2.js", { scope: "./" });
            let lookup = await sw.getRegistration();
            for (let i = 0; i < 20 && !lookup.waiting; i++) {
              await new Promise(resolve => setTimeout(resolve, 0));
              lookup = await sw.getRegistration();
            }
            const active = lookup.active;
            const waiting = lookup.waiting;
            if (!active || !waiting) {
              throw new Error("expected active and waiting workers");
            }

            async function ask(worker, data) {
              return await new Promise(resolve => {
                const channel = new MessageChannel();
                channel.port1.onmessage = event => resolve(event.data);
                worker.postMessage(data, [channel.port2]);
              });
            }

            const loopback = await ask(active, "loopback");
            const waitingReply = await ask(active, "waiting");
            const workerArrayBuffer = await ask(active, "worker-arraybuffer");
            const workerTransferFailures = await ask(active, "worker-transfer-failures");
            globalThis.__serviceWorkerGlobalPostMessageProbe = [
              loopback,
              waitingReply,
              workerArrayBuffer,
              workerTransferFailures
            ].join(";");
          })().catch(error => {
            globalThis.__serviceWorkerGlobalPostMessageProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker global postMessage probe should evaluate");

    let expected_v1_url = format!("{base_url}/app/worker-v1.js");
    let expected_v2_url = format!("{base_url}/app/worker-v2.js");
    let expected = format!(
        "active-loopback|true|{expected_v1_url}|activated;waiting-pong|true|{expected_v1_url}|activated|{expected_v2_url}|installed;worker-arraybuffer|0|active-return:Uint8Array:3:4-5-6;active-target-after:0:true:activated;worker-transfer-failures|invalid-entry:DataCloneError:true:n/a|duplicate-arraybuffer:DataCloneError:true:1|detached-arraybuffer:DataCloneError:true:0|duplicate-messageport:DataCloneError:true:n/a|detached-messageport:DataCloneError:true:n/a|source-messageport:DataCloneError:true:n/a|dictionary-invalid-entry:DataCloneError:true:n/a"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerGlobalPostMessageProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker global postMessage script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_post_message_to_redundant_worker_is_dropped() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker-v1.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("message", event => {
              event.source.postMessage("v1:" + event.data);
            });
            "#,
        ),
        (
            "/app/worker-v2.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("message", event => {
              event.source.postMessage("v2:" + event.data);
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerRedundantPostMessageProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker-v1.js", { scope: "./" });
            await sw.ready;
            const firstWorker = registration.active;
            if (!firstWorker) {
              throw new Error("expected first active worker");
            }
            const firstRedundant = firstWorker.state === "redundant"
              ? Promise.resolve()
              : new Promise(resolve => {
                  firstWorker.addEventListener("statechange", () => {
                    if (firstWorker.state === "redundant") {
                      resolve();
                    }
                  });
                });
            await sw.register("worker-v2.js", { scope: "./" });
            await firstRedundant;
            let lookup = await sw.getRegistration();
            for (let i = 0; i < 20 && lookup.active === firstWorker; i++) {
              await new Promise(resolve => setTimeout(resolve, 0));
              lookup = await sw.getRegistration();
            }
            const secondWorker = lookup.active;
            if (!secondWorker || secondWorker === firstWorker) {
              throw new Error("expected replacement active worker");
            }

            const messages = [];
            sw.addEventListener("message", event => {
              messages.push(event.data);
              if (messages.length === 1) {
                globalThis.__serviceWorkerRedundantPostMessageProbe = [
                  firstWorker.state,
                  secondWorker.state,
                  messages.join(",")
                ].join("|");
              }
            });
            firstWorker.postMessage("old");
            secondWorker.postMessage("new");
          })().catch(error => {
            globalThis.__serviceWorkerRedundantPostMessageProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker redundant postMessage probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRedundantPostMessageProbe)",
        "redundant|activated|v2:new",
    )
    .await;
    server
        .await
        .expect("service worker redundant postMessage script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_client_message_round_trips_from_event_source() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
              Promise.resolve().then(() => {
                event.waitUntil(Promise.resolve().then(() => {
                  self.__installMicrotaskWaitUntil = "install-microtask-ok";
                }));
              });
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              let resolvePending;
              const pending = new Promise(resolve => {
                resolvePending = resolve;
              });
              event.waitUntil(pending);
              pending.then(() => {
                event.waitUntil(Promise.resolve().then(() => {
                  event.source.postMessage("same-turn-pending-ok");
                }));
              });
              setTimeout(resolvePending, 0);
              Promise.resolve().then(() => {
                event.waitUntil(Promise.resolve().then(() => {
                  event.source.postMessage("message-microtask-ok");
                }));
              });
              event.waitUntil(Promise.resolve().then(() => {
                event.source.postMessage([
                  "reply",
                  event.data,
                  event.origin,
                  event.source && event.source.url,
                  event.source && event.source.type,
                  self.__installMicrotaskWaitUntil || "install-missing"
                ].join("|"));
              }));
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerClientMessageProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const messages = [];
            sw.onmessage = event => {
              messages.push([
                event.data,
                event.origin,
                event.source && event.source.scriptURL,
                event.source && event.source.state
              ].join(";"));
              if (messages.length === 3) {
                globalThis.__serviceWorkerClientMessageProbe = messages.sort().join("||");
              }
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            registration.active.postMessage("ping");
          })().catch(error => {
            globalThis.__serviceWorkerClientMessageProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker client message probe should evaluate");

    let expected_origin = base_url.as_str();
    let expected_page_url = format!("{base_url}/app/page.html");
    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "message-microtask-ok;{expected_origin};{expected_worker_url};activated||reply|ping|{expected_origin}|{expected_page_url}|window|install-microtask-ok;{expected_origin};{expected_worker_url};activated||same-turn-pending-ok;{expected_origin};{expected_worker_url};activated"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerClientMessageProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker client message script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_client_message_transfers_readable_stream_to_page() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              if (event.data !== "SEND") {
                event.waitUntil((async () => {
                  const reader = event.data.getReader();
                  const first = await reader.read();
                  const second = await reader.read();
                  event.source.postMessage([
                    "OK",
                    first.value,
                    first.done,
                    second.done
                  ].join(":"));
                })());
                return;
              }
              const stream = new ReadableStream({
                start(controller) {
                  controller.enqueue("a");
                  controller.close();
                }
              });
              event.source.postMessage(stream, [stream]);
            });
            "#,
        ),
        (
            "/app/frame.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>stream client</title>",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerReadableTransferProbe = "registering";
          (async () => {
            await navigator.serviceWorker.register("worker.js", { scope: "./" });
            await navigator.serviceWorker.ready;
            const frame = document.createElement("iframe");
            const loaded = new Promise((resolve, reject) => {
              frame.onload = resolve;
              frame.onerror = () => reject(new Error("frame load failed"));
            });
            frame.src = "frame.html";
            (document.body || document.documentElement || document).appendChild(frame);
            await loaded;
            const client = frame.contentWindow;
            const serviceWorkers = client.navigator.serviceWorker;
            const controller = serviceWorkers.controller;
            serviceWorkers.onmessage = event => {
              if (event.data !== "OK:a:false:true") {
                return;
              }
              globalThis.__serviceWorkerReadableTransferProbe = "first-direction-done";
              serviceWorkers.addEventListener("message", async event => {
                if (typeof event.data === "string") {
                  return;
                }
                const hasChildRealm =
                  event.data.constructor === client.ReadableStream &&
                  event.data instanceof client.ReadableStream;
                globalThis.__serviceWorkerReadableTransferProbe =
                  "received:" + hasChildRealm;
                try {
                  const reader = event.data.getReader();
                  const first = await reader.read();
                  globalThis.__serviceWorkerReadableTransferProbe =
                    "first:" + first.value + ":" + first.done;
                  const second = await reader.read();
                  globalThis.__serviceWorkerReadableTransferProbe =
                    "done:" + second.done + ":child-realm:" + hasChildRealm;
                } catch (error) {
                  globalThis.__serviceWorkerReadableTransferProbe =
                    "error:" + error.name + ":" + error.message;
                }
              }, { once: true });
              controller.postMessage("SEND");
            };
            const stream = new client.ReadableStream({
              start(controller) {
                controller.enqueue("a");
                controller.close();
              }
            });
            controller.postMessage(stream, [stream]);
          })().catch(error => {
            globalThis.__serviceWorkerReadableTransferProbe =
              "setup-error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker ReadableStream transfer probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerReadableTransferProbe)",
        "done:true:child-realm:true",
    )
    .await;
    server
        .await
        .expect("service worker ReadableStream script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_message_ports_transfer_both_directions() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil((async () => {
                if (event.data === "page-to-worker-port") {
                  const port = event.ports[0];
                  port.postMessage([
                    "from-worker",
                    event.ports.length,
                    event.origin,
                    event.source && event.source.type
                  ].join("|"));
                  return;
                }
                if (event.data === "closed-port-delivery") {
                  const port = event.ports[0];
                  port.postMessage("closed-target-delivered");
                  event.source.postMessage("closed-port-attempted");
                  return;
                }
                if (event.data === "worker-to-page-port") {
                  const channel = new MessageChannel();
                  channel.port1.onmessage = portEvent => {
                    channel.port1.postMessage("from-worker-port:" + portEvent.data);
                  };
                  event.source.postMessage("worker-port-ready", [channel.port2]);
                }
              })());
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerMessagePortProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const results = [];
            let closedDelivered = false;
            sw.onmessage = event => {
              if (event.data === "closed-port-attempted") {
                results.push("closed-port-attempted");
                results.push("closed-port-delivered:" + closedDelivered);
                registration.active.postMessage("worker-to-page-port");
                return;
              }
              if (event.data !== "worker-port-ready") {
                results.push("unexpected-sw-message:" + event.data);
                return;
              }
              const port = event.ports[0];
              results.push([
                "worker-to-page",
                event.ports.length,
                event.origin,
                event.source && event.source.state
              ].join("|"));
              port.onmessage = portEvent => {
                if (portEvent.data === "from-worker-port:ping-page-port") {
                  results.push(portEvent.data);
                  globalThis.__serviceWorkerMessagePortProbe = results.join(";");
                  return;
                }
                results.push("unexpected-port-message:" + String(portEvent.data));
                globalThis.__serviceWorkerMessagePortProbe = results.join(";");
              };
              port.postMessage("ping-page-port");
            };

            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;

            const channel = new MessageChannel();
            channel.port1.onmessage = event => {
              results.push("page-to-worker|" + event.data);
              const closedChannel = new MessageChannel();
              closedChannel.port1.onmessage = () => {
                closedDelivered = true;
              };
              closedChannel.port1.close();
              registration.active.postMessage(
                "closed-port-delivery",
                [closedChannel.port2]
              );
            };
            registration.active.postMessage("page-to-worker-port", [channel.port2]);
          })().catch(error => {
            globalThis.__serviceWorkerMessagePortProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker message port probe should evaluate");

    let expected_origin = base_url.as_str();
    let expected = format!(
        "page-to-worker|from-worker|1|{expected_origin}|window;closed-port-attempted;closed-port-delivered:false;worker-to-page|1|{expected_origin}|activated;from-worker-port:ping-page-port"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerMessagePortProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker message port script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_wasm_module_cross_agent_messages_fire_messageerror() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              if (event.data === "send-worker-module") {
                event.source.postMessage(new WebAssembly.Module(bytes));
              }
            });
            self.addEventListener("messageerror", event => {
              event.source.postMessage([
                "worker-messageerror",
                event.constructor.name,
                event.data === null,
                event.origin,
                event.source && event.source.constructor.name,
                event.ports.length
              ].join("|"));
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerWasmModuleMessageProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessageerror = event => {
              globalThis.__serviceWorkerWasmModuleMessageProbe += ";" + [
                "window-messageerror",
                event.data === null,
                event.origin,
                event.source && event.source.scriptURL,
                event.source && event.source.state,
                event.ports.length
              ].join("|");
            };
            sw.onmessage = event => {
              globalThis.__serviceWorkerWasmModuleMessageProbe = event.data;
              try {
                registration.active.postMessage("send-worker-module");
                globalThis.__serviceWorkerWasmModuleMessageProbe += ";posted";
              } catch (error) {
                globalThis.__serviceWorkerWasmModuleMessageProbe +=
                  ";post-error:" + error.constructor.name + ":" + error.message;
              }
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            const pageModule = new WebAssembly.Module(
              new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
            );
            registration.active.postMessage(pageModule);
          })().catch(error => {
            globalThis.__serviceWorkerWasmModuleMessageProbe =
              "error:" + String(error && error.name) + ":" + String(error && error.message);
          });
        })()
        "#,
    )
    .expect("service worker wasm module message probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "worker-messageerror|ExtendableMessageEvent|true|{base_url}|WindowClient|0;posted;window-messageerror|true|{base_url}|{expected_worker_url}|activated|0"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerWasmModuleMessageProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker wasm module message script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_message_port_wasm_module_to_sandbox_fires_messageerror() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil(Promise.resolve().then(() => {
                if (event.data !== "send-wasm-over-port") {
                  event.source.postMessage("unexpected-worker-message:" + event.data);
                  return;
                }
                const port = event.ports[0];
                const module = new WebAssembly.Module(bytes);
                port.postMessage({ kind: "worker-module", module });
                event.source.postMessage("worker-sent-module");
              }));
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerSandboxPortMessageErrorProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;

            let frame;
            let resolveFrameReady;
            let resolvePortReady;
            let resolvePortMessageError;
            const frameReady = new Promise(resolve => { resolveFrameReady = resolve; });
            const portReady = new Promise(resolve => { resolvePortReady = resolve; });
            const portMessageError = new Promise(resolve => {
              resolvePortMessageError = resolve;
            });
            addEventListener("message", event => {
              if (!frame || event.source !== frame.contentWindow) {
                return;
              }
              const data = event.data;
              if (data && data.kind === "frame-ready") {
                resolveFrameReady(true);
                return;
              }
              if (data && data.kind === "port-ready") {
                resolvePortReady(true);
                return;
              }
              if (data && data.kind === "port-messageerror") {
                resolvePortMessageError(data);
                return;
              }
              globalThis.__serviceWorkerSandboxPortMessageErrorProbe =
                "unexpected-frame-message:" + JSON.stringify(data);
            });

            frame = document.createElement("iframe");
            frame.setAttribute("sandbox", "allow-scripts");
            frame.srcdoc = `
              <script>
                onmessage = event => {
                  const port = event.ports[0];
                  port.onmessage = event => {
                    parent.postMessage({
                      kind: "unexpected-port-message",
                      module: event.data && event.data.module instanceof WebAssembly.Module
                    }, "*");
                  };
                  port.onmessageerror = event => {
                    parent.postMessage({
                      kind: "port-messageerror",
                      data: event.data,
                      origin: event.origin,
                      source: event.source,
                      ports: event.ports.length
                    }, "*");
                  };
                  port.start();
                  parent.postMessage({ kind: "port-ready" }, "*");
                };
                parent.postMessage({ kind: "frame-ready" }, "*");
              </` + `script>`;
            (document.body || document.documentElement || document).appendChild(frame);
            await frameReady;

            const channel = new MessageChannel();
            frame.contentWindow.postMessage("bind-port", "*", [channel.port2]);
            await portReady;

            const workerAck = new Promise(resolve => {
              sw.onmessage = event => {
                resolve({
                  data: event.data,
                  origin: event.origin,
                  sourceState: event.source && event.source.state
                });
              };
            });
            registration.active.postMessage("send-wasm-over-port", [channel.port1]);
            const outcome = await Promise.all([workerAck, portMessageError]);
            globalThis.__serviceWorkerSandboxPortMessageErrorProbe = JSON.stringify({
              frameReady: true,
              portReady: true,
              workerAck: outcome[0],
              messageError: outcome[1]
            });
          })().catch(error => {
            globalThis.__serviceWorkerSandboxPortMessageErrorProbe =
              "error:" + String(error && error.name) + ":" + String(error && error.message);
          });
        })()
        "#,
    )
    .expect("service worker sandbox wasm module MessagePort probe should evaluate");

    let expected = format!(
        r#"{{"frameReady":true,"portReady":true,"workerAck":{{"data":"worker-sent-module","origin":"{base_url}","sourceState":"activated"}},"messageError":{{"kind":"port-messageerror","data":null,"origin":"","source":null,"ports":0}}}}"#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSandboxPortMessageErrorProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker sandbox wasm MessagePort script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_clients_query_live_window_clients() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil((async () => {
                const controlled = await clients.matchAll();
                const all = await clients.matchAll({ includeUncontrolled: true, type: "all" });
                const first = all[0] && await clients.get(all[0].id);
                let focusError = null;
                try {
                  if (first) {
                    await first.focus();
                  }
                } catch (error) {
                  focusError = {
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                  };
                }
                let openWindowError = null;
                try {
                  await clients.openWindow("./opened.html");
                } catch (error) {
                  openWindowError = {
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                  };
                }
                let navigateSettled = false;
                const navigatePromise = first && first.navigate("./next.html");
                if (navigatePromise) {
                  navigatePromise.then(
                    () => { navigateSettled = true; },
                    () => { navigateSettled = true; }
                  );
                }
                const workerClients = await clients.matchAll({
                  includeUncontrolled: true,
                  type: "worker"
                });
                event.source.postMessage(JSON.stringify({
                  controlledLength: controlled.length,
                  allLength: all.length,
                  firstId: first && first.id,
                  firstIdIsInternal: first && first.id === "1",
                  firstUrl: first && first.url,
                  firstType: first && first.type,
                  firstFrameType: first && first.frameType,
                  firstLifecycleState: first && first.lifecycleState,
                  firstVisibilityState: first && first.visibilityState,
                  firstFocused: first && first.focused,
                  firstPostMessage: typeof (first && first.postMessage),
                  firstFocus: typeof (first && first.focus),
                  firstNavigate: typeof (first && first.navigate),
                  clientsOpenWindow: typeof clients.openWindow,
                  focusError,
                  openWindowError,
                  navigatePromiseThen: typeof (navigatePromise && navigatePromise.then),
                  navigateSettled,
                  sameId: first && first.id === all[0].id,
                  workerLength: workerClients.length
                }));
              })());
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html#client-fragment"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerClientsQueryProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerClientsQueryProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            registration.active.postMessage("query-clients");
          })().catch(error => {
            globalThis.__serviceWorkerClientsQueryProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker clients query probe should evaluate");

    let expected_page_url = format!("{base_url}/app/page.html#client-fragment");
    let expected = format!(
        r#"{{"controlledLength":1,"allLength":1,"firstId":"client-0000000000000001","firstIdIsInternal":false,"firstUrl":"{expected_page_url}","firstType":"window","firstFrameType":"top-level","firstLifecycleState":"active","firstVisibilityState":"visible","firstFocused":false,"firstPostMessage":"function","firstFocus":"function","firstNavigate":"function","clientsOpenWindow":"function","focusError":{{"name":"InvalidAccessError","message":"Not allowed to focus a window.","isDomException":true}},"openWindowError":{{"name":"InvalidAccessError","message":"Not allowed to open a window.","isDomException":true}},"navigatePromiseThen":"function","navigateSettled":false,"sameId":true,"workerLength":0}}"#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerClientsQueryProbe)",
        &expected,
    )
    .await;
    let pending_navigation = vm
        .take_pending_location_navigation_with_seed()
        .expect("WindowClient.navigate should record page-owner pending navigation");
    assert_eq!(
        pending_navigation.url,
        url::Url::parse(&format!("{base_url}/app/next.html")).unwrap()
    );
    assert!(pending_navigation.entry_seed.is_none());
    let continuation = pending_navigation
        .service_worker_client_navigate
        .expect("WindowClient.navigate should wait for page-owner navigation completion");
    assert_eq!(continuation.request_id, 1);
    server
        .await
        .expect("service worker clients query script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_clients_query_nested_frame_type() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil((async () => {
                const windows = await clients.matchAll({
                  includeUncontrolled: true,
                  type: "window"
                });
                if (event.data === "navigate-nested") {
                  const nested = windows.find(client => client.frameType === "nested");
                  const navigated = nested && await nested.navigate("./next-frame.html");
                  const afterNavigate = await clients.matchAll({
                    includeUncontrolled: true,
                    type: "window"
                  });
                  const rows = afterNavigate.map(client => ({
                    url: client.url,
                    type: client.type,
                    frameType: client.frameType
                  })).sort((left, right) => left.url.localeCompare(right.url));
                  event.source.postMessage(JSON.stringify({
                    navigatedUrl: navigated && navigated.url,
                    navigatedFrameType: navigated && navigated.frameType,
                    rows
                  }));
                  return;
                }
                const rows = windows.map(client => ({
                  url: client.url,
                  type: client.type,
                  frameType: client.frameType
                })).sort((left, right) => left.url.localeCompare(right.url));
                event.source.postMessage(JSON.stringify(rows));
              })());
            });
            "#,
        ),
        (
            "/app/frame.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>frame</title><body>frame</body>",
        ),
        (
            "/app/next-frame.html",
            "text/html; charset=utf-8",
            "<!doctype html><title>next frame</title><body>next frame</body>",
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerNestedFrameTypeProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerNestedFrameTypeProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            globalThis.__serviceWorkerNestedFrameRegistration = registration;
            const frame = document.createElement("iframe");
            frame.src = "./frame.html";
            (document.body || document.documentElement || document).appendChild(frame);
            globalThis.__serviceWorkerNestedFrameTypeProbe = "frame-appended";
          })().catch(error => {
            globalThis.__serviceWorkerNestedFrameTypeProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker nested frameType probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNestedFrameTypeProbe)",
        "frame-appended",
    )
    .await;
    let pending_diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(pending_diagnostics.live_client_count, 2);
    assert_eq!(pending_diagnostics.controlled_client_count, 2);
    let pending_clients = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 401,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll {
                options: crate::runtime::ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: crate::runtime::ServiceWorkerClientQueryType::Window,
                },
            },
        });
    assert_eq!(
        pending_clients
            .clients
            .iter()
            .map(|client| (client.url.to_string(), client.frame_type.as_webidl_str()))
            .collect::<Vec<_>>(),
        vec![(format!("{base_url}/app/page.html"), "top-level")]
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        r#"String(document.querySelector("iframe").contentDocument.title)"#,
        "frame",
    )
    .await;
    let committed_diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(committed_diagnostics.live_client_count, 2);
    assert_eq!(committed_diagnostics.controlled_client_count, 2);
    vm.eval(r#"__serviceWorkerNestedFrameRegistration.active.postMessage("query-frame-types")"#)
        .expect("service worker nested frameType query should post");

    let expected = format!(
        r#"[{{"url":"{base_url}/app/frame.html","type":"window","frameType":"nested"}},{{"url":"{base_url}/app/page.html","type":"window","frameType":"top-level"}}]"#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNestedFrameTypeProbe)",
        &expected,
    )
    .await;
    vm.eval(
        r#"
        globalThis.__serviceWorkerNestedFrameTypeProbe = "navigate-pending";
        __serviceWorkerNestedFrameRegistration.active.postMessage("navigate-nested");
        "#,
    )
    .expect("service worker nested WindowClient.navigate should post");
    let expected = format!(
        r#"{{"navigatedUrl":"{base_url}/app/next-frame.html","navigatedFrameType":"nested","rows":[{{"url":"{base_url}/app/next-frame.html","type":"window","frameType":"nested"}},{{"url":"{base_url}/app/page.html","type":"window","frameType":"top-level"}}]}}"#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNestedFrameTypeProbe)",
        &expected,
    )
    .await;
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        r#"String(document.querySelector("iframe").contentDocument.title)"#,
        "next frame",
    )
    .await;

    server
        .await
        .expect("service worker nested frameType script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_window_client_navigate_rejects_during_pending_navigation() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil((async () => {
                const all = await clients.matchAll({ includeUncontrolled: true });
                let navigateError = null;
                try {
                  await all[0].navigate("./sw-next.html");
                } catch (error) {
                  navigateError = {
                    name: error && error.name,
                    message: error && error.message
                  };
                }
                event.source.postMessage(JSON.stringify({ navigateError }));
              })());
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerNavigateGuardProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerNavigateGuardProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            location.href = "./browser-navigation.html";
            registration.active.postMessage("navigate-during-browser-navigation");
          })().catch(error => {
            globalThis.__serviceWorkerNavigateGuardProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker navigate guard probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNavigateGuardProbe)",
        r#"{"navigateError":{"name":"TypeError","message":"The client is already navigating."}}"#,
    )
    .await;
    let pending_navigation = vm
        .take_pending_location_navigation_with_seed()
        .expect("browser navigation should remain pending");
    assert_eq!(
        pending_navigation.url,
        url::Url::parse(&format!("{base_url}/app/browser-navigation.html")).unwrap()
    );
    assert!(pending_navigation.service_worker_client_navigate.is_none());
    server
        .await
        .expect("service worker navigate guard script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_window_client_navigate_rejects_when_overwritten() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            self.addEventListener("message", event => {
              event.waitUntil((async () => {
                const all = await clients.matchAll({ includeUncontrolled: true });
                let navigateError = null;
                try {
                  await all[0].navigate("./sw-next.html");
                } catch (error) {
                  navigateError = {
                    name: error && error.name,
                    message: error && error.message
                  };
                }
                event.source.postMessage(JSON.stringify({ navigateError }));
              })());
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerNavigateCancelProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerNavigateCancelProbe = event.data;
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            globalThis.__serviceWorkerNavigateCancelRegistration = registration;
            globalThis.__serviceWorkerNavigateCancelProbe = "ready";
          })().catch(error => {
            globalThis.__serviceWorkerNavigateCancelProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker navigate cancel setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNavigateCancelProbe)",
        "ready",
    )
    .await;

    vm.eval(
        r#"
        globalThis.__serviceWorkerNavigateCancelProbe = "waiting";
        globalThis.__serviceWorkerNavigateCancelRegistration.active.postMessage("navigate");
        "#,
    )
    .expect("service worker navigate cancel message should evaluate");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !vm.has_pending_location_navigation() {
        assert!(
            std::time::Instant::now() < deadline,
            "service worker client.navigate did not record pending navigation"
        );
        drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
    }

    vm.eval("location.href = './browser-next.html'")
        .expect("browser navigation should overwrite service worker client.navigate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerNavigateCancelProbe)",
        r#"{"navigateError":{"name":"TypeError","message":"The navigation was canceled."}}"#,
    )
    .await;

    let pending_navigation = vm
        .take_pending_location_navigation_with_seed()
        .expect("browser navigation should remain pending after canceling service worker navigate");
    assert_eq!(
        pending_navigation.url,
        url::Url::parse(&format!("{base_url}/app/browser-next.html")).unwrap()
    );
    assert!(pending_navigation.service_worker_client_navigate.is_none());

    server
        .await
        .expect("service worker navigate cancel script server should finish");
}
#[tokio::test]
async fn service_worker_window_client_owner_requests_reject_on_stale_document_owner() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerStaleOwnerProbe = "pending";
          (async () => {
            await navigator.serviceWorker.register("worker.js", { scope: "./" });
            const registration = await navigator.serviceWorker.ready;
            globalThis.__serviceWorkerStaleOwnerProbe = registration.active.state;
          })().catch(error => {
            globalThis.__serviceWorkerStaleOwnerProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker stale owner setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerStaleOwnerProbe)",
        "activated",
    )
    .await;

    let service = browser_context_runtime.service_worker_runtime();
    assert_eq!(service.pending_service_lane_event_count(), 0);
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("service worker test page should retain a main Document owner");
    let stale_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        current_owner.scheduler_lane_id,
        current_owner.local_window_id,
        crate::frame_owner_model::DocumentId(
            current_owner
                .document_id
                .0
                .checked_add(1)
                .expect("test Document id should have a successor"),
        ),
    );
    run_service_worker_client_navigate_request_task_for_test(
        &mut vm,
        &loader,
        "stale navigate request",
        crate::types::ServiceWorkerClientNavigateRequestCompletion {
            target: service_worker_window_client_target_for_test(
                crate::runtime::ServiceWorkerClientId::from_u64_for_test(1),
                crate::native_bridge::WindowDocumentOwner::Frame(stale_owner),
            ),
            request_id: 101,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            url: url::Url::parse(&format!("{base_url}/app/stale-navigate.html")).unwrap(),
        },
    )
    .await;
    assert_eq!(service.pending_service_lane_event_count(), 1);

    run_service_worker_client_focus_request_task_for_test(
        &mut vm,
        &loader,
        "stale focus request",
        crate::types::ServiceWorkerClientFocusRequestCompletion {
            target: service_worker_window_client_target_for_test(
                crate::runtime::ServiceWorkerClientId::from_u64_for_test(1),
                crate::native_bridge::WindowDocumentOwner::Frame(stale_owner),
            ),
            request_id: 102,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        },
    )
    .await;
    assert_eq!(service.pending_service_lane_event_count(), 2);

    run_service_worker_clients_open_window_request_task_for_test(
        &mut vm,
        &loader,
        "stale openWindow request",
        crate::types::ServiceWorkerClientsOpenWindowRequestCompletion {
            host: service_worker_window_client_target_for_test(
                crate::runtime::ServiceWorkerClientId::from_u64_for_test(1),
                crate::native_bridge::WindowDocumentOwner::Frame(stale_owner),
            ),
            request_id: 103,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            url: url::Url::parse(&format!("{base_url}/app/opened.html")).unwrap(),
        },
    )
    .await;
    assert_eq!(service.pending_service_lane_event_count(), 3);
    assert!(vm.take_pending_popup_activations().is_empty());
    assert!(!vm.has_pending_lightweight_popup_document_loads());

    assert_eq!(
        browser_context_runtime.drain_service_worker_service_lane(),
        3
    );
    assert_eq!(service.pending_service_lane_event_count(), 0);

    server
        .await
        .expect("service worker stale owner script server should finish");
}
#[tokio::test]
async fn service_worker_client_focus_request_marks_current_page_focused() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerFocusProbe = "pending";
          (async () => {
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            globalThis.__serviceWorkerFocusProbe = registration.active.state;
          })().catch(error => {
            globalThis.__serviceWorkerFocusProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker focus setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerFocusProbe)",
        "activated",
    )
    .await;

    let current_target = vm
        .service_worker_internal_window_client_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )
        .expect("current top-level ServiceWorker client target");
    run_service_worker_client_focus_request_task_for_test(
        &mut vm,
        &loader,
        "current Page focus request",
        crate::types::ServiceWorkerClientFocusRequestCompletion {
            target: current_target,
            request_id: 77,
            source_version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        },
    )
    .await;
    browser_context_runtime.drain_service_worker_service_lane();

    let clients = browser_context_runtime
        .service_worker_runtime()
        .query_clients(&crate::runtime::ServiceWorkerClientQuery {
            request_id: 78,
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll {
                options: crate::runtime::ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: crate::runtime::ServiceWorkerClientQueryType::Window,
                },
            },
        });
    let current = clients
        .clients
        .iter()
        .find(|client| client.id == crate::runtime::ServiceWorkerClientId::from_u64_for_test(1))
        .expect("current page client should be queryable");
    assert!(current.focused);

    server
        .await
        .expect("service worker focus script server should finish");
}
