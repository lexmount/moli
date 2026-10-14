use super::*;

fn check_uri_receivers(vm: &mut ScriptVm) {
    vm.eval(include_str!("svg_uri_receivers.js")).unwrap();
    assert_eq!(vm.eval("JSON.stringify(__svgUriFailures)").unwrap(), "[]");
}

#[test]
fn svg_uri_references_validate_receivers_and_preserve_producer_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-uri-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    check_uri_receivers(&mut vm);
}

#[test]
fn svg_uri_references_share_state_with_registered_native_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-uri-native-proxies.test/");
    vm.eval(r#"
      document.body.innerHTML = '<iframe></iframe>';
      const foreign = document.querySelector('iframe').contentWindow;
      const detached = foreign.document.implementation.createHTMLDocument('');
      globalThis.__svgNativeUris = Object.create(null);
      for (const tag of ['a','image','use','textPath','pattern','script','linearGradient','filter','mpath']) {
        __svgNativeUris[tag] = {target: detached.createElementNS('http://www.w3.org/2000/svg', tag)};
      }
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__svgNativeUris");
        let entries = global.get(scope, key.into()).unwrap();
        let entries = v8::Local::<v8::Object>::try_from(entries).unwrap();
        for tag in [
            "a",
            "image",
            "use",
            "textPath",
            "pattern",
            "script",
            "linearGradient",
            "filter",
            "mpath",
        ] {
            let key = crate::util::v8str(scope, tag);
            let entry = entries.get(scope, key.into()).unwrap();
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
        Ok(())
    })
    .unwrap();
    check_uri_receivers(&mut vm);
}
