use super::*;

pub(super) fn child_context_for_element(vm: &mut ScriptVm, element_id: &str) -> i64 {
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
        .expect("the committed child should have a default realm")
        .context_id
}

#[tokio::test(flavor = "current_thread")]
async fn frame_owner_content_document_uses_its_node_document_origin() {
    const ROOT: &str = "root.frame-owner.test";
    const PARENT: &str = "parent.frame-owner.test";
    const FOREIGN: &str = "foreign.frame-owner.test";
    let server = StaticHttpServer::spawn_with_responder(
        6,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, request| {
            let nested = if request.target == "/parent" {
                let (_, port) = request.host.as_deref().unwrap().split_once(':').unwrap();
                format!(
                    r#"<iframe id="root-origin" src="http://{ROOT}:{port}/root-child"></iframe>
                    <iframe id="parent-origin" src="/parent-child"></iframe>
                    <iframe id="foreign-origin" src="http://{FOREIGN}:{port}/foreign-child"></iframe>"#
                )
            } else {
                String::new()
            };
            format!(
                "<!doctype html><body>{nested}<p>private document</p>\
                 <script>top.postMessage(location.pathname, '*')</script>"
            )
        },
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry(ROOT),
        server.resolve_entry(PARENT),
        server.resolve_entry(FOREIGN),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        server.url_for_host(ROOT, "/root").as_str(),
        &loader,
    );
    let parent_url = server.url_for_host(PARENT, "/parent");
    vm.eval(&format!(
        r#"
        globalThis.loaded = new Set();
        addEventListener('message', event => loaded.add(event.data));
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.appendChild(Object.assign(document.createElement('iframe'), {{id: 'parent-frame', src: {parent_url:?}}}));
        "#,
        parent_url = parent_url.as_str(),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(['/parent', '/root-child', '/parent-child', '/foreign-child'].every(path => loaded.has(path)))",
        "true",
        "three-origin nested frame documents should commit and execute",
    )
    .await;
    let parent = child_context_for_element(&mut vm, "parent-frame");
    assert_eq!(
        vm.eval_in_child_default_context(parent, "frameElement === null")
            .unwrap(),
        "true",
        "a cross-origin child must not obtain its embedding Document through frameElement"
    );
    for (id, visible) in [
        ("root-origin", false),
        ("parent-origin", true),
        ("foreign-origin", false),
    ] {
        let child = child_context_for_element(&mut vm, id);
        assert_eq!(
            vm.eval_in_child_default_context(child, "String(frameElement !== null)")
                .unwrap(),
            visible.to_string(),
            "frameElement must compare the child with its actual parent: {id}"
        );
    }
    let result = vm.eval_in_child_default_context(parent, r#"
        const probe = callback => {
          try { return callback(); }
          catch (error) { return error.name; }
        };
        const getter = Object.getOwnPropertyDescriptor(window, 'document').get;
        JSON.stringify(['root-origin', 'parent-origin', 'foreign-origin'].map(id => {
          const frame = document.getElementById(id);
          return {
            document: probe(() => frame.contentDocument === null ? null : frame.contentDocument.querySelector('p').textContent),
            windowDocument: probe(() => frame.contentWindow.document === frame.contentDocument),
            borrowedGetter: probe(() => getter.call(frame.contentWindow) === frame.contentDocument),
            svgDocument: frame.getSVGDocument()
          };
        }))
    "#).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([
            { "document": null, "windowDocument": "SecurityError", "borrowedGetter": "SecurityError", "svgDocument": null },
            { "document": "private document", "windowDocument": true, "borrowedGetter": true, "svgDocument": null },
            { "document": null, "windowDocument": "SecurityError", "borrowedGetter": "SecurityError", "svgDocument": null }
        ])
    );
    vm.eval_in_child_default_context(
        parent,
        "globalThis.retainedChild = document.getElementById('parent-origin').contentWindow",
    )
    .unwrap();
    for (host, path, visible) in [
        (ROOT, "/root-navigation", false),
        (PARENT, "/parent-navigation", true),
    ] {
        let url = server.url_for_host(host, path);
        vm.eval_in_child_default_context(
            parent,
            &format!(
                "document.getElementById('parent-origin').src = {:?}",
                url.as_str()
            ),
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            &format!("String(loaded.has({path:?}))"),
            "true",
            "contentDocument access should follow the committed child origin",
        )
        .await;
        assert_eq!(
            vm.eval_in_child_default_context(
                parent,
                "JSON.stringify([document.getElementById('parent-origin').contentDocument !== null, document.getElementById('parent-origin').contentWindow === retainedChild])",
            )
            .unwrap(),
            format!("[{visible},true]")
        );
    }

    // contentDocument uses same origin-domain, unlike the stricter origin
    // tuple comparison for Window named child properties.
    vm.eval_in_child_default_context(parent, "document.domain = document.domain")
        .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(
            parent,
            "document.getElementById('parent-origin').contentDocument === null",
        )
        .unwrap(),
        "true",
        "one-sided document.domain assignment must revoke access"
    );
    let child = child_context_for_element(&mut vm, "parent-origin");
    assert_eq!(
        vm.eval_in_child_default_context(child, "frameElement === null")
            .unwrap(),
        "true",
        "the cached embedding element must be hidden after one-sided domain mutation"
    );
    vm.eval_in_child_default_context(child, "document.domain = document.domain")
        .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(child, "frameElement.ownerDocument === parent.document")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            parent,
            "document.getElementById('parent-origin').contentDocument === retainedChild.document",
        )
        .unwrap(),
        "true",
        "matching document.domain assignments must restore access"
    );
    assert_eq!(server.finish().await.len(), 6);
}
