use super::*;

fn transaction_scope_probe(kind: &str) -> serde_json::Value {
    let mut vm = new_storage_page_task_executor_test_vm("https://idb-transaction-scope.test/");
    vm.eval(include_str!("idb_transaction_scope.js")).unwrap();
    vm.eval(&format!(
        "globalThis.scopeProbeReport = 'pending'; runIdbTransactionScopeProbe({kind:?}).then(value => scopeProbeReport = value, error => scopeProbeReport = {{error: error.name, message: error.message}});"
    ))
    .unwrap();
    let result = vm
        .eval_after_selected_page_tasks("JSON.stringify(scopeProbeReport)")
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[test]
fn indexed_db_transaction_object_store_checks_native_scope_and_finished_state() {
    assert_eq!(
        transaction_scope_probe("admission"),
        serde_json::json!({
            "repeatedHandle": true,
            "emptyName": true,
            "surrogateName": 0xd800,
            "outside": "NotFoundError",
            "otherSurrogate": "NotFoundError",
            "scope": [[], [107,101,112,116], [0xd800]],
            "shadowIgnored": "NotFoundError",
            "committingHandle": "InvalidStateError",
            "committingOutside": "InvalidStateError",
            "committingRequest": "TransactionInactiveError",
            "finishedInside": "InvalidStateError",
            "finishedOutside": "InvalidStateError",
            "conversionBeforeState": true,
            "retainedScope": [[], [107,101,112,116], [0xd800]],
            "abortedInside": "InvalidStateError",
            "abortedOutside": "InvalidStateError"
        })
    );
}

#[test]
fn indexed_db_transaction_scope_snapshots_ignore_author_shadows_and_restore_on_abort() {
    assert_eq!(
        transaction_scope_probe("snapshots"),
        serde_json::json!({
            "accessor": true,
            "noOwnProperty": true,
            "freshLists": true,
            "before": [[107,101,112,116], [0xd800]],
            "after": [[99,114,101,97,116,101,100], [0xd800]],
            "oldSnapshot": [[107,101,112,116], [0xd800]],
            "rollback": [[107,101,112,116], [0xd800]],
            "authorShadowPreserved": true,
            "error": "AbortError"
        })
    );
}
