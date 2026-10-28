use super::*;

#[test]
fn svg_length_absolute_unit_roundtrips_preserve_float_precision() {
    let mut vm = new_parsed_test_vm(
        "https://svg-length-absolute-roundtrip.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    // The probe also runs unchanged in Chromium. String formatting and its
    // large-length clamp are recorded separately from float rounding.
    let probe = include_str!("../../../tests/fixtures/svg-length-absolute-roundtrip.js");
    let validate = r#"(rows => {
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const bits = new Int32Array(1);
      const floats = new Float32Array(bits.buffer);
      const ordered = value => {
        floats[0] = value;
        const word = bits[0];
        return word < 0 ? -2147483648 - word : word;
      };
      const near = (actual, expected, label) => {
        check(Number.isFinite(actual) && actual === Math.fround(actual), label + ' finite float');
        check(Math.abs(ordered(actual) - ordered(expected)) <= 1,
          label + ': ' + actual + ' versus ' + expected);
      };
      for (const row of rows) {
        const label = [row.realm, row.kind, row.unit, row.input, row.field].join(':');
        const factor = {cm: 96 / 2.54, in: 96, px: 1}[row.unit];
        const unit = {cm: 6, in: 8, px: 5}[row.unit];
        const input = Math.fround(row.input);
        const specified = row.field === 'value' ? Math.fround(input / factor) : input;
        const user = row.field === 'value' ? input : Math.fround(input * factor);
        check(row.first.unit === unit, label + ' preserved unit');
        check(row.first.specified === specified, label + ' float specified units');
        near(row.first.value, user, label + ' user-unit readback');
        if (row.kind !== 'standalone') {
          check(row.serialized === row.first.text, label + ' reflected text');
        }
        check(row.reparsed.unit === unit, label + ' reparsed unit');
        check(row.reparsed.specified === row.first.specified, label + ' reparsed specified units');
        near(row.reparsed.value, row.first.value, label + ' reparsed user units');
        check(row.pixels.unit === 5, label + ' converted px unit');
        check(row.pixels.value === row.first.value, label + ' converted user units');
        check(row.pixels.specified === row.pixels.value, label + ' converted specified units');
        check(row.roundtrip.unit === unit, label + ' roundtrip unit');
        near(row.roundtrip.value, row.first.value, label + ' roundtrip user units');
        near(row.roundtrip.specified, row.first.specified, label + ' roundtrip specified units');
      }
      return rows.length;
    })"#;
    assert_eq!(
        vm.eval(&format!("{validate}({probe})")).expect(
            "absolute-unit setters, serialization and conversions should retain float precision"
        ),
        "324"
    );
}

#[test]
fn svg_length_unit_conversion_keeps_finite_text_beyond_float_range() {
    let mut vm = new_parsed_test_vm(
        "https://svg-length-unit-overflow.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const ns = 'http://www.w3.org/2000/svg';
      for (const realm of [window, document.querySelector('iframe').contentWindow]) {
        const svg = realm.document.createElementNS(ns, 'svg');
        const rect = realm.document.createElementNS(ns, 'rect');
        const text = realm.document.createElementNS(ns, 'text');
        text.setAttribute('x', '0');
        for (const [length, owner, attribute] of [
          [svg.createSVGLength(), null, null],
          [rect.x.baseVal, rect, 'x'],
          [text.x.baseVal.getItem(0), text, 'x']
        ]) {
          for (const input of [1e38, -1e38]) {
            length.newValueSpecifiedUnits(realm.SVGLength.SVG_LENGTHTYPE_CM, input);
            length.convertToSpecifiedUnits(realm.SVGLength.SVG_LENGTHTYPE_PX);
            const serialized = length.valueAsString;
            check(serialized.endsWith('px'), 'converted unit suffix');
            const number = Number(serialized.slice(0, -2));
            check(Number.isFinite(number) && Math.abs(number) > 3.4028235e38,
              'finite derived value beyond float range: ' + serialized);
            if (owner) check(owner.getAttribute(attribute) === serialized, 'valid reflected value');
            const copy = svg.createSVGLength();
            copy.valueAsString = serialized;
            check(copy.valueAsString === serialized, 'converted text parses again');
            length.convertToSpecifiedUnits(realm.SVGLength.SVG_LENGTHTYPE_CM);
            check(length.valueInSpecifiedUnits === Math.fround(input), 'conversion roundtrip');
          }
          length.valueAsString = '1q';
          length.value = 3.4e38;
          check(Number.isFinite(Number(length.valueAsString.slice(0, -1))),
            'finite specified units after user-unit setter');
          length.valueAsString = '0cm';
          length.value = 1e-44;
          check(Number(length.valueAsString.slice(0, -2)) > 0,
            'nonzero derived specified value survives underflow');
        }
      }
      return true;
    })()"#
        )
        .expect("large finite SVGLength conversions should evaluate"),
        "true"
    );
}

#[test]
fn svg_length_numeric_setters_convert_float_inputs_before_storage_and_reflection() {
    let mut vm = new_parsed_test_vm(
        "https://svg-length-float-input.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const ns = 'http://www.w3.org/2000/svg';
      for (const realm of [window, document.querySelector('iframe').contentWindow]) {
        const svg = realm.document.createElementNS(ns, 'svg');
        const rect = realm.document.createElementNS(ns, 'rect');
        const text = realm.document.createElementNS(ns, 'text');
        text.setAttribute('x', '0');
        for (const [length, owner, attribute] of [
          [svg.createSVGLength(), null, null],
          [rect.width.baseVal, rect, 'width'],
          [text.x.baseVal.getItem(0), text, 'x']
        ]) {
          const reflected = expected => {
            check(length.valueAsString === expected, 'serialized ' + expected);
            if (owner) check(owner.getAttribute(attribute) === expected, 'reflected ' + expected);
          };
          for (const field of ['value', 'valueInSpecifiedUnits']) {
            length.valueAsString = '0';
            length[field] = 16777217;
            check(length.value === 16777216, field + ' user float');
            check(length.valueInSpecifiedUnits === 16777216, field + ' specified float');
            reflected('16777216');
            let conversions = 0;
            length[field] = {valueOf() { conversions++; return 2.54; }};
            check(conversions === 1, field + ' converts once');
            check(length[field] === Math.fround(2.54), field + ' decimal float');
            reflected('2.54');
          }
          length.valueAsString = '1cm';
          length.valueInSpecifiedUnits = 2.54;
          check(length.value === 96, 'specified decimal converts to user units');
          check(length.valueInSpecifiedUnits === Math.fround(2.54), 'specified decimal input');
          reflected('2.54cm');
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
        for (const invalid of [NaN, Infinity, -Infinity, 1e39, -1e39, Number.MAX_VALUE]) {
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
