(owner, callee, kind, label) => {
    const checks = [];
    const check = (name, run) => {
        try { checks.push({ name: label + ':' + name, passed: run() === true }); }
        catch (error) { checks.push({ name: label + ':' + name, passed: false, error: String(error) }); }
    };
    const make = (width = 8, height = 8) => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(width, height);
        canvas.width = width; canvas.height = height;
        return [canvas, canvas.getContext('2d')];
    };
    const iface = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
    const proto = callee[iface].prototype;
    const descriptor = name => Object.getOwnPropertyDescriptor(proto, name);
    const get = (context, name) => Reflect.apply(descriptor(name).get, context, []);
    const set = (context, name, value) => Reflect.apply(descriptor(name).set, context, [value]);
    const call = (context, name) => Reflect.apply(proto[name], context, []);
    const attributes = [
        ['shadowBlur', 0, 2.5, 3.5],
        ['shadowOffsetX', 0, -2.5, 3.5],
        ['shadowOffsetY', 0, 2.5, -3.5],
        ['shadowColor', 'rgba(0, 0, 0, 0)', '#112233', '#445566'],
        ['textAlign', 'start', 'center', 'right'],
        ['textBaseline', 'alphabetic', 'top', 'bottom']
    ];
    for (const [name, initial, first, second] of attributes) {
        const [, context] = make();
        check(name + ':prototype accessor', () => {
            const d = descriptor(name);
            return d.enumerable && d.configurable && d.get.name === 'get ' + name && d.get.length === 0 && d.set.name === 'set ' + name && d.set.length === 1 && !Object.hasOwn(context, name);
        });
        check(name + ':initial value', () => get(context, name) === initial);
        check(name + ':assignment', () => set(context, name, first) === undefined && get(context, name) === first);
        check(name + ':nested restore', () => {
            set(context, name, first); call(context, 'save');
            set(context, name, second); call(context, 'save'); set(context, name, initial);
            call(context, 'restore'); const inner = get(context, name) === second;
            call(context, 'restore'); call(context, 'restore');
            return inner && get(context, name) === first;
        });
        for (const operation of ['width', 'height', 'reset']) {
            check(name + ':' + operation + ' resets value and stack', () => {
                const [canvas, ctx] = make(); set(ctx, name, first); call(ctx, 'save'); set(ctx, name, second);
                if (operation === 'reset') call(ctx, 'reset'); else canvas[operation] = canvas[operation];
                call(ctx, 'restore'); return get(ctx, name) === initial;
            });
        }
        check(name + ':zero-sized context', () => {
            const [, ctx] = make(0, 0); set(ctx, name, first); call(ctx, 'save'); set(ctx, name, second); call(ctx, 'restore');
            return get(ctx, name) === first;
        });
        check(name + ':state independent of author properties', () => {
            const [, ctx] = make(); set(ctx, name, first); call(ctx, 'save');
            let hooks = 0;
            Object.defineProperty(ctx, name, { get() { hooks++; throw Error('getter'); }, set() { hooks++; throw Error('setter'); } });
            Object.setPrototypeOf(ctx, null); Object.freeze(ctx);
            set(ctx, name, second); call(ctx, 'restore');
            return get(ctx, name) === first && hooks === 0;
        });
        check(name + ':independent contexts', () => {
            const [, a] = make(), [, b] = make(); set(a, name, first); set(b, name, second);
            return get(a, name) === first && get(b, name) === second;
        });
        check(name + ':conversion exactly once', () => {
            let count = 0;
            set(context, name, { [Symbol.toPrimitive](hint) { count++; if (hint !== (typeof first === 'number' ? 'number' : 'string')) throw Error('conversion hint'); return first; } });
            return count === 1 && get(context, name) === first;
        });
        check(name + ':conversion exception identity', () => {
            const sentinel = {}; set(context, name, first);
            try { set(context, name, { [Symbol.toPrimitive]() { throw sentinel; } }); }
            catch (error) { return error === sentinel && get(context, name) === first; }
            return false;
        });
        check(name + ':conversion may reset the context', () => {
            call(context, 'save');
            set(context, name, { [Symbol.toPrimitive]() { call(context, 'reset'); return second; } });
            call(context, 'restore'); return get(context, name) === second;
        });
        check(name + ':Symbol conversion throws in callee realm', () => {
            try { set(context, name, Symbol()); } catch (error) { return error instanceof callee.TypeError; }
            return false;
        });
        const revoked = owner.Proxy.revocable(context, {}); revoked.revoke();
        let traps = 0, conversions = 0;
        const author = new owner.Proxy(context, { get() { traps++; throw Error('get trap'); }, getPrototypeOf() { traps++; throw Error('prototype trap'); } });
        const receivers = [null, undefined, {}, Object.create(proto), Object.create(context), author, revoked.proxy];
        if (owner.document) receivers.push(kind === 'element' ? new owner.OffscreenCanvas(1, 1).getContext('2d') : owner.document.createElement('canvas').getContext('2d'));
        for (let index = 0; index < receivers.length; index++) {
            check(name + ':invalid getter receiver ' + index, () => {
                try { get(receivers[index], name); } catch (error) { return error instanceof callee.TypeError; }
                return false;
            });
            check(name + ':invalid setter receiver ' + index, () => {
                try { set(receivers[index], name, { [Symbol.toPrimitive]() { conversions++; return first; } }); }
                catch (error) { return error instanceof callee.TypeError && conversions === 0 && traps === 0; }
                return false;
            });
        }
        check(name + ':receiver checks bypass author hooks', () => traps === 0 && conversions === 0);
    }
    for (const name of ['shadowBlur', 'shadowOffsetX', 'shadowOffsetY']) {
        for (const value of [NaN, Infinity, -Infinity, undefined, 'bad', ...(name === 'shadowBlur' ? [-1] : [])]) {
            check(name + ':ignore ' + String(value), () => {
                const [, context] = make(); set(context, name, 2); set(context, name, value); return get(context, name) === 2;
            });
        }
        for (const value of [0, -0, 2.25, 1e20, ...(name === 'shadowBlur' ? [] : [-2.25])]) {
            check(name + ':finite value ' + (Object.is(value, -0) ? '-0' : value), () => {
                const [, context] = make(); set(context, name, value); return get(context, name) === value;
            });
        }
        check(name + ':null becomes zero', () => { const [, ctx] = make(); set(ctx, name, 2); set(ctx, name, null); return get(ctx, name) === 0; });
        check(name + ':numeric string', () => { const [, ctx] = make(); set(ctx, name, '2.25'); return get(ctx, name) === 2.25; });
        check(name + ':BigInt rejects', () => { const [, ctx] = make(); try { set(ctx, name, 1n); } catch (error) { return error instanceof callee.TypeError; } return false; });
    }
    for (const [name, values] of [
        ['textAlign', ['start', 'end', 'left', 'right', 'center']],
        ['textBaseline', ['top', 'hanging', 'middle', 'alphabetic', 'ideographic', 'bottom']]
    ]) {
        for (const value of values) check(name + ':enum ' + value, () => { const [, ctx] = make(); set(ctx, name, value); return get(ctx, name) === value; });
        for (const value of ['', 'CENTER', ' center ', 'bogus', null, undefined, 1n]) {
            check(name + ':ignore enum ' + String(value), () => {
                const [, ctx] = make(); set(ctx, name, values[1]); set(ctx, name, value); return get(ctx, name) === values[1];
            });
        }
    }
    for (const [value, expected] of [
        ['red', '#ff0000'], ['#AbC', '#aabbcc'], ['rgb(1, 2, 3)', '#010203'],
        ['transparent', 'rgba(0, 0, 0, 0)'], ['rgba(255, 0, 0, 0.5)', 'rgba(255, 0, 0, 0.5)'],
        ['hsl(120 100% 50%)', '#00ff00'], ['color(display-p3 1 0 0)', 'color(display-p3 1 0 0)']
    ]) {
        check('shadowColor:CSS ' + value, () => { const [, ctx] = make(); set(ctx, 'shadowColor', value); return get(ctx, 'shadowColor') === expected; });
    }
    for (const value of ['', '#ggg', 'red blue', 'rgb(1,2,3,0.5,1)', 'rgba(1,2,3,NaN)', 'inherit', 'var(--color)', null, undefined]) {
        check('shadowColor:ignore invalid CSS ' + String(value), () => {
            const [, ctx] = make(); set(ctx, 'shadowColor', '#112233'); set(ctx, 'shadowColor', value); return get(ctx, 'shadowColor') === '#112233';
        });
    }
    if (kind === 'offscreen') {
        check('shadowColor:offscreen currentcolor is black', () => { const [, ctx] = make(); set(ctx, 'shadowColor', 'currentcolor'); return get(ctx, 'shadowColor') === '#000000'; });
        check('transfer preserves all style state', () => {
            const [canvas, ctx] = make();
            for (const [name, , first] of attributes) set(ctx, name, first);
            call(ctx, 'save'); const bitmap = canvas.transferToImageBitmap();
            for (const [name, , , second] of attributes) set(ctx, name, second);
            call(ctx, 'restore'); bitmap.close();
            return attributes.every(([name, , first]) => get(ctx, name) === first);
        });
    }
    return checks;
}
