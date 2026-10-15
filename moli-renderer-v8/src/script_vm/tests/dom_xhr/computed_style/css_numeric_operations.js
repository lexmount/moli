(() => {
    'use strict';
    const check = (ok, label) => { if (!ok) throw new Error(label); };
    const eq = (a, b, label) => check(Object.is(a, b), label);
    const close = (a, b, label) => check(Math.abs(a - b) <= 1e-12 * Math.max(1, Math.abs(b)), label);
    const throws = (fn, C = TypeError) => {
        let caught;
        try { fn(); } catch (e) { caught = e; }
        check(caught instanceof C, `expected ${C.name}`);
        return caught;
    };
    const methods = ['add', 'sub', 'mul', 'div', 'min', 'max', 'equals', 'to', 'toSum'];
    for (const name of methods) {
        const desc = Object.getOwnPropertyDescriptor(CSSNumericValue.prototype, name);
        check(desc.enumerable && desc.writable && desc.configurable, `${name} descriptor`);
        eq(desc.value.length, name === 'to' ? 1 : 0, `${name} length`);
    }
    eq(CSS.number(2 ** 53).add(1, -(2 ** 53)).value, 0, 'addition is left to right');
    eq(CSS.number(1e-200).mul(1e-200, 1e200).value, 0, 'multiplication is left to right');
    eq(CSS.px(16777217).add(CSS.px(2)).value, 16777219, 'no f32 rounding');
    eq(CSS.px(16777217).to('px').value, 16777217, 'conversion retains double precision');
    const leaf = CSS.px(3), em = CSS.em(2);
    const sum = new CSSMathSum(leaf, em);
    const appended = sum.add(CSS.percent(4));
    check(appended !== sum && appended.values[0] === leaf && appended.values[1] === em, 'flatten receiver, retain children');
    eq(leaf.add(sum).values[1], sum, 'do not flatten argument');
    eq(CSS.px(1).sub(new CSSMathNegate(em)).values[1], em, 'unwrap native negate');
    eq(CSS.px(1).div(new CSSMathInvert(em)).values[1], em, 'unwrap native inverse');
    const clone = leaf.add();
    check(clone !== leaf, 'unit result is fresh');
    leaf.value = 7;
    eq(clone.value, 3, 'arithmetic result is a snapshot');
    check(sum.equals(new CSSMathSum(CSS.px(7), CSS.em(2))), 'structural equality');
    check(!sum.equals(new CSSMathSum(CSS.em(2), CSS.px(7))), 'equality preserves order');
    check(!CSS.in(1).equals(CSS.px(96)), 'equality does not convert units');
    check(sum.equals() && CSS.number(5).equals('5', 5), 'empty and numberish equality');
    const sentinel = {};
    const failNumber = {valueOf() { throw sentinel; }};
    const failString = {toString() { throw sentinel; }};
    for (const fn of [
        () => CSS.number(1).div(0, failNumber),
        () => CSS.number(1).add(CSS.px(1), failNumber),
        () => CSS.number(1).equals(2, failNumber),
        () => CSS.px(1).toSum('invalid', failString),
    ]) {
        let caught;
        try { fn(); } catch (e) { caught = e; }
        eq(caught, sentinel, 'convert all arguments before native operation');
    }
    const mutable = CSS.number(1), order = [];
    eq(mutable.add(
        {valueOf() { order.push(1); mutable.value = 4; return 2; }},
        {valueOf() { order.push(2); mutable.value = 5; return 3; }},
    ).value, 10, 'read mutable slots after conversion');
    eq(order.join(), '1,2', 'WebIDL conversion order');
    for (const zero of [0, -0]) throws(() => CSS.number(1).div(zero), RangeError);
    eq(throws(() => leaf.to('lemon'), DOMException).name, 'SyntaxError', 'invalid unit');
    throws(() => leaf.to());
    eq(leaf.to('PX').unit, 'px', 'unit normalization');
    eq(CSS.in(1).to('px').value, 96, 'absolute length');
    close(CSS.Q(40).to('cm').value, 1, 'quarter millimetres');
    close(CSS.rad(Math.PI).to('deg').value, 180, 'radians');
    eq(CSS.kHz(1).to('hz').value, 1000, 'frequency');
    eq(CSS.ms(1000).to('s').value, 1, 'time');
    eq(CSS.dpi(96).to('dppx').value, 1, 'resolution');
    close(CSS.dpcm(1).to('dpi').value, 2.54, 'resolution ratio');
    throws(() => CSS.em(1).to('px'));
    throws(() => CSS.vw(1).to('vh'));
    throws(() => CSS.percent(1).to('number'));
    const distributed = new CSSMathProduct(new CSSMathSum(CSS.px(1), CSS.em(2)), 3).toSum('em', 'px');
    eq(distributed.values[0].value, 6, 'distributive product em');
    eq(distributed.values[1].value, 3, 'distributive product px');
    const greedy = new CSSMathSum(CSS.cm(1), CSS.mm(10)).toSum('mm', 'cm', 'mm');
    close(greedy.values[0].value, 20, 'greedy conversion');
    eq(greedy.values[1].value, 0, 'unconsumed compatible unit');
    eq(greedy.values[2].value, 0, 'duplicate requested unit');
    eq([...sum.toSum().values].map(v => v.unit).join(), 'em,px', 'sort unit names');
    throws(() => sum.toSum('px'));
    throws(() => leaf.toSum('px', 's'));
    const opaque = new CSSMathSum(CSS.px(1), CSS.em(2));
    const unit = opaque.values[0];
    for (const [object, keys] of [[unit, ['value','unit','toString',Symbol.toPrimitive]], [opaque, ['values','operator','constructor']]]) {
        for (const key of keys) Object.defineProperty(object, key, {get() { throw sentinel; }});
    }
    check(opaque.equals(new CSSMathSum(CSS.px(1), CSS.em(2))), 'comparison uses native slots');
    eq(opaque.toSum('px','em').values[0].value, 1, 'conversion uses native slots');
    eq(opaque.add(CSS.percent(1)).values[0], unit, 'arithmetic uses native slots');
    let proxyReads = 0;
    const numberProxy = new Proxy(CSS.px(3), {get(target, key) { proxyReads++; if (key === Symbol.toPrimitive) return () => 4; throw sentinel; }});
    eq(CSS.number(2).add(numberProxy).value, 6, 'author Proxy uses numeric union');
    eq(proxyReads, 1, 'single ToNumber hook');
    const realms = [globalThis];
    if (typeof document !== 'undefined') realms.push(document.querySelector('iframe').contentWindow);
    for (const realm of realms) {
        throws(() => realm.CSSNumericValue.prototype.div.call(CSS.number(1), 0), realm.RangeError);
        throws(() => realm.CSSNumericValue.prototype.add.call(CSS.px(1), CSS.s(1)), realm.TypeError);
        eq(throws(() => realm.CSSNumericValue.prototype.to.call(CSS.px(1), 'lemon'), realm.DOMException).name, 'SyntaxError', 'callee exception realm');
        const receiver = CSS.px(1);
        for (const name of methods) {
            const method = realm.CSSNumericValue.prototype[name];
            const revoked = Proxy.revocable(receiver, {}); revoked.revoke();
            let conversions = 0, traps = 0;
            const badArgument = {valueOf() { conversions++; throw sentinel; }, toString() { conversions++; throw sentinel; }};
            for (const bad of [{}, Object.create(receiver), Object.create(CSSNumericValue.prototype), new Proxy(receiver, {get() { traps++; throw sentinel; }}), revoked.proxy]) {
                throws(() => method.call(bad, badArgument), realm.TypeError);
            }
            eq(conversions, 0, `${name} brand check precedes conversion`);
            eq(traps, 0, `${name} brand check ignores Proxy traps`);
            for (const owner of realms) {
                const receiver = owner.CSS.number(1);
                const arg = name === 'to' || name === 'toSum' ? 'number' : 1;
                const good = method.call(receiver, arg);
                if (name !== 'equals') check(good instanceof (name === 'toSum' ? owner.CSSMathSum : owner.CSSUnitValue), `${name} receiver realm`);
                const U = owner.CSSUnitValue, S = owner.CSSMathSum;
                owner.CSSUnitValue = owner.CSSMathSum = function () { throw sentinel; };
                try {
                    check(realm.CSSNumericValue.prototype.add.call(owner.CSS.px(1), CSS.percent(1)) instanceof S, 'intrinsic math constructor');
                    check(realm.CSSNumericValue.prototype.to.call(receiver, 'number') instanceof U, 'intrinsic unit constructor');
                } finally { owner.CSSUnitValue = U; owner.CSSMathSum = S; }
                const product = realm.CSSNumericValue.prototype.mul.call(new owner.CSSMathProduct(owner.CSS.px(1), owner.CSS.px(1)), 2);
                check(product instanceof owner.CSSMathProduct && product.values[2] instanceof U, 'rectified argument realm');
                const inverse = realm.CSSNumericValue.prototype.div.call(owner.CSS.px(1), realm.CSS.em(2));
                check(inverse.values[1] instanceof owner.CSSMathInvert, 'inverse realm');
                const converted = realm.CSSNumericValue.prototype.toSum.call(owner.CSS.px(1));
                check(converted.values[0] instanceof U, 'converted child realm');
            }
        }
    }
    return true;
})()
