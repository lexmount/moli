use super::*;

#[test]
fn rtc_configuration_converts_dictionaries_and_keeps_atomic_independent_snapshots() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-configuration.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    let result = vm
        .eval(include_str!("../../../tests/fixtures/rtc-configuration.js"))
        .unwrap();
    if result != "true" {
        panic!(
            "{}",
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap()
        );
    }
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "184");
    assert_eq!(
        vm.eval("new Set(__uiEventResults.checks.map(row=>row.name)).size")
            .unwrap(),
        "184"
    );
}

#[test]
fn rtc_configuration_uses_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-configuration.test/");
    vm.eval("globalThis.pc = new RTCPeerConnection({iceTransportPolicy:'relay'})")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let target = global
            .get(scope, crate::util::v8str(scope, "pc").into())
            .unwrap()
            .to_object(scope)
            .unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativePc").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      if (nativePc.getConfiguration().iceTransportPolicy !== 'relay') return false;
      nativePc.setConfiguration({iceServers:[{urls:'stun:host.example'}]});
      const config=pc.getConfiguration();
      if (config.iceTransportPolicy !== 'all' || config.iceServers[0].urls[0] !== 'stun:host.example') return false;
      let converted=0;
      try { RTCPeerConnection.prototype.setConfiguration.call(new Proxy(nativePc, {}), {get iceServers(){converted++;return [];}}); return false; }
      catch (error) { if (!(error instanceof TypeError) || converted !== 0) return false; }
      pc.close();
      try {nativePc.setConfiguration();return false;}catch(error){return error.name === 'InvalidStateError';}
    })()"#).unwrap(), "true");
}

#[test]
fn rtc_configuration_applies_reentrant_updates_before_validating_the_outer_call() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-configuration.test/");
    assert_eq!(vm.eval(r#"(() => {
      const pc = new RTCPeerConnection();
      let sentinel={};
      try {
        pc.setConfiguration({get iceServers(){pc.setConfiguration({iceTransportPolicy:'relay'});throw sentinel;}});
        return false;
      } catch (error) {if (error !== sentinel || pc.getConfiguration().iceTransportPolicy !== 'relay') return false;}
      pc.setConfiguration({get iceServers(){pc.setConfiguration({iceTransportPolicy:'relay'});return [];}});
      const result=pc.getConfiguration().iceTransportPolicy === 'all';pc.close();return result;
    })()"#).unwrap(), "true");
}

#[test]
fn rtc_configuration_cannot_resize_the_pool_after_setting_a_local_description() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-configuration.test/");
    vm.eval(r#"globalThis.pc = new RTCPeerConnection({iceCandidatePoolSize:2});
      globalThis.configResult = false;
      pc.createOffer().then(offer => pc.setLocalDescription(offer)).then(() => {
        const before=JSON.stringify(pc.getConfiguration());
        pc.setConfiguration({iceCandidatePoolSize:2,iceTransportPolicy:'relay'});
        try {pc.setConfiguration({iceCandidatePoolSize:3});}
        catch (error) {configResult=error.name === 'InvalidModificationError' && pc.getConfiguration().iceCandidatePoolSize === 2 && before !== JSON.stringify(pc.getConfiguration());}
        pc.close();
      });"#).unwrap();
    assert_eq!(vm.eval("configResult").unwrap(), "true");
}
