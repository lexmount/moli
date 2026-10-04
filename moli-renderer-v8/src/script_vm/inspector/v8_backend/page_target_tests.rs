use super::*;
use std::{
    pin::pin,
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::Duration,
};

struct CloseProbe {
    started: mpsc::Sender<()>,
    processed: Arc<AtomicBool>,
}

struct ResumeFailureGuard(Option<RendererDevToolsTargetHandle>);

impl Drop for ResumeFailureGuard {
    fn drop(&mut self) {
        if let Some(target) = self.0.take() {
            target.close("test controller failed to resume the Page");
        }
    }
}

fn signal_started(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let probe = scope
        .get_current_context()
        .get_slot::<CloseProbe>()
        .unwrap();
    probe.started.send(()).unwrap();
}

fn close_processed(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let probe = scope
        .get_current_context()
        .get_slot::<CloseProbe>()
        .unwrap();
    rv.set_bool(probe.processed.load(Ordering::Acquire));
}

unsafe extern "C" fn acknowledge_close(_: v8::UnsafeRawIsolatePtr, data: *mut std::ffi::c_void) {
    // The accepted interrupt owns the one Arc transferred by the test thread.
    let processed = unsafe { Arc::from_raw(data.cast::<AtomicBool>()) };
    processed.store(true, Ordering::Release);
}

fn context_for_page(
    isolate: &mut v8::Isolate,
    backend: &RendererInspectorIsolateBackend,
    page: &super::super::DocumentInspectorBinding,
    probe: Option<CloseProbe>,
) -> v8::Global<v8::Context> {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    if let Some(probe) = probe {
        context.set_slot(Rc::new(probe));
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        let key = v8::String::new(scope, "started").unwrap();
        let callback = v8::Function::new(scope, signal_started).unwrap();
        global.set(scope, key.into(), callback.into()).unwrap();
        let key = v8::String::new(scope, "closeProcessed").unwrap();
        let callback = v8::Function::new(scope, close_processed).unwrap();
        global.set(scope, key.into(), callback.into()).unwrap();
    }
    backend.context_created_with_unique_id(
        context,
        page.agent.context_group_id(),
        b"page",
        b"https://example.test",
        b"{}",
    );
    v8::Global::new(scope, context)
}

fn evaluate(
    isolate: &mut v8::Isolate,
    context: &v8::Global<v8::Context>,
    source: &str,
) -> Option<String> {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let context = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let scope = pin!(v8::TryCatch::new(scope));
    let scope = &mut scope.init();
    let source = v8::String::new(scope, source).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    let result = crate::script_execution::execute_compiled_script(scope, script);
    if result.is_none() {
        assert!(
            scope.has_terminated(),
            "test evaluation failed without target-close termination"
        );
    }
    result.map(|value| value.to_rust_string_lossy(scope))
}

fn run_close_during_script(close_running_page: bool) {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let first_handle = backend.handle().new_page_handle(&mut isolate).unwrap();
    let second_handle = first_handle.new_page_handle(&mut isolate).unwrap();
    let first = super::super::DocumentInspectorBinding::new(first_handle);
    let second = super::super::DocumentInspectorBinding::new(second_handle);
    let (started_tx, started_rx) = mpsc::channel();
    let processed = Arc::new(AtomicBool::new(false));
    let first_context = context_for_page(&mut isolate, &backend, &first, None);
    let second_context = context_for_page(
        &mut isolate,
        &backend,
        &second,
        Some(CloseProbe {
            started: started_tx,
            processed: processed.clone(),
        }),
    );
    let closing_target = if close_running_page {
        second.devtools_target()
    } else {
        first.devtools_target()
    };
    let isolate_handle = isolate.thread_safe_handle();
    let closer = std::thread::spawn(move || {
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(closing_target.close("test closes one Page"));
        let data = Arc::into_raw(processed);
        if !isolate_handle.request_interrupt(acknowledge_close, data.cast_mut().cast()) {
            unsafe {
                drop(Arc::from_raw(data));
            }
            panic!("close acknowledgement interrupt rejected");
        }
    });
    let result = evaluate(
        &mut isolate,
        &second_context,
        if close_running_page {
            "started(); const deadline = Date.now() + 4000; while (Date.now() < deadline) {} 'survived'"
        } else {
            "started(); const deadline = Date.now() + 4000; while (!closeProcessed() && Date.now() < deadline) {} for (let i = 0; i < 10000; ++i) {} closeProcessed() ? 'survived' : 'timed-out'"
        },
    );
    closer.join().unwrap();
    if close_running_page {
        assert_eq!(
            result, None,
            "closing the entered Page must interrupt its script"
        );
    } else {
        assert_eq!(
            result.as_deref(),
            Some("survived"),
            "an unrelated Page close must preserve this script"
        );
    }
    finish_page_close_termination(&mut isolate);
    let survivor = if close_running_page {
        &first_context
    } else {
        &second_context
    };
    assert_eq!(
        evaluate(&mut isolate, survivor, "6 * 7").as_deref(),
        Some("42")
    );
    {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        backend.context_destroyed(v8::Local::new(scope, &first_context));
        backend.context_destroyed(v8::Local::new(scope, &second_context));
    }
}

