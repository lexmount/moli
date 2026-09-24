use super::*;

#[tokio::test(flavor = "current_thread")]
async fn frame_element_checks_receiver_and_container_origins_in_the_getter_realm() {
    let child = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/frame-element-security-child.html"
    ));
    let server = StaticHttpServer::spawn_with_bodies(vec![child.to_owned(); 6]).await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let same_url = server.url_for_host("www.example.test", "/child.html");
    let cross_url = server.url_for_host("remote.example.test", "/child.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/frame-element-security.js"
    ));
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__frameElementResult = null;
({script})({{sameURL: {same_url:?}, crossURL: {cross_url:?}}}).then(
  result => {{ __frameElementResult = result; }},
  error => {{ __frameElementResult = {{error: String(error)}}; }}
);
"#,
            same_url = same_url.as_str(),
            cross_url = cross_url.as_str(),
        ),
        None,
    )
    .expect("frameElement origin probe should start");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__frameElementResult !== null)",
        "true",
        "frameElement origin probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__frameElementResult)")
            .expect("frameElement observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 28, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        ["/child.html", "/child.html?nested"].repeat(3)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn frame_element_uses_the_isolated_world_access_policy() {
    for cross_origin in [false, true] {
        let server = StaticHttpServer::spawn_with_bodies(vec![
            "<!doctype html><body>isolated frameElement</body>".to_owned(),
        ])
        .await;
        let loader = static_http_loader([
            server.resolve_entry("www.example.test"),
            server.resolve_entry("remote.example.test"),
        ]);
        let parent_url = server.url_for_host("www.example.test", "/page.html");
        let child_url = server.url_for_host(
            if cross_origin {
                "remote.example.test"
            } else {
                "www.example.test"
            },
            "/child.html",
        );
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        vm.exec(
            &format!(
                r#"
globalThis.loaded = false;
const frame = document.createElement('iframe');
frame.onload = () => {{ loaded = true; }};
frame.src = {child_url:?};
(document.body || document.documentElement || document).appendChild(frame);
"#,
                child_url = child_url.as_str(),
            ),
            None,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loaded)",
            "true",
            "child should load before creating isolated worlds",
        )
        .await;
        let frame_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .next()
            .and_then(|realm| realm.frame_id)
            .expect("child frame id");
        for universal in [false, true] {
            let context_id = vm
                .create_isolated_world_for_frame(
                    &frame_id,
                    if universal { "universal" } else { "enforced" },
                    universal,
                )
                .expect("child isolated world");
            assert_eq!(
                vm.eval_in_isolated_context(context_id, "String(frameElement === null)")
                    .expect("isolated frameElement"),
                (cross_origin && !universal).to_string(),
                "cross_origin={cross_origin}, universal={universal}"
            );
        }
        assert_eq!(server.finish_targets().await, vec!["/child.html"]);
    }
}
