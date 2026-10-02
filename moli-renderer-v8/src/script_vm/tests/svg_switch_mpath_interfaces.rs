use super::*;

#[test]
fn svg_switch_mpath_interfaces_preserve_native_inheritance_and_realm_contracts() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-switch-mpath-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.getElementById('child').contentWindow;
  const detached = document.implementation.createHTMLDocument('');
  const xml = new DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml');
  for (const [name, parentName] of [["SVGSwitchElement","SVGGraphicsElement"],["SVGMPathElement","SVGElement"]]) {
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
  for (const [tag, name] of [["switch","SVGSwitchElement"],["mpath","SVGMPathElement"]]) {
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
