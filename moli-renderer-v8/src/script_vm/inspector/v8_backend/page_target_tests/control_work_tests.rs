use super::*;
use crate::runtime::{
    PageId, RendererDevToolsIoCommandEnvelope, RendererInspectorCommandEnvelope,
    RendererInspectorCommandRoute, RendererInspectorIngressTicket, RendererOutputStreamIdentity,
    RendererOwnerLocalHostId, RendererTurnOutputJournal,
};

fn attach_frontends(
    backend: &mut RendererInspectorIsolateBackend,
    page: &super::super::super::DocumentInspectorBinding,
    page_id: u64,
) -> RendererTurnOutputJournal {
    let journal = RendererTurnOutputJournal::new(RendererOutputStreamIdentity::new_page(
        RendererOwnerLocalHostId::new_for_testing(1),
        PageId::new_for_testing(page_id),
        page.agent_token(),
    ));
    page.devtools_target()
        .pause_ref()
        .configure_page_route(journal.clone());
    page.agent.bind_output_journal(journal.clone());
    for session_id in [None, Some("second")] {
        page.with_session_and_outbound(
            backend,
            super::super::super::PageInspectorSessionTarget::Frontend(session_id),
            |session, _, _| {
                session.dispatch_protocol_message(v8::inspector::StringView::from(
                    br#"{"id":1,"method":"Debugger.enable"}"#.as_slice(),
                ));
            },
        );
    }
    journal
}

fn pause_events(
    rx: &mut crate::runtime::RendererOutputTransportReceiver,
) -> HashMap<DevToolsSessionKey, Vec<String>> {
    let mut events = HashMap::<_, Vec<_>>::new();
    while let Ok(message) = rx.try_recv() {
        let crate::runtime::RendererOutputTransportMessage::Publication(publication) = message
        else {
            continue;
        };
        for record in publication.records() {
            let crate::runtime::RendererOutputItem::Observation(
                crate::runtime::RendererProtocolObservation::RuntimeInspector(batch),
            ) = record.item()
            else {
                continue;
            };
            for message in &batch.messages {
                let value = message.clone().into_v8_inspector_message();
                if let Some(method @ ("Debugger.paused" | "Debugger.resumed")) =
                    value.get("method").and_then(serde_json::Value::as_str)
                {
                    events
                        .entry(batch.session.clone())
                        .or_default()
                        .push(method.to_owned());
                }
            }
        }
    }
    events
}

