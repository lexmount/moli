(owner, callee, kind, label) => {
    const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(100, 100);
    const context = canvas.getContext('2d');
    const iface = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
    const fill = (...args) => Reflect.apply(callee[iface].prototype.isPointInPath, context, args);
    const stroke = (...args) => Reflect.apply(callee[iface].prototype.isPointInStroke, context, args);
    const rows = [];
    const check = (name, run) => {
        try { rows.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { rows.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const shapes = [
        ['rectangle', p => p.rect(-10, -10, 20, 20)],
        ['polygon', p => { p.moveTo(-10, -10); p.lineTo(10, -10); p.lineTo(10, 10); p.lineTo(-10, 10); }],
        ['quadratic', p => { p.moveTo(-10, -10); p.quadraticCurveTo(0, 30, 10, -10); p.closePath(); }],
        ['cubic', p => { p.moveTo(-10, -10); p.bezierCurveTo(-10, 20, 10, 20, 10, -10); p.closePath(); }],
        ['arc', p => p.arc(0, 0, 10, 0, 2 * Math.PI)],
    ];
    for (const scale of [1e40, 1e150, Number.MAX_VALUE, 1e-40, 1e-200, Number.MIN_VALUE]) {
        for (const [name, draw] of shapes) {
            const prefix = name + ' x-scale ' + scale;
            context.reset(); context.setTransform(scale, 0, 0, 1, 0, 0); draw(context);
            check(prefix + ' default interior', () => fill(0, 0));
            check(prefix + ' default exterior', () => !fill(0, 31));
            context.resetTransform();
            check(prefix + ' frozen default interior', () => fill(0, 0));
            const path = new owner.Path2D(); draw(path);
            context.setTransform(scale, 0, 0, 1, 0, 0);
            check(prefix + ' explicit interior', () => fill(path, 0, 0));
            check(prefix + ' explicit exterior', () => !fill(path, 0, 31));
        }
        for (const rule of ['nonzero', 'evenodd']) {
            context.reset(); context.scale(scale, scale); context.rect(-10, -10, 20, 20);
            check('isotropic ' + scale + ' ' + rule, () => fill(0, 0, rule));
            context.rect(-10, -10, 20, 20);
            check('overlapping ' + scale + ' ' + rule, () => fill(0, 0, rule) === (rule === 'nonzero'));
        }
        context.reset(); context.scale(scale, scale); context.rect(-1, -1, 2, 2);
        check('large or small right boundary ' + scale, () => fill(scale, 0));
        context.beginPath(); context.moveTo(-1, -1); context.lineTo(1, 1); context.lineTo(-1, 1);
        check('large or small diagonal boundary ' + scale, () => fill(0, 0));
        check('large or small diagonal exterior ' + scale, () => !fill(scale, -scale));
    }
    check('overflowing determinant preserves default stroke', () => {
        context.reset(); context.scale(1e200, 1e200); context.lineWidth = 2;
        context.moveTo(-5, 0); context.lineTo(5, 0);
        return stroke(0, 0.5e200) && !stroke(0, 1.5e200);
    });
    check('underflowing determinant preserves default stroke', () => {
        context.reset(); context.scale(1e-200, 1e-200); context.lineWidth = 2;
        context.moveTo(-5, 0); context.lineTo(5, 0);
        return stroke(0, 0.5e-200) && !stroke(0, 1.5e-200);
    });
    check('overflowing determinant preserves explicit stroke', () => {
        const path = new owner.Path2D(); path.moveTo(-5, 0); path.lineTo(5, 0);
        context.reset(); context.scale(1e200, 1e200); context.lineWidth = 2;
        return stroke(path, 0, 0.5e200) && !stroke(path, 0, 1.5e200);
    });
    check('small nonzero area below JS coordinate range', () => {
        context.reset(); context.scale(Number.MIN_VALUE, Number.MIN_VALUE);
        context.rect(-0.25, -0.25, 0.5, 0.5);
        return fill(0, 0) && !fill(Number.MIN_VALUE, 0);
    });
    check('large offscreen subpath does not mask a small area', () => {
        context.reset(); context.scale(Number.MAX_VALUE, Number.MAX_VALUE); context.rect(2, 2, 2, 2);
        context.resetTransform(); context.rect(0, 0, 1, 1);
        return fill(0.5, 0.5) && fill(1, 0.5) && !fill(1.5, 0.5);
    });
    check('Path2D copy and addPath retain overflowing affine products', () => {
        context.reset();
        const source = new owner.Path2D(); source.rect(-10, -10, 20, 20);
        const large = new owner.Path2D(); large.addPath(source, {a: Number.MAX_VALUE, d: Number.MAX_VALUE});
        const copy = new owner.Path2D(large);
        const restored = new owner.Path2D(); restored.addPath(copy, {a: 1 / Number.MAX_VALUE, d: 1 / Number.MAX_VALUE});
        return fill(large, 0, 0) && fill(copy, 0, 0) && fill(restored, 0, 0) && !fill(restored, 20, 0)
            && fill(source, 5, 0) && !fill(source, 20, 0);
    });
    check('finite double path coordinates do not narrow to float', () => {
        context.reset(); context.rect(1, 1, 2 ** -30, 2 ** -30);
        return fill(1 + 2 ** -31, 1 + 2 ** -31) && !fill(1 + 2 ** -29, 1 + 2 ** -31);
    });
    check('opposite winding cancels large boundary', () => {
        context.reset(); context.scale(Number.MAX_VALUE, Number.MAX_VALUE);
        context.rect(0, -1, 1, 2); context.rect(1, -1, -1, 2);
        return !fill(0, 0, 'nonzero') && !fill(0, 0, 'evenodd');
    });
    check('anisotropic underflow and overflow retain nonzero area', () => {
        context.reset(); context.scale(Number.MAX_VALUE, Number.MIN_VALUE); context.rect(-10, -0.25, 20, 0.5);
        return fill(0, 0) && !fill(0, Number.MIN_VALUE);
    });
    check('acute large vertex includes its narrow filled sector', () => {
        context.reset(); context.scale(1e200, 1e200);
        context.moveTo(0, 0); context.lineTo(10, 1); context.lineTo(10, 2); context.closePath();
        return fill(0, 0) && fill(5e200, 0.75e200) && !fill(5e200, 2e200);
    });
    check('cancelled acute large vertex has no filled sector', () => {
        const path = new owner.Path2D(); path.moveTo(0, 0); path.lineTo(10, 1); path.lineTo(10, 2); path.closePath();
        path.addPath(new owner.Path2D(path));
        return !fill(path, 0, 0, 'evenodd');
    });
    return rows;
}
