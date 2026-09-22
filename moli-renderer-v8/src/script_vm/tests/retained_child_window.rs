use super::*;

#[test]
fn child_document_open_navigation_keeps_window_accessible_before_realm_turn() {
    for materialized in [false, true] {
        let mut vm = new_storage_test_vm("https://document-open-proxy.test/parent");
        vm.eval(
            r#"
globalThis.frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
globalThis.heldWindow = frame.contentWindow;
"#,
        )
        .unwrap();
        if materialized {
            materialize_single_child_default_realm_for_test(&mut vm, "initial child Window");
        }
        vm.eval(
            r#"
heldWindow.document.open();
heldWindow.document.write('<p>old document');
heldWindow.document.close();
globalThis.heldDocument = heldWindow.document;
globalThis.heldObject = heldWindow.Object;
heldWindow.marker = 'retired';
globalThis.heldRead = heldWindow.Function('return marker');
globalThis.heldNavigator = heldWindow.navigator;
globalThis.heldReadNavigator = heldWindow.Function('return navigator');
"#,
        )
        .unwrap();
        let previous = current_single_child_document_owner_for_test(&vm, "opened child");
        vm.eval("frame.src = 'about:blank'").unwrap();
        for _ in 0..8 {
            if vm
                .run_next_child_navigation_commit_body_for_test()
                .unwrap()
                .is_none()
                || current_single_child_document_owner_for_test(&vm, "navigating child")
                    .local_window_id
                    != previous.local_window_id
            {
                break;
            }
        }
        assert_ne!(
            current_single_child_document_owner_for_test(&vm, "committed child").local_window_id,
            previous.local_window_id,
            "the replacement must commit without consuming a realm task"
        );
        assert_eq!(vm.child_frame_realm_store.len(), 0);
        assert!(vm.has_pending_child_frame_realm_materialization());
        assert_eq!(
            vm.eval(
                r#"JSON.stringify([
heldWindow.document !== heldDocument,
heldWindow.document.URL,
heldWindow.Object !== heldObject,
typeof heldWindow.marker,
heldRead(),
heldWindow === frame.contentWindow,
heldReadNavigator() === heldWindow.navigator,
heldReadNavigator() !== heldNavigator
])"#,
            )
            .unwrap(),
            r#"[true,"about:blank",true,"undefined","retired",true,true,true]"#,
            "materialized={materialized}: commit must reconnect the proxy before another JS task"
        );
        assert_eq!(
            vm.child_frame_realm_store.len(),
            0,
            "synchronous access must leave Inspector registration to the queued realm turn"
        );
    }
}

