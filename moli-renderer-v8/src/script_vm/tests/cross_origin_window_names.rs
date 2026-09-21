use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_window_names_follow_target_names_and_origin_filtering() {
    let host = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-window-names-host.html"
    ));
    let child = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-window-names-child.html"
    ));
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-window-names.js"
    ));
    let server = StaticHttpServer::spawn_with_bodies(vec![
        host.to_owned(),
        child.to_owned(),
        child.to_owned(),
        child.to_owned(),
    ])
    .await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let host_url = server.url_for_host("remote.example.test", "/host.html");
    let child_url = server.url_for_host("www.example.test", "/child.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__windowNamesResult = null;
({script})({{hostURL: {host_url:?}, crossURL: {child_url:?}}}).then(
  result => {{ __windowNamesResult = result; }},
  error => {{ __windowNamesResult = {{error: String(error)}}; }}
);
"#,
            host_url = host_url.as_str(),
            child_url = child_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__windowNamesResult !== null)",
        "true",
        "Window named property probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__windowNamesResult)")
            .expect("Window named property observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 97, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        vec!["/host.html", "/child.html", "/child.html", "/child.html"]
    );
}
