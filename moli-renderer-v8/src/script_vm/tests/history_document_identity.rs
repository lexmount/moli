use super::*;

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
