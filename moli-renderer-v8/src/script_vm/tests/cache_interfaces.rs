use super::*;

#[test]
fn cache_interfaces_share_native_prototypes_and_promise_receiver_checks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://cache-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__cacheInterfaceDone = value, error => globalThis.__cacheInterfaceDone = String(error));",
        include_str!("cache_interfaces.js")
    )).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__cacheInterfaceDone").unwrap(), "true");
}

#[test]
fn cache_interface_globals_require_a_secure_window() {
    let mut vm = new_storage_html_test_vm("http://cache-interfaces.test/");
    assert_eq!(
        vm.eval(
            r#"
    (() => {
      const iframe = document.createElement('iframe');document.body.append(iframe);
      const popup = open();
      try {
        return [window,iframe.contentWindow,popup].every(realm =>
          !realm.isSecureContext && !('Cache' in realm) && !('CacheStorage' in realm));
      } finally {iframe.remove();popup.close();}
    })()
    "#
        )
        .unwrap(),
        "true"
    );
}