#[test]
fn canceled_navigation_unpauses_real_v8_once_per_frontend_and_rejects_stale_tasks() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let handle = backend.handle().new_page_handle(&mut isolate).unwrap();
    let old = super::super::super::DocumentInspectorBinding::new(handle.clone());
    let new = super::super::super::DocumentInspectorBinding::new(handle);
    let peer = super::super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    let old_context = context_for_page(&mut isolate, &backend, &old, None);
    let new_context = context_for_page(&mut isolate, &backend, &new, None);
    let peer_context = context_for_page(&mut isolate, &backend, &peer, None);
    let journal = attach_frontends(&mut backend, &old, 1);
    attach_frontends(&mut backend, &new, 1);
    attach_frontends(&mut backend, &peer, 2);
    let (transport, mut rx) = crate::runtime::renderer_output_transport_channel();
    journal.bind_transport(transport);
    let target = old.devtools_target();
    let agent = old.agent_token();
    let controller_target = target.clone();
    let controller = std::thread::spawn(move || {
        let mut cleanup = ResumeFailureGuard(Some(controller_target.clone()));
        wait_for_pause(&controller_target);
        // Delay publication at the producer boundary until both overlapping
        // scopes have ended, deterministically exercising begin/cancel before
        // the owner can claim the task against a genuinely paused V8 isolate.
        let begin = controller_target
            .pause_ref()
            .begin_document_replacement(agent)
            .unwrap();
        assert!(begin.first);
        let pause = begin.pause.expect("capture the reported pause");
        assert!(
            !controller_target
                .pause_ref()
                .begin_document_replacement(agent)
                .unwrap()
                .first
        );
        assert!(
            !controller_target
                .pause_ref()
                .finish_document_replacement(agent)
        );
        assert!(
            controller_target
                .pause_ref()
                .finish_document_replacement(agent)
        );
        assert!(
            !controller_target
                .pause_ref()
                .document_replacement_active(agent)
        );
        controller_target.io_ref().enqueue_control_work(
            InspectorPageControlWork::UnpauseAndTerminateOldDocument { pause },
        );
        wait_for_unpause(&controller_target);
        cleanup.0 = None;
        pause
    });
    assert_eq!(
        evaluate(
            &mut isolate,
            &old_context,
            "debugger; globalThis.afterNavigationPause = true; 42"
        ),
        None
    );
    let retired = controller.join().unwrap();
    let events = pause_events(&mut rx);
    for session in [
        DevToolsSessionKey::Primary,
        DevToolsSessionKey::Attached("second".to_owned()),
    ] {
        assert_eq!(
            events.get(&session).unwrap(),
            &["Debugger.paused", "Debugger.resumed"]
        );
    }
    assert_native_policy(&mut backend, &old, false);
    assert_eq!(
        evaluate(&mut isolate, &old_context, "typeof afterNavigationPause").as_deref(),
        Some("undefined")
    );
    // The delayed edge may be republished after acknowledgement. Deliver it
    // while each later real pause is active, including the same Agent, a new
    // Agent sharing its Page, and another Page in this isolate.
    for (page, context) in [
        (&old, &old_context),
        (&new, &new_context),
        (&peer, &peer_context),
    ] {
        let paused_target = page.devtools_target();
        let paused_agent = page.agent_token();
        let producer = target.clone();
        let controller = std::thread::spawn(move || {
            let mut cleanup = ResumeFailureGuard(Some(paused_target.clone()));
            wait_for_pause(&paused_target);
            assert!(producer.io_ref().enqueue_control_work(
                InspectorPageControlWork::UnpauseAndTerminateOldDocument { pause: retired },
            ));
            let route = resume_route(&paused_target, paused_agent);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(5), route.wait_for_first_dispatch())
                    .await
                    .unwrap()
                    .unwrap();
            });
            cleanup.0 = None;
        });
        assert_eq!(
            evaluate(&mut isolate, context, "debugger; 42").as_deref(),
            Some("42")
        );
        controller.join().unwrap();
    }
    assert_native_policy(&mut backend, &old, false);
    assert_native_policy(&mut backend, &new, false);
    assert_native_policy(&mut backend, &peer, false);
}

#[test]
fn owner_and_interrupt_unpause_tasks_preserve_entering_quit_and_reject_later_generations() {
    for owner in [true, false] {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let page = super::super::super::DocumentInspectorBinding::new(
            backend.handle().new_page_handle(&mut isolate).unwrap(),
        );
        let context = context_for_page(&mut isolate, &backend, &page, None);
        attach_frontends(&mut backend, &page, 1);
        let target = page.devtools_target();
        let agent = page.agent_token();
        let route = target.outbound_route(agent, DevToolsSessionKey::Primary);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        target.io_ref().configure_owner_wake(wake_tx);
        route.route_notification(&serde_json::json!({"method":"Debugger.paused","params":{}}));
        let begin = target
            .pause_ref()
            .begin_document_replacement(agent)
            .unwrap();
        let retired = begin.pause.unwrap();
        assert!(target.pause_ref().finish_document_replacement(agent));
        assert!(target.io_ref().enqueue_control_work(
            InspectorPageControlWork::UnpauseAndTerminateOldDocument { pause: retired },
        ));
        if owner {
            dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
        } else {
            assert_eq!(
                evaluate(&mut isolate, &context, "42").as_deref(),
                Some("42")
            );
        }
        assert!(target.pause_ref().enter_pause().is_some());
        assert_eq!(
            target.pause_ref().wait_for_pause_work(|_| Some(())),
            Err(crate::devtools::pause::RendererInspectorPauseExitReason::Resumed)
        );
        target.pause_ref().leave_pause();
        route.route_notification(&serde_json::json!({"method":"Debugger.resumed","params":{}}));
        // Rearm both real wake mechanisms before reporting a later pause.
        while let Ok(wake) = wake_rx.try_recv() {
            dispatch_inspector_io_owner_wake(wake);
        }
        route.route_notification(&serde_json::json!({"method":"Debugger.paused","params":{}}));
        assert!(target.io_ref().enqueue_control_work(
            InspectorPageControlWork::UnpauseAndTerminateOldDocument { pause: retired },
        ));
        if owner {
            dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
        } else {
            assert_eq!(
                evaluate(&mut isolate, &context, "42").as_deref(),
                Some("42")
            );
        }
        assert!(target.pause_ref().enter_pause().is_some());
        assert_eq!(target.pause_ref().wait_for_pause_work(|_| Some(42)), Ok(42));
        target.pause_ref().leave_pause();
        assert_native_policy(&mut backend, &page, false);
    }
}

