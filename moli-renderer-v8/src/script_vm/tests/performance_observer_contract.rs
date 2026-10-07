use super::*;

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
