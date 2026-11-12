use super::*;

#[tokio::test]
async fn navigation_preload_inactive_registration_reads_state_in_window_and_worker() {
    let (base_url, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
        let releaseInstall;
        const installation = new Promise(resolve => { releaseInstall = resolve; });
        self.addEventListener('install', event => event.waitUntil(installation));
        self.addEventListener('activate', event => event.waitUntil(clients.claim()));
        self.addEventListener('message', event => {
            event.waitUntil((async () => {
                const manager = self.registration.navigationPreload;
                const activeIsNull = self.registration.active === null;
                const state = await manager.getState().catch(error => ({error: error.name}));
                const mutations = await Promise.all([
                    manager.enable(), manager.disable(), manager.setHeaderValue('blocked')
                ].map(promise => promise.then(() => 'resolved', error => error.name)));
                event.source.postMessage({activeIsNull, state, mutations});
                releaseInstall();
            })());
        });
        "#,
    )])
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            &format!("{base_url}/app/page.html"),
            &loader,
        );
    vm.eval(
        r#"
        Object.defineProperty(globalThis, "__moliTestActivatedServiceWorker", {
          configurable: true,
          value: async function(container = navigator.serviceWorker) {
            const registration = await container.ready;
            const worker = registration.active;
            if (worker.state !== "activated") {
              await new Promise((resolve, reject) => {
                const changed = () => {
                  if (worker.state === "activated" || worker.state === "redundant") {
                    worker.removeEventListener("statechange", changed);
                    if (worker.state === "activated") resolve();
                    else reject(new Error("worker became redundant before activation"));
                  }
                };
                worker.addEventListener("statechange", changed);
              });
            }
            return registration;
          }
        });
        void 0;
        "#,
    )
    .expect("service worker activation fixture helper should install");
    vm.eval(
        r#"
        globalThis.navigationPreloadInactiveResult = 'pending';
        (async () => {
            const sw = navigator.serviceWorker;
            const registration = await sw.register('worker.js', {scope: './'});
            const manager = registration.navigationPreload;
            const activeIsNull = registration.active === null;
            const state = await manager.getState().catch(error => ({error: error.name}));
            const mutations = await Promise.all([
                manager.enable(), manager.disable(), manager.setHeaderValue('blocked')
            ].map(promise => promise.then(() => 'resolved', error => error.name)));
            const worker = await new Promise(resolve => {
                sw.onmessage = event => resolve(event.data);
                registration.installing.postMessage('probe');
            });
            await __moliTestActivatedServiceWorker(sw);
            await manager.enable();
            const afterActivation = await manager.getState();
            await registration.unregister();
            return JSON.stringify({window: {activeIsNull, state, mutations}, worker, afterActivation});
        })().then(value => navigationPreloadInactiveResult = value,
                  error => navigationPreloadInactiveResult = JSON.stringify({error: String(error)}));
        "#,
    )
    .unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &browser_context_runtime,
        &loader,
        "String(navigationPreloadInactiveResult !== 'pending')",
        "true",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("navigationPreloadInactiveResult").unwrap()).unwrap();
    let before_activation = serde_json::json!({
        "activeIsNull": true,
        "state": {"enabled": false, "headerValue": "true"},
        "mutations": ["InvalidStateError", "InvalidStateError", "InvalidStateError"]
    });
    assert_eq!(
        result,
        serde_json::json!({
            "window": before_activation,
            "worker": before_activation,
            "afterActivation": {"enabled": true, "headerValue": "true"}
        })
    );
    server.await.unwrap();
}
