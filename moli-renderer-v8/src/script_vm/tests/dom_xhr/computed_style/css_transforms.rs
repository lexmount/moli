use super::*;

#[test]
fn css_transforms_preserve_types_identity_matrices_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://css-transforms.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(vm.eval(include_str!("css_transforms.js")).unwrap(), "true");
}

#[test]
fn css_transforms_reify_native_declarations_and_computed_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-transform-projection.test/",
        "<!doctype html><body><div id=target></div><iframe></iframe>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const check = (ok, message) => {if (!ok) throw new Error(message);};
        const source = 'translateX(2em) rotate3d(1, 2, 3, 90deg) scale(2, 3)';
        const parsed = CSSStyleValue.parse('transform', source);
        check(parsed instanceof CSSTransformValue && parsed.length === 3, 'parse transform list');
        check(parsed[0] instanceof CSSTranslate && parsed[0].x.unit === 'em' && parsed[0].y.value === 0, 'native normalization');
        check(!parsed[1].is2D && parsed[1] instanceof CSSRotate && parsed[2] instanceof CSSScale, 'component kinds');
        const target = document.querySelector('#target');
        target.style.fontSize = '10px';
        target.attributeStyleMap.set('transform', parsed);
        const declared = target.attributeStyleMap.get('transform');
        check(declared instanceof CSSTransformValue && declared[0].x.unit === 'em', 'declaration projection');
        const computed = target.computedStyleMap().get('transform');
        check(computed instanceof CSSTransformValue && computed[0].x.unit === 'px' && computed[0].x.value === 20, 'computed native units');
        parsed[0].x.value = 4;
        check(target.attributeStyleMap.get('transform')[0].x.value === 2, 'write snapshots value');
        declared[0].x.value = 3;
        check(target.attributeStyleMap.get('transform')[0].x.value === 2, 'read snapshots declaration');
        const child = document.querySelector('iframe').contentWindow;
        const childTarget = child.document.body;
        childTarget.style.transform = 'translate(5px)';
        const value = StylePropertyMapReadOnly.prototype.get.call(childTarget.attributeStyleMap, 'transform');
        check(value instanceof child.CSSTransformValue && value[0] instanceof child.CSSTranslate && value[0].x instanceof child.CSSUnitValue, 'reification owner realm');
        globalThis.CSSTranslate = () => {throw new Error('author constructor');};
        check(target.attributeStyleMap.get('transform') instanceof CSSTransformValue, 'intrinsic component allocation');
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn css_transforms_preserve_normative_dimension_flags_and_perspective_limits() {
    let mut vm = new_storage_test_vm("https://css-transform-dimensions.test/");
    assert_eq!(vm.eval(r#"(() => {
        const translation = new CSSTranslate(CSS.px(1), CSS.px(2), CSS.em(3));
        translation.is2D = true;
        if (translation.toMatrix().e !== 1 || translation.z.unit !== 'em') return false;
        const threeD = new CSSScale(1, 1, 1);
        if (threeD.toMatrix().is2D || new CSSTransformValue([threeD]).toMatrix().is2D) return false;
        for (const value of [-1, 0, .5, 1]) {
            const p = new CSSPerspective(CSS.px(value));
            if (p.toMatrix().is2D || p.toMatrix().m34 !== -1 || p.length.value !== value) return false;
        }
        if (new CSSPerspective('none').toMatrix().is2D) return false;
        if (String(new CSSScale(2, 2)) !== 'scale(2)') return false;
        if (String(new CSSScale(new CSSMathSum(1, 2), new CSSMathSum(1, 2))) !== 'scale(calc(1 + 2))') return false;
        return true;
    })()"#).unwrap(), "true");
}
