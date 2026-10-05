use super::*;

#[test]
fn svg_filter_set_std_deviation_uses_native_receivers_and_float_arguments() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-std-deviation.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_std_deviation.js"))
        .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "804");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_set_std_deviation_updates_retained_isolated_world_numbers() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-filter-deviation-worlds.test/");
    vm.eval(
        r#"
      globalThis.filters = ['feGaussianBlur', 'feDropShadow'].map(tag => {
        const element = document.createElementNS('http://www.w3.org/2000/svg', tag);
        element.id = tag; document.body.appendChild(element);
        return {element, x: element.stdDeviationX, y: element.stdDeviationY};
      });
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("svg-filter-deviation", false)
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
      globalThis.filters = ['feGaussianBlur', 'feDropShadow'].map(tag => {
        const element = document.getElementById(tag);
        const x = element.stdDeviationX, y = element.stdDeviationY;
        element.setStdDeviation(2.5, 3.25);
        return {element, x, y};
      });
      filters.every(({element, x, y}) => x.baseVal === 2.5 && y.baseVal === 3.25 &&
        x === element.stdDeviationX && y === element.stdDeviationY);
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"
      filters.every(({element, x, y}) => {
        if (x.baseVal !== 2.5 || y.animVal !== 3.25) return false;
        element.setStdDeviation(4.5, 5.25);
        return x.baseVal === 4.5 && y.baseVal === 5.25;
      });
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
      filters.every(({element, x, y}) => x.baseVal === 4.5 && y.animVal === 5.25 &&
        x === element.stdDeviationX && y === element.stdDeviationY);
    "#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn svg_number_lists_synchronize_each_native_attribute_change() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-number-list-sync.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_number_list_sync.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_number_lists_synchronize_across_isolated_worlds() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-number-list-worlds.test/");
    vm.eval(
        r#"
      globalThis.matrix = document.createElementNS('http://www.w3.org/2000/svg', 'feColorMatrix');
      matrix.id = 'matrix'; document.body.appendChild(matrix); matrix.setAttribute('values', '1 2');
      globalThis.mainList = matrix.values;
      globalThis.mainOld = mainList.animVal.getItem(0);
    "#,
    )
    .unwrap();
    let isolated = vm.create_isolated_world("svg-number-lists", false).unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
      globalThis.matrix = document.getElementById('matrix');
      globalThis.list = matrix.values;
      globalThis.old = list.animVal.getItem(0);
    "#,
    )
    .unwrap();
    vm.eval("matrix.removeAttribute('values'); matrix.setAttribute('values', '1 2')")
        .unwrap();
    assert_eq!(vm.eval_in_isolated_context(isolated, r#"(() => {
      old.value = 7;
      if (old.value !== 7 || list.animVal.getItem(0) === old || list.baseVal.getItem(0).value !== 1) throw Error('isolated detach');
      if (Object.getPrototypeOf(list.animVal.getItem(0)) !== SVGNumber.prototype) throw Error('isolated producer realm');
      globalThis.nextOld = list.animVal.getItem(0);
      matrix.removeAttribute('values'); matrix.setAttribute('values', '1 2');
      return true;
    })()"#).unwrap(), "true");
    assert_eq!(vm.eval(r#"(() => {
      mainOld.value = 8;
      const item = mainList.animVal.getItem(0);
      return mainOld.value === 8 && item !== mainOld && item.value === 1 && Object.getPrototypeOf(item) === SVGNumber.prototype;
    })()"#).unwrap(), "true");
    assert_eq!(vm.eval_in_isolated_context(isolated, "nextOld.value = 9; nextOld.value === 9 && list.animVal.getItem(0) !== nextOld && list.animVal.getItem(0).value === 1").unwrap(), "true");
}

#[test]
fn svg_number_lists_preserve_intermediate_native_batch_changes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-number-list-batch.test/");
    vm.eval(
        r#"
      globalThis.matrix = document.createElementNS('http://www.w3.org/2000/svg', 'feColorMatrix');
      matrix.setAttribute('values', '1 2');
      globalThis.list = matrix.values;
      globalThis.old = list.animVal.getItem(1);
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, host_ptr| {
        let global = scope.get_current_context().global(scope);
        let value = global
            .get(scope, crate::util::v8str(scope, "matrix").into())
            .unwrap();
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        let (_, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)
                .unwrap();
        let effects = {
            let host = unsafe { &mut *host_ptr };
            let dom = host.dom_host_mut();
            let mut effects = dom.set_attribute_effects(handle, "values", "1");
            effects.merge(dom.set_attribute_effects(handle, "values", "1 2"));
            effects
        };
        unsafe { &mut *host_ptr }.with_dom_host_parse_step(|runtime| {
            runtime.apply_parser_stream_mutation_effects_to_live_dom_host(scope, host_ptr, effects);
        });
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("old.value = 7; old.value === 7 && list.animVal.getItem(1) !== old && list.baseVal.getItem(1).value === 2").unwrap(), "true");
}

