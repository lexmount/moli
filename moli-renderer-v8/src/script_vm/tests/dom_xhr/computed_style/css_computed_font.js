(() => {
    const check = (ok, message) => {if (!ok) throw new Error(message);};
    const cases = [
        ['font:16px serif', '16px serif'],
        ['font:italic bold 20px / 1.5 monospace', 'italic 700 20px / 30px monospace'],
        ['font:small-caps bold condensed 24px / 2 serif', 'small-caps 700 condensed 24px / 48px serif'],
        ['font:oblique 20deg 20px serif', 'oblique 20deg 20px serif'],
        ['font-size:24px;font-family:serif;font-weight:700;line-height:1.25', '700 24px / 30px serif'],
    ];
    for (const [text, expected] of cases) {
        const element = document.createElement('div');
        element.style.cssText = text;
        document.body.append(element);
        const computed = getComputedStyle(element);
        const value = computed.font;
        const map = element.computedStyleMap();
        const snapshot = Object.freeze(map.get('font'));
        check(getComputedStyle(element).font === expected, text + ': ' + getComputedStyle(element).font);
        check(value === expected, text + ': serialized value');
        check(String(snapshot) === expected, text + ': Typed OM snapshot');
        check(Object.getPrototypeOf(snapshot) === CSSStyleValue.prototype, 'opaque font');
        check(map.getAll('font').length === 1, 'one complete font value');
        element.style.font = '12px sans-serif';
        check(value === expected, 'retained font string');
        check(computed.font === '12px sans-serif', 'live computed declaration');
        check(String(snapshot) === expected, 'retained font snapshot');
        check(String(map.get('font')) === '12px sans-serif', 'live font map');
        element.remove();
    }
    const element = document.createElement('div');
    document.body.append(element);
    for (const [property, value] of [
        ['font-variant-ligatures', 'none'],
        ['font-variant-east-asian', 'full-width'],
        ['font-kerning', 'normal'],
    ]) {
        element.style.cssText = 'font:16px serif';
        element.style.setProperty(property, value);
        check(element.style.font === '', property + ': specified font cannot represent value');
        check(getComputedStyle(element).font === '', property + ': computed font cannot represent value');
    }
    const parent = document.createElement('div');
    parent.style.font = 'bold 20px / 1.5 monospace';
    parent.append(element); document.body.append(parent);
    element.style.cssText = 'font-size:2em';
    check(getComputedStyle(element).font === '700 40px / 60px monospace', 'inherited computed longhands');
    parent.remove();
    return true;
})()
