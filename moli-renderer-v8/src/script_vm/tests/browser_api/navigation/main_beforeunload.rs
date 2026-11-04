use super::*;

#[tokio::test]
async fn history_fallback_defers_beforeunload_until_a_browser_destination_exists() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (action, delta) in [
        ("history.back()", -1),
        ("history.forward()", 1),
        ("history.go(-100)", -100),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://history-beforeunload.test/source",
            &loader,
        );
        vm.eval(&format!(
            "globalThis.beforeUnloadCount=0; addEventListener('beforeunload',()=>{{beforeUnloadCount++; history.back();}}); {action}; true"
        ))
        .unwrap();
        assert!(
            vm.run_one_history_traversal_executor_turn(&loader)
                .await
                .unwrap()
        );
        assert_eq!(
            vm.eval("String(beforeUnloadCount)").unwrap(),
            "0",
            "{action}: an unresolved browser history target must not fire beforeunload"
        );
        let traversal = vm
            .take_pending_top_level_history_traversal()
            .expect("the browser must resolve the fallback destination");
        assert_eq!(traversal.delta, delta);
        assert!(
            !traversal.beforeunload_checked,
            "the browser must check beforeunload after resolving the target"
        );
        assert!(
            !vm.run_one_history_traversal_executor_turn(&loader)
                .await
                .unwrap(),
            "{action}: an out-of-range request must not create a beforeunload traversal loop"
        );
    }
}

#[tokio::test]
async fn history_fallback_from_beforeunload_preserves_the_pending_location() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (action, delta) in [
        ("history.back()", -1),
        ("history.forward()", 1),
        ("history.go(-100)", -100),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://history-beforeunload.test/source",
            &loader,
        );
        vm.eval(&format!(
            "globalThis.beforeUnloadCount=0; addEventListener('beforeunload',()=>{{beforeUnloadCount++; {action};}}); location.href='/destination'; true"
        ))
        .unwrap();
        assert_eq!(vm.eval("String(beforeUnloadCount)").unwrap(), "1");
        assert!(vm.has_pending_location_navigation());
        assert!(
            vm.run_one_history_traversal_executor_turn(&loader)
                .await
                .unwrap()
        );
        assert_eq!(vm.eval("String(beforeUnloadCount)").unwrap(), "1");
        assert!(
            vm.has_pending_location_navigation(),
            "{action}: an unresolved traversal must keep the pending destination"
        );
        let traversal = vm.take_pending_top_level_history_traversal().unwrap();
        assert_eq!(traversal.delta, delta);
        assert!(!traversal.beforeunload_checked);
        assert_eq!(
            vm.take_pending_location_navigation_with_seed()
                .expect("the Location request must survive the traversal fallback")
                .url
                .path(),
            "/destination"
        );
        assert!(
            !vm.run_one_history_traversal_executor_turn(&loader)
                .await
                .unwrap()
        );
    }
}

const SETUP: &str = r#"((child, grandchild) => {
    const makeFrame = owner => {
        const doc = owner.document;
        if (!doc.documentElement) doc.appendChild(doc.createElement('html'));
        const frame = doc.createElement('iframe');
        doc.documentElement.append(frame);
        return frame;
    };
    child ??= makeFrame(window);
    grandchild ??= makeFrame(child.contentWindow);
    const events = [];
    const beforeUrls = [];
    const windows = [[window, 'root'], [child.contentWindow, 'child'],
        [grandchild.contentWindow, 'grandchild']];
    for (const [win, name] of windows) {
        for (const type of ['beforeunload', 'pagehide', 'unload']) {
            win.addEventListener(type, event => {
                events.push(name + ':' + type);
                if (type === 'beforeunload')
                    beforeUrls.push([name, win.location.href, win.document.URL,
                        win.document.hidden, event.isTrusted, event.cancelable]);
            });
        }
    }
    globalThis.beforeUnloadProbe = {events, beforeUrls, child, grandchild, windows};
})"#;

