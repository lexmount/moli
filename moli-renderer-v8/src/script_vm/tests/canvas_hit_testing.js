(owner, callee, kind, label) => {
    const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(100, 100);
    const context = canvas.getContext('2d');
    const iface = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
    const hit = (...args) => Reflect.apply(callee[iface].prototype.isPointInPath, context, args);
    const rows = [];
    const check = (name, run) => {
        try { rows.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { rows.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const throws = run => {
        try { run(); } catch (error) { return error instanceof callee.TypeError; }
        return false;
    };
    check('method length', () => callee[iface].prototype.isPointInPath.length === 2);
    check('empty path', () => hit(0, 0) === false);
    context.rect(0, 0, 20, 20);
    for (const [x, y] of [[10, 10], [0, 0], [10, 0], [20, 0], [20, 10], [20, 20], [10, 20], [0, 20], [0, 10]]) {
        check('inside or boundary ' + x + ',' + y, () => hit(x, y));
    }
    for (const [x, y] of [[10, -0.01], [10, 20.01], [-0.01, 10], [20.01, 10], [Infinity, 0], [NaN, 0], [0, -Infinity], [0, NaN]]) {
        check('outside or nonfinite ' + x + ',' + y, () => hit(x, y) === false);
    }
    context.rect(5, 5, 10, 10);
    check('nonzero nested paths', () => hit(10, 10, 'nonzero'));
    check('evenodd nested paths', () => hit(10, 10, 'evenodd') === false);
    check('undefined fill rule', () => hit(10, 10, undefined));
    context.beginPath();
    context.moveTo(0, 0); context.lineTo(20, 0); context.lineTo(20, 20);
    check('implicit closure includes diagonal boundary', () => hit(10, 10));
    check('implicit closure area', () => hit(15, 5));
    check('implicit closure excludes outside', () => hit(5, 15) === false);
    check('query does not close original subpath', () => {
        context.lineTo(0, 20);
        return hit(5, 15);
    });
    context.beginPath(); context.translate(30, 0); context.rect(0, 0, 20, 20); context.translate(0, 30);
    check('default geometry freezes construction transform', () => hit(40, 10));
    check('query point remains in canvas coordinates', () => !hit(10, 10) && !hit(40, 40));
    const path = new owner.Path2D(); path.rect(0, 0, 20, 20);
    check('explicit geometry uses current transform', () => hit(path, 40, 40));
    check('explicit geometry excludes untransformed point', () => !hit(path, 10, 10));
    check('explicit path is not modified by query', () => {
        context.resetTransform();
        return hit(path, 10, 10) && !hit(path, 40, 40);
    });
    check('move-only path has no fill', () => {
        const p = new owner.Path2D(); p.moveTo(1, 1);
        return !hit(p, 1, 1);
    });
    check('line-only path has no fill', () => {
        const p = new owner.Path2D(); p.moveTo(1, 1); p.lineTo(2, 1);
        return !hit(p, 1.5, 1);
    });
    check('line-only subpath has no fill beside an area', () => {
        const p = new owner.Path2D(path); p.moveTo(40, 40); p.lineTo(50, 40);
        return !hit(p, 45, 40);
    });
    check('zero-width rectangle has no fill', () => {
        const p = new owner.Path2D(); p.rect(0, 0, 0, 20);
        return !hit(p, 0, 10);
    });
    check('cancelled evenodd boundaries have no fill', () => {
        const p = new owner.Path2D(path); p.addPath(path);
        return !hit(p, 0, 10, 'evenodd');
    });
    check('doubled nonzero boundary remains included', () => {
        const p = new owner.Path2D(path); p.addPath(path);
        return hit(p, 0, 10, 'nonzero');
    });
    check('just outside horizontal edge', () => !hit(path, 10, -1e-10));
    check('just outside vertical edge', () => !hit(path, -1e-10, 10));
    check('fully collapsed explicit path has no fill', () => {
        context.setTransform(0, 0, 0, 0, 0, 0);
        const result = !hit(path, 0, 0); context.resetTransform(); return result;
    });
    check('singular explicit path has no fill', () => {
        context.setTransform(0, 0, 0, 1, 0, 0);
        const result = !hit(path, 0, 10); context.resetTransform(); return result;
    });
    check('diagonal boundary remains included', () => {
        const p = new owner.Path2D(); p.moveTo(0, 0); p.lineTo(20, 20); p.lineTo(0, 20);
        return hit(p, 3.3, 3.3);
    });
    check('queries ignore canvas bitmap bounds', () => {
        context.beginPath(); context.rect(-100, -100, 10, 10);
        return hit(-95, -95);
    });
    check('conversion occurs before path snapshot', () => {
        context.beginPath();
        return hit({valueOf() { context.rect(0, 0, 20, 20); return 10; }}, 10);
    });
    check('explicit path conversion occurs before path snapshot', () => {
        const other = new owner.Path2D();
        return hit(other, {valueOf() { other.rect(0, 0, 20, 20); return 10; }}, 10);
    });
    check('dimension reset during conversion clears default path', () => {
        context.rect(0, 0, 20, 20);
        return !hit({valueOf() { canvas.width = canvas.width; return 10; }}, 10);
    });
    check('all conversions precede nonfinite early return', () => {
        const order = [];
        const result = hit({valueOf() { order.push('x'); return NaN; }}, {valueOf() { order.push('y'); return 10; }},
            {toString() { order.push('rule'); return 'evenodd'; }});
        return result === false && order.join(',') === 'x,y,rule';
    });
    check('original conversion exception propagates', () => {
        const marker = {};
        try { hit(1, {valueOf() { throw marker; }}); } catch (error) { return error === marker; }
        return false;
    });
    check('nonfinite arguments still validate enum', () => throws(() => hit(NaN, 0, 'bad')));
    check('explicit nonfinite arguments still validate enum', () => throws(() => hit(path, NaN, 0, 'bad')));
    for (const args of [[], [1], [1, Symbol('y')], [1, 1n], [path, 1, 1, null], [1, 1, null]]) {
        check('invalid arguments ' + args.map(String).join(','), () => throws(() => hit(...args)));
    }
    check('required arity before conversion', () => {
        let conversions = 0;
        return throws(() => hit({valueOf() { conversions++; return 1; }})) && conversions === 0;
    });
    const revoked = owner.Proxy.revocable(context, {}); revoked.revoke();
    for (const [index, receiver] of [null, undefined, {}, Object.create(callee[iface].prototype), Object.create(context), new owner.Proxy(context, {}), revoked.proxy].entries()) {
        check('invalid receiver before conversion ' + index, () => {
            let conversions = 0;
            return throws(() => Reflect.apply(callee[iface].prototype.isPointInPath, receiver,
                [{valueOf() { conversions++; return 1; }}, 1])) && conversions === 0;
        });
    }
    const revokedPath = owner.Proxy.revocable(path, {}); revokedPath.revoke();
    for (const [index, argument] of [null, undefined, {}, Object.create(owner.Path2D.prototype), Object.create(path), new owner.Proxy(path, {}), revokedPath.proxy].entries()) {
        check('invalid explicit path before coordinates ' + index, () => {
            let conversions = 0;
            return throws(() => hit(argument, {valueOf() { conversions++; return 1; }}, 1, 'nonzero')) && conversions === 0;
        });
    }
    return rows;
}
