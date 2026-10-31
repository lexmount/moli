use super::*;

#[test]
fn mutation_observer_tracks_removed_subtrees_and_delivers_reentrant_queues_in_order() {
    let mut vm = new_storage_html_test_vm("https://mutation-observer-lifecycle.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!(
        "../../../tests/fixtures/mutation-observer-lifecycle.js"
    ))
    .unwrap();
    let facts: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(globalThis.__uiEventResults)")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(facts["complete"], true);
    let checks = facts["checks"].as_array().unwrap();
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["passed"] != true)
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(facts["total"], 378);
    assert_eq!(facts["passed"], 378);
    assert_eq!(checks.len(), 378);
    assert_eq!(
        checks
            .iter()
            .map(|check| check["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        378
    );
}
