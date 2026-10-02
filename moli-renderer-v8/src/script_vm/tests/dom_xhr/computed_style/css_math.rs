use super::*;

#[test]
fn css_math_objects_preserve_native_types_identity_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://css-math.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(vm.eval(include_str!("css_math.js")).unwrap(), "true");
}

#[test]
fn css_numeric_array_indices_follow_webidl_without_mutating_the_expression() {
    let mut vm = new_storage_test_vm("https://css-numeric-array.test/");
    assert_eq!(vm.eval(r#"(() => {
        'use strict';
        const a = new CSSMathSum(1, 2).values;
        const first = a[0];
        const desc = Object.getOwnPropertyDescriptor(a, '0');
        if (desc.value !== first || desc.writable || !desc.enumerable || !desc.configurable) return false;
        for (const i of [0, 2, 100]) {
            if (Reflect.set(a, i, 99) || Reflect.defineProperty(a, i, {value: 99})) return false;
            try { a[i] = 99; return false; } catch (e) { if (!(e instanceof TypeError)) throw e; }
        }
        if (a[0] !== first || a.length !== 2 || a[2] !== undefined) return false;
        if (Reflect.deleteProperty(a, 0) || !Reflect.deleteProperty(a, 2)) return false;
        if (Reflect.preventExtensions(a) || !Object.isExtensible(a)) return false;
        const receiver = Object.create(a);
        if (Reflect.set(receiver, 0, 99) || Object.hasOwn(receiver, 0)) return false;
        if (!Reflect.set(receiver, 2, 99) || receiver[2] !== 99 || a[2] !== undefined) return false;
        a.extra = 3;
        return a.extra === 3 && Object.keys(a).join() === '0,1,extra';
    })()"#).unwrap(), "true");
}

#[test]
fn css_math_deep_graphs_do_not_recurse_on_the_native_stack() {
    let mut vm = new_storage_test_vm("https://css-math-depth.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
        let nested = CSS.number(1);
        for (let i = 0; i < 3000; ++i) nested = new CSSMathNegate(nested);
        if (String(nested).length !== 9005 || Object.keys(nested.type()).length) return false;
        let square = CSS.px(1);
        try {
            for (let i = 0; i < 32; ++i) square = new CSSMathProduct(square, square);
            return false;
        } catch (e) { if (!(e instanceof TypeError)) throw e; }
        return new CSSMathSum(1, 2).toString() === 'calc(1 + 2)';
    })()"#
        )
        .unwrap(),
        "true"
    );
}