#[test]
fn current_child_isolated_world_can_post_messages_to_its_native_window() {
    let mut vm = new_storage_test_vm("https://isolated-child-message.test/");
    vm.eval(
        r#"
      globalThis.frame = document.createElement('iframe');
      (document.body || document.documentElement || document).appendChild(frame);
      void frame.contentWindow;
    "#,
    )
    .expect("child Window should be exposed");
    let context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "isolated child postMessage");
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == context_id)
        .and_then(|realm| realm.frame_id)
        .expect("child frame id");
    let isolated = vm
        .create_isolated_world_for_frame(&frame_id, "child-post-message", false)
        .expect("child isolated world should be created");
    assert!(!vm._context_host.borrow().has_pending_window_messages());
    vm.eval_in_isolated_context(
        isolated,
        "window.postMessage('isolated-self', '*'); 'posted'",
    )
    .expect("isolated child Window should accept postMessage");
    assert!(
        vm._context_host.borrow().has_pending_window_messages(),
        "a current isolated global must resolve to the native child Window, despite having a distinct V8 proxy"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn removed_cross_origin_windows_keep_their_surface_without_targeting_replacements() {
    for shadow in [false, true] {
        for mode in ["remove", "ancestor", "replace"] {
            let child = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/retained-cross-origin-child.html"
            ));
            let leaf = "<!doctype html><title>leaf</title>";
            let server = StaticHttpServer::spawn_with_bodies(
                [child, leaf, child, leaf].map(str::to_owned).to_vec(),
            )
            .await;
            let loader = static_http_loader([
                server.resolve_entry("www.example.test"),
                server.resolve_entry("remote.example.test"),
            ]);
            let parent_url = server.url_for_host("www.example.test", "/page.html");
            let child_url = server.url_for_host("remote.example.test", "/child.html");
            let child_url = child_url.as_str();
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
            let script = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/retained-cross-origin-window.js"
            ));
            vm.exec(
                &format!(
                    r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__retainedCrossOriginResult = null;
({script})({{childURL: {child_url:?}, mode: {mode:?}, shadow: {shadow}}}).then(
  result => {{ __retainedCrossOriginResult = result; }},
  error => {{ __retainedCrossOriginResult = {{error: String(error)}}; }}
);
"#
                ),
                None,
            )
            .expect("cross-origin Window removal probe should start");
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__retainedCrossOriginResult !== null)",
                "true",
                "cross-origin Window removal probe should finish",
            )
            .await;
            let result: serde_json::Value = serde_json::from_str(
                &vm.eval("JSON.stringify(__retainedCrossOriginResult)")
                    .expect("cross-origin Window observations"),
            )
            .unwrap();
            assert_eq!(result["checks"], 103, "{mode}/shadow={shadow}: {result}");
            assert_eq!(
                result["failures"],
                serde_json::json!([]),
                "{mode}/shadow={shadow}: {result}"
            );
            assert_eq!(
                server.finish_targets().await,
                vec!["/child.html", "/leaf.html", "/child.html", "/leaf.html"],
                "{mode}/shadow={shadow}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn removed_windows_retain_aliases_through_unload_without_reviving_on_reattachment() {
    for shadow in [false, true] {
        for mode in ["remove", "replace", "ancestor", "fragment"] {
            let server = StaticHttpServer::spawn_with_bodies(vec![
                "<!doctype html><body>Window removal target</body>".to_owned(); 3
            ])
            .await;
            let loader = static_http_loader([server.resolve_entry("www.example.test")]);
            let parent_url = server.url_for_host("www.example.test", "/page.html");
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
            let script = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/removed-window-lifecycle.js"
            ));
            vm.exec(
                &format!(
                    r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__removedWindowResult = null;
({script})({{mode: {mode:?}, shadow: {shadow}}}).then(
  result => {{ __removedWindowResult = result; }},
  error => {{ __removedWindowResult = {{error: String(error)}}; }}
);
"#
                ),
                None,
            )
            .expect("Window removal probe should start");
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__removedWindowResult !== null)",
                "true",
                "Window removal probe should finish",
            )
            .await;
            let result: serde_json::Value = serde_json::from_str(
                &vm.eval("JSON.stringify(__removedWindowResult)")
                    .expect("Window removal observations"),
            )
            .unwrap();
            let observations = (0..2)
                .flat_map(|index| {
                    ["pagehide", "visibilitychange", "unload"].map(|event| {
                        serde_json::json!({
                            "index": index, "type": event,
                            "top": true, "parent": true, "frameElement": true,
                            "closed": false, "hidden": event != "pagehide",
                        })
                    })
                })
                .collect::<Vec<_>>();
            let retired = serde_json::json!([
                {"top": true, "parent": true, "frameElement": true, "closed": true},
                {"top": true, "parent": true, "frameElement": true, "closed": true},
            ]);
            assert_eq!(
                result,
                serde_json::json!({
                    "observations": observations, "after": retired, "reinserted": retired,
                    "fresh": {"top": true, "parent": true, "frameElement": true,
                        "closed": false, "different": true},
                }),
                "{mode}/shadow={shadow}: {result}"
            );
            assert_eq!(
                server.finish_targets().await,
                vec![
                    "/history.html?root",
                    "/history.html?child",
                    "/history.html?replacement"
                ],
                "{mode}/shadow={shadow}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn inactive_locations_have_blank_urls_and_cannot_navigate() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><body>Location lifecycle target</body>".to_owned(); 4
    ])
    .await;
    let loader = static_http_loader([server.resolve_entry("www.example.test")]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/inactive-location.js"
    ));
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__inactiveLocationResult = null;
({script})().then(
  result => {{ globalThis.__inactiveLocationResult = result; }},
  error => {{ globalThis.__inactiveLocationResult = {{error: String(error)}}; }}
);
"#,
        ),
        None,
    )
    .expect("inactive Location probe should start");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__inactiveLocationResult !== null)",
        "true",
        "inactive Location probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__inactiveLocationResult)")
            .expect("inactive Location observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 295, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        vec![
            "/removed.html?query=one",
            "/removed.html?query=one",
            "/before.html",
            "/after.html"
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn retained_location_rechecks_origin_domain_access() {
    for parent_first in [false, true] {
        let server = StaticHttpServer::spawn_with_bodies(vec![
            "<!doctype html><body>Location target</body>".to_owned(); 2
        ])
        .await;
        let loader = static_http_loader([server.resolve_entry("www.example.test")]);
        let parent_url = server.url_for_host("www.example.test", "/page.html");
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        let script = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/retained-location-origin.js"
        ));
        vm.exec(
            &format!(
                r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__retainedLocationResult = null;
({script})({{parentFirst: {parent_first}}}).then(
  result => {{ globalThis.__retainedLocationResult = result; }},
  error => {{ globalThis.__retainedLocationResult = {{error: String(error)}}; }}
);
"#,
            ),
            None,
        )
        .expect("retained Location probe should start");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__retainedLocationResult !== null)",
            "true",
            "retained Location probe should finish",
        )
        .await;
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval("JSON.stringify(__retainedLocationResult)")
                .expect("retained Location observations"),
        )
        .unwrap();
        assert_eq!(
            result["checks"], 143,
            "parent_first={parent_first}: {result}"
        );
        assert_eq!(
            result["failures"],
            serde_json::json!([]),
            "parent_first={parent_first}: {result}"
        );
        assert_eq!(
            server.finish_targets().await,
            vec!["/child.html", "/peer.html"]
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn retained_child_window_origin_rebinds_default_and_isolated_realms() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><script>document.domain = 'example.test';</script>".to_owned(),
    ])
    .await;
    let loader = static_http_loader([server.resolve_entry("www.example.test")]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || root.appendChild(document.createElement('body'));
globalThis.frame = document.createElement('iframe');
body.append(frame);
globalThis.heldWindow = frame.contentWindow;
"#,
        None,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 16)
        .await
        .expect("initial child realm should materialize through Page tasks");
    let child_realm = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("initial child realm");
    let child_context_id = child_realm.context_id;
    let frame_id = child_realm.frame_id.expect("initial child frame id");
    for (name, universal) in [("enforcedRead", false), ("universalRead", true)] {
        let context_id = vm
            .create_isolated_world_for_frame(&frame_id, name, universal)
            .expect("initial child isolated world");
        vm.eval_in_isolated_context(
            context_id,
            &format!(
                "parent.{name} = ((target) => () => {{ try {{ return target.status; }} catch (error) {{ return error.name; }} }})(parent); 'installed'"
            ),
        )
        .expect("retain a callback from the initial isolated realm");
    }
    vm.exec(
        "globalThis.loaded = false; frame.onload = () => { loaded = true; }; frame.src = '/child.html';",
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(loaded)",
        "true",
        "network document should replace the initial about:blank document",
    )
    .await;
    assert_eq!(
        vm.live_child_default_runtime_realm_inventory()[0].context_id,
        child_context_id,
        "the first same-origin navigation must reuse the initial realm"
    );
    const PROBE: &str = r#"JSON.stringify([
