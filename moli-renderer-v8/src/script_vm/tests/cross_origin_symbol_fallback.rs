use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_symbol_fallback_preserves_origin_and_realm_boundaries() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-symbol-fallback.js"
    ));
    let child = format!("<!doctype html><body><script>({script})({{role:'child'}})</script>");
    let server = StaticHttpServer::spawn_with_bodies(vec![child; 4]).await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let cross_url = server.url_for_host("remote.example.test", "/child.html");
    let same_url = server.url_for_host("www.example.test", "/child.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__symbolFallbackResult = null;
({script})({{crossURL: {cross_url:?}, sameURL: {same_url:?}}}).then(
  result => {{ __symbolFallbackResult = result; }},
  error => {{ __symbolFallbackResult = {{error: String(error)}}; }}
);
"#,
            cross_url = cross_url.as_str(),
            same_url = same_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__symbolFallbackResult !== null)",
        "true",
        "cross-origin symbol fallback probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__symbolFallbackResult)")
            .expect("cross-origin symbol fallback observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 232, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(server.finish_targets().await, vec!["/child.html"; 4]);
}
