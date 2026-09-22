use super::*;

#[test]
fn history_snapshots_restore_structured_state_after_source_vm_is_dropped() {
    let mut source = new_storage_test_vm("https://history-snapshot.test/current");
    source
        .eval(
            r#"
        const buffer = new ArrayBuffer(4);
        new Uint8Array(buffer).set([1, 2, 3, 4]);
        const state = {
          map: new Map(), bytes: new Uint8Array(buffer), view: new DataView(buffer),
          bigint: 123n, date: new Date(123), blob: new Blob(['state']),
          exception: new DOMException('message', 'DataError'), point: new DOMPoint(1, 2)
        };
        state.map.set(state, state);
        history.replaceState(state, '');
        navigation.updateCurrentEntry({state});
        location.reload();
        "queued"
    "#,
        )
        .unwrap();
    let seed = source
        .take_pending_location_navigation_with_seed()
        .unwrap()
        .entry_seed
        .unwrap();
    drop(source);

    let mut restored = new_storage_test_vm("https://history-snapshot.test/current");
    restored.install_navigation_bootstrap_entry(Some(seed));
    assert_eq!(
        restored
            .eval(
                r#"
        [history.state, navigation.currentEntry.getState()].every(state =>
          state.map instanceof Map && state.map.get(state) === state &&
          state.bytes instanceof Uint8Array && state.view instanceof DataView &&
          state.bytes.buffer === state.view.buffer && state.view.getUint8(2) === 3 &&
          state.bigint === 123n && state.date instanceof Date && +state.date === 123 &&
          state.blob instanceof Blob && state.blob.size === 5 &&
          state.exception instanceof DOMException && state.exception.name === 'DataError' &&
          state.point instanceof DOMPoint && state.point.x === 1 && state.point.y === 2)
    "#
            )
            .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn history_structured_state_survives_document_and_entry_changes_without_json() {
    for mode in [
        "reload",
        "traverse",
        "traverse-builtins",
        "fragment",
        "null-reload",
        "undefined-reload",
    ] {
        let requests = match mode {
            "traverse" | "traverse-builtins" => vec![
                "/common/blank.html?state",
                "/common/blank.html?away",
                "/common/blank.html?state",
            ],
            "fragment" => vec!["/common/blank.html?state"],
            _ => vec!["/common/blank.html?state", "/common/blank.html?state"],
        };
        let server = StaticHttpServer::spawn(requests.len()).await;
        let base = server.base_url().origin().ascii_serialization();
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(&format!("{base}/parent"), &loader);
        let script = include_str!("../../../tests/fixtures/history-structured-state.js");
        vm.eval(&format!(
            "{script}\nglobalThis.stateResult = 'pending';\n\
             historyStructuredState({base:?}, {mode:?}).then(\n\
               value => stateResult = value, error => stateResult = String(error));"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(stateResult !== 'pending')",
            "true",
            mode,
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(stateResult)").unwrap()).unwrap();
        let primitive = mode.starts_with("null") || mode.starts_with("undefined");
        assert_eq!(result["jsonCalls"], 0, "{mode}: {result}");
        assert_eq!(
            result["getterCalls"],
            if primitive { 0 } else { 2 },
            "{mode}: {result}"
        );
        for stage in ["before", "after"] {
            for api in ["history", "navigation"] {
                let checks = result[stage][api]
                    .as_object()
                    .unwrap_or_else(|| panic!("{mode}: {result}"));
                let cleared = mode == "fragment" && stage == "after" && api == "history";
                assert_eq!(
                    checks.len(),
                    if primitive || cleared {
                        1
                    } else if mode == "traverse-builtins" {
                        15
                    } else {
                        17
                    },
                    "{mode}/{stage}/{api}"
                );
                for (property, valid) in checks {
                    assert_eq!(valid, true, "{mode}/{stage}/{api}/{property}: {result}");
                }
            }
        }
        assert_eq!(server.finish_targets().await, requests, "{mode}");
    }
}

#[test]
fn navigation_state_uses_storage_serialization_before_dispatch() {
    let mut vm = new_storage_test_vm("https://history-snapshot.test/current");
    let script = include_str!("../../../tests/fixtures/history-structured-state.js");
    vm.eval(&format!(
        "{script}\nglobalThis.storageResult = null; historyStateStoragePolicy().then(value => storageResult = value);"
    )).unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(storageResult)").unwrap()).unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "errors": ["DataCloneError", "DataCloneError", "DataCloneError", "DataCloneError", "DataCloneError", "DataCloneError"],
            "events": 0, "entryUnchanged": true, "history": "safe", "navigation": true,
            "runtimeCloneAllowed": true,
        })
    );
}

#[test]
fn history_snapshots_ignore_navigation_entry_property_overrides() {
    let mut vm = new_storage_test_vm("https://history-snapshot.test/current");
    let identity: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify([navigation.currentEntry.id, navigation.currentEntry.key])")
            .unwrap(),
    )
    .unwrap();
    vm.eval(
        r#"
        for (const name of ['id', 'key', 'index']) {
          Object.defineProperty(navigation.currentEntry, name, {
            value: name === 'index' ? 99 : 'author-' + name,
            configurable: true,
          });
        }
        location.reload();
        "queued"
        "#,
    )
    .unwrap();
    let pending = vm.take_pending_location_navigation_with_seed().unwrap();
    let seed = pending.entry_seed.unwrap();
    let current = seed
        .entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
        .unwrap();
    assert_eq!(current.id.as_str(), identity[0].as_str().unwrap());
    assert_eq!(current.key.as_str(), identity[1].as_str().unwrap());
    assert_eq!(current.index, 0);
}

