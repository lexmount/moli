use super::*;

#[test]
fn svg_angle_values_convert_units_and_validate_native_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://svg-angle.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } throw Error('expected exception'); };
      const child = document.querySelector('iframe').contentWindow;
      for (const realm of [window, child]) {
        const svg = realm.document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        const a = svg.createSVGAngle();
        check(a instanceof realm.SVGAngle && a.unitType === 1 && a.value === 0, 'native default');
        check(svg.createSVGAngle() !== a, 'fresh value');
        const intrinsic = realm.SVGAngle;
        realm.SVGAngle = function ForgedAngle() { throw Error('author constructor'); };
        const native = svg.createSVGAngle();
        check(realm.Object.getPrototypeOf(native) === intrinsic.prototype && native.value === 0, 'intrinsic factory');
        realm.SVGAngle = intrinsic;
        check(error(() => new realm.SVGAngle()) instanceof realm.TypeError, 'illegal constructor');
        a.newValueSpecifiedUnits(4, 100);
        check(a.value === 90 && a.valueInSpecifiedUnits === 100 && a.valueAsString === '100grad', 'grad conversion');
        a.convertToSpecifiedUnits(2);
        check(a.value === 90 && a.valueInSpecifiedUnits === 90 && a.valueAsString === '90deg', 'convert unit');
        a.valueAsString = '3.141592653589793rad';
        check(Math.abs(a.value - 180) < 0.001 && a.unitType === 3, 'radians');
        a.valueInSpecifiedUnits = 0;
        check(a.value === 0, 'specified-value setter');
        a.value = 45;
        check(a.value === 45, 'degree-value setter');
        let conversions = 0;
        const value = {valueOf(){conversions++; return 10;}};
        const method = realm.SVGAngle.prototype.newValueSpecifiedUnits;
        for (const fake of [{}, Object.create(a), new Proxy(a,{})]) {
          check(error(() => method.call(fake, value, value)) instanceof realm.TypeError, 'native brand');
        }
        check(conversions === 0, 'brand before conversion');
        const before = a.valueAsString;
        for (const invalid of [NaN, Infinity, -Infinity, 1e100]) {
          check(error(() => a.value = invalid) instanceof realm.TypeError, 'finite float conversion');
          check(a.valueAsString === before, 'atomic float failure');
        }
        check(error(() => a.newValueSpecifiedUnits(0, 3)).name === 'NotSupportedError', 'invalid unit');
        check(error(() => a.valueAsString = 'bogus').name === 'SyntaxError', 'invalid string');
        check(a.valueAsString === before, 'atomic invalid syntax');
      }
      return true;
    })()"#).unwrap(), "true");
}
