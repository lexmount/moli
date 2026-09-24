use super::*;

#[tokio::test]
async fn location_proxy_forwarding_preserves_entry_settings_and_caller_exceptions() {
    let script = include_str!("../../../../tests/fixtures/location-entry-settings.js");
    let child = format!("<!doctype html><body><script>({script})({{role:'child'}})</script>");
    // One entry document, its incumbent and 26 navigation targets, each loaded
    // before and after its Location operation.
    let server = StaticHttpServer::spawn_with_bodies(vec![child; 54]).await;
    let loader = static_http_loader([
        server.resolve_entry("localhost"),
        server.resolve_entry("127.0.0.1"),
    ]);
    let source = server.url_for_host("localhost", "/top/page.html");
    let entry = server.url_for_host("localhost", "/entry/page.html");
    let mut vm = new_parsed_page_task_executor_test_vm(
        source.as_str(),
        "<!doctype html><body>source",
        &loader,
    );
    vm.eval(&format!(
        "globalThis.locationEntryResult = null; \
         ({script})({{entryURL:{entry:?}}}).then(\
           result => locationEntryResult = result,\
           error => locationEntryResult = {{error:String(error)}});",
        entry = entry.as_str(),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(locationEntryResult !== null)",
        "true",
        "cross-realm Location entry settings",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(locationEntryResult)").unwrap()).unwrap();
    assert_eq!(result["checks"], 60, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    let requests = server.finish_targets().await;
    assert_eq!(requests.len(), 54);
    assert_eq!(
        requests
            .iter()
            .filter(|path| path.starts_with("/converted/"))
            .count(),
        26,
    );
}