#[test]
fn nested_unpause_control_precedes_pause_bridge_detach_in_both_loop_modes() {
    for reason in ["other", "instrumentation"] {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let page = super::super::super::DocumentInspectorBinding::new(
            backend.handle().new_page_handle(&mut isolate).unwrap(),
        );
        attach_frontends(&mut backend, &page, 1);
        let handle = page.isolate_backend_handle();
        let executor = handle.endpoint.session_executor.as_ref().unwrap();
        let target = page.devtools_target();
        let agent = page.agent_token();
        target
            .outbound_route(agent, DevToolsSessionKey::Primary)
            .route_notification(
                &serde_json::json!({"method":"Debugger.paused","params":{"reason":reason}}),
            );
        assert!(target.begin_document_replacement(agent));
        target.pause_ref().arm_session_detach();
        target
            .io_ref()
            .begin_session_detach(agent, &DevToolsSessionKey::Primary);
        executor.run_message_loop_on_pause(page.agent.context_group_id().get());
        assert!(
            target.io_ref().claim_control_work().is_none(),
            "detach must not bypass the queued unpause task"
        );
        assert_native_policy(&mut backend, &page, true);
        target
            .io_ref()
            .finish_session_detach(agent, &DevToolsSessionKey::Primary);
        target.pause_ref().disarm_session_detach();
        target.finish_document_replacement(agent);
        executor.dispatch_control_work(&mut isolate);
        assert_native_policy(&mut backend, &page, false);
    }
}

#[test]
fn navigation_unpause_acknowledges_each_real_instrumentation_frontend() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page = super::super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    let context = context_for_page(&mut isolate, &backend, &page, None);
    let journal = attach_frontends(&mut backend, &page, 1);
    for session_id in [None, Some("second")] {
        page.with_session_and_outbound(
            &mut backend,
            super::super::super::PageInspectorSessionTarget::Frontend(session_id),
            |session, _, _| {
                session.dispatch_protocol_message(v8::inspector::StringView::from(
                    br#"{"id":3,"method":"Debugger.setInstrumentationBreakpoint","params":{"instrumentation":"beforeScriptExecution"}}"#.as_slice(),
                ));
            },
        );
    }
    let (transport, mut rx) = crate::runtime::renderer_output_transport_channel();
    journal.bind_transport(transport);
    let target = page.devtools_target();
    let agent = page.agent_token();
    let controller = std::thread::spawn(move || {
        let mut cleanup = ResumeFailureGuard(Some(target.clone()));
        wait_for_pause(&target);
        assert!(target.begin_document_replacement(agent));
        target.finish_document_replacement(agent);
        wait_for_unpause(&target);
        cleanup.0 = None;
    });
    // V8's instrumentation branch acknowledges every frontend before quitting
    // and ignores terminateOnResume; preserve those native semantics.
    assert_eq!(
        evaluate(&mut isolate, &context, "42").as_deref(),
        Some("42")
    );
    controller.join().unwrap();
    let events = pause_events(&mut rx);
    for session in [
        DevToolsSessionKey::Primary,
        DevToolsSessionKey::Attached("second".to_owned()),
    ] {
        assert_eq!(
            events.get(&session).unwrap(),
            &["Debugger.paused", "Debugger.resumed"]
        );
    }
    assert!(!page.devtools_target().pause_ref().is_pause_active());
}

fn native_skip(session: &super::super::super::RendererInspectorSession) -> bool {
    // Bypass the wrapper's clean snapshot and lazy policy synchronization.
    let state = v8::inspector::V8InspectorSession::state(session);
    let state: serde_json::Value =
        serde_json::from_slice(&v8::crdtp::cbor_to_json(&state).unwrap()).unwrap();
    // V8 omits the default false setting until it has been explicitly written.
    state["Debugger"]
        .get("skipAllPauses")
        .is_some_and(|value| value.as_bool().expect("V8 pause policy must be boolean"))
}

