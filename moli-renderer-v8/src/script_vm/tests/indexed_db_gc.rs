use super::*;

const PROBE: &str = include_str!("indexed_db_cursor_gc.js");

#[test]
fn indexed_db_completed_cursor_graphs_collect_and_retained_cursors_survive_gc() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-cursor-gc.test/");
    vm.eval(&format!("{PROBE}\nprepareIndexedDbCursorGc().then(value => globalThis.cursorGcReady=value, error => globalThis.cursorGcReady=String(error)); 'scheduled'"))
        .unwrap();
    // There are no author references to the active open request. The selected
    // task itself must retain it until its events and Promise reactions finish.
    let collect = |vm: &mut crate::runtime::PageVmTaskExecutorTestHarness| {
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
    };
    collect(&mut vm);
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(cursorGcReady)")
            .unwrap(),
        "prepared"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("checkIndexedDbCursorGc()").unwrap(), "true");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert_eq!(
            crate::context_bootstrap::indexed_db_native_cursor_count_for_test(scope),
            1
        );
        Ok(())
    })
    .unwrap();
    vm.eval("delete globalThis.cursorGcHeld").unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("checkReleasedIndexedDbCursorGc()").unwrap(), "true");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert_eq!(
            crate::context_bootstrap::indexed_db_native_cursor_count_for_test(scope),
            0
        );
        Ok(())
    })
    .unwrap();
}
