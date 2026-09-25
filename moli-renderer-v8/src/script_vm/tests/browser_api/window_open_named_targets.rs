use super::*;

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