fn assert_native_policy(
    backend: &mut RendererInspectorIsolateBackend,
    page: &super::super::super::DocumentInspectorBinding,
    skip: bool,
) {
    for session_id in [None, Some("second")] {
        page.with_session_and_outbound(
            backend,
            super::super::super::PageInspectorSessionTarget::Frontend(session_id),
            |session, _, _| assert_eq!(native_skip(session), skip),
        );
    }
}

fn wait_for_pause(target: &RendererDevToolsTargetHandle) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !target.pause_ref().is_pause_active() {
        assert!(std::time::Instant::now() < deadline, "Page did not pause");
        std::thread::yield_now();
    }
}

fn wait_for_unpause(target: &RendererDevToolsTargetHandle) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while target.pause_ref().is_pause_active() {
        assert!(std::time::Instant::now() < deadline, "Page did not unpause");
        std::thread::yield_now();
    }
}

fn resume_route(
    target: &RendererDevToolsTargetHandle,
    agent: RendererDevToolsAgentToken,
) -> crate::devtools::ingress::io::RendererRuntimeInspectorIoCommandRoute {
    target.io_ref().enqueue_command(
        agent,
        RendererDevToolsIoCommandEnvelope::inspector(RendererInspectorCommandEnvelope::new_io(
            RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io),
            r#"{"id":2,"method":"Debugger.resume"}"#.to_owned(),
            None,
        )),
    )
}

fn assert_real_pause(
    isolate: &mut v8::Isolate,
    context: &v8::Global<v8::Context>,
    target: RendererDevToolsTargetHandle,
    agent: RendererDevToolsAgentToken,
) {
    let controller = std::thread::spawn(move || {
        let mut cleanup = ResumeFailureGuard(Some(target.clone()));
        wait_for_pause(&target);
        let route = resume_route(&target, agent);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), route.wait_for_first_dispatch())
                .await
                .unwrap()
                .unwrap();
        });
        cleanup.0 = None;
    });
    assert_eq!(
        evaluate(isolate, context, "debugger; 6 * 7").as_deref(),
        Some("42")
    );
    controller.join().unwrap();
}

#[test]
fn replacement_control_preserves_peer_pages_and_new_agents_under_stress() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page_handle = backend.handle().new_page_handle(&mut isolate).unwrap();
    let old = super::super::super::DocumentInspectorBinding::new(page_handle.clone());
    let new = super::super::super::DocumentInspectorBinding::new(page_handle);
    let peer = super::super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    let old_context = context_for_page(&mut isolate, &backend, &old, None);
    let new_context = context_for_page(&mut isolate, &backend, &new, None);
    let peer_context = context_for_page(&mut isolate, &backend, &peer, None);
    for (page, id) in [(&old, 1), (&new, 1), (&peer, 2)] {
        attach_frontends(&mut backend, page, id);
    }
    let target = old.devtools_target();
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    target.io_ref().configure_owner_wake(wake_tx);
    for _ in 0..16 {
        let publisher = target.clone();
        let agent = old.agent_token();
        std::thread::spawn(move || assert!(publisher.begin_document_replacement(agent)))
            .join()
            .unwrap();
        dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
        assert_native_policy(&mut backend, &old, true);
        assert_native_policy(&mut backend, &new, false);
        assert_native_policy(&mut backend, &peer, false);
        assert_eq!(
            evaluate(
                &mut isolate,
                &old_context,
                "for (let i=0;i<4;++i) { debugger; } 42"
            )
            .as_deref(),
            Some("42")
        );
        for (page, context) in [(&new, &new_context), (&peer, &peer_context)] {
            assert_real_pause(
                &mut isolate,
                context,
                page.devtools_target(),
                page.agent_token(),
            );
        }
        // Protocol dispatch may leave harmless owner wakes in the channel.
        while let Ok(wake) = wake_rx.try_recv() {
            dispatch_inspector_io_owner_wake(wake);
        }
        let publisher = target.clone();
        std::thread::spawn(move || publisher.finish_document_replacement(agent))
            .join()
            .unwrap();
        dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
        assert_native_policy(&mut backend, &old, false);
        assert_native_policy(&mut backend, &new, false);
        assert_native_policy(&mut backend, &peer, false);
    }
}

