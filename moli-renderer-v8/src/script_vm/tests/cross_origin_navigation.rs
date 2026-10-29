use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_child_navigation_does_not_dispatch_a_destination_navigate_event() {
    let server = StaticHttpServer::spawn_with_responder(
        12,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            r#"<!doctype html><script>
navigation.onnavigate = event => {
  event.preventDefault();
  top.postMessage('UNEXPECTED NAVIGATE', '*');
};
onload = () => {
  if (location.search === '?next') top.postMessage('DONE', '*');
};
</script>"#
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry("parent.cross-navigation.test"),
        server.resolve_entry("child.cross-navigation.test"),
    ]);
    let parent = server.url_for_host("parent.cross-navigation.test", "/parent");
    let child = server.url_for_host("child.cross-navigation.test", "/child");
    let destination = server.url_for_host("child.cross-navigation.test", "/child?next");
    for action in ["href", "window", "click", "open"] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
        vm.eval(&format!(
            r#"
globalThis.loads = 0;
globalThis.messages = [];
globalThis.sourceNavigations = 0;
navigation.onnavigate = () => ++sourceNavigations;
addEventListener('message', event => messages.push(event.data));
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.name = 'target';
frame.onload = () => ++loads;
frame.src = {child:?};
root.appendChild(frame);
"#,
            child = child.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "1",
            "cross-origin child should finish its initial load",
        )
        .await;
        let url = format!("{:?}", destination.as_str());
        let script = match action {
            "href" => format!("frame.contentWindow.location.href = {url}; 'queued'"),
            "window" => format!("frame.contentWindow.location = {url}; 'queued'"),
            "open" => format!("window.open({url}, 'target'); 'queued'"),
            "click" => format!(
                "const link = document.createElement('a'); link.href = {url}; \
                 link.target = 'target'; root.appendChild(link); link.click(); 'queued'"
            ),
            _ => unreachable!(),
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(messages.includes('DONE'))",
            "true",
            "foreign navigation must commit without firing its canceling navigate handler",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify([messages, sourceNavigations, loads])")
                .unwrap(),
            r#"[["DONE"],0,2]"#,
            "{action}",
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_location_navigation_preserves_push_and_filters_previous_origin_entries() {
    let server = StaticHttpServer::spawn_with_responder(
        4,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| "<!doctype html><p>ready</p>".to_owned(),
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry("parent.cross-history.test"),
        server.resolve_entry("child.cross-history.test"),
    ]);
    let parent = server.url_for_host("parent.cross-history.test", "/parent");
    let child = server.url_for_host("child.cross-history.test", "/child");
    let destination = server.url_for_host("parent.cross-history.test", "/returned");
    for target in [destination.as_str(), "about:blank"] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
        vm.eval(&format!(
            r#"
globalThis.loads = 0;
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.onload = () => ++loads;
frame.src = {child:?};
root.appendChild(frame);
"#,
            child = child.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "1",
            "foreign child should commit before navigation back to its parent's origin",
        )
        .await;
        vm.eval(&format!(
            "frame.contentWindow.location = {target:?}; 'queued'"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "2",
            "new child document should commit",
        )
        .await;
        assert_eq!(
            vm.eval("String(frame.contentWindow.navigation.entries().length)")
                .unwrap(),
            "1",
            "entries from the previous origin must not leak into {target}",
        );
        if target != "about:blank" {
            assert_eq!(
                vm.eval("frame.contentWindow.navigation.activation.navigationType")
                    .unwrap(),
                "push",
                "a loaded foreign document is not an initial empty document",
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_location_same_url_without_fragment_is_cross_document() {
    let server = StaticHttpServer::spawn_with_responder(
        24,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            r#"<!doctype html><script>
navigation.onnavigate = event => {
  top.postMessage('NAVIGATE:' + event.destination.sameDocument + ':' + event.hashChange, '*');
  if (location.search === '?cancel') event.preventDefault();
};
</script>"#
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry("parent.same-url.test"),
        server.resolve_entry("child.same-url.test"),
    ]);
    let parent = server.url_for_host("parent.same-url.test", "/parent");
    for foreign in [true, false] {
        let host = if foreign {
            "child.same-url.test"
        } else {
            "parent.same-url.test"
        };
        let path = if foreign {
            "/child?cancel"
        } else {
            "/child?allow"
        };
        let target = server.url_for_host(host, path);
        for fragment in ["", "#old"] {
            for replace in [false, true] {
                let mut vm =
                    new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
                let initial = format!("{target}{fragment}");
                vm.eval(&format!(
                    r#"
globalThis.loads = 0;
globalThis.messages = [];
addEventListener('message', event => messages.push(event.data));
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.onload = () => ++loads;
frame.src = {initial:?};
root.appendChild(frame);
"#
                ))
                .unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(loads)",
                    "1",
                    "initial child load",
                )
                .await;
                let action = if replace {
                    format!(
                        "frame.contentWindow.location.replace({:?}); 'queued'",
                        target.as_str()
                    )
                } else {
                    format!(
                        "frame.contentWindow.location.href = {:?}; 'queued'",
                        target.as_str()
                    )
                };
                vm.eval(&action).unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(loads)",
                    "2",
                    "a target without a fragment must load a new Document, including an unchanged URL",
                )
                .await;
                let expected = if foreign {
                    "[]"
                } else {
                    r#"["NAVIGATE:false:false"]"#
                };
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "JSON.stringify(messages)",
                    expected,
                    "navigate event must reflect initiator origin and cross-document routing",
                )
                .await;
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_location_fragments_preserve_the_document() {
    let server = StaticHttpServer::spawn_with_responder(
        1,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            r#"<!doctype html><script>
navigation.onnavigate = event => top.postMessage('NAVIGATE:' + event.destination.sameDocument + ':' + event.hashChange, '*');
onhashchange = () => top.postMessage('HASH', '*');
</script>"#
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry("parent.fragment-route.test"),
        server.resolve_entry("child.fragment-route.test"),
    ]);
    let parent = server.url_for_host("parent.fragment-route.test", "/parent");
    let child = server.url_for_host("child.fragment-route.test", "/child");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
    vm.eval(&format!(
        r#"
globalThis.loads = 0;
globalThis.messages = [];
addEventListener('message', event => messages.push(event.data));
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.onload = () => ++loads;
frame.src = {child:?};
root.appendChild(frame);
"#,
        child = child.as_str()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(loads)",
        "1",
        "initial foreign child load",
    )
    .await;
    for (fragment, expected) in [
        ("#", r#"["NAVIGATE:true:true","HASH"]"#),
        (
            "#section",
            r#"["NAVIGATE:true:true","HASH","NAVIGATE:true:true","HASH"]"#,
        ),
    ] {
        let target = format!("{child}{fragment}");
        vm.eval(&format!(
            "frame.contentWindow.location.href = {target:?}; 'queued'"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "JSON.stringify(messages)",
            expected,
            "empty and nonempty fragments must retain the fragment navigation path",
        )
        .await;
        assert_eq!(vm.eval("String(loads)").unwrap(), "1");
    }
    // Returning to an entry without a fragment is still a same-document
    // history traversal, even though assigning that URL would navigate.
    vm.eval(
        "globalThis.retainedTopDocument = document; globalThis.topHashes = 0; \
         addEventListener('hashchange', () => ++topHashes); \
         history.pushState(null, '', '#parent');",
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([location.hash, topHashes])")
            .unwrap(),
        r##"["#parent",0]"##
    );
    vm.eval("history.back(); 'queued'").unwrap();
    let expected = format!("[{:?},1]", parent.as_str());
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify([location.href, topHashes])",
        &expected,
        "history may remove a fragment without replacing the Document",
    )
    .await;
    assert_eq!(
        vm.eval("String(document === retainedTopDocument)").unwrap(),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_facade_location_is_scoped_to_its_browsing_context() {
    let server = StaticHttpServer::spawn_with_responder(
        6,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| r#"<!doctype html><script>onload = () => top.postMessage(location.pathname, '*');</script>"#.to_owned(),
    ).await;
    let loader = static_http_loader([
        server.resolve_entry("parent.facade-location.test"),
        server.resolve_entry("child.facade-location.test"),
    ]);
    let parent = server.url_for_host("parent.facade-location.test", "/parent");
    let child = server.url_for_host("child.facade-location.test", "/child");
    let target = server.url_for_host("child.facade-location.test", "/target");
    for reinsert in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
        vm.eval(&format!(
            r#"
globalThis.loads = 0;
globalThis.messages = [];
addEventListener('message', event => messages.push(event.data));
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.id = 'child';
frame.onload = () => ++loads;
frame.src = {child:?};
root.appendChild(frame);
"#,
            child = child.as_str()
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "JSON.stringify([loads, messages])",
            r#"[1,["/child"]]"#,
            "initial child and message delivery",
        )
        .await;
        vm.eval("messages.length = 0").unwrap();
        let handle = vm.document_runtime.get_element_by_id("child").unwrap();
        let context_id = vm
            .child_frame_realm_store
            .iter_by_execution_context_id()
            .find_map(|(id, context)| (context.child_handle == handle).then_some(id))
            .unwrap();
        // Use the production retirement path, pausing before a successor Realm
        // materializes, so the saved Location really belongs to a facade.
        vm.destroy_child_default_context(context_id);
        let context = &vm.page_default_runtime.context as *const v8::Global<v8::Context>;
        vm.with_context_scope_by_ptr(context, |scope, _| {
            let host_ptr = crate::util::context_host_ptr_from_global_bridge(scope).unwrap();
            let host = unsafe { &mut *host_ptr };
            let window = host
                .child_browsing_context_window_proxy_for_accessing_context(scope, handle)
                .unwrap();
            let facade = window.get_creation_context(scope).unwrap();
            assert!(
                host.window_execution_context_identity_for_access_check(facade)
                    .is_none()
            );
            let location_value = window
                .get(scope, crate::util::v8str(scope, "location").into())
                .unwrap();
            let location = v8::Local::<v8::Object>::try_from(location_value).unwrap();
            assert_eq!(location.get_creation_context(scope), Some(facade));
            let global = scope.get_current_context().global(scope);
            global
                .set(
                    scope,
                    crate::util::v8str(scope, "oldFacadeWindow").into(),
                    window.into(),
                )
                .unwrap();
            global
                .set(
                    scope,
                    crate::util::v8str(scope, "oldFacadeLocation").into(),
                    location.into(),
                )
                .unwrap();
            let code = format!(
                r#"
if ({reinsert}) {{ frame.remove(); root.appendChild(frame); }}
try {{ oldFacadeLocation.href = {target:?}; }} catch (error) {{
  if (!{reinsert} || error.name !== 'SecurityError') throw error;
}}
!{reinsert} || oldFacadeWindow !== frame.contentWindow
"#,
                target = target.as_str()
            );
            let code = crate::util::v8_string(scope, &code).unwrap();
            let script = v8::Script::compile(scope, code, None).unwrap();
            assert!(
                crate::script_execution::execute_compiled_script(scope, script)
                    .unwrap()
                    .boolean_value(scope)
            );
            // A stale facade keeps its own WindowProxy, so the creating-Realm
            // equality check alone cannot protect the reinserted frame.
            assert_eq!(window.get_creation_context(scope), Some(facade));
            assert_eq!(location.get_creation_context(scope), Some(facade));
            Ok(())
        })
        .unwrap();
        let expected = if reinsert {
            r#"[2,["/child"]]"#
        } else {
            r#"[2,["/target"]]"#
        };
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "JSON.stringify([loads, messages])",
            expected,
            "a current facade may navigate, but a retired facade cannot target a reinserted iframe",
        )
        .await;
    }
}
