(owner, callee, kind, label) => {
    const rows = [];
    const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
    const check = (name, run) => {
        try { run(); rows.push({name: label + ': ' + name, passed: true}); }
        catch (error) { rows.push({name: label + ': ' + name, passed: false, error: String(error)}); }
    };
    const prototype = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
    const call = (context, name, ...args) => Reflect.apply(prototype[name], context, args);
    const make = (settings = {}) => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(32, 16);
        canvas.width = 32; canvas.height = 16;
        return [canvas, canvas.getContext('2d', settings)];
    };
    const pixel = (context, x = 8, y = 8) => Array.from(call(context, 'getImageData', x, y, 1, 1).data);
    const near = (actual, expected, tolerance = 3) => assert(actual.every((v, i) => Math.abs(v - expected[i]) <= tolerance), actual + ' != ' + expected);
    const background = context => {context.fillStyle = 'red'; call(context, 'fillRect', 0, 0, 32, 16);};
    check('shadow and source compose in separate xor steps', () => {
        const [, context] = make(); background(context);
        context.globalCompositeOperation = 'xor'; context.shadowColor = 'red'; context.shadowOffsetX = 16; context.fillStyle = 'lime';
        call(context, 'fillRect', -16, 0, 32, 16);
        near(pixel(context), [0, 255, 0, 255]); near(pixel(context, 24), [0, 0, 0, 0]);
    });
    check('blurred shadow composes before the source', () => {
        const [, context] = make(); background(context);
        context.globalCompositeOperation = 'xor'; context.shadowColor = 'red'; context.shadowBlur = 1; context.fillStyle = 'lime';
        call(context, 'fillRect', -10, -10, 52, 36); near(pixel(context), [0, 255, 0, 255]);
    });
    for (const drawing of ['fillRect', 'fill', 'stroke', 'strokeRect', 'drawImage']) {
        check(drawing + ' casts a shadow from outside the bitmap', () => {
            const [, context] = make(); context.shadowColor = 'blue'; context.shadowOffsetX = 8;
            context.fillStyle = context.strokeStyle = 'red'; context.lineWidth = 4;
            if (drawing === 'fillRect') call(context, drawing, -8, 0, 8, 16);
            else if (drawing === 'fill') {call(context, 'rect', -8, 0, 8, 16); call(context, 'fill');}
            else if (drawing === 'stroke') {call(context, 'moveTo', -4, 0); call(context, 'lineTo', -4, 16); call(context, 'stroke');}
            else if (drawing === 'strokeRect') call(context, drawing, -8, -8, 4, 32);
            else {
                const image = new owner.OffscreenCanvas(8, 16), source = image.getContext('2d');
                source.fillStyle = 'red'; source.fillRect(0, 0, 8, 16); call(context, 'drawImage', image, -8, 0);
            }
            near(pixel(context, 4), [0, 0, 255, 255]); near(pixel(context, 24), [0, 0, 0, 0]);
        });
    }
    for (const kind of ['solid', 'gradient', 'pattern', 'image']) {
        check(kind + ' alpha and globalAlpha feed the shadow once', () => {
            const [, context] = make(); context.shadowColor = 'rgba(0,0,255,0.5)'; context.shadowOffsetX = 8; context.globalAlpha = 0.5;
            let style = 'rgba(255,0,0,0.5)';
            if (kind === 'gradient') {
                style = call(context, 'createLinearGradient', -8, 0, 0, 0);
                style.addColorStop(0, 'rgba(255,0,0,0.5)'); style.addColorStop(1, 'rgba(255,0,0,0.5)');
            }
            if (kind === 'pattern' || kind === 'image') {
                const image = new owner.OffscreenCanvas(8, 16), source = image.getContext('2d');
                source.fillStyle = 'rgba(255,0,0,0.5)'; source.fillRect(0, 0, 8, 16);
                if (kind === 'image') {call(context, 'drawImage', image, -8, 0); near(pixel(context, 4), [0, 0, 255, 32]); return;}
                style = call(context, 'createPattern', image, 'repeat');
            }
            context.fillStyle = style; call(context, 'fillRect', -8, 0, 8, 16);
            near(pixel(context, 4), [0, 0, 255, 32]);
        });
    }
    check('final clip does not clip the shadow source', () => {
        const [, context] = make(); call(context, 'rect', 8, 0, 8, 16); call(context, 'clip');
        context.shadowColor = 'blue'; context.shadowOffsetX = 8; context.fillStyle = 'red';
        call(context, 'fillRect', 0, 0, 4, 16);
        near(pixel(context, 9), [0, 0, 255, 255]); near(pixel(context, 1), [0, 0, 0, 0]);
    });
    check('float16 image shadow avoids intermediate alpha quantization', () => {
        const [, context] = make({colorType: 'float16'});
        const image = new owner.OffscreenCanvas(8, 16), source = image.getContext('2d', {colorType: 'float16'});
        const values = new owner.Float16Array(8 * 16 * 4);
        for (let i = 0; i < values.length; i += 4) values.set([1, 0, 0, 0.5], i);
        source.putImageData(new owner.ImageData(values, 8, 16, {pixelFormat: 'rgba-float16'}), 0, 0);
        context.globalAlpha = 0.1; context.shadowColor = 'rgba(0,0,255,0.1)'; context.shadowOffsetX = 8;
        call(context, 'drawImage', image, -8, 0);
        const actual = call(context, 'getImageData', 4, 8, 1, 1, {pixelFormat: 'rgba-float16'}).data[3];
        const expected = new owner.Float16Array([0.005])[0];
        assert(actual === expected, actual + ' != ' + expected);
    });
    check('shadow offset is independent of the CTM', () => {
        const [, context] = make(); call(context, 'scale', 2, 2);
        context.shadowColor = 'blue'; context.shadowOffsetX = 4; context.fillStyle = 'red';
        call(context, 'fillRect', -2, 0, 2, 8); near(pixel(context, 2), [0, 0, 255, 255]);
    });
    check('Gaussian sigma is half shadowBlur and independent of the CTM', () => {
        const [, context] = make(); call(context, 'scale', 2, 2);
        context.shadowColor = 'blue'; context.shadowBlur = 4; context.fillStyle = 'red';
        call(context, 'fillRect', -50, -50, 50, 100);
        // Gaussian half-plane integrals at distances .5, 1.5 and 2.5, sigma 2.
        near(pixel(context, 0), [0, 0, 255, 102], 5);
        near(pixel(context, 1), [0, 0, 255, 58], 5);
        near(pixel(context, 2), [0, 0, 255, 27], 5);
    });
    check('opaque destination is made opaque between the two xor steps', () => {
        const [, context] = make({alpha: false}); background(context);
        context.globalCompositeOperation = 'xor'; context.globalAlpha = 0.5;
        context.shadowColor = 'blue'; context.shadowOffsetX = 1; context.fillStyle = 'lime';
        call(context, 'fillRect', -4, -4, 40, 24); near(pixel(context), [64, 0, 0, 255]);
    });
    for (const method of ['fillText', 'strokeText']) {
        check(method + ' casts a shadow from outside the bitmap', () => {
            const [, context] = make(); context.shadowColor = 'blue'; context.shadowOffsetX = 16; context.fillStyle = context.strokeStyle = 'red';
            call(context, method, 'M', -16, 13);
            const data = call(context, 'getImageData', 0, 0, 16, 16).data;
            let blue = false;
            for (let i = 0; i < data.length; i += 4) if (data[i + 2] > 0 && data[i + 3] > 0) blue = true;
            assert(blue, 'no text shadow reached the bitmap');
        });
    }
    for (const reset of ['restore', 'reset', 'resize']) {
        check('shadow drawing follows ' + reset, () => {
            const [canvas, context] = make(); context.shadowColor = 'blue'; context.shadowOffsetX = 8;
            call(context, 'save'); context.shadowColor = 'lime'; context.shadowOffsetX = 16;
            if (reset === 'restore') call(context, 'restore');
            else if (reset === 'reset') call(context, 'reset');
            else canvas.width = 32;
            context.fillStyle = 'red'; call(context, 'fillRect', -8, 0, 8, 16);
            near(pixel(context, 4), reset === 'restore' ? [0, 0, 255, 255] : [0, 0, 0, 0]);
        });
    }
    check('native shadow state ignores public getters', () => {
        const [, context] = make(); context.shadowColor = 'blue'; context.shadowOffsetX = 8; context.fillStyle = 'red';
        for (const name of ['shadowColor', 'shadowBlur', 'shadowOffsetX', 'shadowOffsetY']) Object.defineProperty(context, name, {get() {throw Error('public shadow getter');}});
        call(context, 'fillRect', -8, 0, 8, 16); near(pixel(context, 4), [0, 0, 255, 255]);
    });
    check('all author conversions precede the shadow snapshot', () => {
        const [, context] = make(); context.fillStyle = 'red';
        call(context, 'fillRect', {valueOf() {context.shadowColor = 'blue'; context.shadowOffsetX = 8; return -8;}}, 0, 8, 16);
        near(pixel(context, 4), [0, 0, 255, 255]);
    });
    check('conversion exceptions do not draw either image', () => {
        const [, context] = make(); background(context); const sentinel = new Error('sentinel'); let caught;
        try {call(context, 'fillRect', {valueOf() {context.shadowColor = 'blue'; context.shadowOffsetX = 8; return -8;}}, {valueOf() {throw sentinel;}}, 8, 16);}
        catch (error) {caught = error;}
        assert(caught === sentinel); near(pixel(context, 4), [255, 0, 0, 255]);
    });
    check('raw pixel upload and clearing do not cast shadows', () => {
        const [, context] = make(); context.shadowColor = 'blue'; context.shadowOffsetX = 8;
        call(context, 'putImageData', new owner.ImageData(new owner.Uint8ClampedArray([255, 0, 0, 255]), 1, 1), 0, 0);
        near(pixel(context, 8, 0), [0, 0, 0, 0]); call(context, 'clearRect', 0, 0, 1, 1);
        near(pixel(context, 0, 0), [0, 0, 0, 0]); near(pixel(context, 8, 0), [0, 0, 0, 0]);
    });
    return rows;
}
