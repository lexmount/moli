use super::*;

#[test]
fn completed_worker_abort_state_preserves_live_reasons_without_rooting_cycles() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let mut store = WorkerAbortStore::default();
    let context;
    let mut ids = [0; 2];
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Context::new(scope, Default::default());
        context = v8::Global::new(scope, local);
        let scope = &mut v8::ContextScope::new(scope, local);
        let global = local.global(scope);
        for (index, retained) in [false, true].into_iter().enumerate() {
            let signal = v8::Object::new(scope);
            let reason = v8::Object::new(scope);
            let key = v8str(scope, "signal");
            assert_eq!(reason.set(scope, key.into(), signal.into()), Some(true));
            let id = store.init_signal(scope, signal, true, Some(reason.into()));
            let key = v8str(scope, "signal");
            assert_eq!(global.set(scope, key.into(), signal.into()), Some(true));
            let source = v8str(scope, "new WeakRef(signal)");
            let script = v8::Script::compile(scope, source, None).unwrap();
            let weak = crate::script_execution::execute_compiled_script(scope, script).unwrap();
            let key = v8str(
                scope,
                if retained {
                    "retainedWeak"
                } else {
                    "abandonedWeak"
                },
            );
            assert_eq!(global.set(scope, key.into(), weak), Some(true));
            ids[index] = id;
        }
    }
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let local = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, local);
    assert!(store.signal_object(scope, ids[0]).is_none());
    let signal = store.signal_object(scope, ids[1]).unwrap();
    let reason = store.signal_reason(scope, signal).unwrap();
    let reason = v8::Local::<v8::Object>::try_from(reason).unwrap();
    let key = v8str(scope, "signal");
    assert!(
        reason
            .get(scope, key.into())
            .unwrap()
            .strict_equals(signal.into())
    );
    let source = v8str(
        scope,
        "abandonedWeak.deref() === undefined && retainedWeak.deref() === signal",
    );
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(
        crate::script_execution::execute_compiled_script(scope, script)
            .unwrap()
            .is_true()
    );
}