struct ReplacementProbe(Weak<super::super::super::RendererInspectorSession>);

fn replacement_applied(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let context = scope.get_current_context();
    let probe = context.get_slot::<ReplacementProbe>().unwrap();
    rv.set_bool(native_skip(&probe.0.upgrade().unwrap()));
}

fn interrupt_control_delivery_samples(rounds: usize) -> Vec<u128> {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page = super::super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    attach_frontends(&mut backend, &page, 1);
    let (started_tx, mut started_rx) = mpsc::channel();
    let context = context_for_page(
        &mut isolate,
        &backend,
        &page,
        Some(CloseProbe {
            started: started_tx,
            processed: Arc::new(AtomicBool::new(false)),
        }),
    );
    let target = page.devtools_target();
    let agent = page.agent_token();
    let executor = backend
        .page_targets
        .executor_for_group(page.agent.context_group_id().get())
        .unwrap();
    let session = executor
        .interrupt_sessions
        .borrow()
        .get(&(agent, DevToolsSessionKey::Primary))
        .unwrap()
        .session
        .clone();
    {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &context);
        context.set_slot(Rc::new(ReplacementProbe(session)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let name = v8::String::new(scope, "replacementApplied").unwrap();
        let callback = v8::Function::new(scope, replacement_applied).unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into())
            .unwrap();
    }
    let mut samples = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        std::thread::scope(|threads| {
            let started_rx = &mut started_rx;
            let target = &target;
            let controller = threads.spawn(move || {
                started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let published = std::time::Instant::now();
                assert!(target.begin_document_replacement(agent));
                published
            });
            assert_eq!(evaluate(&mut isolate, &context,
                "(() => { started(); const deadline = Date.now()+4000; while (!replacementApplied() && Date.now()<deadline) {} return replacementApplied(); })()"
            ).as_deref(), Some("true"));
            samples.push(controller.join().unwrap().elapsed().as_nanos());
        });
        assert_native_policy(&mut backend, &page, true);
        target.finish_document_replacement(agent);
        executor.dispatch_control_work(&mut isolate);
        assert_native_policy(&mut backend, &page, false);
    }
    samples
}

#[test]
fn long_running_javascript_services_replacement_control_via_interrupt() {
    interrupt_control_delivery_samples(1);
}

#[test]
fn shared_isolate_environment_is_claimed_once_before_page_reconciliation() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let first = backend.handle().new_page_handle(&mut isolate).unwrap();
    let second = backend.handle().new_page_handle(&mut isolate).unwrap();
    let first_executor = first.endpoint.session_executor.as_ref().unwrap();
    let second_executor = second.endpoint.session_executor.as_ref().unwrap();
    let agent = RendererDevToolsAgentToken::allocate();
    first
        .devtools_target()
        .io_ref()
        .enqueue_control_work(InspectorPageControlWork::ReconcileDocumentReplacement { agent });
    let notifier = first_executor.environment_ingress().environment_notifier();
    for _ in 0..10_000 {
        notifier.notify(moli_v8_platform::ProcessEnvironmentInvalidation::DateTime);
        notifier.notify(moli_v8_platform::ProcessEnvironmentInvalidation::LocaleAndDateTime);
    }
    let batch = first_executor.claim_control_work().unwrap();
    assert!(matches!(&batch, [
        Some(InspectorControlWork::EnvironmentInvalidation(moli_v8_platform::ProcessEnvironmentInvalidation::LocaleAndDateTime)),
        Some(InspectorControlWork::Page(InspectorPageControlWork::ReconcileDocumentReplacement { agent: claimed })),
    ] if *claimed == agent));
    first_executor.apply_control_work(batch, &mut isolate);
    assert!(second_executor.claim_control_work().is_none());
    assert!(first_executor.claim_control_work().is_none());
}

