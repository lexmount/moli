use super::*;

fn request_state_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let source = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
    let count = crate::context_bootstrap::indexed_db_request_state_entries_for_test(scope, source);
    rv.set(v8::Number::new(scope, count as f64).into());
}

async fn assert_write_preparation(case: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-write-preparation.test/",
        &loader,
    );
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.with_default_context_scope(|scope, _| {
        let count = v8::Function::new(scope, request_state_entries).unwrap();
        let name = v8::String::new(scope, "__idbRequestStateEntries").unwrap();
        let global = scope.get_current_context().global(scope);
        global.set(scope, name.into(), count.into()).unwrap();
        Ok(())
    })
    .unwrap();
    let fixture = include_str!("idb_write_preparation.js");
    vm.eval(&format!(
        r#"
        globalThis.__idbWritePreparation = {{done: false}};
        (async () => {{
          for (const [index, realm] of [window, document.getElementById('child').contentWindow].entries()) {{
            await ({fixture})(realm, {case:?}, index, __idbRequestStateEntries);
          }}
        }})().then(
          () => globalThis.__idbWritePreparation = {{done: true, result: 'ok'}},
          error => globalThis.__idbWritePreparation = {{done: true, result: String(error), stack: error?.stack}}
        );
        "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__idbWritePreparation.done)",
        "true",
        case,
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__idbWritePreparation)").unwrap(),
        r#"{"done":true,"result":"ok"}"#,
        "{case}"
    );
}

#[tokio::test]
async fn queued_writes_validate_and_snapshot_before_transaction_start() {
    assert_write_preparation("queued").await;
}

#[tokio::test]
async fn rejected_writes_do_not_consume_generated_keys() {
    assert_write_preparation("generated-keys").await;
}

#[tokio::test]
async fn write_getters_run_once_and_propagate_exceptions_without_reentrancy() {
    assert_write_preparation("getters").await;
}

#[tokio::test]
async fn rejected_writes_allocate_no_native_request_state() {
    assert_write_preparation("native-state").await;
}

#[tokio::test]
async fn invalid_write_keys_preserve_chromium_clone_and_getter_error_order() {
    assert_write_preparation("key-errors").await;
}

#[tokio::test]
async fn prepared_writes_preserve_snapshots_generated_keys_and_index_constraints() {
    assert_write_preparation("execution-indexes").await;
}

#[tokio::test]
async fn multientry_deduplicates_equal_keys_within_each_record() {
    assert_write_preparation("multientry").await;
}

#[tokio::test]
async fn cursor_updates_validate_before_admission_and_preserve_snapshots() {
    assert_write_preparation("cursor-rejections").await;
}

#[tokio::test]
async fn cursor_updates_reject_readonly_before_cloning() {
    assert_write_preparation("cursor-readonly").await;
}
