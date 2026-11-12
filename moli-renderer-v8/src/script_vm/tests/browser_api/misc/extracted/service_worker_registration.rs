use super::*;

#[tokio::test]
async fn service_worker_getter_uses_native_window_origin_before_exposing_container() {
    const PROBE: &str = r#"<!doctype html><script>
        onmessage = event => {
            try {
            const inspect = () => {
                try {
                    const sw = navigator.serviceWorker;
                    return sw instanceof ServiceWorkerContainer && sw === navigator.serviceWorker;
                } catch (error) {
                    return error.name + ':' + (error instanceof DOMException);
                }
            };
            const checks = [inspect()];
            self.origin = 'https://spoofed.test';
            for (const key of ['URL', 'baseURI']) {
                Object.defineProperty(document, key, {configurable: true, get() { throw new Error('author ' + key); }});
            }
            checks.push(inspect());
            parent.postMessage({id: event.data, result: checks.join('|')}, '*');
            } catch (error) {
                parent.postMessage({id: event.data, result: 'probe-error:' + error.name + ':' + error.message}, '*');
            }
        };
    </script>"#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/opaque.html", "text/html", PROBE),
        ("/allowed.html", "text/html", PROBE),
        ("/normal.html", "text/html", PROBE),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(&format!(
        "globalThis.__getterProbeSource = {};",
        serde_json::to_string(PROBE).unwrap()
    ))
    .unwrap();
    vm.eval(r#"
        globalThis.__getterOriginDone = false;
        globalThis.__getterOriginPhase = 'start';
        (async () => {
            const checks = [];
            let probeId = 0;
            const makeFrame = async (sandbox, src, srcdoc) => {
                __getterOriginPhase = 'load:' + src + ':' + sandbox;
                const frame = document.createElement('iframe');
                if (sandbox !== null) frame.sandbox = sandbox;
                if (src !== null) frame.src = src;
                if (srcdoc !== null) frame.srcdoc = srcdoc;
                const loaded = new Promise(resolve => { frame.onload = resolve; });
                document.body.appendChild(frame);
                await loaded;
                __getterOriginPhase = 'loaded:' + src + ':' + sandbox;
                return frame;
            };
            const probe = frame => new Promise(resolve => {
                const id = ++probeId;
                const handler = event => {
                    if (!event.data || event.data.id !== id) return;
                    removeEventListener('message', handler);
                    resolve(event.data.result);
                };
                addEventListener('message', handler);
                frame.contentWindow.postMessage(id, '*');
            });
            const opaque = await makeFrame('allow-scripts', '/opaque.html', null);
            checks.push(await probe(opaque));
            opaque.removeAttribute('sandbox');
            checks.push(await probe(opaque));
            checks.push(await probe(await makeFrame('allow-scripts allow-same-origin', '/allowed.html', null)));
            checks.push(await probe(await makeFrame(null, '/normal.html', null)));
            checks.push(await probe(await makeFrame('allow-scripts', null, __getterProbeSource)));
            checks.push(await probe(await makeFrame('allow-scripts allow-same-origin', null, __getterProbeSource)));
            const blank = await makeFrame(null, 'about:blank', null);
            const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'serviceWorker').get;
            const childGetter = Object.getOwnPropertyDescriptor(blank.contentWindow.Navigator.prototype, 'serviceWorker').get;
            const childNavigator = blank.contentWindow.navigator;
            checks.push(getter.call(childNavigator) === childNavigator.serviceWorker);
            checks.push(childGetter.call(navigator) === navigator.serviceWorker);
            try { getter.call({}); checks.push('accepted'); }
            catch (error) { checks.push(error.name); }
            const popup = open('about:blank', 'service-worker-getter-origin');
            checks.push(popup.navigator.serviceWorker === popup.navigator.serviceWorker);
            popup.close();
            globalThis.__getterOrigin = checks.join(';');
        })().catch(error => { __getterOrigin = 'error:' + error.name + ':' + error.message; })
          .finally(() => { __getterOriginPhase = 'done'; __getterOriginDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__getterOriginDone) + ':' + __getterOriginPhase",
        "true:done",
    )
    .await;
    assert_eq!(
        vm.eval("String(__getterOrigin)").unwrap(),
        concat!(
            "SecurityError:true|SecurityError:true;SecurityError:true|SecurityError:true;",
            "true|true;true|true;SecurityError:true|SecurityError:true;true|true;",
            "true;true;TypeError;true"
        )
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_get_registration_separates_api_base_url_from_native_origin() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/other/worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__registrationBaseDone = false;
        (async () => {
            const sw = navigator.serviceWorker;
            const app = await sw.register('worker.js');
            const other = await sw.register('/other/worker.js');
            const checks = [await sw.getRegistration() === app];
            const base = document.createElement('base');
            base.href = other.scope;
            document.head.appendChild(base);
            checks.push(await sw.getRegistration() === other);
            checks.push(await sw.getRegistration('page.html#fragment') === other);
            base.href = 'https://different-origin.test/';
            checks.push(await sw.getRegistration(app.scope + 'page.html#fragment') === app);
            const errorName = promise => promise.then(() => 'resolved', error => error.name);
            checks.push(await errorName(sw.getRegistration('page.html')));
            checks.push(await errorName(sw.getRegistration('http://[')));
            checks.push(await errorName(sw.getRegistration('data:text/html,opaque')));
            checks.push(await sw.getRegistration({toString() {
                base.href = other.scope;
                return 'page.html#converted';
            }}) === other);
            Object.defineProperty(document, 'baseURI', {get() { throw new Error('author baseURI'); }});
            Object.defineProperty(document, 'URL', {get() { throw new Error('author URL'); }});
            checks.push(await sw.getRegistration() === other);
            globalThis.__registrationBase = checks.join('|');
        })().catch(error => { __registrationBase = 'error:' + error.name + ':' + error.message; })
          .finally(() => { __registrationBaseDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__registrationBaseDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(__registrationBase)").unwrap(),
        "true|true|true|true|SecurityError|TypeError|SecurityError|true|true"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_get_registration_uses_inherited_blank_popup_and_srcdoc_settings() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/other/worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__registrationInheritedDone = false;
        globalThis.__registrationInheritedPhase = 'register';
        (async () => {
            const sw = navigator.serviceWorker;
            const app = await sw.register('worker.js');
            const other = await sw.register('/other/worker.js');
            const checks = [];
            const setBase = (realm, href) => {
                const doc = realm.document;
                const html = doc.documentElement || doc.appendChild(doc.createElement('html'));
                const head = doc.head || html.insertBefore(doc.createElement('head'), html.firstChild);
                const base = doc.createElement('base');
                base.href = href;
                head.appendChild(base);
                return base;
            };
            __registrationInheritedPhase = 'blank';
            const blank = document.createElement('iframe');
            document.body.appendChild(blank);
            const child = blank.contentWindow;
            const childSw = child.navigator.serviceWorker;
            checks.push((await childSw.getRegistration(app.scope + 'page.html#fragment')).scope === app.scope);
            checks.push((await childSw.getRegistration()).scope === app.scope);
            checks.push((await childSw.getRegistration('deep/page.html')).scope === app.scope);
            const childBase = setBase(child, other.scope);
            checks.push((await childSw.getRegistration()).scope === other.scope);
            checks.push((await sw.getRegistration.call(childSw, 'page.html')).scope === other.scope);
            checks.push((await childSw.getRegistration.call(sw, 'page.html')).scope === app.scope);
            childBase.href = 'https://different-origin.test/';
            checks.push((await childSw.getRegistration(app.scope + 'page.html')).scope === app.scope);
            checks.push(await childSw.getRegistration('page.html').then(() => 'resolved', error => error.name));
            __registrationInheritedPhase = 'popup';
            const popup = open('about:blank', 'get-registration-settings');
            const popupSw = popup.navigator.serviceWorker;
            checks.push((await popupSw.getRegistration()).scope === app.scope);
            const popupBase = setBase(popup, other.scope);
            checks.push((await popupSw.getRegistration('page.html#fragment')).scope === other.scope);
            popupBase.href = 'https://different-origin.test/';
            checks.push((await popupSw.getRegistration(app.scope + 'page.html')).scope === app.scope);
            checks.push(await popupSw.getRegistration('page.html').then(() => 'resolved', error => error.name));
            __registrationInheritedPhase = 'srcdoc';
            const srcdoc = document.createElement('iframe');
            const loaded = new Promise(resolve => { srcdoc.onload = resolve; });
            srcdoc.srcdoc = '<!doctype html><base href="' + other.scope + '"><title>lookup</title>';
            document.body.appendChild(srcdoc);
            await loaded;
            const srcdocSw = srcdoc.contentWindow.navigator.serviceWorker;
            checks.push((await srcdocSw.getRegistration('page.html#fragment')).scope === other.scope);
            checks.push((await srcdocSw.getRegistration()).scope === other.scope);
            checks.push((await srcdocSw.getRegistration(app.scope + 'page.html')).scope === app.scope);
            globalThis.__registrationInherited = checks.join('|');
        })().catch(error => { __registrationInherited = __registrationInheritedPhase + ':error:' + error.name + ':' + error.message; })
          .finally(() => { __registrationInheritedDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__registrationInheritedDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(__registrationInherited)").unwrap(),
        "true|true|true|true|true|true|true|SecurityError|true|true|true|SecurityError|true|true|true"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn service_worker_get_registration_in_blob_worker_uses_captured_native_origin() {
    let (base_url, server) = spawn_service_worker_script_server(vec!["/app/worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__workerRegistrationDone = false;
        (async () => {
            const registration = await navigator.serviceWorker.register('worker.js');
            const source = `onmessage = async event => {
                try {
                    self.origin = 'https://different-origin.test';
                    const sw = navigator.serviceWorker;
                    const registration = await sw.getRegistration(event.data + 'page.html#fragment');
                    const relative = await sw.getRegistration('relative.html').then(() => 'resolved', e => e.name);
                    const foreign = await sw.getRegistration('https://different-origin.test/page.html').then(() => 'resolved', e => e.name);
                    postMessage([registration.scope === event.data, relative, foreign, self.origin].join('|'));
                } catch (error) { postMessage('error:' + error.name + ':' + error.message); }
            };`;
            const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
            const worker = new Worker(url);
            globalThis.__workerRegistration = await new Promise(resolve => {
                worker.onmessage = event => resolve(event.data);
                worker.onerror = event => resolve('worker-error:' + event.message);
                worker.postMessage(registration.scope);
            });
            worker.terminate();
            URL.revokeObjectURL(url);
        })().catch(error => { __workerRegistration = 'error:' + error.name + ':' + error.message; })
          .finally(() => { __workerRegistrationDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__workerRegistrationDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(__workerRegistration)").unwrap(),
        "true|TypeError|SecurityError|https://different-origin.test"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn navigator_service_worker_get_registrations_preserves_registration_order() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/empty.js",
            "text/javascript; charset=utf-8",
            "// empty worker"
        );
        16
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__registrationOrderDone = false;
      (async () => {
        const expected = [];
        for (let i = 15; i >= 0; --i) {
          const registration = await navigator.serviceWorker.register("empty.js", { scope: "scope-" + i + "/" });
          const worker = registration.installing || registration.waiting || registration.active;
          if (worker.state !== "activated") {
            await new Promise(resolve => worker.addEventListener("statechange", () => {
              if (worker.state === "activated") resolve();
            }));
          }
          expected.push(registration);
        }
        const first = await navigator.serviceWorker.getRegistrations();
        const second = await navigator.serviceWorker.getRegistrations();
        globalThis.__registrationOrder = [
          first.length === expected.length && first.every((r, i) => r === expected[i]),
          second.length === expected.length && second.every((r, i) => r === expected[i])
        ].join(":");
      })().catch(error => {
        globalThis.__registrationOrder = "error:" + error.name + ":" + error.message;
      }).finally(() => { globalThis.__registrationOrderDone = true; });
    "#).expect("registration order probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__registrationOrderDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__registrationOrder)").unwrap(),
        "true:true"
    );
    server.await.expect("all worker scripts should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_updates_keep_order_and_reregistration_moves_to_end() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/first.js", "text/javascript", "// first"),
        ("/app/second.js", "text/javascript", "// second"),
        ("/app/third.js", "text/javascript", "// third"),
        ("/app/update.js", "text/javascript", "// update"),
        ("/app/replacement.js", "text/javascript", "// replacement"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__registrationReplacementDone = false;
      (async () => {
        const sw = navigator.serviceWorker;
        const expected = [];
        async function register(script, scope) {
          const r = await sw.register(script, {scope});
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
          return r;
        }
        for (const [script, scope] of [['first.js', 'z/'], ['second.js', 'b/'], ['third.js', 'a/']]) {
          expected.push(await register(script, scope));
        }
        const updated = await register('update.js', 'b/');
        const all = await sw.getRegistrations();
        const same = updated === expected[1] && all.every((r,i) => r === expected[i]) && all.length === 3;
        await expected[0].unregister();
        const replacement = await register('replacement.js', 'z/');
        const final = await sw.getRegistrations();
        const after = [expected[1], expected[2], replacement];
        globalThis.__registrationReplacement = [same, replacement !== expected[0],
          final.length === 3 && final.every((r,i) => r === after[i])].join(':');
      })().catch(error => {
        globalThis.__registrationReplacement = 'error:' + error.name + ':' + error.message;
      }).finally(() => { globalThis.__registrationReplacementDone = true; });
    "#).expect("registration replacement probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__registrationReplacementDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__registrationReplacement)")
            .unwrap(),
        "true:true:true"
    );
    server.await.expect("all worker scripts should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_global_registration_starts_with_empty_installing_slot() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/bootstrap.js",
        "text/javascript; charset=utf-8",
        r#"
        const initial = [registration.installing, registration.waiting, registration.active]
          .map(worker => worker === null).join(":");
        self.addEventListener("message", event => event.ports[0].postMessage(initial));
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__registrationBootstrapDone = false;
      (async () => {
        const sw = navigator.serviceWorker;
        await sw.register("bootstrap.js", { scope: "./" });
        const registration = await __moliTestActivatedServiceWorker(sw);
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        registration.active.postMessage("initial", [channel.port2]);
        globalThis.__registrationBootstrap = await result;
        channel.port1.close();
      })().catch(error => {
        globalThis.__registrationBootstrap = "error:" + error.name + ":" + error.message;
      }).finally(() => { globalThis.__registrationBootstrapDone = true; });
    "#).expect("worker registration bootstrap probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__registrationBootstrapDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__registrationBootstrap)")
            .unwrap(),
        "true:true:true"
    );
    server.await.expect("worker script should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_global_worker_objects_keep_identity_and_statechange_order() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/lifecycle.js",
        "text/javascript; charset=utf-8",
        r#"
        const seen = [];
        let worker;
        registration.addEventListener("updatefound", () => {
          worker = registration.installing;
          seen.push("updatefound:" + (worker === registration.installing) + ":" + worker.state);
          worker.addEventListener("statechange", event => {
            seen.push("statechange:" + worker.state + ":" + event.isTrusted + ":" +
              (event.target === worker) + ":" +
              (worker === (registration.installing || registration.waiting || registration.active)));
          });
        });
        self.addEventListener("install", () => {
          seen.push("install:" + (worker === registration.installing) + ":" + registration.installing.state);
        });
        self.addEventListener("activate", () => {
          seen.push("activate:" + (worker === registration.active) + ":" + registration.active.state);
        });
        self.addEventListener("message", event => event.ports[0].postMessage(seen.join("|")));
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__registrationLifecycleDone = false;
      (async () => {
        const sw = navigator.serviceWorker;
        await sw.register("lifecycle.js", { scope: "./" });
        const registration = await __moliTestActivatedServiceWorker(sw);
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        registration.active.postMessage("events", [channel.port2]);
        globalThis.__registrationLifecycle = await result;
        channel.port1.close();
      })().catch(error => {
        globalThis.__registrationLifecycle = "error:" + error.name + ":" + error.message;
      }).finally(() => { globalThis.__registrationLifecycleDone = true; });
    "#).expect("worker registration lifecycle probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__registrationLifecycleDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__registrationLifecycle)")
            .unwrap(),
        "updatefound:true:installing|install:true:installing|statechange:installed:true:true:true|statechange:activating:true:true:true|activate:true:activating|statechange:activated:true:true:true"
    );
    server.await.expect("worker script should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_module_global_registration_applies_lifecycle_tasks() {
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/lifecycle-module.js",
        "text/javascript; charset=utf-8",
        Vec::new(),
        r#"
        const seen = ["evaluate:" + (registration.installing === null)];
        registration.onupdatefound = () => {
          const worker = registration.installing;
          seen.push("updatefound:" + worker.state);
          worker.onstatechange = () => seen.push("statechange:" + worker.state);
        };
        self.oninstall = () => seen.push("install:" + registration.installing.state);
        self.onactivate = () => seen.push("activate:" + registration.active.state);
        self.onmessage = event => event.ports[0].postMessage(seen.join("|"));
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__moduleRegistrationDone = false;
      (async () => {
        const sw = navigator.serviceWorker;
        await sw.register("lifecycle-module.js", { scope: "./", type: "module" });
        const registration = await __moliTestActivatedServiceWorker(sw);
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        registration.active.postMessage("events", [channel.port2]);
        globalThis.__moduleRegistration = await result;
        channel.port1.close();
      })().catch(error => {
        globalThis.__moduleRegistration = "error:" + error.name + ":" + error.message;
      }).finally(() => { globalThis.__moduleRegistrationDone = true; });
    "#).expect("module registration lifecycle probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__moduleRegistrationDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__moduleRegistration)").unwrap(),
        "evaluate:true|updatefound:installing|install:installing|statechange:installed|statechange:activating|activate:activating|statechange:activated"
    );
    server
        .await
        .expect("module worker script should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_skip_waiting_resolves_after_activate_dispatch() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/initial.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("activate", event => event.waitUntil(clients.claim()));
            "#,
        ),
        (
            "/app/waiting.js",
            "text/javascript; charset=utf-8",
            r#"
            let sawActivate = false;
            self.addEventListener("activate", () => { sawActivate = true; });
            self.addEventListener("message", event => {
                event.waitUntil(skipWaiting().then(result => {
                    event.ports[0].postMessage([
                        typeof result, sawActivate,
                        self.registration.active && self.registration.active.state
                    ].join(":"));
                }));
            });
            "#,
        ),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__skipWaitingDone = false;
      (async () => {
        const sw = navigator.serviceWorker;
        await sw.register("initial.js", { scope: "./" });
        await __moliTestActivatedServiceWorker(sw);
        if (!sw.controller) {
          await new Promise(resolve => sw.addEventListener("controllerchange", resolve, { once: true }));
        }
        const registration = await sw.register("waiting.js", { scope: "./" });
        const worker = registration.installing || registration.waiting;
        if (worker.state !== "installed") {
          await new Promise(resolve => worker.addEventListener("statechange", () => {
            if (worker.state === "installed") resolve();
          }));
        }
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        worker.postMessage("skip", [channel.port2]);
        globalThis.__skipWaitingResult = await result;
        channel.port1.close();
      })().catch(error => {
        globalThis.__skipWaitingResult = "error:" + error.name + ":" + error.message;
      }).finally(() => { globalThis.__skipWaitingDone = true; });
    "#).expect("installed worker skipWaiting probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__skipWaitingDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__skipWaitingResult)").unwrap(),
        "undefined:true:activating"
    );
    server.await.expect("both worker scripts should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_module_bootstrap_skip_waiting_completes_install() {
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/skip-bootstrap.js",
        "text/javascript; charset=utf-8",
        Vec::new(),
        r#"
        const startupSkip = skipWaiting().then(value => String(value));
        self.addEventListener("install", event => event.waitUntil(startupSkip));
        self.addEventListener("activate", event => event.waitUntil(clients.claim()));
        self.addEventListener("message", event => {
          event.waitUntil(startupSkip.then(value => event.ports[0].postMessage(value)));
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__skipBootstrap = "pending";
      (async () => {
        const sw = navigator.serviceWorker;
        await sw.register("skip-bootstrap.js", { scope: "./", type: "module" });
        const registration = await __moliTestActivatedServiceWorker(sw);
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        registration.active.postMessage("startup", [channel.port2]);
        globalThis.__skipBootstrap = (await result) + ":" + registration.active.state + ":" + String(sw.controller !== null);
        channel.port1.close();
      })().catch(error => {
        globalThis.__skipBootstrap = "error:" + error.name + ":" + error.message;
      });
    "#).expect("module startup skipWaiting probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__skipBootstrap)",
        "undefined:activated:true",
    )
    .await;
    server
        .await
        .expect("module worker script should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_module_bootstrap_claim_rejects_and_activation_claim_completes() {
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/claim-bootstrap.js",
        "text/javascript; charset=utf-8",
        Vec::new(),
        r#"
        const startupClaim = clients.claim().then(
          () => "resolved", error => error.name
        );
        self.addEventListener("activate", event => {
          event.waitUntil(startupClaim.then(() => clients.claim()));
        });
        self.addEventListener("message", event => {
          event.waitUntil(startupClaim.then(result => event.ports[0].postMessage(result)));
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__claimBootstrapProbe = "pending";
      (async () => {
        await navigator.serviceWorker.register("claim-bootstrap.js", { scope: "./", type: "module" });
        const registration = await navigator.serviceWorker.ready;
        const worker = registration.active;
        if (worker.state !== "activated") {
          await new Promise(resolve => worker.addEventListener("statechange", () => {
            if (worker.state === "activated") resolve();
          }));
        }
        const channel = new MessageChannel();
        const result = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
        worker.postMessage("startup-claim", [channel.port2]);
        globalThis.__claimBootstrapProbe = (await result) + ":" + worker.state + ":" + String(navigator.serviceWorker.controller !== null);
        channel.port1.close();
      })().catch(error => {
        globalThis.__claimBootstrapProbe = "error:" + error.name + ":" + error.message;
      });
    "#).expect("module claim bootstrap probe should evaluate");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__claimBootstrapProbe)",
        "InvalidStateError:activated:true",
    )
    .await;
    server
        .await
        .expect("module worker script should be fetched");
}

#[tokio::test]
async fn navigator_service_worker_unregister_keeps_old_clients_and_clears_old_objects_in_tasks() {
    let worker = r#"
        self.addEventListener('fetch', event => {
          const isClient = new URL(event.request.url).pathname.endsWith('/client.html');
          event.respondWith(new Response(isClient
            ? "<script>parent.postMessage('controlled-ready', '*')</script>"
            : 'controlled-fetch',
            {headers: {'Content-Type': isClient ? 'text/html' : 'text/plain'}}));
        });
    "#;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for (path, status, content_type, body) in [
            ("/controlled/sw.js", "200 OK", "text/javascript", worker),
            (
                "/controlled/invalid.js",
                "404 Not Found",
                "text/html",
                "not found",
            ),
            ("/controlled/sw.js", "200 OK", "text/javascript", worker),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_storage_bucket_partition_request_head(&mut stream)
                .await
                .unwrap();
            assert_eq!(
                request.lines().next().unwrap(),
                format!("GET {path} HTTP/1.1")
            );
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(include_str!("../../../service_worker_unregister.js"))
        .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(unregisteredReplacementProbe.phase === 'error' || \
         (unregisteredReplacementProbe.phase === 'registered' && \
          unregisteredReplacementProbe.oldRegistration.active === null && \
          unregisteredReplacementProbe.events.length === 2))",
        "true",
    )
    .await;
    let observed = vm.eval(r#"
        JSON.stringify({
          error: unregisteredReplacementProbe.error || null,
          flags: Object.values(unregisteredReplacementProbe.result),
          events: unregisteredReplacementProbe.events,
          oldSlotsCleared: unregisteredReplacementProbe.oldRegistration.installing === null &&
            unregisteredReplacementProbe.oldRegistration.waiting === null &&
            unregisteredReplacementProbe.oldRegistration.active === null,
          oldWorkerState: unregisteredReplacementProbe.oldWorker.state,
          replacementActivated: unregisteredReplacementProbe.replacement.active.state === 'activated'
        })
    "#).unwrap();
    assert_eq!(
        observed,
        r#"{"error":null,"flags":[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true],"events":["event:true","microtask:true"],"oldSlotsCleared":true,"oldWorkerState":"redundant","replacementActivated":true}"#
    );
    server
        .await
        .expect("both registrations must fetch a worker script");
}

#[tokio::test]
async fn navigator_service_worker_register_applies_script_path_restriction() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            Vec::new(),
            worker_body,
        ),
        (
            "/app/allowed-worker.js",
            "text/javascript; charset=utf-8",
            vec![("Service-Worker-Allowed", "/")],
            worker_body,
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
              globalThis.__serviceWorkerPathRestrictionProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                let rejected;
                try {
                  await sw.register("worker.js", { scope: "/" });
                  rejected = "resolved";
                } catch (error) {
                  const lookup = await sw.getRegistration("/");
                  const all = await sw.getRegistrations();
                  rejected = [
                    "rejected",
                    String(error && error.message).includes("max scope allowed"),
                    lookup === undefined,
                    all.length
                  ].join("|");
                }

                try {
                  const registration = await sw.register("allowed-worker.js", { scope: "/" });
                  const all = await sw.getRegistrations();
                  globalThis.__serviceWorkerPathRestrictionProbe =
                    rejected + "||allowed:" + [registration.scope, all.length].join("|");
                } catch (error) {
                  globalThis.__serviceWorkerPathRestrictionProbe =
                    rejected + "||allowed-rejected:" + (error && error.message);
                }
              })().catch((error) => {
                globalThis.__serviceWorkerPathRestrictionProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker path restriction probe should evaluate");

    let expected = format!("rejected|true|true|0||allowed:{base_url}/|1");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPathRestrictionProbe)",
        &expected,
    )
    .await;

    server
        .await
        .expect("service worker path restriction server should finish");
}
#[tokio::test]
async fn navigator_service_worker_register_identical_main_script_keeps_existing_version() {
    let worker_body = r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_body,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            worker_body,
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
              globalThis.__serviceWorkerIdenticalUpdateProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const first = await sw.register("worker.js", { scope: "./" });
                await __moliTestActivatedServiceWorker(sw);
                let updatefoundCount = 0;
                first.addEventListener("updatefound", () => {
                  updatefoundCount += 1;
                });
                const second = await sw.register("worker.js", { scope: "./" });
                const all = await sw.getRegistrations();
                globalThis.__serviceWorkerIdenticalUpdateProbe = [
                  updatefoundCount,
                  second.installing === null,
                  second.waiting === null,
                  second.active && second.active.scriptURL,
                  all.length
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerIdenticalUpdateProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker identical update probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!("0|true|true|{expected_worker_url}|1");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerIdenticalUpdateProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker identical update server should finish");
}
#[tokio::test]
async fn navigator_service_worker_register_changed_main_script_fires_updatefound() {
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
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
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
              globalThis.__serviceWorkerChangedUpdateProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const registration = await sw.register("worker.js", { scope: "./" });
                await __moliTestActivatedServiceWorker(sw);
                const events = [];
                registration.addEventListener("updatefound", () => {
                  const installing = registration.installing;
                  events.push([
                    "updatefound",
                    installing && installing.scriptURL,
                    installing && installing.state
                  ].join(":"));
                  installing.addEventListener("statechange", () => {
                    events.push("statechange:" + installing.state);
                  });
                });
                const second = await sw.register("worker.js", { scope: "./" });
                let lookup = await sw.getRegistration();
                for (let i = 0; i < 20 && !events.includes("statechange:installed"); i++) {
                  await new Promise(resolve => setTimeout(resolve, 0));
                  lookup = await sw.getRegistration();
                }
                globalThis.__serviceWorkerChangedUpdateProbe = [
                  events.join(","),
                  second.installing === null,
                  second.waiting && second.waiting.scriptURL,
                  second.active && second.active.scriptURL,
                  lookup.waiting && lookup.waiting.scriptURL
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerChangedUpdateProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker changed update probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "updatefound:{expected_worker_url}:installing,statechange:installed|true|{expected_worker_url}|{expected_worker_url}|{expected_worker_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerChangedUpdateProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.installing_version_count, 0);
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker changed update server should finish");
}
#[tokio::test]
async fn navigator_service_worker_registration_lifecycle_attributes_reflect_active_runtime() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/lifecycle-worker.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerLifecycleProbe = "started";
              (async () => {
                const sw = navigator.serviceWorker;
                await sw.register("lifecycle-worker.js", {
                  scope: "./"
                });
                const registration = await sw.ready;
                {
                  // ready may resolve while activate (and its claim) is still pending.
                  const active = (await sw.ready).active;
                  if (active.state !== "activated") {
                    await new Promise(resolve => active.addEventListener("statechange", () => {
                      if (active.state === "activated") resolve();
                    }));
                  }
                }
                globalThis.__serviceWorkerLifecycleRegistration = registration;
                const newest = registration.active;
                const log = [
                  [
                    "active",
                    newest instanceof ServiceWorker,
                    newest && newest.state,
                    registration.installing === null,
                    registration.waiting === null,
                    registration.active === newest,
                    typeof registration.addEventListener,
                    typeof newest.addEventListener
                  ].join(":")
                ];
                const active = registration.active;
                await Promise.all([
                  new Promise((resolve) => {
                    active.addEventListener("statechange", () => {
                      log.push([
                        "redundant",
                        active.state,
                        registration.installing === null,
                        registration.waiting === null,
                        registration.active === active
                      ].join(":"));
                      if (active.state === "redundant") {
                        resolve();
                      }
                    }, { once: true });
                  }),
                  registration.unregister()
                ]);
                log.push([
                  "after-unregister",
                  registration.installing === null,
                  registration.waiting === null,
                  registration.active === active
                ].join(":"));
                globalThis.__serviceWorkerLifecycleProbe = log.join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerLifecycleProbe = "error:" + String(error);
              });
            })()
            "#,
    )
    .expect("service worker lifecycle probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerLifecycleProbe.includes('after-unregister'))",
        "true",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__serviceWorkerLifecycleProbe)")
        .expect("service worker lifecycle promises should settle");

    assert_eq!(
        result,
        r#""active:true:activated:true:true:true:function:function|redundant:redundant:true:true:true|after-unregister:true:true:true""#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(__serviceWorkerLifecycleRegistration.installing === null && \
         __serviceWorkerLifecycleRegistration.waiting === null && \
         __serviceWorkerLifecycleRegistration.active === null)",
        "true",
    )
    .await;
    server
        .await
        .expect("service worker lifecycle script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_update_via_cache_option_reflects_registration() {
    let (base_url, server) =
        spawn_service_worker_script_server(vec!["/app/worker.js", "/app/worker-all.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerUpdateViaCacheProbe = "pending";
              (async () => {
                const first = await navigator.serviceWorker.register("worker.js", {
                  scope: "./",
                  updateViaCache: "none"
                });
                const second = await navigator.serviceWorker.register("worker-all.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
                let invalid = "not-rejected";
                try {
                  await navigator.serviceWorker.register("worker.js", {
                    scope: "./",
                    updateViaCache: "invalid"
                  });
                } catch (error) {
                  invalid = error && error.name;
                }
                globalThis.__serviceWorkerUpdateViaCacheProbe = [
                  first.updateViaCache,
                  second.updateViaCache,
                  invalid
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateViaCacheProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker updateViaCache probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateViaCacheProbe)",
        "all|all|TypeError",
    )
    .await;
    server
        .await
        .expect("service worker updateViaCache script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_update_via_cache_all_uses_fresh_main_script_cache() {
    let cache_dir = service_worker_http_cache_test_root("update-via-cache-all");
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        vec![("Cache-Control", "max-age=60")],
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(clients.claim());
        });
        "#,
    )])
    .await;
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let browser_context_runtime = crate::runtime::RendererBrowserContextRuntime::new();
    let resource_runtime = browser_context_runtime
        .replace_browser_resource_runtime(crate::network::BrowserResourceRuntimeOwner::new(
            &fetch_config,
            moli_cookie_jar::new_shared_browser_cookie_store(),
        ))
        .expect("cache-enabled browser resource runtime");
    let loader = ResourceRequestClient::from_browser_resource_runtime(resource_runtime);
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
        url::Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
        &loader,
        browser_context_runtime.handle(),
    );
    // Execute register and update with the same Document transport, including
    // its browser-site context used to partition the HTTP cache.
    let loader = vm
        ._context_host
        .borrow()
        .current_main_document_resource_loader()
        .expect("current Document loader")
        .request_client()
        .clone();
    assert!(
        loader
            .browser_resource_runtime()
            .matches_fetch_config(&fetch_config)
    );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerUpdateViaCacheAllProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const registration = await sw.register("worker.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
                await sw.ready;
                {
                  const worker = registration.active;
                  if (worker.state !== "activated") {
                    await new Promise(resolve => worker.addEventListener("statechange", () => {
                      if (worker.state === "activated") resolve();
                    }));
                  }
                }
                let updatefoundCount = 0;
                registration.addEventListener("updatefound", () => {
                  updatefoundCount += 1;
                });
                const second = await sw.register("worker.js", {
                  scope: "./",
                  updateViaCache: "all"
                });
                const updated = await registration.update();
                if (updated !== registration) throw new Error('update identity changed');
                const all = await sw.getRegistrations();
                globalThis.__serviceWorkerUpdateViaCacheAllProbe = [
                  updatefoundCount,
                  second.installing === null,
                  second.waiting === null,
                  second.active && second.active.scriptURL,
                  all.length
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateViaCacheAllProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker updateViaCache all cache probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!("0|true|true|{expected_worker_url}|1");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateViaCacheAllProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 1);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker updateViaCache all cache server should finish");
    let _ = std::fs::remove_dir_all(cache_dir);
}
#[tokio::test]
async fn navigator_service_worker_default_update_via_cache_revalidates_fresh_main_script_cache() {
    let cache_dir = service_worker_http_cache_test_root("update-via-cache-imports");
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "max-age=60")],
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
            });
            "#,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "max-age=60")],
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            "#,
        ),
    ])
    .await;
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );

    vm.eval(
        r#"
            (() => {
              globalThis.__serviceWorkerUpdateViaCacheImportsProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const registration = await sw.register("worker.js", { scope: "./" });
                await __moliTestActivatedServiceWorker(sw);
                const events = [];
                registration.addEventListener("updatefound", () => {
                  const installing = registration.installing;
                  events.push([
                    "updatefound",
                    installing && installing.scriptURL,
                    installing && installing.state
                  ].join(":"));
                  installing.addEventListener("statechange", () => {
                    events.push("statechange:" + installing.state);
                  });
                });
                const second = await sw.register("worker.js", { scope: "./" });
                let lookup = await sw.getRegistration();
                for (let i = 0; i < 20 && !events.includes("statechange:installed"); i++) {
                  await new Promise(resolve => setTimeout(resolve, 0));
                  lookup = await sw.getRegistration();
                }
                globalThis.__serviceWorkerUpdateViaCacheImportsProbe = [
                  events.join(","),
                  second.installing === null,
                  second.waiting && second.waiting.scriptURL,
                  second.active && second.active.scriptURL,
                  lookup.waiting && lookup.waiting.scriptURL
                ].join("|");
              })().catch((error) => {
                globalThis.__serviceWorkerUpdateViaCacheImportsProbe =
                  "error:" + (error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker updateViaCache imports cache probe should evaluate");

    let expected_worker_url = format!("{base_url}/app/worker.js");
    let expected = format!(
        "updatefound:{expected_worker_url}:installing,statechange:installed|true|{expected_worker_url}|{expected_worker_url}|{expected_worker_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateViaCacheImportsProbe)",
        &expected,
    )
    .await;

    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.registration_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.pending_main_script_update_check_count, 0);

    server
        .await
        .expect("service worker updateViaCache imports cache server should finish");
    let _ = std::fs::remove_dir_all(cache_dir);
}
#[tokio::test]
async fn navigator_service_worker_register_resolves_when_activate_wait_until_rejects() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/activate-reject-worker.js",
        "text/javascript; charset=utf-8",
        r#"
        self.addEventListener("install", event => {
          event.waitUntil(Promise.resolve());
        });
        self.addEventListener("activate", event => {
          event.waitUntil(Promise.reject(new Error("activate failed")));
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
          globalThis.__serviceWorkerActivateRejectProbe = "pending";
          navigator.serviceWorker.register("activate-reject-worker.js", { scope: "./" })
            .then(
              async registration => {
                const registerState = [
                  Boolean(registration.installing),
                  Boolean(registration.waiting),
                  registration.waiting && registration.waiting.state,
                  Boolean(registration.active)
                ].join(":");
                const readyRegistration = await navigator.serviceWorker.ready;
                {
                  // ready may resolve while activate (and its claim) is still pending.
                  const active = (await navigator.serviceWorker.ready).active;
                  if (active.state !== "activated") {
                    await new Promise(resolve => active.addEventListener("statechange", () => {
                      if (active.state === "activated") resolve();
                    }));
                  }
                }
                globalThis.__serviceWorkerActivateRejectProbe =
                  "resolved:" + registerState + "|" +
                  Boolean(readyRegistration.active) + ":" +
                  (readyRegistration.active && readyRegistration.active.state);
              },
              error => {
                globalThis.__serviceWorkerActivateRejectProbe =
                  "rejected:" + (error && error.message);
              }
            );
        })()
        "#,
    )
    .expect("service worker activate rejection probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerActivateRejectProbe)",
        "resolved:true:false::false|true:activated",
    )
    .await;
    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.redundant_version_count, 0);
    assert_eq!(diagnostics.in_flight_event_count, 0);
    assert_eq!(diagnostics.failed_start_count, 1);
    server
        .await
        .expect("service worker activate rejection script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_update_waits_without_skip_waiting() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker-v1.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(Promise.resolve());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(clients.claim());
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
            self.addEventListener("activate", event => {
              event.waitUntil(Promise.resolve());
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
          globalThis.__serviceWorkerUpdateProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            await sw.register("worker-v1.js", { scope: "./" });
            await __moliTestActivatedServiceWorker(sw);
            const update = await sw.register("worker-v2.js", { scope: "./" });
            let lookup = await sw.getRegistration();
            for (let i = 0; i < 20 && !lookup.waiting; i++) {
              await new Promise(resolve => setTimeout(resolve, 0));
              lookup = await sw.getRegistration();
            }
            globalThis.__serviceWorkerUpdateProbe = [
              "controller",
              sw.controller && sw.controller.scriptURL,
              "returned",
              update.installing && update.installing.scriptURL,
              update.waiting && update.waiting.scriptURL,
              update.active && update.active.scriptURL,
              "lookup",
              lookup.installing === null,
              lookup.waiting && lookup.waiting.scriptURL,
              lookup.active && lookup.active.scriptURL
            ].join("|");
          })().catch(error => {
            globalThis.__serviceWorkerUpdateProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker update waiting probe should evaluate");

    let expected_v1_url = format!("{base_url}/app/worker-v1.js");
    let expected_v2_url = format!("{base_url}/app/worker-v2.js");
    let expected = format!(
        "controller|{expected_v1_url}|returned||{expected_v2_url}|{expected_v1_url}|lookup|true|{expected_v2_url}|{expected_v1_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerUpdateProbe)",
        &expected,
    )
    .await;
    let diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(diagnostics.activated_version_count, 1);
    assert_eq!(diagnostics.version_count, 2);
    assert_eq!(diagnostics.in_flight_event_count, 0);
    server
        .await
        .expect("service worker update waiting script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_updatefound_and_statechange_are_runtime_driven() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/worker-v1.js",
            "text/javascript; charset=utf-8",
            r#"
            self.addEventListener("install", event => {
              event.waitUntil(self.skipWaiting());
            });
            self.addEventListener("activate", event => {
              event.waitUntil(self.clients.claim());
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
              event.waitUntil(self.clients.claim());
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
          globalThis.__serviceWorkerRuntimeEventsProbe = "pending";
          (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register("worker-v1.js", { scope: "./" });
            await sw.ready;
            {
              // ready may resolve while activate (and its claim) is still pending.
              const active = (await sw.ready).active;
              if (active.state !== "activated") {
                await new Promise(resolve => active.addEventListener("statechange", () => {
                  if (active.state === "activated") resolve();
                }));
              }
            }
            const events = [];
            const expectedV2Url = new URL("worker-v2.js", location.href).href;
            const publishProbe = () => {
              globalThis.__serviceWorkerRuntimeEventsProbe = [
                events.join(","),
                registration.installing === null,
                registration.waiting === null,
                registration.active && registration.active.scriptURL
              ].join("|");
            };
            const controllerChanged = new Promise((resolve, reject) => {
              sw.addEventListener("controllerchange", () => {
                events.push("controllerchange");
                publishProbe();
                const controller = sw.controller;
                if (!controller || controller.scriptURL !== expectedV2Url) {
                  reject(new Error("replacement controllerchange had the wrong controller"));
                  return;
                }
                resolve();
              }, { once: true });
            });
            const activated = new Promise((resolve, reject) => {
              registration.addEventListener("updatefound", () => {
                const installing = registration.installing;
                events.push([
                  "updatefound",
                  installing && installing.scriptURL,
                  installing && installing.state
                ].join(":"));
                publishProbe();
                if (!installing || installing.scriptURL !== expectedV2Url) {
                  reject(new Error("updatefound had the wrong installing worker"));
                  return;
                }
                installing.addEventListener("statechange", () => {
                  events.push("statechange:" + installing.state);
                  publishProbe();
                  if (installing.state === "activated") {
                    resolve();
                  } else if (installing.state === "redundant") {
                    reject(new Error("replacement worker became redundant"));
                  }
                });
              }, { once: true });
            });
            await sw.register("worker-v2.js", { scope: "./" });
            await Promise.all([activated, controllerChanged]);
            publishProbe();
          })().catch(error => {
            globalThis.__serviceWorkerRuntimeEventsProbe = "error:" + String(error);
          });
        })()
        "#,
    )
    .expect("service worker runtime event probe should evaluate");

    let expected_v2_url = format!("{base_url}/app/worker-v2.js");
    let expected = format!(
        "updatefound:{expected_v2_url}:installing,statechange:installed,statechange:activating,controllerchange,statechange:activated|true|true|{expected_v2_url}"
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRuntimeEventsProbe)",
        &expected,
    )
    .await;
    server
        .await
        .expect("service worker runtime event script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_sync_register_dispatches_sync_event() {
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
              if (event.data === "register-worker-sync") {
                event.waitUntil(self.registration.sync.register("worker-sync"));
              }
            });
            self.addEventListener("sync", event => {
              event.waitUntil((async () => {
                const tags = await self.registration.sync.getTags();
                const windows = await clients.matchAll({ includeUncontrolled: true });
                if (windows[0]) {
                  windows[0].postMessage(JSON.stringify({
                    type: event.type,
                    tag: event.tag,
                    lastChance: event.lastChance,
                    tags: tags.slice().sort().join("|")
                  }));
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
          globalThis.__serviceWorkerSyncProbe = "pending";
          globalThis.__serviceWorkerSyncRegisterProbe = "pending";
          globalThis.__serviceWorkerSyncMessages = [];
          (async () => {
            const sw = navigator.serviceWorker;
            sw.onmessage = event => {
              globalThis.__serviceWorkerSyncMessages.push(event.data);
              globalThis.__serviceWorkerSyncProbe =
                JSON.stringify(globalThis.__serviceWorkerSyncMessages);
              if (globalThis.__serviceWorkerSyncMessages.length === 1) {
                globalThis.__serviceWorkerSyncRegistration.active.postMessage(
                  "register-worker-sync"
                );
              }
            };
            const registration = await sw.register("worker.js", { scope: "./" });
            await sw.ready;
            globalThis.__serviceWorkerSyncRegistration = registration;
            const tagsBefore = await registration.sync.getTags();
            const registerValue = await registration.sync.register("sync-tag");
            globalThis.__serviceWorkerSyncRegisterProbe = JSON.stringify({
              syncType: typeof registration.sync,
              registerType: typeof registration.sync.register,
              getTagsType: typeof registration.sync.getTags,
              tagsBeforeLength: tagsBefore.length,
              registerValue: String(registerValue)
            });
          })().catch(error => {
            globalThis.__serviceWorkerSyncProbe = "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker sync setup should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncRegisterProbe)",
        r#"{"syncType":"object","registerType":"function","getTagsType":"function","tagsBeforeLength":0,"registerValue":"undefined"}"#,
    )
    .await;
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncProbe)",
        r#"["{\"type\":\"sync\",\"tag\":\"sync-tag\",\"lastChance\":false,\"tags\":\"sync-tag\"}","{\"type\":\"sync\",\"tag\":\"worker-sync\",\"lastChance\":false,\"tags\":\"worker-sync\"}"]"#,
    )
    .await;

    server
        .await
        .expect("service worker sync script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_sync_register_respects_background_sync_permission() {
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
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("background-sync".to_owned()),
        setting: "denied".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerSyncPermissionProbe = "pending";
          (async () => {
            const permission = await navigator.permissions.query({
              name: "background-sync"
            });
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            let registerError = null;
            try {
              await registration.sync.register("denied-sync");
            } catch (error) {
              registerError = {
                name: error && error.name,
                message: error && error.message,
                isDomException: error instanceof DOMException
              };
            }
            const tags = await registration.sync.getTags();
            globalThis.__serviceWorkerSyncPermissionProbe = JSON.stringify({
              permission: permission.state,
              registerError,
              tagsLength: tags.length
            });
          })().catch(error => {
            globalThis.__serviceWorkerSyncPermissionProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker sync permission probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerSyncPermissionProbe)",
        r#"{"permission":"denied","registerError":{"name":"NotAllowedError","message":"Background Sync permission has not been granted.","isDomException":true},"tagsLength":0}"#,
    )
    .await;

    server
        .await
        .expect("service worker sync permission script server should finish");
}
#[tokio::test]
async fn navigator_service_worker_periodic_sync_register_respects_permission() {
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
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("periodic-background-sync".to_owned()),
        setting: "denied".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);

    vm.eval(
        r#"
        (() => {
          globalThis.__serviceWorkerPeriodicSyncPermissionProbe = "pending";
          (async () => {
            const permission = await navigator.permissions.query({
              name: "periodic-background-sync"
            });
            const registration = await navigator.serviceWorker.register("worker.js", {
              scope: "./"
            });
            await navigator.serviceWorker.ready;
            let registerError = null;
            try {
              await registration.periodicSync.register("denied-periodic", {
                minInterval: 60000
              });
            } catch (error) {
              registerError = {
                name: error && error.name,
                message: error && error.message,
                isDomException: error instanceof DOMException
              };
            }
            const tags = await registration.periodicSync.getTags();
            globalThis.__serviceWorkerPeriodicSyncPermissionProbe = JSON.stringify({
              permission: permission.state,
              registerError,
              tagsLength: tags.length
            });
          })().catch(error => {
            globalThis.__serviceWorkerPeriodicSyncPermissionProbe =
              "error:" + error.name + ":" + error.message;
          });
        })()
        "#,
    )
    .expect("service worker periodic sync permission probe should evaluate");

    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerPeriodicSyncPermissionProbe)",
        r#"{"permission":"denied","registerError":{"name":"NotAllowedError","message":"Periodic Background Sync permission has not been granted.","isDomException":true},"tagsLength":0}"#,
    )
    .await;

    server
        .await
        .expect("service worker periodic sync permission script server should finish");
}

#[tokio::test]
async fn navigator_service_worker_register_validates_urls_and_refreshes_cached_workers() {
    let worker_body = "self.addEventListener('install', () => {});";
    let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "no-store")],
            worker_body,
        ),
        (
            "/app/worker.js",
            "text/javascript; charset=utf-8",
            vec![("Cache-Control", "no-store")],
            worker_body,
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
              globalThis.__serviceWorkerRegistrationUrlProbe = "pending";
              (async () => {
                const sw = navigator.serviceWorker;
                const rejection = async (script, options) => {
                  try {
                    await sw.register(script, options);
                    return "resolved";
                  } catch (error) {
                    return [
                      error && error.name,
                      error instanceof TypeError,
                      error instanceof DOMException
                    ].join("|");
                  }
                };
                const encodedSlash = await rejection("worker%2f.js", { scope: "./scope/" });
                const dataScript = await rejection("data:text/javascript,", { scope: "./scope/" });
                const dataScope = await rejection("worker.js", { scope: "data:text/html," });

                const newest = registration =>
                  registration.installing || registration.waiting || registration.active;
                const first = await sw.register("././worker.js", { scope: "./scope/" });
                const firstScriptURL = newest(first) && newest(first).scriptURL;
                const unregistered = await first.unregister();
                const second = await sw.register("../app/worker.js", { scope: "./scope/" });
                const secondScriptURL = newest(second) && newest(second).scriptURL;
                // Clear Registration updates worker slots in later tasks.
                const cleared =
                  first.installing === null && first.waiting === null && first.active === null;

                globalThis.__serviceWorkerRegistrationUrlProbe = JSON.stringify({
                  encodedSlash,
                  dataScript,
                  dataScope,
                  firstScriptURL,
                  unregistered,
                  cleared,
                  secondScriptURL
                });
              })().catch((error) => {
                globalThis.__serviceWorkerRegistrationUrlProbe =
                  "error:" + String(error && error.message);
              });
            })()
            "#,
    )
    .expect("service worker registration URL probe should evaluate");

    let expected = format!(
        r#"{{"encodedSlash":"TypeError|true|false","dataScript":"TypeError|true|false","dataScope":"TypeError|true|false","firstScriptURL":"{base_url}/app/worker.js","unregistered":true,"cleared":true,"secondScriptURL":"{base_url}/app/worker.js"}}"#
    );
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(globalThis.__serviceWorkerRegistrationUrlProbe)",
        &expected,
    )
    .await;

    server
        .await
        .expect("service worker registration URL server should finish");
}

