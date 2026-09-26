use super::*;

#[test]
fn observer_element_arguments_use_native_interface_identity() {
    let mut vm = new_parsed_test_vm(
        "https://observer-element-arguments.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(include_str!(
            "../../../tests/fixtures/observer-element-arguments.js"
        ))
        .unwrap();
    assert_eq!(result, r#"{"total":204,"failures":[]}"#);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "__observerNativeElement").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        assert!(
            value.is_proxy(),
            "fixture must cover a registered native Proxy"
        );
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        assert!(crate::web_api_interfaces::Element::is_instance(
            scope, object
        ));
        Ok(())
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_observers_track_detached_native_targets_through_adoption() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-detached-targets.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/observer-detached-targets.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 3);
    for index in 0..count {
        let name = vm
            .eval(&format!("__observerDetached.start({index})"))
            .unwrap();
        vm.advance_timers_until_deadline_for_test(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("__observerDetached.result").unwrap(),
            vm.eval("__observerDetached.expected").unwrap(),
            "{name}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_observer_queues_preserve_target_order_and_reentrant_operations() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://intersection-queues.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/intersection-observer-queues.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 2);
    for index in 0..count {
        let name = vm
            .eval(&format!("__intersectionQueues.start({index})"))
            .unwrap();
        vm.advance_timers_until_deadline_for_test(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("__intersectionQueues.result").unwrap(),
            vm.eval("__intersectionQueues.expected").unwrap(),
            "{name}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_observer_rechecks_document_and_callback_realm_between_deliveries() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://intersection-document.test/");
    vm.eval(
        r#"
const target = document.body.appendChild(document.createElement('div'));
target.style.cssText = 'width:20px;height:20px';
globalThis.log = [];
let phase = 'old';
const first = new IntersectionObserver(() => {
  log.push('first');
  first.disconnect();
  document.open();
  document.write('<!doctype html><p>replacement</p>');
  document.close();
  requestAnimationFrame(() => { phase = 'new'; log.push('frame'); });
});
const second = new IntersectionObserver(() => {
  log.push('second:' + phase);
  second.disconnect();
});
first.observe(target);
second.observe(target);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "first|frame|second:new");

    let mut vm = new_storage_page_task_executor_test_vm("https://intersection-realm.test/");
    vm.eval(
        "globalThis.frame = document.body.appendChild(document.createElement('iframe')); void 0",
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    vm.eval(
        r#"
const child = frame.contentWindow;
const target = document.body.appendChild(document.createElement('div'));
target.style.cssText = 'width:20px;height:20px';
globalThis.log = [];
const first = new IntersectionObserver(() => {
  log.push('first');
  first.disconnect();
  frame.remove();
});
const retired = new IntersectionObserver(child.Function("parent.log.push('retired')"));
const surviving = new IntersectionObserver(() => {
  log.push('surviving');
  surviving.disconnect();
});
first.observe(target);
retired.observe(target);
surviving.observe(target);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "first|surviving");
}
