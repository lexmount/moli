use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_window_indexes_keep_live_native_child_identities() {
    let child = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-window-index-child.html"
    ));
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-window-indexes.js"
    ));
    for nested in [false, true] {
        let mut bodies = Vec::new();
        let mut targets = Vec::new();
        if nested {
            bodies.push("<!doctype html><body>host</body>".to_owned());
            targets.push("/host.html");
        }
        bodies.extend(vec![child.to_owned(); 5]);
        targets.extend(vec!["/child.html"; 5]);
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let loader = static_http_loader([
            server.resolve_entry("www.example.test"),
            server.resolve_entry("remote.example.test"),
        ]);
        let parent_url = server.url_for_host("www.example.test", "/page.html");
        let same_url = server.url_for_host("www.example.test", "/child.html");
        let cross_url = server.url_for_host("remote.example.test", "/child.html");
        let host_url = server.url_for_host("www.example.test", "/host.html");
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        // Only the page's saved method/getter references keep the caller surface
        // alive across collections; the native cache itself holds weak handles.
        let context = vm.page_default_context.clone();
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let callback =
                    v8::Function::new(scope, force_gc_and_report_inspector_policy_for_test)
                        .expect("test GC callback");
                let key = v8::String::new(scope, "__collectWindowGarbage").unwrap();
                assert_eq!(
                    context
                        .global(scope)
                        .set(scope, key.into(), callback.into()),
                    Some(true)
                );
                Ok(())
            })
            .unwrap();
        vm.exec(
            &format!(
                r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__windowIndexResult = null;
({script})({{sameURL: {same_url:?}, crossURL: {cross_url:?}, hostURL: {host_url:?}, nested: {nested}, collectGarbage: __collectWindowGarbage}}).then(
  result => {{ __windowIndexResult = result; }},
  error => {{ __windowIndexResult = {{error: String(error)}}; }}
);
"#,
                same_url = same_url.as_str(),
                cross_url = cross_url.as_str(),
                host_url = host_url.as_str(),
            ),
            None,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__windowIndexResult !== null)",
            "true",
            "cross-origin Window index probe should finish",
        )
        .await;
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval("JSON.stringify(__windowIndexResult)")
                .expect("Window index observations"),
        )
        .unwrap();
        assert_eq!(result["checks"], 148, "nested={nested}: {result}");
        assert_eq!(
            result["failures"],
            serde_json::json!([]),
            "nested={nested}: {result}"
        );
        assert_eq!(server.finish_targets().await, targets);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cached_cross_origin_top_window_indexes_follow_document_domain_changes() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><body>domain child</body>".to_owned(),
    ])
    .await;
    let loader = static_http_loader([server.resolve_entry("www.example.test")]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let same_url = server.url_for_host("www.example.test", "/child.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cross-origin-window-domain.js"
    ));
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__windowDomainResult = null;
({script})({same_url:?}).then(
  result => {{ __windowDomainResult = result; }},
  error => {{ __windowDomainResult = {{error: String(error)}}; }}
);
"#,
            same_url = same_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__windowDomainResult !== null)",
        "true",
        "Window domain transition probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__windowDomainResult)")
            .expect("Window domain observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 21, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}