#[test]
fn owner_control_wakes_coalesce_and_rearm_for_remaining_agents() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page = backend.handle().new_page_handle(&mut isolate).unwrap();
    let target = page.devtools_target();
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    target.io_ref().configure_owner_wake(wake_tx);
    let first = RendererDevToolsAgentToken::allocate();
    let second = RendererDevToolsAgentToken::allocate();
    assert!(target.io_ref().enqueue_control_work(
        InspectorPageControlWork::ReconcileDocumentReplacement { agent: first }
    ));
    for _ in 0..10_000 {
        assert!(!target.io_ref().enqueue_control_work(
            InspectorPageControlWork::ReconcileDocumentReplacement { agent: first }
        ));
    }
    assert!(target.io_ref().enqueue_control_work(
        InspectorPageControlWork::ReconcileDocumentReplacement { agent: second }
    ));
    let wake = wake_rx.try_recv().unwrap();
    assert!(
        wake_rx.try_recv().is_err(),
        "one armed wake covers both Agents"
    );
    dispatch_inspector_io_owner_wake(wake);
    dispatch_inspector_io_owner_wake(wake_rx.try_recv().expect("remaining Agent must rearm"));
    assert!(wake_rx.try_recv().is_err());
    assert!(target.io_ref().claim_control_work().is_none());
    assert!(target.io_ref().enqueue_control_work(
        InspectorPageControlWork::ReconcileDocumentReplacement { agent: first }
    ));
    dispatch_inspector_io_owner_wake(
        wake_rx
            .try_recv()
            .expect("publication after claim must wake"),
    );
    assert!(wake_rx.try_recv().is_err());
}

fn io_only_control_delivery_sample() -> u128 {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let handle = backend.handle().new_page_handle(&mut isolate).unwrap();
    let old = super::super::super::DocumentInspectorBinding::new(handle.clone());
    let new = super::super::super::DocumentInspectorBinding::new(handle.clone());
    attach_frontends(&mut backend, &old, 1);
    attach_frontends(&mut backend, &new, 1);
    let executor = handle.endpoint.session_executor.as_ref().unwrap();
    let target = old.devtools_target();
    let agent = old.agent_token();
    assert!(target.begin_document_replacement(agent));
    executor.dispatch_control_work(&mut isolate);
    assert_native_policy(&mut backend, &old, true);
    let route = resume_route(&target, new.agent_token());
    let mut command = target.io_ref().claim_for_owner().unwrap();
    let mut first_dispatch = target.io_ref().first_dispatch_guard(&mut command);
    target
        .io_ref()
        .begin_session_detach(new.agent_token(), &DevToolsSessionKey::Primary);
    target
        .outbound_route(new.agent_token(), DevToolsSessionKey::Primary)
        .route_notification(&serde_json::json!({
            "method": "Debugger.paused", "params": {"reason": "instrumentation"}
        }));
    let notifier = executor.environment_ingress().environment_notifier();
    notifier.notify(moli_v8_platform::ProcessEnvironmentInvalidation::LocaleAndDateTime);
    let controller_target = target.clone();
    let controller = std::thread::spawn(move || {
        let mut cleanup = ResumeFailureGuard(Some(controller_target.clone()));
        let published = std::time::Instant::now();
        controller_target.finish_document_replacement(agent);
        let deadline = published + Duration::from_secs(5);
        // Insertion can succeed again only after the nested loop has removed
        // the original key. Re-publish while apply may still be in progress.
        while !controller_target
            .io_ref()
            .enqueue_control_work(InspectorPageControlWork::ReconcileDocumentReplacement { agent })
        {
            assert!(
                std::time::Instant::now() < deadline,
                "IoOnly control was blocked"
            );
            std::thread::yield_now();
        }
        controller_target.pause_ref().request_quit();
        cleanup.0 = None;
        published
    });
    executor.run_message_loop_on_pause(new.agent.context_group_id().get());
    let elapsed = controller.join().unwrap().elapsed().as_nanos();
    assert_native_policy(&mut backend, &old, false);
    assert_native_policy(&mut backend, &new, false);
    assert!(
        executor
            .environment_ingress()
            .claim_environment_invalidation()
            .is_none()
    );
    use std::future::Future;
    let mut wait = pin!(route.wait_for_first_dispatch());
    let mut task_context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(wait.as_mut().poll(&mut task_context).is_pending());
    first_dispatch.release();
    assert!(matches!(
        wait.as_mut().poll(&mut task_context),
        std::task::Poll::Ready(Ok(
            crate::devtools::ingress::io::RendererRuntimeInspectorIoCommandClaim::Dispatched
        ))
    ));
    target
        .io_ref()
        .finish_session_detach(new.agent_token(), &DevToolsSessionKey::Primary);
    executor.dispatch_control_work(&mut isolate);
    elapsed
}

