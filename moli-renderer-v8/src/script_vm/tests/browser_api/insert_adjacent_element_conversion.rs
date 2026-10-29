use super::*;

#[test]
fn insert_adjacent_element_converts_interfaces_before_dom_validation() {
    let mut vm = new_parsed_test_vm(
        "https://adjacent-element-conversion.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    let count = vm
        .eval(include_str!(
            "../../../../tests/fixtures/insert-adjacent-element-conversion.js"
        ))
        .expect("insertAdjacentElement receiver and argument matrix");
    assert!(count.parse::<usize>().unwrap() > 1000);
}

#[test]
fn insert_adjacent_element_preserves_registered_native_alias_identity() {
    let mut vm = new_parsed_test_vm(
        "https://adjacent-element-alias.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval("globalThis.adjacentOwner=document.getElementById('child').contentWindow;globalThis.adjacentRoot=adjacentOwner.document.createElement('div');globalThis.adjacentNode=adjacentOwner.document.createElement('span');adjacentOwner.document.body.appendChild(adjacentRoot)").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [
            ("adjacentRoot", "adjacentRootAlias"),
            ("adjacentNode", "adjacentNodeAlias"),
        ] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert!(crate::web_api_interfaces::Element::is_instance(
                scope,
                proxy.into()
            ));
            let key = crate::util::v8str(scope, alias);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
      for (const realm of [window, adjacentOwner]) {
        const method = realm.Element.prototype.insertAdjacentElement;
        if (method.call(adjacentRootAlias, 'beforeend', adjacentNodeAlias) !== adjacentNodeAlias ||
            adjacentNode.parentNode !== adjacentRoot) return false;
        adjacentNode.remove();
        if (method.call(adjacentRootAlias, 'afterbegin', adjacentNode) !== adjacentNode ||
            adjacentRoot.firstChild !== adjacentNode) return false;
        adjacentNode.remove();
      }
      return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
