use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

#[tokio::test]
async fn css_unparsed_values_work_in_dedicated_worker_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://css-unparsed-worker.test/",
            &loader,
        );
    let source = include_str!("css_unparsed_worker.js");
    vm.eval(&format!(
        "({source}).then(value => globalThis.__cssUnparsedWorker = value, error => globalThis.__cssUnparsedWorker = String(error));"
    ))
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm
            .eval("globalThis.__cssUnparsedWorker !== undefined")
            .unwrap()
            != "true"
        {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("CSS Typed OM worker probe should settle");
    assert_eq!(vm.eval("globalThis.__cssUnparsedWorker").unwrap(), "true");
}
