(() => {
    'use strict';
    const check = (condition, message) => { if (!condition) throw new Error(message); };
    const throws = (fn, C = TypeError) => {
        try { fn(); } catch (error) { check(error instanceof C, String(error)); return; }
        throw new Error('expected ' + C.name);
    };
    const matrix = (actual, expected) => {
        check(actual instanceof DOMMatrix, 'native matrix result');
        const a = actual.toFloat64Array(), b = expected.toFloat64Array();
        check(a.every((v, i) => Math.abs(v - b[i]) < 1e-10), 'matrix components');
    };
    throws(() => new CSSTransformComponent());
    throws(() => CSSScale(1, 2));
    throws(() => new CSSTransformValue([]));
    throws(() => new CSSRotate(CSS.deg(1), 2));
    const x = CSS.cm(2.54), y = CSS.px(12), z = CSS.px(3);
    const t = new CSSTranslate(x, y, z);
    check(t.x === x && t.z === z && !t.is2D, 'operand identity and dimensionality');
    throws(() => new CSSTranslate(CSS.em(3), y).toMatrix());
    t.is2D = true;
    matrix(t.toMatrix(), new DOMMatrix().translate(96, 12));
    check(t.z === z && String(t) === 'translate(2.54cm, 12px)', 'flatten without mutating operands');
    throws(() => t.x = CSS.s(1));
    throws(() => t.z = CSS.percent(0));
    check(t.x === x && t.z === z, 'failed setters are atomic');
    x.value = 5.08;
    check(t.toMatrix().e === 192, 'live operand mutation');
    const mixed = new CSSTranslate(new CSSMathSum(CSS.px(1), CSS.percent(2)), CSS.px(0));
    throws(() => mixed.toMatrix());
    throws(() => new CSSTranslate(CSS.number(0), y));
    throws(() => new CSSScale(CSS.percent(2), 1));
    const scale = new CSSScale(new CSSMathProduct(CSS.px(6), new CSSMathInvert(CSS.px(2))), 2);
    matrix(scale.toMatrix(), new DOMMatrix().scale(3, 2));
    const rotate = new CSSRotate(1, 2, 3, CSS.turn(.25));
    matrix(rotate.toMatrix(), new DOMMatrix().rotateAxisAngle(1, 2, 3, 90));
    rotate.is2D = true;
    matrix(rotate.toMatrix(), new DOMMatrix().rotate(90));
    for (const skew of [new CSSSkew(CSS.deg(0), CSS.deg(0)), new CSSSkewX(CSS.deg(0)), new CSSSkewY(CSS.deg(0))]) {
        skew.is2D = false;
        check(skew.is2D, 'skew dimensions are fixed');
        matrix(skew.toMatrix(), new DOMMatrix());
    }
    const perspective = new CSSPerspective('none');
    perspective.is2D = true;
    check(!perspective.is2D && perspective.length instanceof CSSKeywordValue, 'perspective keyword');
    matrix(perspective.toMatrix(), new DOMMatrix());
    throws(() => perspective.length = 'auto');
    check(String(new CSSPerspective(CSS.px(-1))) === 'perspective(calc(-1px))', 'negative perspective serialization');

    const source = new DOMMatrix([1, 2, 3, 4, 5, 6]);
    const component = new CSSMatrixComponent(source, { get is2D() { source.e = 7; return false; } });
    check(component.matrix !== source && component.matrix.e === 7 && !component.is2D, 'matrix snapshot follows options conversion');
    source.e = 9;
    check(component.matrix.e === 7, 'constructor copies matrix');
    component.matrix = source;
    check(component.matrix === source, 'setter retains matrix identity');
    throws(() => component.matrix = new DOMMatrixReadOnly());
    check(String(component).startsWith('matrix3d('), 'explicit matrix dimension in serialization');

    const list = new CSSTransformValue([t, scale]);
    check(list[0] === t && list.length === 2 && list.is2D, 'transform list identity');
    matrix(list.toMatrix(), new DOMMatrix().translate(192, 12).scale(3, 2));
    check(list.keys === Array.prototype.keys && list[Symbol.iterator] === Array.prototype.values, 'value iterable intrinsics');
    list[2] = rotate;
    check([...list.entries()][2][1] === rotate && [...list.values()].length === 3, 'append and iteration');
    throws(() => list[4] = scale, RangeError);
    throws(() => list[0] = {});
    const desc = Object.getOwnPropertyDescriptor(list, '0');
    check(desc.value === t && desc.writable && desc.enumerable && desc.configurable, 'indexed descriptor');
    check(!Reflect.deleteProperty(list, '0') && !Reflect.preventExtensions(list), 'indexed object invariants');
    throws(() => Object.defineProperty(list, '0', {get() {return scale;}}));
    check(Reflect.defineProperty(list, '0', {value: t}), 'indexed defineProperty');
    const derived = Object.create(list);
    derived[0] = scale;
    check(derived[0] === scale && list[0] === t && Object.hasOwn(derived, '0'), 'inherited indexed assignment');
    const log = [];
    const generator = (function* () {try {yield scale; yield {};} finally {log.push('closed');}})();
    throws(() => new CSSTransformValue(generator));
    check(log.length === 0 && generator.next().done && log.join() === 'closed', 'sequence conversion propagates without IteratorClose');

    let conversions = 0, traps = 0;
    const number = {valueOf() {conversions++; return 2;}};
    const setter = Object.getOwnPropertyDescriptor(CSSScale.prototype, 'x').set;
    const revoked = Proxy.revocable(scale, {}); revoked.revoke();
    for (const bad of [{}, Object.create(scale), Object.create(CSSScale.prototype), new Proxy(scale, {get() {traps++;}}), revoked.proxy]) {
        throws(() => setter.call(bad, number));
        throws(() => CSSTransformComponent.prototype.toMatrix.call(bad));
    }
    check(conversions === 0 && traps === 0, 'brand check before conversion and Proxy traps');
    const sentinel = {};
    try {new CSSScale(CSS.px(1), {valueOf() {throw sentinel;}}); throw new Error('no exception');}
    catch (error) {check(error === sentinel, 'IDL conversion before dimension validation');}

    const other = document.querySelector('iframe').contentWindow;
    const foreign = new other.CSSScale(2, 3);
    const result = CSSTransformComponent.prototype.toMatrix.call(foreign);
    check(result instanceof other.DOMMatrix && !(result instanceof DOMMatrix), 'result owner realm');
    throws(() => other.CSSTransformComponent.prototype.toMatrix.call({}), other.TypeError);
    check(new CSSTransformValue([foreign])[0] === foreign, 'cross realm component argument');
    const NativeMatrix = other.DOMMatrix;
    other.DOMMatrix = () => {throw new Error('author constructor');};
    check(foreign.toMatrix() instanceof NativeMatrix, 'intrinsic result constructor');
    other.DOMMatrix = NativeMatrix;
    Object.defineProperties(scale, {x: {get() {throw new Error('public x');}}, toMatrix: {value() {throw new Error('public matrix');}}});
    const protectedList = new CSSTransformValue([scale]);
    matrix(protectedList.toMatrix(), new DOMMatrix().scale(3, 2));
    check(String(protectedList).startsWith('scale('), 'native stringification');
    return true;
})()
