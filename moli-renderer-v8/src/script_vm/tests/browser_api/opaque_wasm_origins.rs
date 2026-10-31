use super::*;

#[tokio::test]
async fn opaque_wasm_origins_follow_window_security_instead_of_storage_nonces() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader("about:blank", &loader);
    vm.eval(&format!(
        "globalThis.opaqueWasmChecks = null; const probe = {}; probe().then(result => {{opaqueWasmChecks = result;}}, error => {{opaqueWasmChecks = {{error: String(error), stack: error.stack}};}});",
        include_str!("opaque_wasm_origins.js"),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "opaqueWasmChecks !== null",
        "true",
        "opaque-origin Wasm probes must finish through normal Page tasks",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(opaqueWasmChecks)").unwrap()).unwrap();
    assert_eq!(result["total"], 20, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["origin"], "null", "{result}");
}
