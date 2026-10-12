(sourceRealm, realm, label) => {
    const checks = [];
    const C = realm.ImageData;
    const check = (name, run) => {
        try { checks.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { checks.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const construct = (...args) => Reflect.construct(C, args);
    const throws = (kind, run) => {
        try { run(); } catch (error) {
            return kind === 'TypeError' || kind === 'RangeError'
                ? error instanceof realm[kind]
                : error.name === kind && error instanceof realm.DOMException;
        }
        return false;
    };
    const same = (a, b) => a.length === b.length && a.every((v, i) => Object.is(v, b[i]));
    check('constructor length', () => C.length === 2);
    check('constructor requires new', () => throws('TypeError', () => C(1, 1)));
    for (const args of [[], [1], [new sourceRealm.Uint8ClampedArray(4)]]) {
        check('required arity ' + args.length + '-' + typeof args[0], () => throws('TypeError', () => Reflect.construct(C, args)));
    }
    check('required arity precedes conversion', () => {
        let calls = 0;
        const rejected = throws('TypeError', () => construct({valueOf() { calls++; return 1; }}));
        return rejected && calls === 0;
    });
    for (const [width, height, expectedWidth, expectedHeight] of [
        [1, 2, 1, 2], [2.9, 3.9, 2, 3], [4294967298, 4294967299, 2, 3],
        [-4294967295, -4294967294, 1, 2], ['2', true, 2, 1]
    ]) check('unsigned dimensions ' + String(width), () => {
        const image = construct(width, height);
        return image.width === expectedWidth && image.height === expectedHeight &&
            image.data instanceof realm.Uint8ClampedArray && image.data.length === expectedWidth * expectedHeight * 4 &&
            image.data.every(v => v === 0);
    });
    for (const [index, value] of [0, -0, 0.9, -0.9, NaN, Infinity, -Infinity, 4294967296, undefined, null, 'bad'].entries()) {
        check('zero converted width ' + index, () => throws('IndexSizeError', () => construct(value, 2)));
        check('zero converted height ' + index, () => throws('IndexSizeError', () => construct(2, value)));
    }
    for (const value of [Symbol('size'), 1n]) check('invalid numeric ' + typeof value, () => throws('TypeError', () => construct(1, value)));
    check('dimension and settings conversion order before size check', () => {
        const order = [];
        const rejected = throws('IndexSizeError', () => construct(
            {valueOf() { order.push('width'); return 0; }},
            {valueOf() { order.push('height'); return 1; }},
            {get colorSpace() { order.push('colorSpace'); return 'srgb'; }, get pixelFormat() { order.push('pixelFormat'); return 'rgba-unorm8'; }}));
        return rejected && order.join(',') === 'width,height,colorSpace,pixelFormat';
    });
    check('dictionary exception identity before size check', () => {
        const marker = {};
        try { construct(0, 0, {get colorSpace() { throw marker; }}); }
        catch (error) { return error === marker; }
        return false;
    });
    check('pixelFormat exception identity', () => {
        const marker = {};
        try { construct(1, 1, {get pixelFormat() { throw marker; }}); }
        catch (error) { return error === marker; }
        return false;
    });
    for (const settings of [null, undefined, {}]) check('default settings ' + String(settings), () => {
        const image = construct(1, 1, settings);
        return image.colorSpace === 'srgb' && image.pixelFormat === 'rgba-unorm8';
    });
    for (const field of ['colorSpace', 'pixelFormat']) {
        for (const value of ['invalid', null, Symbol(field)]) check('invalid ' + field + ' ' + String(value), () =>
            throws('TypeError', () => construct(1, 1, {[field]: value})));
    }
    check('four arguments select data overload before numeric conversion', () => {
        let calls = 0;
        const rejected = throws('TypeError', () => construct({valueOf() { calls++; return 1; }}, 1, {}, {}));
        return rejected && calls === 0;
    });
    for (const [type, format, bytesPerElement] of [
        ['Uint8ClampedArray', 'rgba-unorm8', 1], ['Float16Array', 'rgba-float16', 2]
    ]) {
        const settings = {colorSpace: 'display-p3', pixelFormat: format};
        check(type + ' allocated storage', () => {
            const image = construct(3, 2, settings);
            return image.pixelFormat === format && image.colorSpace === 'display-p3' && image.data instanceof realm[type] &&
                image.data.length === 24 && image.data.byteOffset === 0 && image.data.buffer.byteLength === 24 * bytesPerElement && image.data.every(v => v === 0);
        });
        for (const explicitHeight of [false, true]) check(type + ' native subview identity ' + explicitHeight, () => {
            const buffer = new sourceRealm.ArrayBuffer(40 * bytesPerElement);
            const data = new sourceRealm[type](buffer, 4 * bytesPerElement, 24);
            const image = construct(data, 3, explicitHeight ? 2 : undefined, settings);
            data[1] = type === 'Float16Array' ? -0.5 : 7;
            return image.width === 3 && image.height === 2 && image.data === data && image.data[1] === data[1] &&
                image.data.buffer === buffer && image.data.byteOffset === 4 * bytesPerElement && image.pixelFormat === format;
        });
        for (const length of [0, 1, 3, 5, 7]) check(type + ' invalid element count ' + length, () =>
            throws('InvalidStateError', () => construct(new sourceRealm[type](length), 0, undefined, settings)));
        for (const [width, height] of [[0, undefined], [3, undefined], [1, 0], [1, 1], [2, 2]]) {
            check(type + ' inconsistent dimensions ' + width + '-' + height, () =>
                throws('IndexSizeError', () => construct(new sourceRealm[type](8), width, height, settings)));
        }
        check(type + ' mismatched format', () => throws('InvalidStateError', () =>
            construct(new sourceRealm[type](8), 1, undefined, {pixelFormat: type === 'Float16Array' ? 'rgba-unorm8' : 'rgba-float16'})));
        check(type + ' settings precede invalid length', () => {
            let calls = 0;
            const rejected = throws('InvalidStateError', () => construct(new sourceRealm[type](0), 0, undefined,
                {get pixelFormat() { calls++; return format; }}));
            return rejected && calls === 1;
        });
        check(type + ' dimension conversion can detach data', () => {
            const data = new sourceRealm[type](4), order = [];
            const rejected = throws('InvalidStateError', () => construct(data, {valueOf() {
                order.push('width'); sourceRealm.structuredClone(data.buffer, {transfer: [data.buffer]}); return 1;
            }}, undefined, {get pixelFormat() { order.push('pixelFormat'); return format; }}));
            return rejected && order.join(',') === 'width,pixelFormat';
        });
        check(type + ' dictionary conversion can detach data', () => {
            const data = new sourceRealm[type](4);
            return throws('InvalidStateError', () => construct(data, 1, 1, {get pixelFormat() {
                sourceRealm.structuredClone(data.buffer, {transfer: [data.buffer]}); return format;
            }}));
        });
        for (const detach of [false, true]) check(type + ' rejects resizable buffer before conversion ' + detach, () => {
            const buffer = new sourceRealm.ArrayBuffer(8 * bytesPerElement, {maxByteLength: 16 * bytesPerElement});
            const data = new sourceRealm[type](buffer);
            if (detach) sourceRealm.structuredClone(buffer, {transfer: [buffer]});
            let calls = 0;
            const rejected = throws('TypeError', () => construct(data, {valueOf() { calls++; return 1; }}, undefined, settings));
            return rejected && calls === 0;
        });
        check(type + ' rejects shared buffer before conversion', () => {
            const buffer = new sourceRealm.WebAssembly.Memory({initial: 1, maximum: 1, shared: true}).buffer;
            const data = new sourceRealm[type](buffer, 0, 4);
            let calls = 0;
            const rejected = throws('TypeError', () => construct(data, {valueOf() { calls++; return 1; }}, undefined, settings));
            return rejected && calls === 0;
        });
        check(type + ' clone preserves format pixels and independence', () => {
            const data = new sourceRealm[type](type === 'Float16Array' ? [-1, 0.5, 1024, 1] : [1, 127, 255, 17]);
            const image = construct(data, 1, 1, settings);
            const copy = realm.structuredClone(image);
            return copy instanceof C && copy.width === 1 && copy.height === 1 && copy.colorSpace === 'display-p3' &&
                copy.pixelFormat === format && copy.data instanceof realm[type] && copy.data !== data && copy.data.buffer !== data.buffer &&
                same(Array.from(copy.data), Array.from(data));
        });
        for (const first of ['image', 'data']) check(type + ' clone graph identity ' + first, () => {
            const data = new sourceRealm[type](new sourceRealm.ArrayBuffer(12 * bytesPerElement), 4 * bytesPerElement, 4);
            const image = construct(data, 1, 1, settings);
            const object = first === 'image' ? {image, data, buffer: data.buffer} : {data, buffer: data.buffer, image};
            object.again = image;
            const copy = realm.structuredClone(object);
            return copy.image === copy.again && copy.image.data === copy.data && copy.data.buffer === copy.buffer &&
                copy.data.byteOffset === 4 * bytesPerElement && copy.buffer.byteLength === 12 * bytesPerElement;
        });
        check(type + ' clone transferred backing buffer graph', () => {
            const data = new sourceRealm[type]([1, 0, 1, 1]);
            const image = construct(data, 1, 1, settings);
            const copy = realm.structuredClone({image, data, buffer: data.buffer}, {transfer: [data.buffer]});
            return data.byteLength === 0 && image.width === 1 && copy.image.data === copy.data && copy.data.buffer === copy.buffer &&
                same(Array.from(copy.data), [1, 0, 1, 1]);
        });
        check(type + ' detached data cannot clone', () => {
            const data = new sourceRealm[type](4), image = construct(data, 1, 1, settings);
            sourceRealm.structuredClone(data.buffer, {transfer: [data.buffer]});
            return throws('DataCloneError', () => realm.structuredClone(image));
        });
    }
    const image = construct(1, 1);
    for (const attribute of ['width', 'height', 'data', 'colorSpace', 'pixelFormat']) {
        const getter = Object.getOwnPropertyDescriptor(C.prototype, attribute).get;
        const revoked = sourceRealm.Proxy.revocable(image, {}); revoked.revoke();
        for (const [index, receiver] of [null, undefined, {}, Object.create(C.prototype), Object.create(image), new sourceRealm.Proxy(image, {}), revoked.proxy].entries()) {
            check(attribute + ' rejects receiver ' + index, () => throws('TypeError', () => Reflect.apply(getter, receiver, [])));
        }
    }
    check('native dimensions ignore author properties', () => {
        const data = new sourceRealm.Uint8ClampedArray(8);
        for (const key of ['byteLength', 'length', 'buffer']) Object.defineProperty(data, key, {get() { throw Error('author ' + key); }});
        const result = construct(data, 2);
        return result.width === 2 && result.height === 1 && result.data === data;
    });
    check('subclass initializes native ImageData state', () => {
        class Child extends C {}
        const result = new Child(1, 2);
        return result instanceof Child && result instanceof C && result.width === 1 && result.height === 2;
    });
    check('clone ignores poisoned globals and instance accessors', () => {
        const image = construct(new sourceRealm.Uint8ClampedArray([1, 2, 3, 4]), 1, 1);
        for (const key of ['width', 'height', 'data', 'colorSpace', 'pixelFormat']) Object.defineProperty(image, key, {get() { throw Error('author ' + key); }});
        const original = realm.ImageData;
        try {
            realm.ImageData = function() { throw Error('author constructor'); };
            const copy = realm.structuredClone(image);
            return copy instanceof C && copy.width === 1 && same(Array.from(copy.data), [1, 2, 3, 4]);
        } finally { realm.ImageData = original; }
    });
    return checks;
}
