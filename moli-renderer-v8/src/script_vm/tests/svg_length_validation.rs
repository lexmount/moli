use super::*;

#[test]
fn svg_lengths_round_numeric_getters_and_serialize_float_unit_arguments() {
    let mut vm = new_parsed_test_vm(
        "https://svg-length-float-output.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      for (const realm of [window, child]) {
        const svg = realm.document.createElementNS(ns, 'svg');
        const rect = realm.document.createElementNS(ns, 'rect');
        for (const length of [svg.createSVGLength(), rect.width.baseVal]) {
          length.newValueSpecifiedUnits(realm.SVGLength.SVG_LENGTHTYPE_CM, 2.54);
          check(length.value === 96, 'converted float user units');
          check(length.valueInSpecifiedUnits === Math.fround(2.54), 'specified float units');
          check(length.valueAsString === '2.54cm', 'short float serialization');
          check(length.unitType === realm.SVGLength.SVG_LENGTHTYPE_CM, 'specified unit');
          length.valueAsString = '16777217px';
          check(length.value === 16777216, 'parsed user units round to float');
          check(length.valueInSpecifiedUnits === 16777216, 'parsed specified units round to float');
          check(length.valueAsString === '16777217px', 'getters preserve parsed text');
          length.valueAsString = '1cm';
          check(length.value === Math.fround(96 / 2.54), 'absolute units round to float');
        }
      }
      return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn svg_lengths_reject_invalid_mutations_without_changing_the_owner() {
    let mut vm = new_parsed_test_vm(
        "https://svg-length-validation.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } };
      for (const realm of [window, child]) {
        const rect = realm.document.createElementNS(ns, 'rect');
        rect.setAttribute('x', '8px');
        const value = rect.x.baseVal;
        for (const invalid of ['', 'garbage', '1 px', '1px garbage']) {
          const e = error(() => { value.valueAsString = invalid; });
          check(e instanceof realm.DOMException && e.name === 'SyntaxError', 'invalid length text');
          check(value.value === 8 && rect.getAttribute('x') === '8px', 'atomic text mutation');
        }
        for (const invalid of [NaN, Infinity, -Infinity]) {
          for (const field of ['value', 'valueInSpecifiedUnits']) {
            check(error(() => { value[field] = invalid; }) instanceof realm.TypeError, 'nonfinite length');
            check(value.value === 8 && rect.getAttribute('x') === '8px', 'atomic numeric mutation');
          }
        }
        for (const method of ['newValueSpecifiedUnits', 'convertToSpecifiedUnits']) {
          const fn = realm.SVGLength.prototype[method];
          for (const unit of [0, 11, 65535]) {
            const e = error(() => fn.call(value, unit, 5));
            check(e instanceof realm.DOMException && e.name === 'NotSupportedError', 'invalid unit type');
            check(value.value === 8 && rect.getAttribute('x') === '8px', 'atomic unit mutation');
          }
          let conversions = 0;
          const argument = {valueOf() { conversions++; return 5; }};
          for (const receiver of [{}, Object.create(value), new Proxy(value, {})]) {
            check(error(() => fn.call(receiver, argument, 4)) instanceof realm.TypeError, 'invalid receiver');
          }
          check(conversions === 0, 'receiver check precedes conversion');
        }
        value.newValueSpecifiedUnits(realm.SVGLength.SVG_LENGTHTYPE_PX, 12);
        check(value.value === 12 && rect.getAttribute('x') === '12px', 'valid mutation reflects');
      }
      return true;
    })()"#).unwrap(), "true");
}
