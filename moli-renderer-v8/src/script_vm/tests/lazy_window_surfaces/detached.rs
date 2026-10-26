use super::*;

#[test]
fn detached_non_lazy_window_slots_preserve_their_captured_fallbacks() {
    let mut vm = new_storage_html_test_vm("https://detached-window-slot-boundary.test/");
    vm.eval(
        r#"
        globalThis.frame = document.createElement('iframe');
        document.body.append(frame);
        globalThis.child = frame.contentWindow;
        globalThis.oldHistory = child.history;
        globalThis.oldNavigation = child.navigation;
        "#,
    )
    .unwrap();
    let context_id = materialize_single_child_default_realm_for_test(&mut vm, "cold IndexedDB");
    let context_ptr = vm
        .child_frame_realm_context_ptr_for_execution_context_id(context_id)
        .unwrap();
    let context = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            assert_eq!(
                crate::context_bootstrap::lazy_constructor_materialization_count(
                    scope,
                    "IDBFactory"
                ),
                0
            );
            Ok(v8::Global::new(scope, scope.get_current_context()))
        })
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
            frame.remove();
            JSON.stringify([
              child.indexedDB === undefined,
              child.indexedDB === undefined,
              child.history === oldHistory,
              child.navigation === oldNavigation
            ])
            "#,
        )
        .unwrap(),
        "[true,true,true,true]"
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(&context, |scope, _| {
        assert_eq!(
            crate::context_bootstrap::lazy_constructor_materialization_count(scope, "IDBFactory"),
            0,
            "an undefined captured IndexedDB fallback must not cold-start a retired runtime"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(constructor_materialization_count(&mut vm, "IDBFactory"), 0);
}