#[test]
fn closing_related_page_preserves_running_peer() {
    run_close_during_script(false);
}

#[test]
fn closing_running_page_leaves_peer_isolate_usable() {
    run_close_during_script(true);
}

#[test]
fn closing_peer_preserves_debugger_pause_and_its_resume_route() {
    use crate::runtime::{
        PageId, RendererDevToolsIoCommandEnvelope, RendererInspectorCommandEnvelope,
        RendererInspectorCommandRoute, RendererInspectorIngressTicket,
        RendererOutputStreamIdentity, RendererOwnerLocalHostId, RendererTurnOutputJournal,
    };
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let first = super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    let second = super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    let context = context_for_page(&mut isolate, &backend, &second, None);
    let journal = RendererTurnOutputJournal::new(RendererOutputStreamIdentity::new_page(
        RendererOwnerLocalHostId::new_for_testing(1),
        PageId::new_for_testing(2),
        second.agent_token(),
    ));
    second
        .devtools_target()
        .pause_ref()
        .configure_page_route(journal.clone());
    second.agent.bind_output_journal(journal);
    with_scoped_inspector_microtasks(&mut isolate, || {
        second.with_session_and_outbound(
            &mut backend,
            super::super::PageInspectorSessionTarget::Frontend(None),
            |session, _, _| {
                session.dispatch_protocol_message(v8::inspector::StringView::from(
                    br#"{"id":1,"method":"Debugger.enable"}"#.as_slice(),
                ));
            },
        );
    });
    let first_target = first.devtools_target();
    let second_target = second.devtools_target();
    let agent = second.agent_token();
    let controller = std::thread::spawn(move || {
        let mut cleanup = ResumeFailureGuard(Some(second_target.clone()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !second_target.pause_ref().is_pause_active() {
            if std::time::Instant::now() >= deadline {
                second_target.close("test pause timed out");
                panic!("the second Page did not enter its own debugger pause");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!first_target.pause_ref().is_pause_active());
        first_target.close("test closes a peer while paused");
        assert!(second_target.pause_ref().is_pause_active());
        let route = second_target.io_ref().enqueue_command(
            agent,
            RendererDevToolsIoCommandEnvelope::inspector(RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io),
                r#"{"id":2,"method":"Debugger.resume"}"#.to_owned(),
                None,
            )),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), route.wait_for_first_dispatch())
                .await
                .expect("resume must reach the paused Page executor")
                .unwrap();
        });
        cleanup.0 = None;
    });
    assert_eq!(
        evaluate(&mut isolate, &context, "debugger; 6 * 7").as_deref(),
        Some("42")
    );
    controller.join().unwrap();
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    backend.context_destroyed(v8::Local::new(scope, &context));
}

#[tokio::test]
async fn owner_shutdown_seals_all_page_queues_and_rejects_new_page() {
    use crate::{
        devtools::{
            ingress::io::RendererRuntimeInspectorIoCommandClaim,
            target::RendererDevToolsTargetShutdownRegistry,
        },
        runtime::{
            RendererDevToolsIoCommandEnvelope, RendererInspectorCommandRoute,
            RendererInspectorIngressTicket,
        },
    };
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let registry = RendererDevToolsTargetShutdownRegistry::default();
    let backend = RendererInspectorIsolateBackend::new(&mut isolate)
        .with_shutdown_registry(Some(registry.clone()));
    let first = backend.handle().new_page_handle(&mut isolate).unwrap();
    let second = first.new_page_handle(&mut isolate).unwrap();
    let enqueue = |page: &RendererInspectorIsolateBackendHandle| {
        page.devtools_target().io_ref().enqueue_command(
            RendererDevToolsAgentToken::allocate(),
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io),
            ),
        )
    };
    let first_pending = enqueue(&first);
    let second_pending = enqueue(&second);
    registry.terminate_all();
    for pending in [first_pending, second_pending, enqueue(&second)] {
        assert!(matches!(
            pending.wait_for_first_dispatch().await.unwrap(),
            RendererRuntimeInspectorIoCommandClaim::Canceled(_)
        ));
    }
    assert!(first.new_page_handle(&mut isolate).is_err());
    // Drain accepted close interrupts in an unrelated native test context.
    // These callbacks may not revive a rejected endpoint or stop a peer.
    let context = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        v8::Global::new(scope, context)
    };
    assert_eq!(
        evaluate(&mut isolate, &context, "6 * 7").as_deref(),
        Some("42")
    );
}
