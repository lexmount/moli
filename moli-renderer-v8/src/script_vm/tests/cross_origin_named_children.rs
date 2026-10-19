use super::*;

fn child_context_for_element(vm: &mut ScriptVm, element_id: &str) -> i64 {
    vm.live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| {
            vm.child_frame_realm_store
                .get(&realm.context_id)
                .is_some_and(|record| {
                    vm._context_host
                        .borrow()
                        .dom_host()
                        .get_attribute(record.child_handle, "id")
                        .as_deref()
                        == Some(element_id)
                })
        })
        .unwrap()
        .context_id
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_named_children_use_the_parent_origin_and_first_document_tree_match() {
    const PARENT: &str = "parent.named-children.test";
    const OBSERVER: &str = "observer.named-children.test";
    const FOREIGN: &str = "foreign.named-children.test";
    let server = StaticHttpServer::spawn_with_responder(
        6,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            "<!doctype html><body><script>\
             const url = new URL(location.href);\
             window.name = url.searchParams.get('name');\
             top.postMessage(url.pathname, '*');\
             </script>"
                .to_owned()
        },
    )
    .await;
    let parent_url = server.url_for_host(PARENT, "/parent");
    let observer_url = server.url_for_host(OBSERVER, "/observer?name=observer");
    let foreign_url = server.url_for_host(FOREIGN, "/foreign?name=foreign");
    let observer_origin_url = server.url_for_host(OBSERVER, "/observer-origin?name=observerOrigin");
    let duplicate_url = server.url_for_host(FOREIGN, "/duplicate?name=duplicate");
    let loader = static_http_loader([
        server.resolve_entry(PARENT),
        server.resolve_entry(OBSERVER),
        server.resolve_entry(FOREIGN),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        r#"
        globalThis.loaded = new Set();
        addEventListener('message', event => loaded.add(event.data));
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        const definitions = [
          ['observer-frame', 'observer', {observer_url:?}],
          ['foreign-frame', 'foreign', {foreign_url:?}],
          ['observer-origin-frame', 'observerOrigin', {observer_origin_url:?}],
          ['allowed-frame', 'allowed', 'about:blank'],
          ['duplicate-first', 'duplicate', {duplicate_url:?}],
          ['duplicate-second', 'duplicate', 'about:blank']
        ];
        for (const [id, name, src] of definitions) {{
          const frame = document.createElement('iframe');
          // Loaded documents publish window.name themselves. Leave their
          // container name unset so embedder authorization cannot mask the
          // parent-origin filtering and first-match checks below.
          Object.assign(frame, {{id, src}});
          if (src === 'about:blank') frame.name = name;
          body.appendChild(frame);
        }}
        "#,
        observer_url = observer_url.as_str(),
        foreign_url = foreign_url.as_str(),
        observer_origin_url = observer_origin_url.as_str(),
        duplicate_url = duplicate_url.as_str(),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(['/observer', '/foreign', '/observer-origin', '/duplicate'].every(path => loaded.has(path)))",
        "true",
        "three-origin child documents should commit and execute",
    )
    .await;
    let observer = child_context_for_element(&mut vm, "observer-frame");
    let result = vm.eval_in_child_default_context(observer, r#"
        globalThis.retained = top[1];
        globalThis.allowedDuplicate = top[5];
        globalThis.probe = callback => {
          try {
            const value = callback();
            return value === undefined ? 'undefined' : typeof value === 'boolean' ? value : 'exposed';
          }
          catch (error) { return error.name; }
        };
        globalThis.namedAccess = name => [
          probe(() => top[name]),
          probe(() => Object.getOwnPropertyDescriptor(top, name)),
          probe(() => name in top)
        ];
        JSON.stringify({
          foreign: namedAccess('foreign'),
          observerOrigin: namedAccess('observerOrigin'),
          duplicate: namedAccess('duplicate'),
          allowed: top.allowed === top[3],
          descriptor: Object.getOwnPropertyDescriptor(top, 'allowed').value === top[3],
          indexed: top[1].parent === top && top[2].parent === top
        })
    "#).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "foreign": ["SecurityError", "SecurityError", "SecurityError"],
            "observerOrigin": ["SecurityError", "SecurityError", "SecurityError"],
            "duplicate": ["SecurityError", "SecurityError", "SecurityError"],
            "allowed": true,
            "descriptor": true,
            "indexed": true
        })
    );
    assert_eq!(
        vm.eval("window.foreign === undefined && window.duplicate === undefined")
            .unwrap(),
        "true"
    );

    // The receiver may itself be a child Window. Its own child inherits that
    // Window's origin, which differs from the top-level caller's origin.
    vm.eval_in_child_default_context(
        observer,
        r#"
        const nested = document.createElement('iframe');
        nested.name = 'inherited';
        document.body.appendChild(nested);
        void nested.contentWindow;
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("document.getElementById('observer-frame').contentWindow.inherited === document.getElementById('observer-frame').contentWindow[0]").unwrap(),
        "true"
    );

    // Reordering must select the first name in the current document tree,
    // rather than preserving insertion order or skipping a cross-origin first.
    vm.eval("document.body.moveBefore(document.getElementById('duplicate-first'), null)")
        .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(observer, "top.duplicate === allowedDuplicate")
            .unwrap(),
        "true"
    );
    vm.eval("document.getElementById('duplicate-second').remove()")
        .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(observer, "JSON.stringify(namedAccess('duplicate'))")
            .unwrap(),
        r#"["SecurityError","SecurityError","SecurityError"]"#
    );

    for (host, path, visible) in [
        (PARENT, "/parent-origin?name=foreign", true),
        (FOREIGN, "/foreign-again?name=foreign", false),
    ] {
        let url = server.url_for_host(host, path);
        vm.eval(&format!(
            "document.getElementById('foreign-frame').src = {:?};",
            url.as_str()
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            &format!("String(loaded.has({:?}))", url.path()),
            "true",
            "named visibility should follow the committed child origin",
        )
        .await;
        assert_eq!(
            vm.eval_in_child_default_context(observer, "top[1] === retained")
                .unwrap(),
            "true",
            "numeric WindowProxy identity survives origin changes"
        );
        assert_eq!(
            vm.eval_in_child_default_context(observer, "probe(() => top.foreign === retained)")
                .unwrap(),
            if visible { "true" } else { "SecurityError" }
        );
    }
    // Setting document.domain does not change the origin tuple used for this
    // property set, even when it changes ordinary script access checks.
    vm.eval("document.domain = document.domain").unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(observer, "top.allowed.parent === top")
            .unwrap(),
        "true"
    );
    let foreign = child_context_for_element(&mut vm, "foreign-frame");
    vm.eval_in_child_default_context(foreign, "window.name = 'then'")
        .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(observer,
            "top.then === undefined && Object.getOwnPropertyDescriptor(top, 'then').value === undefined").unwrap(),
        "true",
        "an ineligible child must not shadow the cross-origin fallback"
    );
    // Same-origin named access can still expose a cross-origin named object
    // when an element id independently supplies the supported property name.
    vm.eval(
        "document.body.appendChild(Object.assign(document.createElement('div'), {id: 'then'}))",
    )
    .unwrap();
    assert_eq!(
        vm.eval("window.then === document.getElementById('foreign-frame').contentWindow")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(observer, "top.then === undefined")
            .unwrap(),
        "true"
    );
    assert_eq!(server.finish().await.len(), 6);
}
