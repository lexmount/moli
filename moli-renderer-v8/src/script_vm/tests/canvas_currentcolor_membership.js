(owner, callee, label) => {
    const checks = [];
    const check = (name, run) => {
        try { checks.push({name: label + ':' + name, passed: run() === true}); }
        catch(error) { checks.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const setter = Object.getOwnPropertyDescriptor(callee.CanvasRenderingContext2D.prototype, 'shadowColor').set;
    const get = Object.getOwnPropertyDescriptor(callee.CanvasRenderingContext2D.prototype, 'shadowColor').get;
    const color = context => { setter.call(context, 'currentcolor'); return get.call(context); };
    const canvas = owner.document.createElement('canvas');
    const context = canvas.getContext('2d');
    let hooks = 0;
    Object.defineProperty(canvas, 'isConnected', {get() {hooks++; throw Error('author isConnected');}});
    Object.defineProperty(canvas, 'ownerDocument', {get() {hooks++; throw Error('author ownerDocument');}});
    canvas.style.color = 'rgb(1, 2, 3)';
    check('detached canvas is black', () => color(context) === '#000000');
    check('connected canvas uses its color', () => {owner.document.body.appendChild(canvas); return color(context) === '#010203';});
    check('removal discards computed context', () => {canvas.remove(); return color(context) === '#000000';});
    check('reattachment computes again', () => {owner.document.body.appendChild(canvas); canvas.style.color='rgb(4,5,6)'; return color(context)==='#040506';});
    check('conversion removes canvas before resolution', () => {
        setter.call(context, {toString() {canvas.remove(); return 'currentcolor';}});
        return get.call(context)==='#000000';
    });
    check('conversion attaches canvas before resolution', () => {
        setter.call(context, {toString() {owner.document.body.appendChild(canvas); return 'currentcolor';}});
        return get.call(context)==='#040506';
    });
    const inert = owner.document.implementation.createHTMLDocument('');
    const inertCanvas=inert.createElement('canvas');inertCanvas.style.color='red';inert.body.appendChild(inertCanvas);
    check('windowless Document keeps connected color context', () => color(inertCanvas.getContext('2d'))==='#ff0000');
    const host=owner.document.createElement('div');owner.document.body.appendChild(host);
    const shadow=host.attachShadow({mode:'open'});
    check('unslotted light DOM keeps connected color context', () => {host.appendChild(canvas);return color(context)==='#040506';});
    check('connected shadow canvas keeps computed context', () => {shadow.appendChild(canvas);return color(context)==='#040506';});
    check('detached shadow root has no computed context', () => {host.remove();return color(context)==='#000000';});
    check('native membership bypasses author properties', () => hooks===0);
    canvas.remove(); host.remove();
    return checks;
}
