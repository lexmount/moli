use super::*;

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
