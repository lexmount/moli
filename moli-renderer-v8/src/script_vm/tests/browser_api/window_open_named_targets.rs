use super::*;

#[tokio::test]
async fn named_window_targets_filter_unfamiliar_related_frames() {
    for mode in ["same-origin", "cross-origin", "later-same-origin"] {
        let mut bodies = vec![
            r#"<!doctype html><body><iframe name="related-target"></iframe><script>
            const childURL = new URL(location.href).searchParams.get('child');
            if (childURL) {
                const child = document.createElement('iframe');
                child.name = 'related-target'; child.src = childURL;
                document.body.append(child);
            } else {
                opener.postMessage('ready', '*');
            }
            </script>"#
                .into(),
        ];
        if mode == "later-same-origin" {
            bodies.push(
                "<!doctype html><script>parent.opener.postMessage('ready', '*')</script>".into(),
            );
        }
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let origin = server.base_url().origin().ascii_serialization();
        let mut popup_url = server.base_url().join("popup").unwrap();
        if mode != "same-origin" {
            popup_url.set_host(Some("localhost")).unwrap();
        }
        if mode == "later-same-origin" {
            popup_url
                .query_pairs_mut()
                .append_pair("child", &format!("{origin}/child"));
        }
        let mut targets = vec![format!(
            "/popup{}",
            popup_url
                .query()
                .map_or(String::new(), |query| format!("?{query}"))
        )];
        if mode == "later-same-origin" {
            targets.push("/child".into());
        }
        let loader = static_http_loader([]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            &format!("{origin}/root"),
            "<!doctype html><body>root",
            &loader,
        );
        vm.eval(&format!(
            r#"
            globalThis.relatedReady = false;
            onmessage = event => {{ if (event.data === 'ready') relatedReady = true; }};
            const popup = open({:?}, 'related-page');
            "#,
            popup_url.as_str()
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(relatedReady)",
            "true",
            mode,
        )
        .await;
        vm.take_pending_popup_activations();
        assert_eq!(
            vm.eval(&format!(
                r#"(() => {{
                    const selected = open('', 'related-target');
                    return JSON.stringify([selected !== popup, selected.opener === window,
                        {mode:?} === 'cross-origin' ? selected.top === selected :
                            selected === popup[{mode:?} === 'later-same-origin' ? 1 : 0],
                        {mode:?} !== 'later-same-origin' || selected.location.pathname === '/child'
                    ]);
                }})()"#
            ))
            .unwrap(),
            "[true,true,true,true]",
            "{mode}"
        );
        assert_eq!(
            vm.take_pending_popup_activations().len(),
            usize::from(mode == "cross-origin"),
            "{mode}"
        );
        assert_eq!(server.finish_targets().await, targets, "{mode}");
    }
}

#[tokio::test]
async fn window_open_named_targets_include_top_level_windows_and_native_openers() {
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://named-window.test/",
        "<!doctype html><body>root",
        &loader,
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
        .await
        .unwrap();
    vm.eval(&format!(
        "globalThis.namedTargetResults = {}",
        include_str!("../../../../tests/fixtures/window-open-root-names.js")
    ))
    .unwrap();
    assert_eq!(
        vm.eval(
            "JSON.stringify({...namedTargetResults, \
             rows: namedTargetResults.rows.filter(row => !row.selected || !row.opener), \
             count: namedTargetResults.rows.length})"
        )
        .unwrap(),
        r#"{"rows":[],"nameReads":0,"renamed":true,"oldName":true,"disowned":true,"selectedAfterShadowing":true,"shadowPreserved":true,"nativeOpenerUpdated":true,"count":36}"#,
    );
}