#[tokio::test]
async fn history_traversal_preserves_documents_and_contiguous_navigation_entries() {
    for method in ["joint", "child", "navigation"] {
        for mode in [
            "local-same",
            "local-fragment",
            "local-different",
            "cross-same",
            "cross-fragment",
            "cross-different",
        ] {
            let cross_origin = mode.starts_with("cross-");
            if cross_origin && method == "navigation" {
                // Navigation only exposes entries in the contiguous same-origin region.
                continue;
            }
            let server = StaticHttpServer::spawn(if cross_origin { 4 } else { 5 }).await;
            let other_server = StaticHttpServer::spawn(usize::from(cross_origin)).await;
            let base = server.base_url().origin().ascii_serialization();
            let other = other_server.base_url().origin().ascii_serialization();
            let loader = static_http_loader([]);
            let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                &format!("{base}/parent"),
                &loader,
            );
            let script = include_str!("../../../tests/fixtures/history-document-identity.js");
            vm.eval(&format!(
                "{script}\nglobalThis.identityResult = 'pending';\n\
                 historyDocumentIdentity({base:?}, {other:?}, {mode:?}, {method:?}).then(\n\
                   value => identityResult = value, error => identityResult = String(error));"
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(identityResult !== 'pending')",
                "true",
                &format!("{method}/{mode}"),
            )
            .await;
            let result: serde_json::Value =
                serde_json::from_str(&vm.eval("JSON.stringify(identityResult)").unwrap()).unwrap();
            let first = "<base>/common/blank.html?first#start";
            let last = if mode.ends_with("-same") {
                first
            } else if mode.ends_with("-fragment") {
                "<base>/common/blank.html?first#last"
            } else {
                "<base>/common/blank.html?last"
            };
            let rows: Vec<_> = ["last", "back", "forward", "same-document"]
                .into_iter()
                .enumerate()
                .map(|(index, stage)| {
                    let back = stage == "back";
                    let same_document = stage == "same-document";
                    let current = if back { first } else { last };
                    let state = if back { "first" } else { "last" };
                    let mut entries = if cross_origin {
                        vec![current]
                    } else {
                        vec![first, "<base>/common/blank.html?middle", last]
                    };
                    let mut same_documents = if cross_origin {
                        vec![true]
                    } else {
                        vec![back, false, !back]
                    };
                    if same_document {
                        entries.push("<base>/common/blank.html?state");
                        same_documents.push(true);
                    }
                    let from = if cross_origin {
                        None
                    } else if stage == "last" {
                        Some("<base>/common/blank.html?middle")
                    } else if back {
                        Some(last)
                    } else {
                        Some(first)
                    };
                    serde_json::json!({
                        "stage": stage, "loads": (index + 3).min(5),
                        "history": if same_document { 3 } else { 2 },
                        "url": current, "entries": entries, "sameDocument": same_documents,
                        "index": if back || cross_origin { 0 } else { 2 },
                        "identity": true, "classicState": {"classic": state},
                        "navigationState": {"navigation": state}, "from": from,
                        "type": if stage == "last" { "push" } else { "traverse" },
                        "activationEntry": stage != "last",
                    })
                })
                .collect();
            assert_eq!(
                result,
                serde_json::json!({
                    "rows": rows, "sameDocumentPreserved": true, "activationPreserved": true,
                }),
                "{method}/{mode}"
            );
            let last_path = if mode.ends_with("-different") {
                "/common/blank.html?last"
            } else {
                "/common/blank.html?first"
            };
            let mut requests = vec!["/common/blank.html?first"];
            if !cross_origin {
                requests.push("/common/blank.html?middle");
            }
            requests.extend([last_path, "/common/blank.html?first", last_path]);
            assert_eq!(server.finish_targets().await, requests, "{method}/{mode}");
            assert_eq!(
                other_server.finish_targets().await,
                if cross_origin {
                    vec!["/common/blank.html?middle"]
                } else {
                    vec![]
                },
                "{method}/{mode}"
            );
        }
    }
}

#[test]
fn restored_cross_origin_history_fragment_mutations_use_native_entry_indices() {
    fn complete_load(vm: &mut StandaloneScriptVmHarness) {
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
    }

    let mut source = new_storage_test_vm("https://history-source.test/current");
    complete_load(&mut source);
    source
        .eval("location.href = 'https://history-destination.test/current'; 'queued'")
        .unwrap();
    let seed = source
        .take_pending_location_navigation_with_seed()
        .unwrap()
        .entry_seed
        .unwrap();
    assert!(seed.entries.iter().any(|entry| {
        entry.history_index < seed.current_index
            && entry.url == "https://history-source.test/current"
    }));
    drop(source);
    let mut restored = new_storage_test_vm("https://history-destination.test/current");
    restored.install_navigation_bootstrap_entry(Some(seed));
    complete_load(&mut restored);
    assert_eq!(restored.eval(r#"
        const first = navigation.currentEntry;
        location.hash = '#next';
        const second = navigation.currentEntry;
        const before = [navigation.entries().length, navigation.entries()[0] === first,
                        first.index, second.index];
        let indexReads = 0;
        Object.defineProperty(second, 'index', {configurable: true, get() { indexReads++; return 999; }});
        location.hash = '#last';
        delete second.index;
        JSON.stringify([...before, indexReads, navigation.entries().length, navigation.currentEntry.index]);
    "#).unwrap(), "[2,true,0,1,0,3,2]");
}