#[tokio::test]
async fn main_beforeunload_checks_the_document_tree_before_scripted_navigation() {
    for action in [
        "location.href = '/destination'",
        "location.assign('/destination')",
        "location.replace('/destination')",
        "location.reload()",
        "navigation.navigate('/destination')",
        "navigation.reload()",
        "const a = document.createElement('a'); a.href='/destination'; document.documentElement.append(a); a.click()",
        "const f = document.createElement('form'); f.action='/destination'; document.documentElement.append(f); f.submit()",
        "const f = document.createElement('form'); f.action='/destination'; f.method='post'; document.documentElement.append(f); f.submit()",
    ] {
        let server = StaticHttpServer::spawn_with_bodies(vec![
            "<!doctype html><iframe id=grandchild src=/grandchild></iframe>".into(),
            "<!doctype html><body>grandchild".into(),
        ])
        .await;
        let source_url = server.url_for_host("beforeunload.test", "/source");
        let child_url = source_url.join("/child").unwrap();
        let grandchild_url = source_url.join("/grandchild").unwrap();
        let loader = static_http_loader([server.resolve_entry("beforeunload.test")]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(source_url.as_str(), &loader);
        let owner = vm.current_main_document_task_owner().unwrap();
        let interactive = vm.finish_current_main_document_parsing(owner).unwrap();
        vm.apply_main_document_interactive_lifecycle_action(interactive)
            .unwrap();
        vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
        assert!(
            vm.dispatch_main_document_window_load_lifecycle(owner)
                .unwrap()
                .is_none()
        );
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
            .await
            .unwrap();
        vm.eval(
            r#"
            globalThis.beforeUnloadChild = document.createElement('iframe');
            globalThis.beforeUnloadChildLoaded = false;
            beforeUnloadChild.onload = () => { beforeUnloadChildLoaded = true; };
            beforeUnloadChild.src = '/child';
            (document.documentElement || document).appendChild(beforeUnloadChild);
            "#,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(beforeUnloadChildLoaded)",
            "true",
            "child and grandchild must finish loading before navigation",
        )
        .await;
        vm.eval(&format!(
            "{SETUP}(beforeUnloadChild, beforeUnloadChild.contentDocument.getElementById('grandchild'))"
        ))
        .unwrap();
        assert_eq!(
            vm.eval("beforeUnloadProbe.windows.map(([win])=>win.document.readyState).join('|')")
                .unwrap(),
            "complete|complete|complete"
        );
        vm.eval(&format!("{{ {action}; }}; true")).unwrap();
        if action.contains("f.submit()") {
            assert!(
                vm.run_one_dom_manipulation_task_executor_turn(
                    crate::runtime::PageDomManipulationTestFamily::FormNavigation,
                    &loader,
                )
                .await
                .unwrap()
            );
        }
        assert_eq!(
            vm.eval("beforeUnloadProbe.events.join('|')").unwrap(),
            "root:beforeunload|child:beforeunload|grandchild:beforeunload",
            "{action}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(beforeUnloadProbe.beforeUrls)")
                .unwrap(),
            serde_json::json!([
                ["root", source_url, source_url, false, true, true],
                ["child", child_url, child_url, false, true, true],
                [
                    "grandchild",
                    grandchild_url,
                    grandchild_url,
                    false,
                    true,
                    true
                ],
            ])
            .to_string(),
            "{action}"
        );
        assert_eq!(
            vm.eval("[location.href,document.URL,document.hidden].join('|')")
                .unwrap(),
            format!("{source_url}|{source_url}|false"),
            "{action} must not commit the requested URL or hide the source Document"
        );
        let pending = vm.take_pending_location_navigation_with_seed().unwrap();
        assert_eq!(
            pending.url.path(),
            if action.contains("reload()") {
                "/source"
            } else {
                "/destination"
            }
        );
        assert_eq!(
            pending.request_method,
            if action.contains("'post'") {
                "POST"
            } else {
                "GET"
            }
        );
        vm.unload_main_document_for_navigation_commit().unwrap();
        vm.unload_main_document_for_navigation_commit().unwrap();
        assert_eq!(
            vm.eval("beforeUnloadProbe.events.join('|')").unwrap(),
            "root:beforeunload|child:beforeunload|grandchild:beforeunload|root:pagehide|root:unload|child:pagehide|child:unload|grandchild:pagehide|grandchild:unload",
            "{action} must not repeat beforeunload at commit"
        );
        assert_eq!(server.finish_targets().await, ["/child", "/grandchild"]);
    }
}