#[tokio::test]
async fn named_window_navigation_routes_roots_for_open_links_and_forms() {
    for root in ["main", "popup"] {
        for source in ["root", "child"] {
            for action in ["open", "anchor", "submit", "requestSubmit"] {
                let bodies = if root == "main" {
                    Vec::new()
                } else {
                    vec!["<!doctype html><body>selected".into()]
                };
                let server = StaticHttpServer::spawn_with_bodies(bodies).await;
                let origin = server.base_url().origin().ascii_serialization();
                let loader = static_http_loader([]);
                let mut vm = new_parsed_page_task_executor_test_vm(
                    &format!("{origin}/entry/index"),
                    "<!doctype html><body>entry",
                    &loader,
                );
                vm.eval(&format!(r#"
                    const root = {root:?} === 'main' ? window : open('', 'named-destination');
                    root.name = 'named-destination';
                    const duplicate = root.document.createElement('iframe');
                    duplicate.name = 'named-destination'; root.document.body.append(duplicate);
                    const frame = root.document.createElement('iframe'); root.document.body.append(frame);
                    const child = frame.contentWindow;
                    const base = child.document.createElement('base');
                    base.href = '{origin}/source/'; child.document.head.append(base);
                    const sourceWindow = {source:?} === 'root' ? root : child;
                    const before = root.document;
                    globalThis.nameReads = 0;
                    Object.defineProperty(root, 'name', {{ get() {{
                        ++nameReads; throw new Error('script-visible name read');
                    }} }});
                "#)).unwrap();
                vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
                    .await
                    .unwrap();
                vm.take_pending_popup_activations();
                vm.eval(&format!(r#"
                    if ({action:?} === 'open') {{
                        if (open.call(sourceWindow, 'selected?from=open', 'named-destination') !== root)
                            throw new Error('incorrect Window.open target');
                    }} else {{
                        const doc = sourceWindow.document;
                        const element = doc.createElement({action:?} === 'anchor' ? 'a' : 'form');
                        element.target = 'named-destination';
                        if ({action:?} === 'anchor') element.href = 'selected?from=anchor';
                        else {{
                            element.action = 'selected';
                            element.innerHTML = '<input name=from value="{action}">';
                        }}
                        doc.body.append(element);
                        if ({action:?} === 'anchor') element.click();
                        else element[{action:?}]();
                    }}
                "#)).unwrap();
                vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
                    .await
                    .unwrap();
                let directory = if action == "open" || source == "root" {
                    "entry"
                } else {
                    "source"
                };
                let path = format!("/{directory}/selected?from={action}");
                if root == "main" {
                    let navigation = vm.take_pending_location_navigation_with_seed().unwrap();
                    assert_eq!(
                        navigation.url.as_str(),
                        format!("{origin}{path}"),
                        "{source}, {action}"
                    );
                    assert!(vm.take_pending_popup_activations().is_empty());
                } else {
                    assert!(vm.take_pending_location_navigation_with_seed().is_none());
                    advance_page_task_executor_until_eval_equals(
                        &mut vm, &loader,
                        "String(root.document !== before && root.document.body?.textContent === 'selected')",
                        "true", &format!("{root}, {source}, {action}"),
                    ).await;
                }
                assert_eq!(vm.eval("String(nameReads)").unwrap(), "0");
                assert_eq!(
                    server.finish_targets().await,
                    if root == "main" {
                        Vec::new()
                    } else {
                        vec![path]
                    }
                );
            }
        }
    }
}

#[tokio::test]
async fn window_open_named_root_preserves_entry_sandbox_authority() {
    for allow_top in [false, true] {
        let loader = static_http_loader([]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://named-window.test/root",
            "<!doctype html><body>root",
            &loader,
        );
        vm.eval(&format!(
            r#"
            window.name = 'named-root'; globalThis.selectionDone = false;
            const frame = document.createElement('iframe');
            frame.sandbox = 'allow-scripts allow-same-origin {}';
            frame.srcdoc = `<script>
                try {{ parent.open('/selected', 'named-root'); }}
                catch (error) {{ if (error.name !== 'SecurityError') throw error; }}
                parent.selectionDone = true;
            <\/script>`;
            document.body.append(frame);
        "#,
            if allow_top {
                "allow-top-navigation"
            } else {
                ""
            }
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(selectionDone)",
            "true",
            "sandboxed named root navigation",
        )
        .await;
        let navigation = vm.take_pending_location_navigation_with_seed();
        assert_eq!(navigation.is_some(), allow_top);
        if let Some(navigation) = navigation {
            assert_eq!(
                navigation.url.as_str(),
                "https://named-window.test/selected"
            );
        }
        assert!(vm.take_pending_popup_activations().is_empty());
    }
}

#[tokio::test]
async fn named_root_opener_updates_cached_cross_origin_accessors() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        r#"<!doctype html><body><script>
        const getOpener = Object.getOwnPropertyDescriptor(parent, 'opener').get;
        onmessage = event => {
            const expected = event.data === 'set' ? parent[1] : null;
            parent.postMessage([event.data, getOpener.call(parent) === expected,
                parent.opener === expected], '*');
        };
        parent.postMessage(['ready', getOpener.call(parent) === null], '*');
        </script>"#
            .into(),
    ])
    .await;
    let origin = server.base_url().origin().ascii_serialization();
    let observer_origin = origin.replace("127.0.0.1", "localhost");
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        &format!("{origin}/root"),
        "<!doctype html><body>root",
        &loader,
    );
    vm.eval(&format!(r#"
        window.name = 'opener-target'; globalThis.openerMessages = [];
        onmessage = event => openerMessages.push(event.data);
        const observer = document.createElement('iframe'); observer.src = '{observer_origin}/observer';
        document.body.append(observer);
        const source = document.createElement('iframe'); document.body.append(source);
    "#)).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(openerMessages)",
        r#"[["ready",true]]"#,
        "cross-origin opener accessor captured before named lookup",
    )
    .await;
    assert_eq!(
        vm.eval("String(open.call(source.contentWindow, '', 'opener-target') === window)")
            .unwrap(),
        "true"
    );
    vm.eval("observer.contentWindow.postMessage('set', '*')")
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(openerMessages)",
        r#"[["ready",true],["set",true,true]]"#,
        "cached cross-origin getter sees updated opener",
    )
    .await;
    vm.eval("window.opener = null; observer.contentWindow.postMessage('clear', '*')")
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(openerMessages)",
        r#"[["ready",true],["set",true,true],["clear",true,true]]"#,
        "cached cross-origin getter sees cleared opener",
    )
    .await;
    assert_eq!(server.finish_targets().await, vec!["/observer"]);
    assert!(vm.take_pending_popup_activations().is_empty());
}

