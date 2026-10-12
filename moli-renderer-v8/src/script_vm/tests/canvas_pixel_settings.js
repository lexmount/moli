(owner, callee, kind, label) => {
    const checks = [];
    const check = (name, run) => {
        try { checks.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { checks.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(3, 2);
    canvas.width = 3; canvas.height = 2;
    const context = canvas.getContext('2d');
    const proto = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
    const create = (...args) => Reflect.apply(proto.createImageData, context, args);
    const get = (...args) => Reflect.apply(proto.getImageData, context, args);
    const put = (...args) => Reflect.apply(proto.putImageData, context, args);
    const same = (actual, expected) => actual.length === expected.length && actual.every((v, i) => v === expected[i]);
    const near = (actual, expected, epsilon) => actual.length === expected.length && actual.every((v, i) => Math.abs(v-expected[i]) <= epsilon);
    const valid = (image, colorSpace, pixelFormat) => image instanceof owner.ImageData && image.colorSpace === colorSpace &&
        image.pixelFormat === pixelFormat && image.data instanceof owner[pixelFormat === 'rgba-float16' ? 'Float16Array' : 'Uint8ClampedArray'];
    for (const colorSpace of ['srgb', 'display-p3']) for (const pixelFormat of ['rgba-unorm8', 'rgba-float16']) {
        const settings = {colorSpace, pixelFormat};
        check('create metadata '+colorSpace+' '+pixelFormat, () => {
            const image = create(-2, -1, settings);
            return valid(image, colorSpace, pixelFormat) && image.width === 2 && image.height === 1 &&
                image.data.length === 8 && image.data.every(v => v === 0);
        });
        check('get metadata '+colorSpace+' '+pixelFormat, () => {
            const image = get(2, 1, -2, -1, settings);
            return valid(image, colorSpace, pixelFormat) && image.width === 2 && image.height === 1 && image.data.length === 8;
        });
        check('source native settings '+colorSpace+' '+pixelFormat, () => {
            const source = new owner.ImageData(2, 1, settings);
            source.data.fill(1);
            for (const name of ['width', 'height', 'colorSpace', 'pixelFormat', 'data'])
                Object.defineProperty(source, name, {get() { throw Error('author '+name); }});
            const image = create(source);
            return valid(image, colorSpace, pixelFormat) && image.width === 2 && image.height === 1 && image.data.every(v => v === 0);
        });
        check('detached source keeps settings '+colorSpace+' '+pixelFormat, () => {
            const source = new owner.ImageData(2, 1, settings), buffer = source.data.buffer;
            owner.structuredClone(buffer, {transfer: [buffer]});
            const image = create(source);
            return valid(image, colorSpace, pixelFormat) && image.data.length === 8 && image.data.every(v => v === 0);
        });
    }
    for (const name of ['createImageData', 'getImageData']) {
        const operation = settings => name === 'createImageData' ? create(1, 1, settings) : get(0, 0, 1, 1, settings);
        for (const settings of [undefined, null, {}]) check(name+' default '+String(settings), () => valid(operation(settings), 'srgb', 'rgba-unorm8'));
        for (const member of ['colorSpace', 'pixelFormat']) for (const value of ['', 'invalid', Symbol('invalid')]) {
            check(name+' invalid '+member+' '+String(value), () => {
                try { operation({[member]: value}); } catch (error) { return error instanceof callee.TypeError; }
                return false;
            });
        }
        for (const value of [1, true, 'settings', Symbol('settings')]) check(name+' dictionary '+String(value), () => {
            try { operation(value); } catch (error) { return error instanceof callee.TypeError; }
            return false;
        });
        check(name+' dictionary conversion order', () => {
            const log = [], number = n => ({valueOf() { log.push(String(n)); return n; }});
            const settings = {get colorSpace() { log.push('colorSpace'); return {toString() { log.push('space-string'); return 'display-p3'; }}; },
                get pixelFormat() { log.push('pixelFormat'); return {toString() { log.push('format-string'); return 'rgba-float16'; }}; }};
            const numbers = name === 'createImageData' ? [1, 2] : [0, 0, 1, 2];
            const image = Reflect.apply(proto[name], context, [...numbers.map(number), settings]);
            return valid(image, 'display-p3', 'rgba-float16') && same(log, [...numbers.map(String), 'colorSpace', 'space-string', 'pixelFormat', 'format-string']);
        });
        check(name+' settings before zero-size validation', () => {
            const args = name === 'createImageData' ? [0, 1] : [0, 0, 0, 1];
            try { Reflect.apply(proto[name], context, [...args, {pixelFormat: 'invalid'}]); }
            catch (error) { return error instanceof callee.TypeError; }
            return false;
        });
        check(name+' getter error identity', () => {
            const sentinel = new owner.Error('settings getter');
            try { operation({get colorSpace() { throw sentinel; }}); } catch (error) { return error === sentinel; }
            return false;
        });
        check(name+' number failure stops settings', () => {
            let hooks = 0;
            const settings = {get colorSpace() { hooks++; return 'srgb'; }};
            const args = name === 'createImageData' ? [Symbol(), 1] : [Symbol(), 0, 1, 1];
            try { Reflect.apply(proto[name], context, [...args, settings]); } catch (error) { return error instanceof callee.TypeError && hooks === 0; }
            return false;
        });
        check(name+' native allocation ignores global constructors', () => {
            const saved = [owner.ImageData, owner.Float16Array, owner.Uint8ClampedArray];
            let image;
            try {
                owner.ImageData = owner.Float16Array = owner.Uint8ClampedArray = () => { throw Error('global constructor'); };
                image = operation({pixelFormat: 'rgba-float16'});
            } finally { [owner.ImageData, owner.Float16Array, owner.Uint8ClampedArray] = saved; }
            return valid(image, 'srgb', 'rgba-float16');
        });
        check(name+' receiver checked before settings', () => {
            let calls = 0;
            const settings = {get colorSpace() { calls++; return 'srgb'; }};
            const args = name === 'createImageData' ? [1, 1] : [0, 0, 1, 1];
            try { Reflect.apply(proto[name], new owner.Proxy(context, {}), [...args, settings]); }
            catch (error) { return error instanceof callee.TypeError && calls === 0; }
            return false;
        });
    }
    check('float16 normalized readback', () => {
        canvas.width = 256; canvas.height = 1;
        const source = new owner.ImageData(256, 1);
        for (let i = 0; i < 256; i++) source.data.set([i, i, i, 255], i*4);
        put(source, 0, 0);
        const image = get(0, 0, 256, 1, {pixelFormat: 'rgba-float16'});
        if (!valid(image, 'srgb', 'rgba-float16')) return false;
        for (let i = 0; i < 256; i++) if (!near(Array.from(image.data.slice(i*4, i*4+4)), [i/255, i/255, i/255, 1], 0.00025)) return false;
        const again = create(source);
        put(image, 0, 0);
        return again.data.every(v => v === 0) && same(Array.from(get(0, 0, 256, 1).data), Array.from(source.data));
    });
    check('readback converts sRGB into Display-P3', () => {
        canvas.width = 3; canvas.height = 2;
        put(new owner.ImageData(new owner.Uint8ClampedArray([255, 0, 0, 255]), 1, 1), 0, 0);
        const bytes = get(0, 0, 1, 1, {colorSpace: 'display-p3'});
        const floats = get(0, 0, 1, 1, {colorSpace: 'display-p3', pixelFormat: 'rgba-float16'});
        // CSS Color 4 relative-colorimetric sRGB red expressed in Display-P3.
        return near(Array.from(bytes.data), [234, 51, 35, 255], 1) &&
            near(Array.from(floats.data), [0.91749, 0.20029, 0.13856, 1], 0.001);
    });
    check('write converts Display-P3 before 8-bit quantization', () => {
        const storage = new owner.Float16Array(12);
        storage.set([0.91748046875, 0.2003173828125, 0.1385498046875, 1], 4);
        const source = new owner.ImageData(storage.subarray(4, 8), 1, 1, {colorSpace: 'display-p3', pixelFormat: 'rgba-float16'});
        put(source, 0, 0);
        return near(Array.from(get(0, 0, 1, 1).data), [255, 0, 0, 255], 1) && source.data.buffer === storage.buffer;
    });
    check('float16 readback pads outside pixels', () => {
        const image = get(-1, -1, 2, 2, {pixelFormat: 'rgba-float16'});
        return image.data.length === 16 && image.data.slice(0, 12).every(v => v === 0) && near(Array.from(image.data.slice(12)), [1, 0, 0, 1], 1/255);
    });
    return checks;
}
