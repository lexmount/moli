use super::*;

#[tokio::test]
async fn borrowed_service_worker_register_uses_the_container_promise_and_error_realm() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        "https://register-realm.test/app/page.html",
        &loader,
    );
    vm.eval(r#"
        globalThis.registerRealmDone = false;
        (async () => {
            const frame = document.createElement('iframe');
            (document.body || document.documentElement || document).append(frame);
            const child = frame.contentWindow;
            const rootMethod = navigator.serviceWorker.register;
            const childMethod = child.navigator.serviceWorker.register;
            const rows = [];
            for (const [method, container, own, other] of [
                [rootMethod, child.navigator.serviceWorker, child, window],
                [childMethod, navigator.serviceWorker, window, child],
            ]) {
                const promise = method.call(container, 'http://[');
                const promiseRealm = promise instanceof own.Promise && !(promise instanceof other.Promise);
                const errorRealm = await promise.then(() => false,
                    error => error instanceof own.TypeError && !(error instanceof other.TypeError));
                const sentinel = {};
                const conversion = method.call(container, {toString() { throw sentinel; }});
                const preservedConversion = await conversion.then(() => false, error => error === sentinel);
                const conversionRealm = conversion instanceof other.Promise && !(conversion instanceof own.Promise);
                rows.push({promiseRealm, errorRealm, preservedConversion, conversionRealm});
            }
            globalThis.registerRealmResult = JSON.stringify(rows);
        })().catch(error => { globalThis.registerRealmResult = String(error); })
            .finally(() => { globalThis.registerRealmDone = true; });
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(registerRealmDone)",
        "true",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("registerRealmResult").unwrap()).unwrap();
    assert_eq!(
        result,
        serde_json::json!([
            {"promiseRealm":true,"errorRealm":true,"preservedConversion":true,"conversionRealm":true},
            {"promiseRealm":true,"errorRealm":true,"preservedConversion":true,"conversionRealm":true}
        ])
    );
}