(() => { try { return heldWindow.status; } catch (error) { return error.name; } })(),
enforcedRead(),
universalRead()
])"#;
    let expected = r#"["SecurityError","SecurityError",""]"#;
    assert_eq!(vm.eval(PROBE).unwrap(), expected);
    vm.eval("frame.remove()").unwrap();
    vm.prune_stale_child_default_execution_contexts();
    assert_eq!(vm.child_frame_realm_store.len(), 0);
    assert_eq!(vm.page_isolated_world_contexts.len(), 0);
    assert_eq!(
        vm.eval(PROBE).unwrap(),
        expected,
        "retired realms must keep the committed origin and their own access policy"
    );
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}

#[tokio::test(flavor = "current_thread")]
async fn retained_child_windows_keep_shared_origin_domain_access() {
    const BODY: &str = r#"<!doctype html><body><script>
const domain = new URL(location.href).searchParams.get('domain');
if (domain) document.domain = domain;
</script></body>"#;
    let server = StaticHttpServer::spawn_with_bodies(vec![BODY.to_owned(); 26]).await;
    let second_server = StaticHttpServer::spawn_with_bodies(vec![BODY.to_owned()]).await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("sub.example.test"),
        server.resolve_entry("other.test"),
        second_server.resolve_entry("www.example.test"),
    ]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/retained-document-domain.js"
    ));
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__retainedDomainResult = null;
({script})({port}).then(
  result => {{ globalThis.__retainedDomainResult = result; }},
  error => {{ globalThis.__retainedDomainResult = {{error: String(error)}}; }}
);
"#,
            port = second_server.base_url().port().unwrap(),
        ),
        None,
    )
    .expect("retained origin-domain probe should start");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__retainedDomainResult !== null)",
        "true",
        "retained origin-domain probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__retainedDomainResult)")
            .expect("retained origin-domain observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 85, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(server.finish_targets().await.len(), 26);
    assert_eq!(second_server.finish_targets().await.len(), 1);
}