#[test]
fn main_beforeunload_blocks_navigation_reentry_through_descendants() {
    for action in [
        "addEventListener('beforeunload', () => location.href='/forbidden')",
        "addEventListener('beforeunload', () => stop())",
        "beforeUnloadProbe.grandchild.contentWindow.addEventListener('beforeunload', () => { window.__lmWindowUnloadEventActive=false; location.href='/forbidden'; stop(); })",
    ] {
        let mut vm = new_unload_lifecycle_test_vm("https://beforeunload.test/source");
        vm.eval(&format!("{SETUP}()")).unwrap();
        vm.eval(&format!("{action}; location.href='/destination'; true"))
            .unwrap();
        assert_eq!(
            vm.eval("beforeUnloadProbe.events.join('|')").unwrap(),
            "root:beforeunload|child:beforeunload|grandchild:beforeunload",
            "{action}"
        );
        assert_eq!(
            vm.take_pending_location_navigation_with_seed()
                .unwrap()
                .url
                .path(),
            "/destination",
            "{action}"
        );
    }
}

#[test]
fn main_beforeunload_protects_the_navigation_target_during_descendant_callbacks() {
    for descendant in ["child", "grandchild"] {
        for operation in ["open", "write", "writeln"] {
            let mut vm = new_unload_lifecycle_test_vm("https://beforeunload.test/source");
            vm.eval(&format!("{SETUP}()")).unwrap();
            vm.eval(&format!(
                r#"
                globalThis.beforeUnloadRoot = document.documentElement;
                beforeUnloadProbe.{descendant}.contentWindow.addEventListener('beforeunload', () => {{
                    document.{operation}('<p>forbidden');
                }});
                location.href='/destination'; true
                "#
            ))
            .unwrap();
            assert_eq!(
                vm.eval("[document.documentElement === beforeUnloadRoot, beforeUnloadProbe.child.isConnected, beforeUnloadProbe.grandchild.isConnected].join('|')").unwrap(),
                "true|true|true",
                "{descendant}/{operation} must preserve the source Document"
            );
            assert_eq!(
                vm.eval("beforeUnloadProbe.events.join('|')").unwrap(),
                "root:beforeunload|child:beforeunload|grandchild:beforeunload",
                "{descendant}/{operation}"
            );
            assert_eq!(
                vm.take_pending_location_navigation_with_seed()
                    .unwrap()
                    .url
                    .path(),
                "/destination"
            );
            assert_eq!(
                vm.eval("document.open(); document.childNodes.length")
                    .unwrap(),
                "0",
                "{descendant}/{operation} must release the write guard after the check"
            );
        }
    }
}

#[test]
fn main_beforeunload_skips_descendants_removed_by_an_ancestor_handler() {
    let mut vm = new_unload_lifecycle_test_vm("https://beforeunload.test/source");
    vm.eval(&format!("{SETUP}()")).unwrap();
    vm.eval("addEventListener('beforeunload', () => beforeUnloadProbe.child.remove()); location.href='/destination'; true").unwrap();
    assert_eq!(
        vm.eval("beforeUnloadProbe.events.filter(e=>e.endsWith(':beforeunload')).join('|')")
            .unwrap(),
        "root:beforeunload"
    );
    assert_eq!(
        vm.eval("String(beforeUnloadProbe.child.isConnected)")
            .unwrap(),
        "false"
    );
    assert_eq!(
        vm.take_pending_location_navigation_with_seed()
            .unwrap()
            .url
            .path(),
        "/destination"
    );
}

#[test]
fn main_beforeunload_does_not_run_for_fragments_javascript_or_canceled_navigate_events() {
    for action in [
        "location.hash = '#fragment'",
        "location.href = 'javascript:void 0'",
        "navigation.addEventListener('navigate', e=>e.preventDefault()); location.href='/destination'",
        "navigation.addEventListener('navigate', e=>e.preventDefault()); navigation.navigate('/destination')",
    ] {
        let mut vm = new_unload_lifecycle_test_vm("https://beforeunload.test/source");
        vm.eval(&format!("{SETUP}()")).unwrap();
        vm.eval(&format!("{action}; true")).unwrap();
        assert_eq!(
            vm.eval("beforeUnloadProbe.events.join('|')").unwrap(),
            "",
            "{action}"
        );
    }
}
