use super::*;

#[test]
fn performance_observer_drop_reporting_preserves_flags_and_callback_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-drops.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(async () => {{
            const probe = {};
            const realms = [globalThis, document.getElementById('child').contentWindow];
            const checks = [];
            for (let owner=0;owner<2;owner++) for (let callee=0;callee<2;callee++) for (let callback=0;callback<2;callback++)
              checks.push(...await probe(realms[owner],realms[callee],realms[callback],`${{owner}}-${{callee}}-${{callback}}`));
            return {{total:checks.length,failures:checks.filter(row=>!row.passed)}};
        }})()"#,
        include_str!("performance_observer_dropped_entries.js"),
    );
    vm.eval(&format!(
        "({source}).then(result=>globalThis.__dropContract=result,error=>globalThis.__dropContract={{error:String(error)}});"
    ))
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__dropContract)")
            .unwrap(),
        r#"{"total":368,"failures":[]}"#
    );
}

#[test]
fn performance_observer_delivery_follows_registration_order_after_reobserve() {
    let mut vm = new_storage_page_task_executor_test_vm(
        "https://performance-observer-registration-order.test/",
    );
    let source = format!(
        "({}).then(result => globalThis.__registrationOrder = result, error => globalThis.__registrationOrder = {{ error: String(error) }});",
        include_str!("../../../tests/fixtures/performance-observer-registration-order.js"),
    );
    vm.eval(&source).expect("queue registered observers");
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__registrationOrder)")
            .unwrap(),
        r#"["second:initial","first:initial","first:reobserved","second:reobserved"]"#,
    );
}

#[test]
fn performance_observer_delivery_preserves_batches_and_reentrant_registration() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://performance-observer-delivery.test/");
    let source = format!(
        "({}).then(result => globalThis.__performanceDeliveryResult = result, error => globalThis.__performanceDeliveryResult = {{ error: String(error) }});",
        include_str!("../../../tests/fixtures/performance-observer-delivery.js"),
    );
    vm.eval(&source)
        .expect("queue PerformanceObserver delivery");
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__performanceDeliveryResult)")
            .unwrap(),
        r#"{"batches":[["first","second"],["after-reobserve"]],"taken":["taken-during-callback"],"proxyCalls":2}"#,
    );
}

#[test]
fn performance_observer_options_and_modes_follow_webidl_and_timeline_algorithms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-contract.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
            const probe = {};
            const realms = [globalThis, document.getElementById('child').contentWindow];
            const checks = [];
            for (let owner = 0; owner < 2; owner++) for (let callee = 0; callee < 2; callee++)
                checks.push(...probe(realms[owner], realms[callee], 'window-' + owner + '-' + callee));
            globalThis.__observerContractChecks = checks;
            return checks.length;
        }})()"#,
        include_str!("performance_observer_contract.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "358");
    assert_eq!(
        vm.eval("JSON.stringify(__observerContractChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn window_performance_observers_are_notified_in_one_task_after_microtasks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-task.test/");
    vm.eval(
        r#"
        globalThis.trace = [];
        new PerformanceObserver(() => {
          trace.push('observer-1');
          Promise.resolve().then(() => trace.push('observer-microtask'));
        }).observe({type: 'mark'});
        new PerformanceObserver(() => trace.push('observer-2')).observe({type: 'mark'});
        performance.mark('before-delivery');
        Promise.resolve().then(() => trace.push('earlier-microtask'));
        "#,
    )
    .unwrap();
    assert_eq!(vm.eval("trace.join(',')").unwrap(), "earlier-microtask");
    assert_eq!(run_performance_observer_tasks_for_test(&mut vm).await, 1);
    assert_eq!(
        vm.eval("trace.join(',')").unwrap(),
        "earlier-microtask,observer-1,observer-2,observer-microtask",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn window_performance_observer_notification_survives_document_open() {
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-task.test/");
    vm.eval(
        r#"
        globalThis.observedMarks = [];
        new PerformanceObserver(list => {
          observedMarks.push(...list.getEntries().map(entry => entry.name));
        }).observe({type: 'mark'});
        performance.mark('old-document');
        "#,
    )
    .unwrap();
    vm.eval(
        "document.open(); document.write('<body>replacement</body>'); document.close(); \
         performance.mark('new-document');",
    )
    .unwrap();
    assert_eq!(run_performance_observer_tasks_for_test(&mut vm).await, 1);
    assert_eq!(
        vm.eval("observedMarks.join(',')").unwrap(),
        "old-document,new-document",
    );
}