#[test]
fn retained_child_window_survives_context_retirement() {
    for materialized in [false, true] {
        for reinserted in [false, true] {
            let mut vm = new_storage_test_vm("https://retained-child-window.test/");
            vm.eval(
                r#"
document.appendChild(document.createElement('html'));
document.documentElement.appendChild(document.createElement('body'));
globalThis.frame = document.createElement('iframe');
document.body.append(frame);
globalThis.heldWindow = frame.contentWindow;
globalThis.heldDocument = heldWindow.document;
globalThis.heldEvent = heldWindow.Event;
heldWindow.marker = {retained: true};
globalThis.heldMarker = heldWindow.marker;
"#,
            )
            .unwrap();
            assert_eq!(vm.prebootstrapped_child_default_contexts.borrow().len(), 1);
            if materialized {
                materialize_single_child_default_realm_for_test(&mut vm, "retained Window");
                assert_eq!(vm.child_frame_realm_store.len(), 1);
            }

            vm.eval(if reinserted {
                "frame.remove(); document.body.append(frame); globalThis.replacement = frame.contentWindow;"
            } else {
                "frame.remove();"
            })
            .unwrap();
            vm.prune_stale_child_default_execution_contexts();
            assert_eq!(vm.child_frame_realm_store.len(), 0);
            assert_eq!(
                vm._context_host
                    .borrow()
                    .window_execution_context_registry_counts_for_test(),
                if reinserted { (2, 2) } else { (1, 1) },
                "retirement must release the old realm registration"
            );
            let observed = vm.eval(
                r#"JSON.stringify([
heldWindow.document === heldDocument,
heldWindow.Event === heldEvent,
heldWindow.marker === heldMarker,
heldWindow.self === heldWindow,
typeof heldWindow.addEventListener === 'function',
heldDocument.open() === heldDocument,
heldWindow.document === heldDocument,
typeof replacement === 'undefined' || replacement !== heldWindow,
typeof replacement === 'undefined' || replacement.document !== heldDocument
])"#,
            );
            assert_eq!(
                observed.unwrap_or_else(|error| panic!(
                    "materialized={materialized}, reinserted={reinserted}: {error}"
                )),
                "[true,true,true,true,true,true,true,true,true]"
            );
        }
    }
}
