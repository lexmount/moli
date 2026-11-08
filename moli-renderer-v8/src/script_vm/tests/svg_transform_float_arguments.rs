use super::*;

#[test]
fn svg_transform_arguments_round_to_float_and_reject_float_overflow_before_mutation() {
    let mut vm = new_storage_test_vm("https://svg-transform-floats.test/");
    assert_eq!(vm.eval(r#"(() => {
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        const transform = svg.createSVGTransform();
        const value = 16777217;
        transform.setTranslate(value, -value);
        if (transform.matrix.e !== Math.fround(value) || transform.matrix.f !== Math.fround(-value)) return 'translate';
        transform.setScale(value, -value);
        if (transform.matrix.a !== Math.fround(value) || transform.matrix.d !== Math.fround(-value)) return 'scale';
        transform.setRotate(value, 0, 0);
        if (transform.angle !== Math.fround(value)) return 'rotate';
        for (const method of ['setSkewX', 'setSkewY']) {
            transform[method](12.345678901);
            if (transform.angle !== Math.fround(12.345678901)) return method;
        }
        const snapshot = () => JSON.stringify([transform.type,transform.angle,...['a','b','c','d','e','f'].map(key => transform.matrix[key])]);
        for (const [method, args] of [['setTranslate',[1,1e39]], ['setScale',[1e39,1]], ['setRotate',[1,1e39,0]], ['setSkewX',[1e39]], ['setSkewY',[1e39]]]) {
            const before = snapshot();
            try { transform[method](...args); return 'overflow accepted'; }
            catch (error) { if (!(error instanceof TypeError)) throw error; }
            if (snapshot() !== before) return 'mutation before conversion';
        }
        const sentinel = {};
        try { transform.setTranslate(1,{valueOf(){throw sentinel;}}); return 'conversion accepted'; }
        catch (error) { if (error !== sentinel) throw error; }
        return true;
    })()"#).unwrap(), "true");
}