#[tokio::test]
async fn navigator_service_worker_update_preserves_identity_and_rejects_stale_receivers() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", "// first"),
        ("/app/workers/sw.js", "text/javascript", "// second"),
        ("/app/workers/sw.js", "text/javascript", "// second"),
        (
            "/app/workers/sw.js",
            "text/javascript",
            "// new registration",
        ),
        ("/app/workers/sw.js", "text/javascript", "// new update"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__updateIdentity = 'pending';
      (async () => {
        const activated = async r => {
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
        };
        const errorName = async p => { try { await p; return 'success'; } catch(e) { return e.name; } };
        const register = () => navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        const r = await register();
        let found = 0;
        r.addEventListener('updatefound', () => found++);
        await activated(r);
        const initial = found;
        const receivers = [];
        for (const fake of [{}, Object.create(r), new Proxy(r, {}), null]) {
          const p = Reflect.apply(r.update, fake, []);
          receivers.push(p instanceof Promise && await errorName(p) === 'TypeError');
        }
        const before = r.active;
        const order = [];
        r.addEventListener('updatefound', () => order.push('event'), {once:true});
        const updated = await r.update(); order.push('promise');
        await activated(r);
        const changed = before !== r.active;
        const current = r.active;
        const unchanged = await r.update({get unused(){throw new Error('argument read');}});
        const stable = unchanged === r && current === r.active && found === 2;
        await r.unregister();
        const removed = await errorName(r.update());
        const replacement = await register();
        await activated(replacement);
        const stale = await errorName(r.update());
        const again = await replacement.update();
        await activated(replacement);
        const lookup = await navigator.serviceWorker.getRegistration('workers/client');
        globalThis.__updateIdentity = [r.update.name, r.update.length, initial,
          receivers.every(Boolean), updated === r, changed, order.join(','), stable,
          removed, stale, replacement !== r, again === replacement, lookup === replacement].join('|');
        await replacement.unregister();
      })().catch(e => globalThis.__updateIdentity = 'error:' + e);
    "#).expect("update identity probe");
    drain_service_worker_test_until_eval_equals(&mut vm, &runtime, &loader,
        "String(globalThis.__updateIdentity)",
        "update|0|1|true|true|true|promise,event|true|InvalidStateError|InvalidStateError|true|true|true",
    ).await;
    server.await.expect("update identity server");
}

