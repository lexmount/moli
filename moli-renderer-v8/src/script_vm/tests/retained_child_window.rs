use super::*;

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
