(owner, callee, kind, label) => {
    const checks = [];
    const check = (name, passed) => checks.push({ name: label + ':' + name, passed: !!passed });
    const make = (width = 8, height = 8) => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(width, height);
        canvas.width = width; canvas.height = height;
        return [canvas, canvas.getContext('2d')];
    };
    const iface = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
    const proto = callee[iface].prototype;
    const call = (ctx, name, ...args) => Reflect.apply(proto[name], ctx, args);
    const get = (ctx, name) => Reflect.apply(Object.getOwnPropertyDescriptor(proto, name).get, ctx, []);
    const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    const matrix = ctx => { const m = call(ctx, 'getTransform'); return [m.a, m.b, m.c, m.d, m.e, m.f]; };
    const pixel = (ctx, x, y) => Array.from(call(ctx, 'getImageData', x, y, 1, 1).data);
    const attributes = [
        ['fillStyle', '#112233', '#445566', '#778899'],
        ['strokeStyle', '#112233', '#445566', '#778899'],
        ['font', '12px serif', '16px serif', '20px serif'],
        ['imageSmoothingEnabled', false, true, false],
        ['imageSmoothingQuality', 'high', 'medium', 'low'],
        ['globalAlpha', 0.25, 0.5, 0.75],
        ['globalCompositeOperation', 'copy', 'xor', 'source-over'],
        ['lineWidth', 2, 3, 4],
        ['lineCap', 'round', 'square', 'butt'],
        ['lineJoin', 'round', 'bevel', 'miter'],
        ['miterLimit', 2, 3, 4],
        ['lineDashOffset', -2, 3, 4]
    ];
    for (const [name, first, second, third] of attributes) {
        const [, ctx] = make();
        ctx[name] = first;
        check(name + ':initial assignment', get(ctx, name) === first);
        call(ctx, 'save');
        check(name + ':save unchanged', get(ctx, name) === first);
        ctx[name] = second; call(ctx, 'save'); ctx[name] = third;
        call(ctx, 'restore'); check(name + ':inner restore', get(ctx, name) === second);
        call(ctx, 'restore'); check(name + ':outer restore', get(ctx, name) === first);
        call(ctx, 'restore'); check(name + ':empty restore unchanged', get(ctx, name) === first);
    }
    for (const name of ['save', 'restore', 'reset']) {
        const descriptor = Object.getOwnPropertyDescriptor(proto, name);
        check(name + ':descriptor', descriptor.enumerable && descriptor.writable && descriptor.configurable);
        check(name + ':name and length', descriptor.value.name === name && descriptor.value.length === 0);
        const [, real] = make();
        const revoked = owner.Proxy.revocable(real, {}); revoked.revoke();
        let traps = 0;
        const author = new owner.Proxy(real, { get() { traps++; throw Error('author trap'); } });
        const receivers = [null, undefined, {}, Object.create(proto), Object.create(real), author, revoked.proxy];
        if (owner.document) receivers.push(kind === 'element' ? new owner.OffscreenCanvas(8, 8).getContext('2d') : owner.document.createElement('canvas').getContext('2d'));
        for (let i = 0; i < receivers.length; i++) {
            let correct = false;
            try { call(receivers[i], name); } catch (error) { correct = error instanceof callee.TypeError; }
            check(name + ':invalid receiver ' + i, correct);
        }
        check(name + ':no author proxy traps', traps === 0);
        check(name + ':returns undefined', call(real, name, { toString() { throw Error('unused argument'); } }) === undefined);
    }
    {
        const [, ctx] = make();
        const segments = [1, 2, 3]; call(ctx, 'setLineDash', segments); call(ctx, 'save');
        segments[0] = 90;
        const returned = call(ctx, 'getLineDash'); returned[0] = 80;
        call(ctx, 'setLineDash', [4, 5]); call(ctx, 'save'); call(ctx, 'setLineDash', [6, 7]);
        call(ctx, 'restore'); check('dash inner snapshot', equal(call(ctx, 'getLineDash'), [4, 5]));
        call(ctx, 'restore'); check('dash outer snapshot has no author alias', equal(call(ctx, 'getLineDash'), [1, 2, 3, 1, 2, 3]));
        const copy = call(ctx, 'getLineDash'); copy.length = 0;
        check('restored dash remains private', call(ctx, 'getLineDash').length === 6);
    }
    {
        const [, ctx] = make();
        call(ctx, 'setTransform', 1, 2, 3, 4, 5, 6); call(ctx, 'save');
        call(ctx, 'setTransform', 7, 8, 9, 10, 11, 12); call(ctx, 'save'); call(ctx, 'resetTransform');
        call(ctx, 'restore'); check('transform inner snapshot', equal(matrix(ctx), [7, 8, 9, 10, 11, 12]));
        call(ctx, 'restore'); check('transform outer snapshot', equal(matrix(ctx), [1, 2, 3, 4, 5, 6]));
        call(ctx, 'getTransform').e = 90;
        call(ctx, 'restore'); check('transform empty restore unchanged', equal(matrix(ctx), [1, 2, 3, 4, 5, 6]));
    }
    {
        const [, ctx] = make();
        call(ctx, 'setTransform', 1e300, 0, 0, 1, 0, 0); call(ctx, 'scale', 1e300, 1);
        const before = matrix(ctx); call(ctx, 'save'); call(ctx, 'resetTransform'); call(ctx, 'restore');
        const after = matrix(ctx);
        check('large transform snapshot preserves native coefficients', before.every((value, index) => Object.is(value, after[index])));
    }
    {
        const [, ctx] = make();
        for (let i = 1; i <= 512; i++) { ctx.lineWidth = i; call(ctx, 'setTransform', 1, 0, 0, 1, i, 0); call(ctx, 'save'); }
        ctx.lineWidth = 999; call(ctx, 'resetTransform');
        let valid = true;
        for (let i = 512; i >= 1; i--) { call(ctx, 'restore'); valid = valid && get(ctx, 'lineWidth') === i && matrix(ctx)[4] === i; }
        check('512 nested snapshots', valid);
        for (let i = 0; i < 16; i++) call(ctx, 'restore');
        check('repeated underflow', get(ctx, 'lineWidth') === 1 && matrix(ctx)[4] === 1);
    }
    for (const operation of ['width', 'height', 'reset']) {
        const [canvas, ctx] = make();
        const defaults = attributes.map(([name]) => get(ctx, name));
        for (const [name, first] of attributes) ctx[name] = first;
        call(ctx, 'setLineDash', [1, 2]); call(ctx, 'translate', 3, 4); call(ctx, 'save');
        call(ctx, 'resetTransform'); ctx.fillStyle = '#ff0000'; call(ctx, 'fillRect', 0, 0, 8, 8);
        call(ctx, 'beginPath'); call(ctx, 'rect', 0, 0, 8, 8);
        if (operation === 'reset') call(ctx, 'reset'); else canvas[operation] = canvas[operation];
        call(ctx, 'restore');
        for (let i = 0; i < attributes.length; i++) check(operation + ':default ' + attributes[i][0], get(ctx, attributes[i][0]) === defaults[i]);
        check(operation + ':stack and transform cleared', equal(matrix(ctx), [1, 0, 0, 1, 0, 0]));
        check(operation + ':dash cleared', call(ctx, 'getLineDash').length === 0);
        check(operation + ':bitmap cleared', equal(pixel(ctx, 1, 1), [0, 0, 0, 0]));
        call(ctx, 'fill'); check(operation + ':path cleared', equal(pixel(ctx, 1, 1), [0, 0, 0, 0]));
        check(operation + ':owner and dimensions unchanged', ctx.canvas === canvas && canvas.width === 8 && canvas.height === 8);
    }
    for (const [width, height] of [[0, 0], [0, 8], [8, 0]]) {
        const [, ctx] = make(width, height);
        ctx.lineWidth = 2; call(ctx, 'save'); ctx.lineWidth = 3; call(ctx, 'restore');
        check('zero-size ' + width + 'x' + height, get(ctx, 'lineWidth') === 2);
    }
    {
        const [, ctx] = make();
        call(ctx, 'save'); call(ctx, 'beginPath'); call(ctx, 'rect', 0, 0, 4, 4); call(ctx, 'restore');
        ctx.fillStyle = '#00ff00'; call(ctx, 'fill');
        check('restore preserves current path', equal(pixel(ctx, 1, 1), [0, 255, 0, 255]) && equal(pixel(ctx, 6, 6), [0, 0, 0, 0]));
        ctx.fillStyle = '#ff0000'; call(ctx, 'fillRect', 0, 0, 8, 8); call(ctx, 'save');
        ctx.fillStyle = '#00ff00'; call(ctx, 'fillRect', 0, 0, 8, 8); call(ctx, 'restore');
        check('restore preserves bitmap', equal(pixel(ctx, 1, 1), [0, 255, 0, 255]));
        call(ctx, 'clearRect', 0, 0, 8, 8);
        ctx.fillStyle = '#00ff00'; call(ctx, 'translate', 4, 0); call(ctx, 'save');
        ctx.fillStyle = '#ff0000'; call(ctx, 'resetTransform'); call(ctx, 'restore'); call(ctx, 'fillRect', 0, 0, 4, 4);
        check('paint uses restored style and transform', equal(pixel(ctx, 5, 1), [0, 255, 0, 255]) && equal(pixel(ctx, 1, 1), [0, 0, 0, 0]));
    }
    {
        const [, a] = make(), [, b] = make();
        a.lineWidth = 2; b.lineWidth = 3; call(a, 'save'); call(b, 'save');
        a.lineWidth = 4; b.lineWidth = 5; call(b, 'restore'); call(a, 'restore');
        check('per-context stacks', get(a, 'lineWidth') === 2 && get(b, 'lineWidth') === 3);
    }
    {
        const [, ctx] = make();
        ctx.lineWidth = 2; ctx.custom = 4; call(ctx, 'save'); ctx.lineWidth = 3; ctx.custom = 5;
        let hooks = 0;
        Object.defineProperty(ctx, 'lineWidth', { get() { hooks++; throw Error('author getter'); }, configurable: true });
        ctx.__moliCanvasSavedDrawingState = { lineWidth: 99 };
        call(ctx, 'save'); call(ctx, 'restore'); call(ctx, 'restore');
        check('private slots bypass author properties', hooks === 0 && get(ctx, 'lineWidth') === 2 && ctx.custom === 5);
        Object.setPrototypeOf(ctx, null); Object.freeze(ctx); call(ctx, 'save'); call(ctx, 'restore');
        check('frozen receiver retains native state', get(ctx, 'lineWidth') === 2);
    }
    {
        const [, ctx] = make();
        const arrayProto = callee.Array.prototype, objectProto = callee.Object.prototype;
        const keys = [[arrayProto, 'push'], [arrayProto, 'pop'], [arrayProto, '0'], [objectProto, '__moliCanvasSavedDrawingState']];
        const originals = keys.map(([object, key]) => Object.getOwnPropertyDescriptor(object, key));
        let hooks = 0, valid = false;
        const fail = () => { hooks++; throw Error('prototype hook'); };
        try {
            for (let i = 0; i < keys.length; i++) {
                const [object, key] = keys[i]; Object.defineProperty(object, key, { get: fail, set: fail, configurable: true });
            }
            ctx.lineWidth = 2; call(ctx, 'setLineDash', [1, 2]); call(ctx, 'save'); ctx.lineWidth = 3;
            call(ctx, 'setLineDash', [3, 4]); call(ctx, 'restore');
            valid = get(ctx, 'lineWidth') === 2 && call(ctx, 'getLineDash')[0] === 1;
        } finally {
            for (let i = 0; i < keys.length; i++) {
                const [object, key] = keys[i]; if (originals[i]) Object.defineProperty(object, key, originals[i]); else delete object[key];
            }
        }
        check('snapshots bypass prototype hooks', valid && hooks === 0);
    }
    if (kind === 'offscreen') {
        const [canvas, ctx] = make();
        ctx.lineWidth = 2; ctx.fillStyle = '#00ff00'; call(ctx, 'translate', 4, 0); call(ctx, 'save');
        ctx.lineWidth = 3; ctx.fillStyle = '#ff0000'; call(ctx, 'resetTransform'); call(ctx, 'fillRect', 0, 0, 8, 8);
        const bitmap = canvas.transferToImageBitmap();
        call(ctx, 'restore');
        check('transfer preserves drawing stack', get(ctx, 'lineWidth') === 2 && get(ctx, 'fillStyle') === '#00ff00' && matrix(ctx)[4] === 4);
        check('restore cannot resurrect transferred pixels', equal(pixel(ctx, 1, 1), [0, 0, 0, 0]));
        call(ctx, 'fillRect', 0, 0, 4, 4);
        check('restored state paints replacement bitmap', equal(pixel(ctx, 5, 1), [0, 255, 0, 255]));
        bitmap.close();
    }
    return checks;
}
