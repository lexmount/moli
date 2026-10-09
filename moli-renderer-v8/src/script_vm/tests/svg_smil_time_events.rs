use super::*;
use crate::network::ResourceRequestClient;

#[test]
fn native_time_events_preserve_brands_realms_and_legacy_argument_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-time-event.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    let context = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let event = crate::context_bootstrap::construct_svg_time_event(
            scope,
            moli_svg::SvgAnimationEventKind::Repeat(4294967295.0),
        )
        .unwrap();
        let global = scope.get_current_context().global(scope);
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeTimeEvent").into(),
                event.into()
            ),
            Some(true)
        );
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, event, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::TimeEvent::is_instance(
            scope,
            proxy.into()
        ));
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeTimeEventProxy").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(include_str!("svg_smil_time_event_brand.js"))
            .unwrap(),
        "time-event:ok"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_pending_seek_at_document_begin_does_not_replay_finished_intervals() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-pending-seek-events.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="first" begin="5s" dur="1s" repeatCount="2"/><set id="second" begin="9s" dur="11s"/></svg>'; globalThis.svg=document.querySelector('svg'); globalThis.events=[];
        for(const animation of document.querySelectorAll('set')) for(const type of ['beginEvent','repeatEvent','endEvent']) animation.addEventListener(type,event=>events.push(animation.id+':'+event.type));
        svg.pauseAnimations(); svg.setCurrentTime(10);"#).unwrap();
    assert_eq!(vm.eval("svg.getCurrentTime()").unwrap(), "0");
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "second:beginEvent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_seeking_events_are_deferred_and_do_not_replay_skipped_intervals() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-seeking-events.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="first" begin="5s" dur="1s" repeatCount="2"/><set id="second" begin="9s" dur="11s"/></svg>'; globalThis.svg=document.querySelector('svg'); globalThis.events=[];
        for(const animation of document.querySelectorAll('set')) for(const type of ['beginEvent','repeatEvent','endEvent']) animation.addEventListener(type,event=>events.push(animation.id+':'+event.type+':'+event.detail));"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.eval("svg.pauseAnimations(); svg.setCurrentTime(10)")
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "");
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "second:beginEvent:0");
    vm.eval("events.length=0; svg.setCurrentTime(5.5)").unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "second:endEvent:0|first:beginEvent:0"
    );
    vm.eval("events.length=0; svg.setCurrentTime(6.5)").unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_time_events_progress_without_author_timers_or_frame_callbacks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-autonomous-events.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="animation" begin="0s" dur="0.02s" repeatCount="2"/></svg>'; globalThis.events=[];
        for(const type of ['beginEvent','repeatEvent','endEvent']) animation.addEventListener(type,event=>events.push([event.type,event.detail,event.isTrusted,event instanceof TimeEvent,event.view===window].join(':')));"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "beginEvent:0:true:true:true|repeatEvent:1:true:true:true|endEvent:0:true:true:true"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_time_events_stop_when_a_listener_replaces_its_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-retired-events.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="first" begin="0s" dur="0.01s" repeatCount="2"/><set id="second" begin="0s" dur="0.01s"/></svg>'; globalThis.events=[];
        for(const animation of document.querySelectorAll('set')) for(const type of ['beginEvent','repeatEvent','endEvent']) animation.addEventListener(type,event=>events.push(animation.id+':'+event.type));
        first.onbegin=()=>{document.open();document.write('<!doctype html><body id="replacement"></body>');document.close();};"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "first:beginEvent");
    assert_eq!(vm.eval("document.body.id").unwrap(), "replacement");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_timing_events_skip_unobserved_repetitions_without_losing_the_end() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-unobserved-repeat-events.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="animation" begin="0s" dur="0.000000000001s" repeatCount="1000000000000000000" end="0.01s"/></svg>'; globalThis.events=[];
        animation.onbegin=()=>events.push('begin'); animation.onend=()=>events.push('end');"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "begin|end");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_begin_handler_microtasks_can_install_repeat_handlers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-repeat-handler-checkpoint.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="animation" begin="0s" dur="0.01s" repeatCount="3"/></svg>'; globalThis.events=[];
        animation.onbegin=()=>queueMicrotask(()=>{animation.onrepeat=event=>events.push(event.detail);}); animation.onend=()=>events.push('end');"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "1|2|end");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_boundaries_crossed_during_a_handler_still_schedule_a_rendering_wake() {
    fn advance_clock(
        _scope: &mut v8::PinScope<'_, '_>,
        args: v8::FunctionCallbackArguments<'_>,
        _rv: v8::ReturnValue<'_, v8::Value>,
    ) {
        let pointer = v8::Local::<v8::External>::try_from(args.data())
            .unwrap()
            .value()
            .cast::<crate::native_bridge::JsContextHost>();
        unsafe { &*pointer }.advance_svg_clocks_for_test(std::time::Duration::from_secs(4_000));
    }

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-crossed-boundaries.test/");
    let context = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, host_ptr| {
        let data = v8::External::new(scope, host_ptr.cast());
        let callback = v8::Function::builder(advance_clock)
            .data(data.into())
            .build(scope)
            .unwrap();
        let global = scope.get_current_context().global(scope);
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "advanceSvgClock").into(),
                callback.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"document.body.innerHTML='<svg><set id="animation" begin="0s" dur="1000s" repeatCount="3"/></svg>'; globalThis.events=[];
        animation.onbegin=()=>{events.push('begin'); advanceSvgClock();};
        animation.onrepeat=event=>events.push(event.detail); animation.onend=()=>events.push('end');"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "begin");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "begin|1|2|end");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_animation_removal_ends_the_interval_without_losing_queued_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-reinsert-events.test/");
    vm.eval(r#"document.body.innerHTML='<svg><set id="animation" begin="5s" dur="10s"/></svg>'; globalThis.svg=document.querySelector('svg'); globalThis.animation=document.querySelector('set'); globalThis.events=[];
        for(const type of ['beginEvent','repeatEvent','endEvent']) animation.addEventListener(type,event=>events.push(event.type));"#).unwrap();
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.eval("svg.pauseAnimations();svg.setCurrentTime(6);animation.remove()")
        .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "beginEvent|endEvent");
    vm.eval("svg.append(animation)").unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "beginEvent|endEvent|beginEvent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn child_svg_repeat_events_reach_the_owning_windows_capture_listener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-child-repeat-events.test/");
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    vm.eval(r#"globalThis.events=[]; globalThis.other=document.querySelector('iframe').contentWindow;
        other.addEventListener('repeatEvent',other.Function('event', 'parent.events.push([event.detail,event.view===window,event instanceof TimeEvent].join(":"))'),true);
        other.document.body.innerHTML='<svg><set id="animation" begin="0s" dur="0.02s" repeatCount="2"/></svg>';"#).unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "1:true:true");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn svg_timing_event_watchdog_bounds_listeners_and_their_microtasks() {
    use crate::v8_execution_watchdog::{V8ExecutionWatchdog, V8ExecutionWatchdogKind};
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for runaway in ["while(true) {}", "queueMicrotask(()=>{while(true) {}})"] {
        let mut vm = new_storage_page_task_executor_test_vm("https://svg-event-watchdog.test/");
        vm.eval(&format!(r#"document.body.innerHTML='<svg><set id="animation" begin="0s" dur="0.01s"/></svg>';globalThis.calls=0;animation.onbegin=()=>{{calls++;{runaway}}};"#)).unwrap();
        vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
            .unwrap();
        let _budget = V8ExecutionWatchdog::override_timeout_for_test(
            V8ExecutionWatchdogKind::SvgAnimationEvents,
            std::time::Duration::from_millis(100),
        );
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("[calls,6*7].join('|')").unwrap(),
            "1|42",
            "{runaway}"
        );
    }
}
