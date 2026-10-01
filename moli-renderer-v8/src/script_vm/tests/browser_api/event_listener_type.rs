use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

#[test]
fn event_listener_types_preserve_utf16_identity_for_native_and_wrapper_local_targets() {
    let mut vm = new_storage_html_test_vm("https://event-listener-type.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    assert_eq!(
        vm.eval(include_str!("event_listener_type.js")).unwrap(),
        "true"
    );
}

#[tokio::test]
async fn event_listener_types_preserve_utf16_identity_in_workers_indexed_db_and_observable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://event-listener-type-runtime.test/",
            &loader,
        );
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    assert_eq!(
        vm.eval(include_str!("event_listener_type.js")).unwrap(),
        "true"
    );
    vm.eval(&format!(
        "({}).then(result => globalThis.__listenerTypeDone = result, error => globalThis.__listenerTypeDone = String(error));",
        include_str!("event_listener_type_runtime.js")
    )).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm
            .eval("globalThis.__listenerTypeDone !== undefined")
            .unwrap()
            != "true"
        {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("worker and IndexedDB listener type fixture should settle");
    assert_eq!(vm.eval("globalThis.__listenerTypeDone").unwrap(), "true");
}