#[tokio::test]
async fn window_open_named_iframes_use_receiver_lookup_and_entry_url() {
    for selection in ["self", "descendant", "sibling"] {
        for call in [
            "child-eval",
            "child-method",
            "borrowed-to-child",
            "borrowed-to-top",
        ] {
            for navigate in [false, true] {
                let bodies = if navigate {
                    vec!["<!doctype html><body>selected".into()]
                } else {
                    Vec::new()
                };
                let server = StaticHttpServer::spawn_with_bodies(bodies).await;
                let base = server.base_url().origin().ascii_serialization();
                let loader = static_http_loader([]);
                let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                    &format!("{base}/parent/index"),
                    &loader,
                );
                vm.eval(&format!(
                    r#"
                    const earlier = document.createElement('iframe'); earlier.name = 'same';
                    const source = document.createElement('iframe');
                    source.name = {selection:?} === 'self' ? 'same' : 'source';
                    document.body.append(earlier, source);
                    const child = source.contentWindow;
                    const nested = child.document.createElement('iframe');
                    nested.name = {selection:?} === 'sibling' ? 'other' : 'same';
                    child.document.body.append(nested);
                    const base = child.document.createElement('base');
                    base.href = '{base}/child/'; child.document.head.append(base);
                    const expectedFrame = {call:?} === 'borrowed-to-top' ? earlier :
                        ({selection:?} === 'self' ? source :
                         {selection:?} === 'descendant' ? nested : earlier);
                    const expectedOpener = {call:?} === 'borrowed-to-top' ? window : child;
                    globalThis.nameReads = 0;
                    Object.defineProperty(child, 'name', {{get() {{
                        ++nameReads; throw new Error('script-visible name read');
                    }}}});
                "#
                ))
                .unwrap();
                vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
                    .await
                    .unwrap();
                let url = if navigate { "selected" } else { "" };
                let expression = match call {
                    "child-eval" => format!("child.eval(\"open('{url}', 'same')\")"),
                    "child-method" => format!("child.open({url:?}, 'same')"),
                    "borrowed-to-child" => format!("open.call(child, {url:?}, 'same')"),
                    "borrowed-to-top" => format!("child.open.call(window, {url:?}, 'same')"),
                    _ => unreachable!(),
                };
                assert_eq!(
                    vm.eval(&format!(
                        "const opened = {expression}; \
                         JSON.stringify([opened === expectedFrame.contentWindow, \
                         opened.opener === expectedOpener, nameReads])"
                    ))
                    .unwrap(),
                    "[true,true,0]",
                    "{selection}, {call}, navigate={navigate}",
                );
                if navigate {
                    advance_page_task_executor_until_eval_equals(
                        &mut vm,
                        &loader,
                        "String(expectedFrame.contentDocument.body?.textContent === 'selected')",
                        "true",
                        &format!("{selection}, {call}"),
                    )
                    .await;
                }
                let expected = if navigate {
                    vec!["/parent/selected".to_owned()]
                } else {
                    Vec::new()
                };
                assert_eq!(
                    server.finish_targets().await,
                    expected,
                    "{selection}, {call}"
                );
                assert!(vm.take_pending_popup_activations().is_empty());
            }
        }
    }
}

#[test]
fn window_open_validates_its_receiver_before_converting_arguments() {
    let mut vm = new_storage_test_vm("https://open-receiver.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
                const frame = document.createElement('iframe');
                (document.body || document.documentElement || document).appendChild(frame);
                const child = frame.contentWindow;
                const revoked = Proxy.revocable(child, {}); revoked.revoke();
                let conversions = 0;
                const arg = {toString() { ++conversions; return ''; }};
                const errors = [];
                for (const method of [open, child.open]) {
                    const TypeError = method === open ? window.TypeError : child.TypeError;
                    for (const receiver of [{}, Object.create(child), new Proxy(child, {}), revoked.proxy]) {
                        try { method.call(receiver, arg, arg, arg); errors.push(false); }
                        catch (error) { errors.push(error instanceof TypeError); }
                    }
                }
                return JSON.stringify([errors.every(Boolean), errors.length, conversions]);
            })()"#,
        )
        .unwrap(),
        "[true,8,0]",
    );
}
