use super::*;

#[tokio::test(flavor = "current_thread")]
async fn window_restricted_accessors_validate_receivers_before_conversion_and_materialization() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/window-restricted-accessors.js"
    ));
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><body>local".to_owned(),
        "<!doctype html><body>remote".to_owned(),
    ])
    .await;
    let loader = static_http_loader([
        server.resolve_entry("localhost"),
        server.resolve_entry("127.0.0.1"),
    ]);
    let parent_url = server.url_for_host("localhost", "/page.html");
    let same_url = server.url_for_host("localhost", "/local.html");
    let cross_url = server.url_for_host("127.0.0.1", "/remote.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__restrictedAccessorsResult = null;
({script})({{sameURL: {same_url:?}, crossURL: {cross_url:?}}}).then(
  result => {{ __restrictedAccessorsResult = result; }},
  error => {{ __restrictedAccessorsResult = {{error: String(error)}}; }}
);
"#,
            same_url = same_url.as_str(),
            cross_url = cross_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__restrictedAccessorsResult !== null)",
        "true",
        "Window restricted accessor regression should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__restrictedAccessorsResult)")
            .expect("Window restricted accessor observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 1336, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        ["/local.html", "/remote.html"]
    );
}
