use super::*;

#[tokio::test(flavor = "current_thread")]
async fn location_put_forwards_uses_the_target_href_setter_and_validates_document_receivers() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/location-put-forwards.js"
    ));
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><body>first child".to_owned(),
        "<!doctype html><body>second child".to_owned(),
        "<!doctype html><body>cross-origin child".to_owned(),
    ])
    .await;
    let loader = static_http_loader([
        server.resolve_entry("localhost"),
        server.resolve_entry("127.0.0.1"),
    ]);
    let parent_url = server.url_for_host("localhost", "/page.html");
    let child_url = server.url_for_host("localhost", "/child.html");
    let cross_url = server.url_for_host("127.0.0.1", "/child.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__locationPutForwardsResult = null;
({script})({{sameURL: {child_url:?}, crossURL: {cross_url:?}}}).then(
  result => {{ __locationPutForwardsResult = result; }},
  error => {{ __locationPutForwardsResult = {{error: String(error)}}; }}
);
"#,
            child_url = child_url.as_str(),
            cross_url = cross_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__locationPutForwardsResult !== null)",
        "true",
        "Location PutForwards cross-realm regression should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__locationPutForwardsResult)")
            .expect("Location PutForwards observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 324, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(server.finish_targets().await, ["/child.html"; 3]);
}

#[test]
fn location_put_forwards_preserves_null_and_undefined_for_href_conversion() {
    for family in ["window", "document", "constructed", "href"] {
        for value in ["null", "undefined"] {
            let mut vm = new_storage_test_vm("https://location-forwarding.test/folder/page.html");
            let result = vm
                .eval(&format!(
                    r#"
(() => {{
  const source = {family:?} === 'window' ? window :
    {family:?} === 'href' ? location :
    {family:?} === 'document' ? document : document.implementation.createHTMLDocument('');
  const receiver = {family:?} === 'window' ? window : {family:?} === 'href' ? location : document;
  const property = {family:?} === 'href' ? 'href' : 'location';
  const setter = Object.getOwnPropertyDescriptor(source, property).set;
  return setter.call(receiver, {value}) === undefined;
}})()
"#,
                ))
                .expect("forwarded Location assignment should evaluate");
            assert_eq!(result, "true", "{family} assignment of {value}");
            assert_eq!(
                vm.take_pending_location_navigation_with_seed()
                    .expect("forwarded Location assignment should queue navigation")
                    .url
                    .as_str(),
                format!("https://location-forwarding.test/folder/{value}"),
                "{family} must pass {value} unchanged to the href setter",
            );
        }
    }
}
