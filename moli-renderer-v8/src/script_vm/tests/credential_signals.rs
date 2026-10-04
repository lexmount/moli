use super::*;

#[test]
fn credential_signals_preserve_dictionary_order_promise_errors_and_function_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://credential-signals.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("credential_signals.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn credential_signals_reject_ip_origin_even_when_the_rp_id_matches() {
    let mut vm = new_storage_page_task_executor_test_vm("https://127.0.0.1/");
    vm.eval(
        "globalThis.failure = 'not rejected'; PublicKeyCredential.signalUnknownCredential({rpId: '127.0.0.1', credentialId: 'AA'}).then(() => {}, error => failure = [error.name, error.code, error instanceof DOMException].join(','))",
    ).unwrap();
    assert_eq!(vm.eval("failure").unwrap(), "SecurityError,18,true");
}
