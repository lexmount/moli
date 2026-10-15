(owner, callee, kind, label, references) => {
    const rows = [];
    const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
    const check = (name, run) => {
        try { run(); rows.push({name: label + ': ' + name, passed: true}); }
        catch (error) { rows.push({name: label + ': ' + name, passed: false, error: String(error)}); }
    };
    const make = (settings = {}) => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(4, 4);
        canvas.width = 4; canvas.height = 4;
        return [canvas, canvas.getContext('2d', settings)];
    };
    const proto = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
    const call = (context, name, ...args) => Reflect.apply(proto[name], context, args);
    const pixel = (context, x = 1, y = 1) => Array.from(call(context, 'getImageData', x, y, 1, 1).data);
    const near = (actual, expected, tolerance = 3) => assert(actual.every((v, i) => Math.abs(v - expected[i]) <= tolerance), actual + ' != ' + expected);
    const upload = (context, rgba) => {
        const data = new owner.Uint8ClampedArray(64);
        for (let i = 0; i < 64; i += 4) data.set(rgba, i);
        call(context, 'putImageData', new owner.ImageData(data, 4, 4), 0, 0);
    };
    const greenStyle = (context, style) => {
        if (style === 'solid') return '#00ff00';
        if (style === 'gradient') {
            const gradient = call(context, 'createLinearGradient', 0, 0, 4, 0);
            gradient.addColorStop(0, '#00ff00'); gradient.addColorStop(1, '#00ff00');
            return gradient;
        }
        const source = new owner.OffscreenCanvas(1, 1), sourceContext = source.getContext('2d');
        sourceContext.fillStyle = '#00ff00'; sourceContext.fillRect(0, 0, 1, 1);
        return call(context, 'createPattern', source, 'repeat');
    };
    const golden = references.split('\n').filter(row => row.trim() && !row.startsWith('#')).map(row => {
        const [operation, ...numbers] = row.trim().split(/\s+/);
        const channels = numbers.map(Number);
        return {operation, source: channels.slice(0, 4), destination: channels.slice(4, 8), expected: channels.slice(8)};
    });
    for (const drawing of ['image', 'rect', 'fill', 'stroke', 'strokeRect']) {
        for (const [index, ref] of golden.entries()) {
            check(`Chromium vector ${index} ${drawing} ${ref.operation}`, () => {
                const [, context] = make(); upload(context, ref.destination);
                context.globalCompositeOperation = ref.operation;
                context.fillStyle = context.strokeStyle = `rgba(${ref.source.slice(0, 3).join(',')},${ref.source[3] / 255})`;
                context.lineWidth = 4;
                if (drawing === 'image') {
                    const source = new owner.OffscreenCanvas(1, 1), sourceContext = source.getContext('2d');
                    sourceContext.putImageData(new owner.ImageData(new owner.Uint8ClampedArray(ref.source), 1, 1), 0, 0);
                    context.imageSmoothingEnabled = false;
                    call(context, 'drawImage', source, 0, 0, 4, 4);
                } else if (drawing === 'rect') call(context, 'fillRect', 0, 0, 4, 4);
                else if (drawing === 'fill') { call(context, 'rect', 0, 0, 4, 4); call(context, 'fill'); }
                else if (drawing === 'strokeRect') call(context, 'strokeRect', 0, 0, 4, 4);
                else { call(context, 'moveTo', 0, 1); call(context, 'lineTo', 4, 1); call(context, 'stroke'); }
                near(pixel(context), ref.expected);
            });
        }
    }
    const expected = (operation, alpha, opaque = false) => {
        let color;
        switch (operation) {
            case 'copy': color = [0, 255, 0, 255 * alpha]; break;
            case 'destination-in': color = [255, 0, 0, 255 * alpha]; break;
            case 'destination-out': case 'xor': color = [255, 0, 0, 255 * (1 - alpha)]; break;
            case 'multiply': color = [255 * (1 - alpha), 0, 0, 255]; break;
            case 'screen': color = [255, 255 * alpha, 0, 255]; break;
        }
        if (opaque) return color.map((value, index) => index === 3 ? 255 : value * color[3] / 255);
        return color[3] === 0 ? [0, 0, 0, 0] : color;
    };
    for (const style of ['solid', 'gradient', 'pattern']) {
        for (const operation of ['copy', 'destination-in', 'destination-out', 'xor', 'multiply', 'screen']) {
            for (const alpha of [0, 0.5, 1]) {
                check(`${style} ${operation} globalAlpha=${alpha}`, () => {
                    const [, context] = make(); upload(context, [255, 0, 0, 255]);
                    context.fillStyle = greenStyle(context, style);
                    context.globalCompositeOperation = operation; context.globalAlpha = alpha;
                    call(context, 'fillRect', 0, 0, 2, 2);
                    near(pixel(context), expected(operation, alpha));
                    near(pixel(context, 3, 3), ['copy', 'destination-in'].includes(operation) ? [0, 0, 0, 0] : [255, 0, 0, 255]);
                });
            }
        }
    }
    for (const operation of ['copy', 'destination-in', 'destination-out', 'xor', 'multiply', 'screen']) {
        for (const alpha of [0, 0.5, 1]) {
            check(`opaque ${operation} globalAlpha=${alpha}`, () => {
                const [, context] = make({alpha: false}); upload(context, [255, 0, 0, 255]);
                context.fillStyle = '#00ff00'; context.globalCompositeOperation = operation; context.globalAlpha = alpha;
                call(context, 'fillRect', 0, 0, 2, 2);
                near(pixel(context), expected(operation, alpha, true));
            });
        }
    }
    for (const operation of ['clear', 'copy', 'source-in', 'source-out', 'destination-in', 'destination-atop']) {
        check(`${operation} affects transparent source only inside clip`, () => {
            const [, context] = make(); upload(context, [255, 0, 0, 255]);
            call(context, 'rect', 1, 1, 2, 2); call(context, 'clip');
            context.globalCompositeOperation = operation;
            call(context, 'fillRect', 20, 20, 1, 1);
            near(pixel(context), [0, 0, 0, 0]); near(pixel(context, 0, 0), [255, 0, 0, 255]);
        });
    }
    check('fractional clip interpolates the composed result', () => {
        const [, context] = make(); upload(context, [255, 0, 0, 255]);
        call(context, 'rect', 0.5, 0, 3, 4); call(context, 'clip');
        context.globalCompositeOperation = 'copy'; context.globalAlpha = 0.5; context.fillStyle = '#00ff00';
        call(context, 'fillRect', 0, 0, 4, 4);
        near(pixel(context, 0, 0), [170, 85, 0, 191]); near(pixel(context), [0, 255, 0, 128]);
    });
    for (const reset of ['restore', 'reset', 'resize']) {
        check('operator follows ' + reset, () => {
            const [canvas, context] = make(); upload(context, [255, 0, 0, 255]);
            context.globalCompositeOperation = 'destination-out'; call(context, 'save');
            context.globalCompositeOperation = 'copy';
            if (reset === 'restore') call(context, 'restore');
            else if (reset === 'reset') call(context, 'reset');
            else canvas.width = 4;
            assert(context.globalCompositeOperation === (reset === 'restore' ? 'destination-out' : 'source-over'));
            context.fillStyle = '#00ff00'; context.globalAlpha = 0.5; call(context, 'fillRect', 0, 0, 4, 4);
            near(pixel(context), reset === 'restore' ? [255, 0, 0, 128] : [0, 255, 0, 128]);
        });
    }
    check('public operator getter cannot replace native state', () => {
        const [, context] = make(); upload(context, [255, 0, 0, 255]);
        context.globalCompositeOperation = 'copy'; context.globalAlpha = 0;
        Object.defineProperty(context, 'globalCompositeOperation', {get() {throw Error('public getter');}});
        call(context, 'fillRect', 0, 0, 1, 1); near(pixel(context), [0, 0, 0, 0]);
    });
    check('numeric conversion updates operator before drawing', () => {
        const [, context] = make(); upload(context, [255, 0, 0, 255]);
        call(context, 'fillRect', {valueOf() {context.globalCompositeOperation = 'copy'; context.globalAlpha = 0; return 0;}}, 0, 1, 1);
        near(pixel(context), [0, 0, 0, 0]);
    });
    check('conversion exception prevents composition', () => {
        const [, context] = make(), sentinel = new Error('conversion'); upload(context, [255, 0, 0, 255]);
        let caught;
        try { call(context, 'fillRect', {valueOf() {context.globalCompositeOperation = 'clear'; return 0;}}, {valueOf() {throw sentinel;}}, 1, 1); }
        catch (error) {caught = error;}
        assert(caught === sentinel); near(pixel(context), [255, 0, 0, 255]);
    });
    check('raw pixel upload and clearRect bypass compositing', () => {
        const [, context] = make(); upload(context, [255, 0, 0, 255]);
        context.globalCompositeOperation = 'clear'; context.globalAlpha = 0;
        call(context, 'putImageData', new owner.ImageData(new owner.Uint8ClampedArray([0, 255, 0, 255]), 1, 1), 0, 0);
        near(pixel(context, 0, 0), [0, 255, 0, 255]);
        call(context, 'clearRect', 0, 0, 1, 1);
        near(pixel(context, 0, 0), [0, 0, 0, 0]); near(pixel(context), [255, 0, 0, 255]);
    });
    for (const method of ['fillText', 'strokeText']) {
        check(method + ' applies operator', () => {
            const [, context] = make(); upload(context, [255, 0, 0, 255]);
            context.globalCompositeOperation = 'clear'; call(context, method, 'M', 0, 4);
            near(pixel(context), [0, 0, 0, 0]);
        });
        for (const values of [['', 0, 1], [' ', 0, 1], ['M', 100, 100]]) {
            check(method + ' composes transparent parts ' + JSON.stringify(values), () => {
                const [, context] = make(); upload(context, [255, 0, 0, 255]);
                context.globalCompositeOperation = 'copy'; call(context, method, ...values);
                near(pixel(context), [0, 0, 0, 0]);
            });
        }
        for (const values of [['M', Infinity, 1], ['M', NaN, 1], ['M', 0, 1, 0], ['M', 0, 1, -1], ['M', 0, 1, NaN], ['M', 0, 1, Infinity]]) {
            check(method + ' invalid arguments do not compose ' + String(values), () => {
                const [, context] = make(); upload(context, [255, 0, 0, 255]);
                context.globalCompositeOperation = 'copy'; call(context, method, ...values);
                near(pixel(context), [255, 0, 0, 255]);
            });
        }
    }
    for (const operation of ['plus-darker', 'plus-lighter']) {
        check(operation + ' uses the native extended operator', () => {
            const [, context] = make(); upload(context, [255, 0, 0, 255]);
            context.globalCompositeOperation = operation; context.fillStyle = '#00ff00';
            call(context, 'fillRect', 0, 0, 4, 4);
            near(pixel(context), operation === 'plus-darker' ? [0, 0, 0, 255] : [255, 255, 0, 255]);
        });
    }
    const noops = [
        c => call(c, 'fillRect', 0, 0, 0, 1), c => call(c, 'strokeRect', 0, 0, 0, 0),
        c => call(c, 'strokeRect', NaN, 0, 1, 1), c => call(c, 'fill'), c => call(c, 'stroke'),
        c => {call(c, 'moveTo', 1, 1); call(c, 'stroke');},
        c => {call(c, 'moveTo', 1, 1); call(c, 'lineTo', 1, 1); call(c, 'stroke');},
        c => {const path = new owner.Path2D(); path.moveTo(1, 1); call(c, 'stroke', path);}
    ];
    noops.forEach((run, index) => check('empty geometry does not compose ' + index, () => {
        const [, context] = make(); upload(context, [255, 0, 0, 255]); context.globalCompositeOperation = 'copy';
        run(context); near(pixel(context), [255, 0, 0, 255]);
    }));
    return rows;
}
