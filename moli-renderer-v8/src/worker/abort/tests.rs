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

#[test]
fn subscription_worker_signals_release_native_entries_and_dependency_ids_after_gc() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut store = WorkerAbortStore::default();
    let context;
    let public_id;
    let retained_id;
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Context::new(scope, Default::default());
        context = v8::Global::new(scope, local);
        let scope = &mut v8::ContextScope::new(scope, local);
        let public = v8::Object::new(scope);
        public_id = store.init_signal(scope, public, false, None);
        let retained = v8::Object::new(scope);
        retained_id = store.init_signal(scope, retained, false, None);
        assert!(store.track_subscription_signal(scope, retained));
        let global = local.global(scope);
        assert_eq!(
            global.set(scope, v8str(scope, "retained").into(), retained.into()),
            Some(true)
        );
        for _ in 0..256 {
            let signal = v8::Object::new(scope);
            let id = store.init_signal(scope, signal, false, None);
            assert!(store.track_subscription_signal(scope, signal));
            assert_eq!(
                signal.set(scope, v8str(scope, "self").into(), signal.into()),
                Some(true)
            );
            store.set_signal_sources(id, [public_id]);
        }
    }
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    store.collect_subscription_signals();
    assert_eq!(store.signals.len(), 2);
    assert!(
        store
            .signal_state(public_id)
            .unwrap()
            .dependent_signals
            .is_empty()
    );
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let local = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, local);
        assert!(
            store.signal_object(scope, public_id).is_some(),
            "public source retention is unchanged"
        );
        assert!(
            store.signal_object(scope, retained_id).is_some(),
            "a live subscription still owns its signal"
        );
        let global = local.global(scope);
        assert_eq!(
            global.delete(scope, v8str(scope, "retained").into()),
            Some(true)
        );
    }
    isolate.clear_kept_objects();
    isolate.low_memory_notification();
    store.collect_subscription_signals();
    assert_eq!(store.signals.len(), 1);
}
