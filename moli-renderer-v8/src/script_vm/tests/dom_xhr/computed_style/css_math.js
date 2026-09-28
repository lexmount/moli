(() => {
    'use strict';
    const check = (ok, label) => { if (!ok) throw new Error(label); };
    const equal = (a, b, label) => check(Object.is(a, b), label + ': ' + a + ' / ' + b);
    const throws = (f, C = TypeError) => {
        let caught;
        try { f(); } catch (e) { caught = e; }
        check(caught instanceof C, 'expected ' + C.name);
        return caught;
    };
    const type = (value, expected) => equal(JSON.stringify(value.type()), JSON.stringify(expected), 'numeric type');
    for (const C of [CSSMathValue, CSSNumericArray]) throws(() => new C());
    const p = CSS.px(2);
    const q = CSS.px(3);
    for (const [C, op] of [[CSSMathSum, 'sum'], [CSSMathProduct, 'product'], [CSSMathMin, 'min'], [CSSMathMax, 'max']]) {
        equal(C.length, 0, 'variadic constructor length');
        throws(() => C(1));
        const empty = throws(() => new C(), DOMException);
        equal(empty.name, 'SyntaxError', 'empty variadic argument list');
        const v = new C(p, q);
        check(v instanceof CSSMathValue && v instanceof CSSNumericValue && v instanceof CSSStyleValue, 'inheritance');
        equal(v.operator, op, 'operator');
        const a = v.values;
        check(a instanceof CSSNumericArray && !Array.isArray(a), 'native numeric array');
        equal(a, v.values, 'stable numeric array identity');
        equal(a.length, 2, 'length');
        equal(a[0], p, 'first operand identity');
        equal(a[1], q, 'second operand identity');
        equal(a[2], undefined, 'out of range');
        equal([...a][0], p, 'iterator identity');
        equal([...a.keys()].join(), '0,1', 'keys');
        equal([...a.entries()][1][1], q, 'entries');
        const calls = [];
        const receiver = {};
        a.forEach(function (value, i, object) { check(this === receiver && object === a, 'forEach arguments'); calls.push(value); }, receiver);
        equal(calls[1], q, 'forEach');
        throws(() => { v.operator = 'no'; });
        throws(() => { v.values = []; });
        throws(() => { a.length = 0; });
        equal(CSSNumericArray.prototype[Symbol.iterator], Array.prototype.values, 'value iterable intrinsic');
    }
    for (const C of [CSSMathNegate, CSSMathInvert]) {
        equal(C.length, 1, 'unary constructor length');
        throws(() => new C());
        equal(new C(p).value, p, 'unary operand identity');
        equal(new C('4').value.value, 4, 'numberish numeric conversion');
        for (const invalid of [Infinity, NaN, Symbol(), 1n, undefined]) throws(() => new C(invalid));
    }
    equal(CSSMathClamp.length, 3, 'clamp constructor length');
    throws(() => new CSSMathClamp(1, 2));
    const clamp = new CSSMathClamp(p, q, CSS.px(4));
    equal(clamp.lower, p, 'clamp lower identity');
    equal(clamp.value, q, 'clamp value identity');
    equal(String(clamp), 'clamp(2px, 3px, 4px)', 'clamp serialization');
    const mixed = new CSSMathSum(CSS.px(1), CSS.percent(2));
    type(mixed, {length: 1, percentHint: 'length'});
    type(new CSSMathInvert(mixed), {length: -1, percentHint: 'length'});
    type(new CSSMathProduct(CSS.px(1), CSS.s(2)), {length: 1, time: 1});
    type(new CSSMathProduct(CSS.px(1), new CSSMathInvert(CSS.em(1))), {});
    type(new CSSMathProduct(mixed, new CSSMathInvert(CSS.px(1))), {percentHint: 'length'});
    type(new CSSMathProduct(mixed, CSS.s(1)), {length: 1, percentHint: 'length', time: 1});
    type(new CSSMathProduct(CSS.px(1), CSS.deg(1), CSS.s(1), CSS.Hz(1), CSS.dpi(1), CSS.fr(1), CSS.percent(1)),
        {angle: 1, flex: 1, frequency: 1, length: 1, percent: 1, resolution: 1, time: 1});
    throws(() => new CSSMathProduct(mixed, new CSSMathSum(CSS.s(1), CSS.percent(2))));
    throws(() => new CSSMathSum(CSS.px(1), CSS.s(1)));
    throws(() => new CSSMathClamp(p, q, CSS.s(1)));
    for (const name of ['cap','rcap','cqmin','svw','dvh','lvmin','fr','kHz','dpcm']) {
        const expected = name === 'fr' ? {flex: 1} : name === 'kHz' ? {frequency: 1} : name === 'dpcm' ? {resolution: 1} : {length: 1};
        type(CSS[name](1), expected);
    }
    const token = {};
    let converted = [];
    const number = n => ({valueOf() { converted.push(n); return n; }});
    new CSSMathClamp(number(1), number(2), number(3));
    equal(converted.join(), '1,2,3', 'conversion order');
    try { new CSSMathSum(p, CSS.s(1), {valueOf() { throw token; }}); throw new Error('missing conversion exception'); }
    catch (e) { equal(e, token, 'all conversions precede type validation'); }
    const sum = new CSSMathSum(p, q);
    p.value = 7;
    equal(String(sum), 'calc(7px + 3px)', 'live operand value');
    for (const key of ['value', 'unit', 'toString', Symbol.toPrimitive]) Object.defineProperty(p, key, {get() { throw token; }, configurable: true});
    Object.defineProperty(sum.values, Symbol.iterator, {get() { throw token; }});
    Object.defineProperty(sum, 'values', {get() { throw token; }});
    equal(String(sum), 'calc(7px + 3px)', 'serialization reads native slots');
    type(sum, {length: 1});
    Object.defineProperty(Object.prototype, 'length', {set() { throw token; }, configurable: true});
    try { type(sum, {length: 1}); } finally { delete Object.prototype.length; }
    const w = document.querySelector('iframe').contentWindow;
    const foreign = new w.CSSMathSum(w.CSS.px(5), w.CSS.px(8));
    const getValues = Object.getOwnPropertyDescriptor(CSSMathSum.prototype, 'values').get;
    check(getValues.call(foreign) instanceof w.CSSNumericArray, 'array belongs to owner realm');
    equal(getValues.call(foreign)[0], foreign.values[0], 'foreign child identity');
    equal(CSSStyleValue.prototype.toString.call(foreign), 'calc(5px + 8px)', 'cross realm native stringifier');
    check(CSSNumericValue.prototype.type.call(foreign) instanceof Object, 'dictionary belongs to callee realm');
    check(!(CSSNumericValue.prototype.type.call(foreign) instanceof w.Object), 'dictionary does not use receiver realm');
    const proxy = Proxy.revocable(sum, {}); proxy.revoke();
    for (const bad of [{}, Object.create(sum), Object.create(CSSMathSum.prototype), new Proxy(sum, {}), proxy.proxy]) {
        throws(() => getValues.call(bad));
        throws(() => CSSNumericValue.prototype.type.call(bad));
        throws(() => w.CSSNumericValue.prototype.type.call(bad), w.TypeError);
    }
    let traps = 0;
    const author = new Proxy(CSS.px(1), {get(target, key) { traps++; if (key === Symbol.toPrimitive) return () => 9; return Reflect.get(target,key); }});
    equal(new CSSMathSum(author).values[0].value, 9, 'unbranded objects use numeric union conversion');
    equal(traps, 1, 'one numeric conversion');
    class SubSum extends CSSMathSum {}
    const subclass = new SubSum(1, 2);
    check(subclass instanceof SubSum && subclass.values[0] instanceof CSSUnitValue, 'subclass constructor');
    w.CSSNumericArray = function () { throw token; };
    w.CSSUnitValue = function () { throw token; };
    const afterTamper = new w.CSSMathSum(6, 7);
    equal(afterTamper.values[0].value, 6, 'intrinsic creation bypasses public constructors');
    return true;
})()