#[test]
fn svg_number_lists_do_not_root_released_world_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-number-list-gc.test/");
    vm.eval(
        r#"
      globalThis.matrix = document.createElementNS('http://www.w3.org/2000/svg', 'feColorMatrix');
      matrix.id = 'matrix'; document.body.appendChild(matrix); matrix.setAttribute('values', '1 2');
      globalThis.mainList = matrix.values;
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("svg-number-list-gc", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        "globalThis.list = document.getElementById('matrix').values",
    )
    .unwrap();
    let context_ptr = &vm
        .page_isolated_world_contexts
        .context(isolated)
        .unwrap()
        .context as *const _;
    let weak_list = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let value = global
                .get(scope, crate::util::v8str(scope, "list").into())
                .unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            Ok(v8::Weak::new(scope, object))
        })
        .unwrap();
    // Release both the author global and native wrapper cache's intentional
    // roots before checking for an independent root in the list registry.
    vm.eval_in_isolated_context(isolated, "list = null; 'released'")
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        crate::native_bridge::identity::clear_context_wrapper_cache_for_teardown(scope, false);
        Ok(())
    })
    .unwrap();
    vm.destroy_isolated_world_context(isolated);
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert!(
            weak_list.to_local(scope).is_none(),
            "registrations must not retain released world wrappers"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("matrix.removeAttribute('values'); matrix.setAttribute('values', '3'); mainList.baseVal.getItem(0).value").unwrap(), "3");
}

#[test]
fn svg_filter_number_lists_reflect_live_values_and_validate_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-number-lists.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_number_lists.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_number_lists_registered_native_proxies_share_state() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-filter-number-list-proxies.test/");
    vm.eval(r#"document.body.innerHTML = '<iframe id=child></iframe>';
      globalThis.childWindow = document.querySelector('iframe').contentWindow;
      globalThis.foreignMatrix = childWindow.document.createElementNS('http://www.w3.org/2000/svg','feColorMatrix');
      foreignMatrix.setAttribute('values','1 2');
      globalThis.foreignAnimated = foreignMatrix.values;
      globalThis.foreignBase = foreignAnimated.baseVal;
      globalThis.foreignNumber = foreignBase.getItem(0);
      globalThis.foreignReadonly = foreignAnimated.animVal.getItem(0);"#).unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("foreignMatrix", "nativeMatrixProxy"),
            ("foreignAnimated", "nativeAnimatedProxy"),
            ("foreignBase", "nativeBaseProxy"),
            ("foreignNumber", "nativeNumberProxy"),
            ("foreignReadonly", "nativeReadonlyProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let object = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(object).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const descriptor = (p,n) => Object.getOwnPropertyDescriptor(p,n);
      const animated = descriptor(SVGFEColorMatrixElement.prototype,'values').get.call(nativeMatrixProxy);
      if(animated !== foreignAnimated || descriptor(SVGAnimatedNumberList.prototype,'baseVal').get.call(nativeAnimatedProxy) !== foreignBase) throw Error('native proxy cache');
      const get = SVGNumberList.prototype.getItem, length = descriptor(SVGNumberList.prototype,'length').get;
      if(get.call(nativeBaseProxy,0) !== foreignNumber || length.call(nativeBaseProxy) !== 2) throw Error('native list state');
      const value = descriptor(SVGNumber.prototype,'value');
      value.set.call(nativeNumberProxy,7);
      if(value.get.call(nativeNumberProxy)!==7 || foreignMatrix.getAttribute('values')!=='7 2') throw Error('native number reflection');
      let error;
      try{value.set.call(nativeReadonlyProxy,8);}catch(caught){error=caught;}
      if(error?.name!=='NoModificationAllowedError' || value.get.call(nativeReadonlyProxy)!==7) throw Error('native readonly number');
      const copied = SVGNumberList.prototype.appendItem.call(nativeBaseProxy,nativeReadonlyProxy);
      if(copied===foreignReadonly || copied.value!==7 || Object.getPrototypeOf(copied)!==childWindow.SVGNumber.prototype) throw Error('native argument clone realm');
      let conversions=0,traps=0;
      const author = new Proxy(nativeNumberProxy,{get(){traps++;throw 42;}});
      try{SVGNumberList.prototype.insertItemBefore.call(nativeBaseProxy,author,{valueOf(){conversions++;return 0;}});}catch(caught){error=caught;}
      if(!(error instanceof TypeError)||conversions||traps) throw Error('native wrapped author brand/order');
      return true;
    })()"#).unwrap(),"true");
}

