use super::*;

fn assert_svg_animated_number_probe(vm: &mut ScriptVm, expected_checks: usize) {
    vm.eval(include_str!("svg_animated_number.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("__uiEventResults.total").unwrap(),
        expected_checks.to_string()
    );
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_animated_number_receivers_and_float_reflection() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-number-reflection.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_svg_animated_number_probe(&mut vm, 31380);
}

#[test]
fn svg_animated_number_registered_proxies_share_owner_and_value_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-number-proxies.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_svg_animated_number_probe(&mut vm, 31380);
    vm.eval(
        r#"
      const childWindow = document.querySelector('iframe').contentWindow;
      const detached = childWindow.document.implementation.createHTMLDocument('');
      globalThis.__nativeSvgNumberOwners = [];
      globalThis.__nativeSvgNumberValues = [];
      for (const [tag, name, members] of __svgNumberDefinitions) {
        for (const [member, attribute, component] of members) {
          const target = detached.createElementNS('http://www.w3.org/2000/svg', tag);
          __nativeSvgNumberOwners.push({tag, name, member, attribute, component, target});
          const owner = detached.createElementNS('http://www.w3.org/2000/svg', tag);
          __nativeSvgNumberValues.push({tag, member, attribute, component, owner, target: owner[member]});
        }
      }
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for name in ["__nativeSvgNumberOwners", "__nativeSvgNumberValues"] {
            let key = crate::util::v8str(scope, name);
            let entries = global.get(scope, key.into()).unwrap();
            let entries = v8::Local::<v8::Array>::try_from(entries).unwrap();
            for index in 0..entries.length() {
                let entry = entries.get_index(scope, index).unwrap();
                let entry = v8::Local::<v8::Object>::try_from(entry).unwrap();
                let key = crate::util::v8str(scope, "target");
                let target = entry.get(scope, key.into()).unwrap();
                let target = v8::Local::<v8::Object>::try_from(target).unwrap();
                let handler = crate::util::new_null_prototype_object(scope);
                let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                let key = crate::util::v8str(scope, "proxy");
                assert_eq!(
                    entry.create_data_property(scope, key.into(), proxy.into()),
                    Some(true)
                );
            }
        }
        Ok(())
    })
    .unwrap();
    assert_svg_animated_number_probe(&mut vm, 32106);
}
