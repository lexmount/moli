use super::*;

#[tokio::test]
async fn window_open_special_targets_use_native_receiver_ancestry() {
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://special-open.test/root/index",
        "<!doctype html><body>root",
        &loader,
    );
    vm.eval(include_str!(
        "../../../../tests/fixtures/window-open-special-targets.js"
    ))
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
        .await
        .unwrap();
    assert_eq!(vm.take_pending_popup_activations().len(), 1);
    assert_eq!(
        vm.eval("JSON.stringify(specialTargetCase.run())").unwrap(),
        r#"{"count":432,"parentReads":0,"failures":[]}"#,
    );
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    assert!(vm.take_pending_popup_activations().is_empty());
    vm.eval("specialTargetCase.close()").unwrap();
}

#[tokio::test]
async fn window_open_special_targets_navigate_receiver_with_entry_url() {
    for target in ["_self", "_parent", "_top"] {
        for root in ["main", "popup"] {
            for callee in ["window", "deep"] {
                let is_top_navigation = root == "main" && target == "_top";
                let bodies = if is_top_navigation {
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
                vm.eval(&format!(
                    r#"
                    const root = {root:?} === 'main' ? window : open('', 'navigation-root');
                    const frame = root.document.createElement('iframe'); root.document.body.append(frame);
                    const middle = frame.contentWindow;
                    const nested = middle.document.createElement('iframe'); middle.document.body.append(nested);
                    const deep = nested.contentWindow;
                    const base = deep.document.createElement('base');
                    base.href = '{origin}/receiver/'; deep.document.head.append(base);
                    const expected = {target:?} === '_self' ? deep : {target:?} === '_parent' ? middle : root;
                    const before = expected.document;
                    const methods = {{window: window.open, deep: deep.open}};
                "#
                ))
                .unwrap();
                vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
                    .await
                    .unwrap();
                vm.take_pending_popup_activations();
                assert_eq!(
                    vm.eval(&format!(
                        "String(methods[{callee:?}].call(deep, 'selected', {target:?}) === expected)"
                    ))
                    .unwrap(),
                    "true",
                    "{root}, {callee}, {target}",
                );
                assert!(vm.take_pending_popup_activations().is_empty());
                if is_top_navigation {
                    let navigation = vm.take_pending_location_navigation_with_seed().unwrap();
                    assert_eq!(navigation.url.as_str(), format!("{origin}/entry/selected"));
                } else {
                    assert!(vm.take_pending_location_navigation_with_seed().is_none());
                    advance_page_task_executor_until_eval_equals(
                        &mut vm,
                        &loader,
                        "String(expected.document !== before && expected.document.body?.textContent === 'selected')",
                        "true",
                        &format!("{root}, {callee}, {target}"),
                    )
                    .await;
                }
                assert_eq!(
                    server.finish_targets().await,
                    if is_top_navigation {
                        Vec::new()
                    } else {
                        vec!["/entry/selected".to_owned()]
                    },
                    "{root}, {callee}, {target}",
                );
            }
        }
    }
}

#[test]
fn window_open_special_targets_do_not_revive_receiver_removed_during_conversion() {
    let mut vm = new_storage_test_vm("https://special-open.test/root");
    for target in ["_self", "_parent", "_top"] {
        assert_eq!(
            vm.eval(&format!(
                r#"(() => {{
                    const frame = document.createElement('iframe');
                    (document.body || document.documentElement || document).append(frame);
                    const child = frame.contentWindow;
                    let conversions = 0;
                    const url = {{toString() {{ ++conversions; frame.remove(); return '/removed'; }}}};
                    const result = open.call(child, url, {target:?});
                    return JSON.stringify([result === null, conversions]);
                }})()"#
            ))
            .unwrap(),
            "[true,1]",
            "{target}",
        );
        assert!(vm.take_pending_location_navigation_with_seed().is_none());
        assert!(vm.take_pending_popup_activations().is_empty());
    }
}

#[tokio::test]
async fn window_open_special_targets_preserve_cross_origin_window_projections() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        r#"<!doctype html><body><script>
            const failures = [];
            for (const target of ['_self', '_parent', '_top']) {
                const expected = target === '_self' ? window : parent;
                const actual = open('', target);
                if (actual !== expected) failures.push('wrong identity: ' + target);
                if (expected === parent) {
                    try { actual.document; failures.push('exposed parent document'); }
                    catch (error) { if (error.name !== 'SecurityError') failures.push(error.name); }
                }
            }
            parent.postMessage(failures, '*');
        </script>"#
            .to_owned(),
    ])
    .await;
    let origin = server.base_url().origin().ascii_serialization();
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        &format!("{}/root", origin.replace("127.0.0.1", "localhost")),
        "<!doctype html><body>root",
        &loader,
    );
    vm.eval(&format!(
        r#"
        globalThis.selectionFailures = null;
        onmessage = event => {{selectionFailures = event.data}};
        const frame = document.createElement('iframe');
        frame.sandbox = 'allow-scripts allow-same-origin allow-top-navigation';
        frame.src = '{origin}/child';
        document.body.append(frame);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(selectionFailures !== null)",
        "true",
        "cross-origin target selection",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(selectionFailures)").unwrap(), "[]");
    assert_eq!(server.finish_targets().await, vec!["/child"]);
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    assert!(vm.take_pending_popup_activations().is_empty());
}

#[tokio::test]
async fn window_open_special_targets_keep_entry_sandbox_authority() {
    for allow_top in [false, true] {
        for target in ["_parent", "_top"] {
            let loader = static_http_loader([]);
            let mut vm = new_parsed_page_task_executor_test_vm(
                "https://special-open.test/root",
                "<!doctype html><body>root",
                &loader,
            );
            let sandbox = if allow_top {
                "allow-scripts allow-same-origin allow-top-navigation"
            } else {
                "allow-scripts allow-same-origin"
            };
            vm.eval(&format!(
                r#"
                globalThis.selectionDone = false;
                const frame = document.createElement('iframe'); frame.sandbox = {sandbox:?};
                frame.srcdoc = `<script>
                    parent.emptyTargetSelected = parent.open.call(parent, '', '{target}') === parent;
                    try {{ parent.open.call(parent, 'https://special-open.test/selected', '{target}'); }}
                    catch (error) {{ if (error.name !== 'SecurityError') throw error; }}
                    parent.selectionDone = true;
                <\/script>`;
                document.body.append(frame);
            "#
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(selectionDone)",
                "true",
                "sandboxed entry borrowing top open",
            )
            .await;
            assert_eq!(vm.eval("String(emptyTargetSelected)").unwrap(), "true");
            let navigation = vm.take_pending_location_navigation_with_seed();
            assert_eq!(navigation.is_some(), allow_top, "{target}");
            if let Some(navigation) = navigation {
                assert_eq!(
                    navigation.url.as_str(),
                    "https://special-open.test/selected"
                );
            }
            assert!(vm.take_pending_popup_activations().is_empty());
        }
    }
}
