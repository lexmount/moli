use super::*;

#[test]
fn svg_numbers_reject_nonfinite_values_without_mutating_the_list() {
    let mut vm = new_parsed_test_vm(
        "https://svg-number-validation.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } };
      for (const realm of [window, child]) {
        const text = realm.document.createElementNS(ns, 'text');
        text.setAttribute('rotate', '10 20');
        const list = text.rotate.baseVal;
        const number = list.getItem(0);
        const setter = Object.getOwnPropertyDescriptor(realm.SVGNumber.prototype, 'value').set;
        for (const invalid of [NaN, Infinity, -Infinity, Symbol(), 1n]) {
          check(error(() => { number.value = invalid; }) instanceof realm.TypeError, 'invalid numeric value');
          check(number.value === 10 && list.getItem(1).value === 20, 'atomic numeric state');
          check(text.getAttribute('rotate') === '10 20', 'atomic owner attribute');
        }
        let conversions = 0;
        const value = { valueOf() { conversions++; return 30; } };
        for (const receiver of [{}, Object.create(number), new Proxy(number, {})]) {
          check(error(() => setter.call(receiver, value)) instanceof realm.TypeError, 'invalid numeric receiver');
        }
        check(conversions === 0, 'receiver validation precedes numeric conversion');
        number.value = value;
        check(conversions === 1 && number.value === 30, 'valid numeric conversion');
        check(text.getAttribute('rotate') === '30 20' && list.getItem(1).value === 20, 'valid owner reflection');
      }
      return true;
    })()"#).unwrap(), "true");
}
