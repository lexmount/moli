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

#[test]
fn credential_signals_use_native_domain_state_and_retained_function_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://sub.example.test/");
    vm.eval(r#"globalThis.domainResult = 'pending';
    (async () => {
      const signal = PublicKeyCredential.signalUnknownCredential;
      const options = rpId => ({rpId, credentialId: 'AA'});
      await signal(options('sub.example.test'));
      document.domain = 'example.test';
      await signal(options('example.test'));
      let caught;
      try { await signal(options('sub.example.test')); } catch (error) { caught = error; }
      if (!(caught instanceof DOMException) || caught.name !== 'SecurityError') throw Error('domain authority');
      let reads = 0;
      Object.defineProperty(globalThis, 'origin', {configurable: true, get() { reads++; throw Error('author origin'); }});
      await signal(options('example.test'));
      if (reads !== 0) throw Error('author getter used');
      const frame = document.createElement('iframe');
      document.body.appendChild(frame);
      const owner = frame.contentWindow;
      const retained = owner.PublicKeyCredential.signalUnknownCredential;
      const Promise = owner.Promise;
      frame.remove();
      const promise = retained(options('example.test'));
      if (!(promise instanceof Promise) || await promise !== undefined) throw Error('retained function realm');
      domainResult = 'true';
    })().catch(error => domainResult = 'error:' + error);
    "#).unwrap();
    assert_eq!(vm.eval("domainResult").unwrap(), "true");
}
