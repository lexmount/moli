use super::*;

#[test]
fn css_numeric_parse_preserves_syntax_types_precision_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://css-numeric-parse.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_numeric_parse.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_numeric_parse_handles_dimensional_algebra_and_unresolved_percentages() {
    let mut vm = new_storage_test_vm("https://css-numeric-parse-algebra.test/");
    assert_eq!(vm.eval(r#"(() => {
        const check = (ok, message) => {if (!ok) throw new Error(message);};
        const parse = CSSNumericValue.parse;
        const ratio = parse('calc(6px / 2s)');
        check(ratio instanceof CSSMathProduct && ratio.values[1] instanceof CSSMathInvert, 'dimension inverse');
        check(ratio.type().length === 1 && ratio.type().time === -1, 'inverse type');
        check(parse('calc(6px / 2px)').to('number').value === 3, 'unit cancellation');
        const relative = parse('calc(6px / 2em)');
        check(relative instanceof CSSMathProduct && Object.keys(relative.type()).length === 0, 'relative ratio remains unresolved');
        try {relative.to('number'); return false;} catch(e) {if (!(e instanceof TypeError)) throw e;}
        check(parse('calc(6em / 2em)').to('number').value === 3, 'matching relative units cancel');
        const min = parse('min(3px, 2%, 1px)');
        check(min.values.length === 2 && min.values[0].value === 1 && min.values[1].unit === 'percent', 'partial comparison simplification');
        check(parse('min(2%,1%)') instanceof CSSMathMin, 'percentages may have a negative basis');
        check(parse('calc(0px + 0%)').values.length === 2, 'zero terms preserve units');
        check(parse('clamp(none, 2%, 10px)') instanceof CSSMathMin, 'unbounded lower clamp');
        check(parse('clamp(10px, 2%, none)') instanceof CSSMathMax, 'unbounded upper clamp');
        check(parse('clamp(none, 2px, none)').value === 2, 'unbounded clamp');
        check(parse('calc(1khz + 1hz)').to('hz').value === 1001, 'frequency');
        check(parse('calc(1fr * 2)').to('fr').value === 2, 'flex type');
        check(parse('calc(1s + 2%)').type().percentHint === 'time', 'property-independent percent hint');
        check(Object.is(parse('max(-0,0)').value,0), 'max signed zero');
        check(Object.is(parse('min(0,-0)').value,-0), 'min signed zero');
        check(Number.isNaN(parse('max(1,NaN)').value), 'NaN propagation');
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn css_numeric_parse_bounds_input_work_and_recovers_after_errors() {
    let mut vm = new_storage_test_vm("https://css-numeric-parse-limits.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
        const parse = CSSNumericValue.parse;
        const good = 'calc('.repeat(64) + '1px' + ')'.repeat(64);
        if (parse(good).to('px').value !== 1) return false;
        for (const input of [
            'calc('.repeat(2000) + '1px' + ')'.repeat(2000),
            'calc(' + Array(40000).fill('1px').join(' + ') + ')',
            'min(' + Array(40000).fill('1px').join(',') + ')',
        ]) {
            try {parse(input); return false;} catch(e) {if (!(e instanceof RangeError)) throw e;}
        }
        return parse('calc(1px + 2px)').to('px').value === 3;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
