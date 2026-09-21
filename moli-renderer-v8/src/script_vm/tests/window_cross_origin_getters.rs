use super::*;

#[tokio::test(flavor = "current_thread")]
async fn window_cross_origin_getters_preserve_receiver_identity_security_and_lifecycle() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/window-cross-origin-getters.js"
    ));
    let child = format!(
        "<!doctype html><body><iframe srcdoc='one'></iframe><iframe srcdoc='two'></iframe><script>({script})({{role:'child'}})</script>"
    );
    let server = StaticHttpServer::spawn_with_bodies(vec![child; 5]).await;
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
globalThis.__windowGetterResult = null;
({script})({{crossURL: {cross_url:?}, sameURL: {same_url:?}}}).then(
  result => {{ __windowGetterResult = result; }},
  error => {{ __windowGetterResult = {{error: String(error)}}; }}
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
        "String(__windowGetterResult !== null)",
        "true",
        "cross-origin Window getter probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__windowGetterResult)")
            .expect("cross-origin Window getter observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 391, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(server.finish_targets().await, vec!["/child.html"; 5]);
}
