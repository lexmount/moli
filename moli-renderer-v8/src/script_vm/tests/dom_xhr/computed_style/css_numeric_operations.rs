use super::*;

#[test]
fn css_numeric_operations_preserve_brands_realms_and_conversion_order() {
    let mut vm = new_parsed_test_vm(
        "https://css-numeric-operations.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_numeric_operations.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_numeric_operations_retain_special_values_and_type_constraints() {
    let mut vm = new_storage_test_vm("https://css-numeric-special-values.test/");
    assert_eq!(vm.eval(r#"(() => {
        const check = (ok, message) => { if (!ok) throw new Error(message); };
        check(Object.is(CSS.number(0).min(-0).value, -0), 'min signed zero');
        check(Object.is(CSS.number(-0).max(0).value, 0), 'max signed zero');
        check(Object.is(new CSSMathMin(0, -0).to('number').value, -0), 'sum min signed zero');
        check(Object.is(new CSSMathMax(-0, 0).to('number').value, 0), 'sum max signed zero');
        const infinity = CSS.number(Number.MAX_VALUE).mul(2);
        const nan = infinity.sub(infinity);
        check(infinity.value === Infinity && Number.isNaN(nan.value), 'double overflow and NaN');
        check(!nan.equals(nan), 'NaN is unequal even on the same object');
        check(Number.isNaN(CSS.number(1).min(nan).value), 'min propagates NaN');
        check(Number.isNaN(CSS.number(1).max(nan).value), 'max propagates NaN');
        check(Number.isNaN(new CSSMathClamp(0, nan, 10).to('number').value), 'clamp propagates NaN');
        check(new CSSMathClamp(20, 0, 10).to('number').value === 20, 'clamp lower bound wins');
        const length = CSS.px(1).add(CSS.percent(2));
        const angle = CSS.deg(1).add(CSS.percent(2));
        for (const method of ['mul', 'div']) {
            try { length[method](angle); return false; } catch (e) { if (!(e instanceof TypeError)) throw e; }
        }
        for (const method of ['min', 'max']) {
            try { CSS.number(1)[method](CSS.px(1)); return false; } catch (e) { if (!(e instanceof TypeError)) throw e; }
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn css_numeric_operations_bound_expansion_and_share_deep_graph_work() {
    let mut vm = new_storage_test_vm("https://css-numeric-graphs.test/");
    assert_eq!(vm.eval(r#"(() => {
        const check = (ok, message) => { if (!ok) throw new Error(message); };
        let a = CSS.number(1), b = CSS.number(1);
        for (let i = 0; i < 3000; ++i) {
            a = new CSSMathNegate(a); b = new CSSMathNegate(b);
        }
        check(a.equals(b) && a.to('number').value === 1, 'deep graph');
        const leaf = CSS.number(1);
        a = leaf; b = CSS.number(1);
        for (let i = 0; i < 200; ++i) {
            a = new CSSMathSum(a, a); b = new CSSMathSum(b, b);
        }
        check(a.equals(b) && a.to('number').value === 2 ** 200, 'shared DAG');
        leaf.value = 3;
        check(!a.equals(b) && a.to('number').value === 3 * 2 ** 200, 'fresh mutable values');
        let expansion = new CSSMathSum(CSS.px(1), CSS.em(1));
        for (let i = 0; i < 6; ++i) expansion = new CSSMathProduct(expansion, expansion);
        try { expansion.toSum(); return false; } catch (e) { if (!(e instanceof RangeError)) throw e; }
        check(CSS.px(4).to('px').value === 4, 'realm remains usable after resource limit');
        return true;
    })()"#).unwrap(), "true");
}