#[tokio::test]
async fn navigator_service_worker_update_reuses_failed_import_responses() {
    const MAIN: &str = "importScripts('a.js', 'z.js');";
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", MAIN),
        ("/app/workers/a.js", "text/javascript", "// a"),
        ("/app/workers/z.js", "text/javascript", "// z1"),
        ("/app/workers/sw.js", "text/javascript", MAIN),
        ("/app/workers/a.js", "text/html", "missing import"),
        ("/app/workers/z.js", "text/javascript", "// z2"),
        (
            "/app/workers/sw.js",
            "text/javascript",
            "importScripts('z.js');",
        ),
        ("/app/workers/z.js", "text/javascript", "// z3"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__updateImports = 'pending';
      (async () => {
        const activated = async r => {
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
        };
        const r = await navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        await activated(r);
        const before = r.active;
        let found = 0;
        r.addEventListener('updatefound', () => found++);
        let error = 'success';
        try { await r.update(); } catch(e) { error = e.name; }
        const failed = error === 'TypeError' && found === 0 && r.active === before && r.installing === null;
        const updated = await r.update();
        await activated(r);
        globalThis.__updateImports = [failed, updated === r, r.active !== before, found].join('|');
        await r.unregister();
      })().catch(e => globalThis.__updateImports = 'error:' + e);
    "#).expect("update import response probe");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__updateImports)",
        "true|true|true|1",
    )
    .await;
    server
        .await
        .expect("update import server should not refetch failed responses");
}