#[test]
fn io_only_pause_services_control_while_protocol_first_dispatch_and_detach_are_blocked() {
    io_only_control_delivery_sample();
}

#[test]
fn late_control_and_pause_callbacks_after_page_teardown_are_harmless() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page = backend.handle().new_page_handle(&mut isolate).unwrap();
    let target = page.devtools_target();
    let agent = RendererDevToolsAgentToken::allocate();
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    target.io_ref().configure_owner_wake(wake_tx);
    assert!(target.begin_document_replacement(agent));
    let wake = wake_rx.try_recv().unwrap();
    drop(page);
    target.finish_document_replacement(agent);
    dispatch_inspector_io_owner_wake(wake);
    assert!(target.io_ref().claim_control_work().is_none());
    assert!(!target.begin_document_replacement(agent));
    // V8 can still deliver the callback-owned Arc after the TLS route is gone.
    let live = super::super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    let context = context_for_page(&mut isolate, &backend, &live, None);
    assert_eq!(
        evaluate(&mut isolate, &context, "42").as_deref(),
        Some("42")
    );
    // Simulate a notification whose group lost its session before V8 enters
    // the client loop. It must reset the pause phase without an expect panic.
    let live_handle = live.isolate_backend_handle();
    let executor = live_handle.endpoint.session_executor.as_ref().unwrap();
    attach_frontends(&mut backend, &live, 2);
    let route = live
        .devtools_target()
        .outbound_route(live.agent_token(), DevToolsSessionKey::Primary);
    route.route_notification(&serde_json::json!({"method":"Debugger.paused","params":{}}));
    executor.run_message_loop_on_pause(i32::MAX);
    assert!(!live.devtools_target().pause_ref().is_pause_active());
}

#[test]
#[ignore = "manual Inspector control delivery latency measurement"]
fn inspector_control_delivery_latency() {
    fn report(label: &str, mut samples: Vec<u128>) {
        samples.sort_unstable();
        let n = samples.len();
        eprintln!(
            "inspector_control_delivery_latency {label} n={n} p50={}ns p95={}ns p99={}ns",
            samples[n / 2],
            samples[n * 95 / 100],
            samples[n * 99 / 100]
        );
    }

    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let page = super::super::super::DocumentInspectorBinding::new(
        backend.handle().new_page_handle(&mut isolate).unwrap(),
    );
    attach_frontends(&mut backend, &page, 1);
    let target = page.devtools_target();
    let agent = page.agent_token();
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    target.io_ref().configure_owner_wake(wake_tx);
    let mut idle = Vec::with_capacity(2_000);
    for round in 0..2_100 {
        let published = std::time::Instant::now();
        assert!(target.begin_document_replacement(agent));
        dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
        if round >= 100 {
            idle.push(published.elapsed().as_nanos());
        }
        target.finish_document_replacement(agent);
        dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
    }
    report("idle-two-sessions", idle);
    for _ in 0..10_000 {
        assert!(target.begin_document_replacement(agent));
    }
    let wake = wake_rx.try_recv().unwrap();
    assert!(wake_rx.try_recv().is_err());
    dispatch_inspector_io_owner_wake(wake);
    for _ in 0..9_999 {
        target.finish_document_replacement(agent);
    }
    assert!(
        wake_rx.try_recv().is_err(),
        "overlapping finishes must not wake"
    );
    target.finish_document_replacement(agent);
    dispatch_inspector_io_owner_wake(wake_rx.try_recv().unwrap());
    assert!(wake_rx.try_recv().is_err());
    assert_native_policy(&mut backend, &page, false);
    eprintln!("inspector_control_burst scopes=10000 owner_wakes=2 sessions_per_agent=2");
    // Separate isolates avoid observing an unrelated fixture's pending work.
    drop(page);
    drop(backend);
    drop(isolate);
    report(
        "active-js-two-sessions",
        interrupt_control_delivery_samples(100),
    );
    report(
        "io-only-pause-blocked-lanes",
        (0..100)
            .map(|_| io_only_control_delivery_sample())
            .collect(),
    );
}
