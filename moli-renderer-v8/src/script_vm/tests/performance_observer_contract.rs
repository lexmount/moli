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
    let mut vm = new_storage_test_vm("https://performance-observer-registration-order.test/");
    let source = format!(
        "({}).then(result => globalThis.__registrationOrder = result, error => globalThis.__registrationOrder = {{ error: String(error) }});",
        include_str!("../../../tests/fixtures/performance-observer-registration-order.js"),
    );
    vm.eval(&source).expect("queue registered observers");
    assert_eq!(
        vm.eval("JSON.stringify(__registrationOrder)").unwrap(),
        r#"["second:initial","first:initial","first:reobserved","second:reobserved"]"#,
    );
}

#[test]
fn performance_observer_delivery_preserves_batches_and_reentrant_registration() {
    let mut vm = new_storage_test_vm("https://performance-observer-delivery.test/");
    let source = format!(
        "({}).then(result => globalThis.__performanceDeliveryResult = result, error => globalThis.__performanceDeliveryResult = {{ error: String(error) }});",
        include_str!("../../../tests/fixtures/performance-observer-delivery.js"),
    );
    vm.eval(&source)
        .expect("queue PerformanceObserver delivery");
    assert_eq!(
        vm.eval("JSON.stringify(__performanceDeliveryResult)")
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
