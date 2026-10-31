(() => {
    const check = (ok, message) => { if (!ok) throw new Error(message); };
    for (const doc of [document, document.implementation.createHTMLDocument('')]) {
        const style = doc.createElement('div').style;
        style.cssText = 'font:16px serif;font-size-adjust:0';
        check(style.font === '', 'non-initial size adjustment prevents font serialization');
        style.font = '12px sans-serif';
        check(style.fontSizeAdjust === 'none', 'font resets size adjustment');
        check(style.font === '12px sans-serif', 'font serializes after reset');
        style.fontSizeAdjust = '0.5';
        style.setProperty('font', '16px serif', 'important');
        check(style.fontSizeAdjust === 'none', 'important font resets size adjustment');
        check(style.getPropertyPriority('font-size-adjust') === 'important', 'reset priority');
    }
    const parent = document.createElement('div');
    const child = document.createElement('div');
    parent.style.cssText = 'font:20px serif;font-size-adjust:0.75;--font:16px serif';
    parent.append(child); document.body.append(parent);
    const computed = getComputedStyle(child);
    const map = child.computedStyleMap();
    function adjusted(expected) {
        check(getComputedStyle(child).fontSizeAdjust === expected, 'computed size adjustment: ' + expected);
        check(computed.fontSizeAdjust === expected, 'live computed size adjustment: ' + expected);
        check(String(map.get('font-size-adjust')) === expected, 'Typed OM size adjustment: ' + expected);
    }
    child.style.font = '16px serif';
    adjusted('none');
    const retained = computed.font;
    const snapshot = Object.freeze(map.get('font'));
    child.style.fontSizeAdjust = '0';
    adjusted('0');
    check(child.style.font === '' && getComputedStyle(child).font === '', 'font cannot express size adjustment');
    check(computed.font === '', 'unrepresentable computed shorthand');
    check(map.get('font') === undefined && map.getAll('font').length === 0, 'unrepresentable Typed OM shorthand');
    child.style.font = '12px sans-serif';
    adjusted('none');
    check(String(retained) === '16px serif', 'retained font value');
    check(computed.font === '12px sans-serif', 'live declaration observes font reset');
    check(String(snapshot) === '16px serif', 'retained Typed OM value');
    check(String(map.get('font')) === '12px sans-serif', 'map observes font reset');
    for (const keyword of ['initial', 'inherit', 'unset', 'revert', 'revert-layer']) {
        child.style.cssText = 'font:16px serif;font-size-adjust:0';
        child.style.font = keyword;
        check(child.style.fontSizeAdjust === keyword, 'font expands CSS-wide keyword: ' + keyword);
        adjusted(keyword === 'initial' ? 'none' : '0.75');
    }
    child.style.cssText = 'font-size-adjust:0;font:var(--font)';
    check(child.style.fontSizeAdjust === '', 'pending shorthand substitution');
    adjusted('none');
    check(getComputedStyle(child).font === '16px serif', 'valid substituted shorthand');
    child.style.font = 'var(--missing)';
    adjusted('0.75');
    check(getComputedStyle(child).font === '', 'invalid substitution inherits non-initial adjustment');
    parent.remove();
    return true;
})()
