use super::*;

#[test]
fn svg_root_value_factories_create_fresh_native_values_in_the_callee_realm() {
    let mut vm = new_parsed_test_vm(
        "https://svg-value-factories.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } };
      for (const realm of [window, child]) {
        for (const doc of [realm.document, realm.document.implementation.createHTMLDocument('')]) {
          const svg = doc.createElementNS(ns, 'svg');
          for (const [method, type] of [['createSVGNumber', 'SVGNumber'], ['createSVGLength', 'SVGLength']]) {
            const descriptor = Object.getOwnPropertyDescriptor(realm.SVGSVGElement.prototype, method);
            check(descriptor.enumerable && descriptor.writable && descriptor.configurable, 'method descriptor');
            const fn = descriptor.value;
            check(fn.name === method && fn.length === 0, 'method name and length');
            const first = fn.call(svg);
            const second = fn.call(svg);
            check(first instanceof realm[type] && Object.getPrototypeOf(first) === realm[type].prototype, 'native value prototype');
            check(first !== second && first.value === 0 && second.value === 0, 'fresh zero values');
            first.value = 12;
            check(first.value === 12 && second.value === 0, 'independent values');
            check(error(() => new realm[type]()) instanceof realm.TypeError, 'illegal interface constructor');
            if (type === 'SVGLength') {
              check(second.unitType === realm.SVGLength.SVG_LENGTHTYPE_NUMBER, 'default unit');
              first.valueAsString = '6px';
              check(first.value === 6 && first.unitType === realm.SVGLength.SVG_LENGTHTYPE_PX, 'mutable standalone length');
            }
            for (const receiver of [{}, Object.create(svg), new Proxy(svg, {}), doc.createElementNS(ns, 'rect')]) {
              check(error(() => fn.call(receiver)) instanceof realm.TypeError, 'invalid factory receiver');
            }
          }
        }
      }
      const otherSvg = child.document.createElementNS(ns, 'svg');
      for (const [method, type] of [['createSVGNumber', 'SVGNumber'], ['createSVGLength', 'SVGLength']]) {
        const value = SVGSVGElement.prototype[method].call(otherSvg);
        check(value instanceof window[type] && !(value instanceof child[type]), 'callee realm allocation');
      }
      return true;
    })()"#).unwrap(), "true");
}