#[tokio::test]
async fn navigator_service_worker_update_discards_prefetched_but_unused_imports_after_install() {
    const MAIN: &str = r#"
      importScripts('flag.js');
      if (!self.skipOld) importScripts('unused.js');
      onmessage = e => {
        try { importScripts('unused.js'); e.ports[0].postMessage('success'); }
        catch(error) { e.ports[0].postMessage(error.name); }
      };
    "#;
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", MAIN),
        (
            "/app/workers/flag.js",
            "text/javascript",
            "self.skipOld = false;",
        ),
        ("/app/workers/unused.js", "text/javascript", "// unused"),
        ("/app/workers/sw.js", "text/javascript", MAIN),
        (
            "/app/workers/flag.js",
            "text/javascript",
            "self.skipOld = true;",
        ),
        ("/app/workers/unused.js", "text/javascript", "// unused"),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
      globalThis.__unusedUpdateImport = 'pending';
      (async () => {
        const activated = async r => {
          const w = r.installing || r.waiting || r.active;
          if (w.state !== 'activated') await new Promise(resolve => w.addEventListener('statechange', () => {
            if (w.state === 'activated') resolve();
          }));
        };
        const r = await navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        await activated(r);
        await r.update(); await activated(r);
        const value = await new Promise(resolve => {
          const c = new MessageChannel();
          c.port1.onmessage = e => { c.port1.close(); resolve(e.data); };
          r.active.postMessage('probe', [c.port2]);
        });
        globalThis.__unusedUpdateImport = value;
        await r.unregister();
      })().catch(e => globalThis.__unusedUpdateImport = 'error:' + e);
    "#).expect("unused update import probe");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__unusedUpdateImport)",
        "NetworkError",
    )
    .await;
    server
        .await
        .expect("prefetched imports should not be fetched again");
}

