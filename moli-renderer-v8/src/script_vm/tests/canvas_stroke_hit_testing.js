(owner, callee, kind, label) => {
    const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(100, 100);
    const context = canvas.getContext('2d');
    const iface = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
    const method = callee[iface].prototype.isPointInStroke;
    const hit = (...args) => Reflect.apply(method, context, args);
    const rows = [];
    const check = (name, run) => {
        try { rows.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { rows.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const throws = run => {
        try { run(); } catch (error) { return error instanceof callee.TypeError; }
        return false;
    };
    const line = () => {
        context.reset(); context.lineWidth = 10;
        context.moveTo(10, 20); context.lineTo(30, 20);
    };
    check('method length', () => method.length === 2);
    check('empty path', () => !hit(0, 0));
    line();
    for (const [x, y] of [[20,20],[10,20],[30,20],[20,15],[20,25]])
        check('butt stroke interior or boundary ' + x + ',' + y, () => hit(x,y));
    for (const [x, y] of [[9.99,20],[30.01,20],[20,14.99],[20,25.01]])
        check('butt stroke exterior ' + x + ',' + y, () => !hit(x,y));
    context.lineCap = 'round';
    check('round cap interior', () => hit(6,20));
    check('round cap boundary', () => hit(5,20));
    check('round cap excludes corner', () => !hit(6,24));
    context.lineCap = 'square';
    check('square cap includes corner', () => hit(6,24));
    check('square cap boundary', () => hit(5,25));
    check('square cap exterior', () => !hit(4.99,20));
    line(); context.beginPath(); context.moveTo(10,30); context.lineTo(30,30); context.lineTo(30,10);
    check('miter join interior', () => hit(34,34));
    check('miter join boundary', () => hit(35,35));
    context.lineJoin = 'bevel';
    check('bevel join excludes outer corner', () => !hit(33,33));
    context.lineJoin = 'round';
    check('round join interior', () => hit(33,33));
    check('round join excludes outer corner', () => !hit(34,34));
    context.lineJoin = 'miter'; context.miterLimit = 1;
    check('miter limit clips join', () => !hit(33,33));
    context.miterLimit = 10;
    check('miter limit restores join', () => hit(33,33));
    line(); context.lineWidth = 2; context.scale(2,3);
    check('default path freezes centerline', () => hit(20,20) && !hit(40,20));
    check('default stroke uses current anisotropic width', () => hit(20,22.5) && !hit(20,23.01));
    const path = new owner.Path2D(); path.moveTo(10,20); path.lineTo(30,20);
    check('explicit path uses current transform', () => hit(path,40,60));
    check('explicit path uses current anisotropic width', () => hit(path,40,62.5) && !hit(path,40,63.01));
    check('explicit path does not use untransformed coordinates', () => !hit(path,20,20));
    context.setTransform(2,1,0.5,3,20,-10); context.lineWidth = 10;
    check('sheared stroke interior', () => hit(path,72,82));
    check('sheared stroke exterior', () => !hit(path,73,88));
    context.resetTransform();
    check('query does not transform Path2D storage', () => hit(path,20,20));
    line(); context.beginPath(); context.moveTo(0,10); context.lineTo(50,10); context.lineWidth = 2; context.setLineDash([10,10]);
    check('dash on segments', () => hit(5,10) && hit(25,10));
    check('dash gap', () => !hit(15,10));
    context.lineDashOffset = 5;
    check('positive dash offset', () => hit(2,10) && !hit(7,10) && hit(17,10));
    context.lineDashOffset = -5;
    check('negative dash offset', () => !hit(2,10) && hit(7,10) && !hit(17,10));
    context.lineDashOffset = 45;
    check('dash offset wraps', () => hit(2,10) && !hit(7,10));
    context.lineDashOffset = 0; context.setLineDash([3,2,1]);
    check('odd dash pattern repeats with parity', () => hit(1,10) && !hit(4,10) && hit(5.5,10) && !hit(7,10) && hit(10,10) && !hit(11.5,10));
    context.setLineDash([0,0]);
    check('all-zero dashes are solid', () => hit(12,10));
    check('all-zero private list is preserved', () => context.getLineDash().join() === '0,0');
    context.save(); context.setLineDash([2,2]); context.restore();
    check('save restore keeps all-zero semantics', () => hit(12,10) && context.getLineDash().join() === '0,0');
    context.setLineDash([0,10]);
    check('zero-length butt dashes have no area', () => !hit(10,10));
    context.lineCap = 'round';
    check('zero-length round dash produces a dot', () => hit(10,10) && !hit(5,10));
    context.lineCap = 'square';
    check('zero-length square dash includes cap corners', () => hit(10.9,10.9));
    check('zero-length square dash exterior', () => !hit(11.01,10));
    check('zero-length square dash gap', () => !hit(5,10));
    context.lineDashOffset = 2;
    check('zero-length dash positive offset', () => hit(8,10) && !hit(3,10));
    context.lineDashOffset = -2;
    check('zero-length dash negative offset', () => hit(2,10) && !hit(7,10));
    context.lineDashOffset = 0; context.beginPath(); context.moveTo(0,0); context.lineTo(20,20);
    context.setLineDash([0,10*Math.SQRT2]);
    check('zero-length square dash follows diagonal direction', () => hit(11.2,10));
    check('diagonal square dash excludes rotated corner', () => !hit(10.8,10.8));
    context.beginPath(); context.moveTo(0,0); context.quadraticCurveTo(10,20,20,0); context.setLineDash([0,10]);
    check('zero-length square dash follows curve tangent', () => hit(-0.4,1.2));
    check('curve-tangent square dash exterior', () => !hit(-0.5,1.5));
    context.beginPath(); context.moveTo(0,0); context.bezierCurveTo(0,0,0,10,10,10);
    check('zero first derivative uses next distinct control direction', () => hit(0.9,0.9));
    context.lineCap = 'round';
    check('zero-length curve dash round cap', () => hit(-0.9,0));
    line(); context.lineWidth = 2; context.beginPath(); context.moveTo(0,0); context.lineTo(20,0); context.lineTo(20,20);
    check('open stroke is not implicitly closed', () => !hit(10,10));
    check('query leaves subpath open', () => { context.lineTo(0,20); return hit(10,20); });
    context.beginPath(); context.rect(0,0,20,20); context.setLineDash([15,10]);
    check('closed dashed seam preserves join', () => hit(-0.5,-0.5));
    line(); context.globalAlpha = 0; context.strokeStyle = 'transparent';
    check('alpha and stroke color do not affect geometry', () => hit(20,20));
    check('queries are not clipped to bitmap bounds', () => {
        context.beginPath(); context.moveTo(-100,-100); context.lineTo(-80,-100); return hit(-90,-100);
    });
    line(); context.scale(0,0);
    check('singular transform has no stroke area', () => !hit(20,20) && !hit(path,0,0));
    context.resetTransform();
    check('singular query does not mutate geometry', () => hit(20,20));
    check('point conversion precedes geometry snapshot', () => {
        context.beginPath();
        return hit({valueOf() { context.moveTo(10,20); context.lineTo(30,20); return 20; }},20);
    });
    check('point conversion precedes style snapshot', () => {
        context.lineWidth = 2;
        return hit({valueOf() { context.lineWidth = 20; return 20; }},28);
    });
    check('point conversion precedes transform snapshot', () => {
        context.lineWidth = 2;
        return hit(path,{valueOf() { context.scale(2,3); return 40; }},62.5);
    });
    check('resize during conversion resets native path', () => !hit({valueOf() { canvas.width = canvas.width; return 20; }},20));
    check('explicit path mutation during conversion is visible', () => {
        const p = new owner.Path2D();
        return hit(p,{valueOf() { p.moveTo(10,20); p.lineTo(30,20); return 20; }},20);
    });
    check('all coordinate conversions precede nonfinite return', () => {
        const order=[];
        return !hit({valueOf(){order.push('x');return NaN;}},{valueOf(){order.push('y');return 0;}}) && order.join() === 'x,y';
    });
    check('original conversion exception propagates', () => {
        const marker={};try { hit(NaN,{valueOf(){throw marker;}}); } catch(error) { return error===marker; } return false;
    });
    for (const [x,y] of [[NaN,0],[0,NaN],[Infinity,0],[0,-Infinity]])
        check('nonfinite query ' + x + ',' + y, () => !hit(x,y));
    for (const args of [[],[1],[1,Symbol('y')],[1,1n],[1,1,1]])
        check('invalid arguments ' + args.map(String).join(','), () => throws(() => hit(...args)));
    check('required arity precedes conversion', () => {
        let calls=0;return throws(() => hit({valueOf(){calls++;return 1;}})) && calls===0;
    });
    check('extra explicit path argument is ignored', () => hit(path,20,20,{toString(){throw Error('unused');}}));
    const revoked = owner.Proxy.revocable(context,{}); revoked.revoke();
    for (const [index,receiver] of [null,undefined,{},Object.create(callee[iface].prototype),Object.create(context),new owner.Proxy(context,{}),revoked.proxy].entries()) {
        check('invalid receiver before coordinates ' + index, () => {
            let calls=0;return throws(() => Reflect.apply(method,receiver,[{valueOf(){calls++;return 1;}},1])) && calls===0;
        });
    }
    const revokedPath = owner.Proxy.revocable(path,{}); revokedPath.revoke();
    for (const [index,value] of [null,undefined,{},Object.create(owner.Path2D.prototype),Object.create(path),new owner.Proxy(path,{}),revokedPath.proxy].entries()) {
        check('invalid path before coordinates ' + index, () => {
            let calls=0;return throws(() => hit(value,{valueOf(){calls++;return 1;}},1)) && calls===0;
        });
    }
    return rows;
}