#[test]
fn svg_filter_primitive_reflection_and_receiver_brands() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-reflection.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_reflection.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_registered_native_proxies_share_receiver_state_and_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-proxies.test/");
    vm.eval(r#"document.body.innerHTML = '<iframe id=child></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        globalThis.foreignBlend = childWindow.document.createElementNS('http://www.w3.org/2000/svg', 'feBlend');
        globalThis.foreignImage = childWindow.document.createElementNS('http://www.w3.org/2000/svg', 'feImage');
        globalThis.foreignInput = foreignBlend.in1;"#).unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("foreignBlend", "nativeBlendProxy"),
            ("foreignImage", "nativeImageProxy"),
            ("foreignInput", "nativeInputProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let object = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(object).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const descriptor = (prototype, name) => Object.getOwnPropertyDescriptor(prototype, name);
        for (const name of ['x','y','width','height','in1','in2','result']) {
          const getter = descriptor(SVGFEBlendElement.prototype, name).get;
          const value = getter.call(nativeBlendProxy);
          const C = ['x','y','width','height'].includes(name) ? childWindow.SVGAnimatedLength : childWindow.SVGAnimatedString;
          if (value !== foreignBlend[name] || Object.getPrototypeOf(value) !== C.prototype) throw Error(name + ' native proxy state/realm');
        }
        for (const name of ['href','preserveAspectRatio']) {
          const getter = descriptor(SVGFEImageElement.prototype, name).get;
          if (getter.call(nativeImageProxy) !== foreignImage[name]) throw Error(name + ' native image state');
        }
        const base = descriptor(SVGAnimatedString.prototype, 'baseVal'), anim = descriptor(SVGAnimatedString.prototype, 'animVal');
        base.set.call(nativeInputProxy, 'native\ud800value');
        if (base.get.call(nativeInputProxy) !== 'native\ud800value' || anim.get.call(nativeInputProxy) !== 'native\ud800value' || foreignBlend.getAttribute('in') !== 'native\ud800value') throw Error('native string proxy');
        let traps = 0, conversions = 0, error;
        const author = new Proxy(nativeInputProxy, {get() {traps++;throw 42;}});
        try {base.set.call(author, {toString() {conversions++;return 'bad';}});} catch (caught) {error = caught;}
        if (!(error instanceof TypeError) || traps !== 0 || conversions !== 0) throw Error('author proxy brand/order');
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn svg_filter_interfaces_preserve_native_inheritance_and_realm_contracts() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.getElementById('child').contentWindow;
  const detached = document.implementation.createHTMLDocument('');
  const xml = new DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml');
  for (const [name, parentName] of [["SVGFEComponentTransferElement","SVGElement"],["SVGFEFloodElement","SVGElement"],["SVGFEImageElement","SVGElement"],["SVGFEMergeElement","SVGElement"],["SVGFEMergeNodeElement","SVGElement"],["SVGFETileElement","SVGElement"]]) {
    for (const w of [window, child]) {
      const C = w[name], parent = w[parentName];
      assert(typeof C === 'function' && C.name === name, name + ' exposed');
      assert(Object.getPrototypeOf(C.prototype) === parent.prototype, name + ' prototype inheritance');
      assert(Object.getPrototypeOf(C) === parent, name + ' interface inheritance');
      let error;
      try { new C(); } catch (caught) { error = caught; }
      assert(error instanceof w.TypeError, name + ' illegal constructor realm');
    }
  }
  const nodeType = Object.getOwnPropertyDescriptor(Node.prototype, 'nodeType').get;
  for (const [tag, name] of [["feComponentTransfer","SVGFEComponentTransferElement"],["feFlood","SVGFEFloodElement"],["feImage","SVGFEImageElement"],["feMerge","SVGFEMergeElement"],["feMergeNode","SVGFEMergeNodeElement"],["feTile","SVGFETileElement"]]) {
    for (const [doc, w] of [[document, window], [child.document, child], [detached, window], [xml, window]]) {
      const element = doc.createElementNS(ns, tag), C = w[name];
      assert(element instanceof C && element instanceof w.SVGElement && element instanceof w.Node, tag + ' native factory brand');
      assert(Object.getPrototypeOf(element) === C.prototype && element.constructor === C, tag + ' factory prototype');
      assert(Object.prototype.toString.call(element) === '[object ' + name + ']', tag + ' native tag');
      assert(element.cloneNode(false) instanceof C, tag + ' cloned brand');
      assert(doc.importNode(element, false) instanceof C, tag + ' imported brand');
      element.setAttribute('data-test', 'value');
      assert(element.getAttribute('data-test') === 'value' && nodeType.call(element) === 1, tag + ' inherited DOM behavior');
      for (const receiver of [Object.create(element), new Proxy(element, {})]) {
        let error;
        try { nodeType.call(receiver); } catch (caught) { error = caught; }
        assert(error instanceof TypeError, tag + ' rejects forged native identity');
      }
      assert(!(doc.createElement(tag) instanceof C), tag + ' namespace-sensitive factory');
    }
    const parsed = new DOMParser().parseFromString('<svg xmlns="' + ns + '"><' + tag + '/></svg>', 'image/svg+xml');
    assert(parsed.documentElement.firstElementChild instanceof window[name], tag + ' parsed XML brand');
  }
  return true;
})()"#).unwrap(), "true");
}
