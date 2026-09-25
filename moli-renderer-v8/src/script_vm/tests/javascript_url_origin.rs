use super::*;

async fn assert_child_javascript_origin(
    action: &str,
    child_host: &str,
    child_domain: Option<&str>,
    parent_domain: Option<&str>,
    after_navigation: &str,
    should_execute: bool,
) {
    let domain_script = child_domain
        .map(|domain| format!("document.domain = {domain:?};"))
        .unwrap_or_default();
    let server = StaticHttpServer::spawn_with_bodies(vec![format!(
        "<!doctype html><script>{domain_script}</script><body>child"
    )])
    .await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let opener_url = server.url_for_host("www.example.test", "/parent");
    let child_url = server.url_for_host(child_host, "/child");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
    vm.eval(&format!(
        r#"
        globalThis.frameReady = false;
        globalThis.frame = document.createElement('iframe');
        frame.name = 'target';
        frame.onload = () => frameReady = true;
        frame.src = {:?};
        document.body.append(frame);
        "#,
        child_url.as_str(),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(frameReady)",
        "true",
        "javascript URL target loaded",
    )
    .await;
    let parent_domain_script = parent_domain
        .map(|domain| format!("document.domain = {domain:?};"))
        .unwrap_or_default();
    let navigation = match action {
        "src" => "frame.src = code;",
        "attribute" => "frame.setAttribute('src', code);",
        "href" => "frame.contentWindow.location.href = code;",
        "window-location" => "frame.contentWindow.location = code;",
        "replace" => "frame.contentWindow.location.replace(code);",
        "form" => {
            "const form = document.createElement('form'); form.target = 'target'; form.action = code; document.body.append(form); form.submit();"
        }
        "hyperlink" => {
            "const link = document.createElement('a'); link.target = 'target'; link.href = code; document.body.append(link); link.click();"
        }
        _ => panic!("unknown javascript URL entry point"),
    };
    vm.eval(&format!(
        r#"
        {parent_domain_script}
        globalThis.messages = [];
        onmessage = event => messages.push(event.data);
        globalThis.originProbeDone = false;
        const code = "javascript:parent.postMessage('ran', '*');void 0";
        {navigation}
        {after_navigation}
        requestIdleCallback(() => originProbeDone = true);
        "#,
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(originProbeDone)",
        "true",
        "javascript URL task settled",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(messages)").unwrap(),
        if should_execute { r#"["ran"]"# } else { "[]" },
        "action={action}, child={child_host}, child domain={child_domain:?}, parent domain={parent_domain:?}, after={after_navigation}",
    );
    assert_eq!(server.finish_targets().await, ["/child"]);
}

#[tokio::test]
async fn child_javascript_url_checks_initiator_origin_across_navigation_entry_points() {
    for action in [
        "src",
        "attribute",
        "href",
        "window-location",
        "replace",
        "form",
        "hyperlink",
    ] {
        for (child_host, child_domain, parent_domain, allowed) in [
            ("www.example.test", None, None, true),
            ("remote.example.test", None, None, false),
            ("www.example.test", Some("www.example.test"), None, false),
            ("remote.example.test", Some("example.test"), None, false),
            (
                "remote.example.test",
                Some("example.test"),
                Some("example.test"),
                true,
            ),
        ] {
            assert_child_javascript_origin(
                action,
                child_host,
                child_domain,
                parent_domain,
                "",
                allowed,
            )
            .await;
        }
    }
}

#[tokio::test]
async fn child_javascript_url_snapshots_source_domain_and_rechecks_target_domain() {
    for action in ["src", "attribute", "href", "replace"] {
        // Changing the source after the request must not change its captured origin.
        assert_child_javascript_origin(
            action,
            "www.example.test",
            None,
            None,
            "document.domain = 'example.test';",
            true,
        )
        .await;
        assert_child_javascript_origin(
            action,
            "www.example.test",
            Some("example.test"),
            None,
            "document.domain = 'example.test';",
            false,
        )
        .await;
        // The target, in contrast, is checked when the queued navigation executes.
        assert_child_javascript_origin(
            action,
            "www.example.test",
            None,
            None,
            "frame.contentDocument.domain = 'example.test';",
            false,
        )
        .await;
    }
}

#[tokio::test]
async fn child_javascript_url_uses_popup_initiator_instead_of_its_opener_realm() {
    for child_opener in [false, true] {
        for (child_host, allowed) in [("www.example.test", false), ("remote.example.test", true)] {
            let server = StaticHttpServer::spawn_with_bodies(vec![
                "<!doctype html><body>child".into(),
                r#"<!doctype html><script>
                onload = () => {
                    opener.frames[0].location.href = "javascript:top.postMessage('ran', '*');void 0";
                    opener.top.postMessage('queued', '*');
                };
                </script>"#.into(),
            ]).await;
            let loader = static_http_loader([
                server.resolve_entry("www.example.test"),
                server.resolve_entry("remote.example.test"),
            ]);
            let main_url = server.url_for_host("www.example.test", "/main");
            let child_url = server.url_for_host(child_host, "/child");
            let popup_url = server.url_for_host("remote.example.test", "/popup");
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(main_url.as_str(), &loader);
            vm.eval(&format!(
                r#"
                globalThis.source = window;
                if ({child_opener}) {{
                    const outer = document.createElement('iframe'); document.body.append(outer);
                    source = outer.contentWindow;
                }}
                globalThis.frameReady = false;
                globalThis.frame = source.document.createElement('iframe');
                frame.onload = () => frameReady = true;
                frame.src = {:?}; source.document.body.append(frame);
                globalThis.messages = []; globalThis.queued = false;
                onmessage = event => {{
                    if (event.data === 'queued') queued = true;
                    else messages.push(event.data);
                }};
            "#,
                child_url.as_str()
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(frameReady)",
                "true",
                "popup navigation target loaded",
            )
            .await;
            vm.eval(&format!(
                "globalThis.popup = source.open({:?});",
                popup_url.as_str()
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(queued)",
                "true",
                "popup queued child javascript URL",
            )
            .await;
            vm.eval("globalThis.settled = false; requestIdleCallback(() => settled = true);")
                .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(settled)",
                "true",
                "popup javascript navigation settled",
            )
            .await;
            assert_eq!(
                vm.eval("JSON.stringify(messages)").unwrap(),
                if allowed { r#"["ran"]"# } else { "[]" },
                "child opener={child_opener}, child={child_host}"
            );
            vm.eval("popup.close()").unwrap();
            assert_eq!(server.finish_targets().await, ["/child", "/popup"]);
        }
    }
}

#[tokio::test]
async fn child_javascript_url_denial_preserves_document_history_and_later_navigation() {
    let server =
        StaticHttpServer::spawn_with_bodies(vec!["<!doctype html><body>child".into()]).await;
    let loader = static_http_loader([server.resolve_entry("www.example.test")]);
    let main_url = server.url_for_host("www.example.test", "/main");
    let child_url = server.url_for_host("www.example.test", "/child");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(main_url.as_str(), &loader);
    vm.eval(&format!(
        r#"
        globalThis.loads = 0;
        globalThis.frame = document.createElement('iframe');
        frame.onload = () => ++loads;
        frame.src = {:?}; document.body.append(frame);
    "#,
        child_url.as_str()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(loads)",
        "1",
        "denied navigation target loaded",
    )
    .await;
    vm.eval(
        r#"
        globalThis.originalDocument = frame.contentDocument;
        globalThis.originalHistoryLength = frame.contentWindow.history.length;
        globalThis.messages = [];
        onmessage = event => messages.push(event.data);
        document.domain = 'example.test';
        frame.src = "javascript:parent.postMessage('denied', '*');'<p>replaced</p>'";
        globalThis.settled = false; requestIdleCallback(() => settled = true);
    "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(settled)",
        "true",
        "denied navigation settled",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"
        originalDocument.domain = 'example.test';
        JSON.stringify([
            messages, loads, frame.contentDocument === originalDocument,
            frame.contentDocument.body.textContent,
            frame.contentWindow.history.length === originalHistoryLength,
            frame.contentWindow.location.href === originalDocument.URL,
            new URL(originalDocument.URL).pathname
        ])
    "#
        )
        .unwrap(),
        r#"[[],1,true,"child",true,true,"/child"]"#
    );
    vm.eval("frame.contentWindow.location.href = \"javascript:parent.postMessage('allowed', '*');void 0\";").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(messages)",
        r#"["allowed"]"#,
        "authorized navigation after denial",
    )
    .await;
    assert_eq!(server.finish_targets().await, ["/child"]);
}