#[tokio::test]
async fn navigator_service_worker_update_in_worker_preserves_events_and_install_rejection() {
    let (base_url, server) = spawn_service_worker_response_server(vec![
        ("/app/workers/sw.js", "text/javascript", r#"
          const original = registration;
          const seen = [];
          let installError;
          original.addEventListener('updatefound', () => seen.push('before'));
          original.onupdatefound = function(e) {
            seen.push([this === original, e.target === original, e.currentTarget === original,
              e instanceof Event, e.isTrusted, e.bubbles, e.cancelable].join(':'));
          };
          original.addEventListener('updatefound', () => seen.push('after'));
          oninstall = e => e.waitUntil(original.update().then(
            () => installError = 'success', e => installError = e.name));
          onactivate = () => seen.push('activate');
          onmessage = e => {
            if (e.data === 'sample') e.ports[0].postMessage(seen.join(',') + '|' + installError);
            else e.waitUntil(original.update().then(r => e.ports[0].postMessage([
              r === original, r.installing?.state, seen.filter(event => event === 'before').length
            ].join(':'))));
          };
          Object.defineProperty(self, 'registration', {get(){throw new Error('public registration read');}});
        "#),
        ("/app/workers/sw.js", "text/javascript", "// updated"),
    ]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(
        r#"
      globalThis.__workerUpdate = 'pending';
      (async () => {
        const r = await navigator.serviceWorker.register('workers/sw.js', {scope:'workers/'});
        const w = r.installing;
        await new Promise(resolve => w.addEventListener('statechange', () => {
          if (w.state === 'activated') resolve();
        }));
        const message = data => new Promise(resolve => {
          const c = new MessageChannel();
          c.port1.onmessage = e => { c.port1.close(); resolve(e.data); };
          w.postMessage(data, [c.port2]);
        });
        const first = await message('sample');
        const same = await message('update');
        globalThis.__workerUpdate = first + '|' + same;
        await r.unregister();
      })().catch(e => globalThis.__workerUpdate = 'error:' + e);
    "#,
    )
    .expect("worker update probe");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(globalThis.__workerUpdate)",
        "before,true:true:true:true:true:false:false,after,activate|InvalidStateError|true:installing:1",
    )
    .await;
    server.await.expect("worker update server");
}

#[tokio::test]
async fn service_worker_registration_queries_settle_in_tasks_and_freeze_lists() {
    let (base_url, server) = spawn_service_worker_script_server(vec!["/app/empty.js"]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__queryTaskDone = false;
        globalThis.__queryTaskChecks = [];
        async function observeQuery(call, inspect) {
            let marker = false;
            const observed = call().then(value => [marker, inspect(value)].join(':'),
                error => [marker, error.name].join(':'));
            queueMicrotask(() => { marker = true; });
            return observed;
        }
        (async () => {
            const sw = navigator.serviceWorker;
            __queryTaskChecks.push(...await Promise.all([
                observeQuery(() => sw.getRegistration(), value => value === undefined),
                observeQuery(() => sw.getRegistrations(), value => Array.isArray(value) && Object.isFrozen(value) && value.length === 0),
                observeQuery(() => sw.getRegistration('http://['), () => 'unexpected'),
                observeQuery(() => sw.getRegistration('https://different-origin.test/'), () => 'unexpected')
            ]));
            const registration = await sw.register('empty.js');
            const worker = registration.installing || registration.waiting || registration.active;
            if (worker.state !== 'activated') await new Promise(resolve => worker.addEventListener('statechange', () => {
                if (worker.state === 'activated') resolve();
            }));
            __queryTaskChecks.push(...await Promise.all([
                observeQuery(() => sw.getRegistration('page.html#fragment'), value => value === registration),
                observeQuery(() => sw.getRegistrations(), value => Object.isFrozen(value) && value.length === 1 && value[0] === registration),
                observeQuery(() => sw.getRegistration('/unregistered/'), value => value === undefined)
            ]));
            const first = await sw.getRegistrations(), second = await sw.getRegistrations();
            __queryTaskChecks.push(first !== second && Object.getPrototypeOf(first) === Array.prototype);
        })().catch(error => { __queryTaskChecks.push('error:' + error.name + ':' + error.message); })
          .finally(() => { __queryTaskDone = true; });
    "#).unwrap();
    assert_eq!(vm.eval("String(__queryTaskChecks.length)").unwrap(), "0");
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__queryTaskDone)",
        "true",
    )
    .await;
    assert_eq!(
        vm.eval("__queryTaskChecks.join('|')").unwrap(),
        "true:true|true:true|false:TypeError|false:SecurityError|true:true|true:true|true:true|true"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_service_worker_registration_queries_settle_during_module_top_level_await() {
    const CLASSIC: &str = concat!(
        include_str!("service_worker_registration_query_worker.js"),
        "\nrun().then(value => postMessage(value), error => postMessage('error:' + error));"
    );
    const MODULE: &str = concat!(
        include_str!("service_worker_registration_query_worker.js"),
        "\npostMessage(await run());"
    );
    let (base_url, server) = spawn_service_worker_response_server(vec![
        (
            "/app/empty.js",
            "text/javascript",
            "// empty service worker",
        ),
        ("/app/query-classic.js", "text/javascript", CLASSIC),
        ("/app/query-module.js", "text/javascript", MODULE),
    ])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base_url}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.__workerQueryTaskDone = false;
        globalThis.__workerQueryTaskResults = [];
        (async () => {
            const reg = await navigator.serviceWorker.register('empty.js');
            const sw = reg.installing || reg.waiting || reg.active;
            if (sw.state !== 'activated') await new Promise(resolve => sw.addEventListener('statechange', () => {
                if (sw.state === 'activated') resolve();
            }));
            for (const type of ['classic', 'module']) {
                __workerQueryTaskResults.push(await new Promise((resolve, reject) => {
                    const worker = new Worker('query-' + type + '.js', {type});
                    worker.onmessage = event => { resolve(event.data); worker.terminate(); };
                    worker.onerror = event => reject(new Error(event.message));
                }));
            }
        })().catch(error => { __workerQueryTaskResults.push('error:' + error.name + ':' + error.message); })
          .finally(() => { __workerQueryTaskDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(__workerQueryTaskDone)",
        "true",
    )
    .await;
    let expected = "true:true|true:true|true:true|false:TypeError|false:SecurityError|true";
    assert_eq!(
        vm.eval("__workerQueryTaskResults.join(';')").unwrap(),
        format!("{expected};{expected}")
    );
    server.await.unwrap();
}
