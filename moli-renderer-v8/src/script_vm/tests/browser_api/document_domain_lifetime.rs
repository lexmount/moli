use super::*;

#[tokio::test(flavor = "current_thread")]
async fn document_domain_survives_stream_replacement_and_pending_navigation() {
    const HOST: &str = "document-domain-lifetime.test";
    let child = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/document-open-origin-child.html"
    ));
    let same = StaticHttpServer::spawn_with_bodies(vec![child.to_owned()]).await;
    let cross = StaticHttpServer::spawn_with_bodies(vec![
        child.to_owned(),
        "<!doctype html><body>replacement".to_owned(),
    ])
    .await;
    let loader = static_http_loader([same.resolve_entry(HOST), cross.resolve_entry(HOST)]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        same.url_for_host(HOST, "/entry.html").as_str(),
        "<!doctype html><body>parent",
        &loader,
    );
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/document-domain-lifetime.js"
    ));
    vm.exec(
        &format!(
            r#"
globalThis.__documentDomainLifetimeResult = null;
({script})({{sameURL:{same_url:?},crossURL:{cross_url:?},replacementURL:{replacement_url:?}}}).then(
  result => {{__documentDomainLifetimeResult = result}},
  error => {{__documentDomainLifetimeResult = {{error:String(error)}}}}
);
"#,
            same_url = same.url_for_host(HOST, "/child.html").as_str(),
            cross_url = cross.url_for_host(HOST, "/child.html").as_str(),
            replacement_url = cross.url_for_host(HOST, "/replacement.html").as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__documentDomainLifetimeResult !== null)",
        "true",
        "document.domain lifetime checks should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__documentDomainLifetimeResult)")
            .unwrap(),
    )
    .unwrap();
    assert!(
        result["checks"].as_u64().is_some_and(|count| count >= 80),
        "{result}"
    );
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(same.finish_targets().await, ["/child.html"]);
    assert_eq!(
        cross.finish_targets().await,
        ["/child.html", "/replacement.html"]
    );
}
