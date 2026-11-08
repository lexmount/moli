use super::*;

#[tokio::test]
async fn xhr_resolves_native_owners_in_window_dedicated_and_shared_workers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        "http://localhost/",
        &loader,
    );
    vm.eval(&format!(
        "globalThis.nativeOwnerXhrChecks = null; const probe = {}; probe().then(result => {{nativeOwnerXhrChecks = result;}}, error => {{nativeOwnerXhrChecks = {{error: String(error), stack: error.stack}};}});",
        include_str!("native_owner_xhr.js"),
    ))
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while vm.eval("nativeOwnerXhrChecks !== null").unwrap() != "true" {
            runtime.drain_shared_worker_service_lane();
            super::service_worker_drain::drain_service_worker_test_turn(&mut vm, &runtime, &loader)
                .await;
        }
    })
    .await
    .expect("XHR owner checks must finish through normal Page and Worker tasks");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(nativeOwnerXhrChecks)").unwrap()).unwrap();
    assert_eq!(result["total"], 18, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
