use super::*;

#[tokio::test(flavor = "current_thread")]
async fn import_meta_resolve_preserves_conversion_exceptions_across_window_realms() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/import-meta-conversion.js"
    ));
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://import-meta-conversion.test/index.html",
        &loader,
    );
    vm.exec(
        &format!(
            r#"globalThis.__importMetaConversion = null;
({script})().then(
    result => {{ __importMetaConversion = result; }},
    error => {{ __importMetaConversion = {{error: String(error)}}; }}
);"#
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__importMetaConversion !== null)",
        "true",
        "import.meta.resolve conversion regression should finish",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__importMetaConversion)").unwrap()).unwrap();
    assert_eq!(result["completed"], true, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
