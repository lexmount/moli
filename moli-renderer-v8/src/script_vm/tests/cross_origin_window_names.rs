use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_window_names_require_embedder_authorization() {
    let child = include_str!("../../../tests/fixtures/window-frame-names-child.html");
    let script = include_str!("../../../tests/fixtures/window-frame-container-names.js");
    for kind in ["main", "child", "popup"] {
        let server = StaticHttpServer::spawn_with_bodies(vec![child.to_owned(); 2]).await;
        let loader = static_http_loader([
            server.resolve_entry("owner.example.test"),
            server.resolve_entry("peer.example.test"),
        ]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            server
                .url_for_host("owner.example.test", "/page.html")
                .as_str(),
            "<!doctype html><body>owner",
            &loader,
        );
        vm.exec(
            &format!(
                r#"
globalThis.__frameContainerNames = null;
(async () => {{
  let ownerWindow = window, frame;
  if ({kind:?} === 'child') {{
    frame = document.createElement('iframe');
    frame.srcdoc = '<!doctype html><body>owner';
    const loaded = new Promise(resolve => {{frame.onload = resolve}});
    document.body.append(frame);
    await loaded;
    ownerWindow = frame.contentWindow;
  }} else if ({kind:?} === 'popup') {{
    ownerWindow = open('about:blank');
  }}
  try {{
    return await ({script})({{childURL:{child_url:?}, ownerWindow}});
  }} finally {{
    if (frame) frame.remove();
    if ({kind:?} === 'popup') ownerWindow.close();
  }}
}})().then(
  result => {{__frameContainerNames = result}},
  error => {{__frameContainerNames = {{error:String(error)}}}}
);
"#,
                child_url = server
                    .url_for_host("peer.example.test", "/child.html")
                    .as_str(),
            ),
            None,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__frameContainerNames !== null)",
            "true",
            "frame container name checks should finish",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(__frameContainerNames)").unwrap())
                .unwrap();
        assert_eq!(result["checks"], 32, "{kind}: {result}");
        assert_eq!(
            result["failures"],
            serde_json::json!([]),
            "{kind}: {result}"
        );
        assert_eq!(
            server.finish_targets().await,
            ["/child.html", "/child.html"]
        );
    }
}

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
