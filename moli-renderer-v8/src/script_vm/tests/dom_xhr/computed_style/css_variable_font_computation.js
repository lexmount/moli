(() => {
    const check = (ok, message) => { if (!ok) throw new Error(message); };
    const parent = document.createElement('div');
    const child = document.createElement('div');
    parent.style.cssText = 'font-optical-sizing:none;font-variation-settings:"wght" 700, "wdth" 90, "wght" 600;--weight:650';
    parent.append(child); document.body.append(parent);
    const map = child.computedStyleMap();
    function computed(optical, variation) {
        const style = getComputedStyle(child);
        for (const [property, expected] of [['font-optical-sizing', optical], ['font-variation-settings', variation]]) {
            check(style.getPropertyValue(property) === expected, property + ': computed value');
            check(map.has(property), property + ': computed map exposure');
            check(String(map.get(property)) === expected, property + ': Typed OM value');
            check(map.getAll(property).length === 1, property + ': one computed value');
        }
    }
    for (const keyword of ['', 'inherit', 'unset', 'revert']) {
        child.style.cssText = keyword ? `font-optical-sizing:${keyword};font-variation-settings:${keyword}` : '';
        computed('none', '"wdth" 90, "wght" 600');
    }
    child.style.cssText = 'font-optical-sizing:initial;font-variation-settings:initial';
    computed('auto', 'normal');
    child.style.cssText = 'font-variation-settings:"wght" var(--weight), "wdth" calc(100 / 2), "wght" calc(600 + 50)';
    computed('none', '"wdth" 50, "wght" 650');
    child.style.fontVariationSettings = 'var(--missing)';
    computed('none', '"wdth" 90, "wght" 600');
    child.style.cssText = 'font:16px serif;font-optical-sizing:none;font-variation-settings:"wght" 600';
    computed('none', '"wght" 600');
    check(child.style.font === '' && getComputedStyle(child).font === '', 'font cannot express non-initial reset-only values');
    const opticalText = getComputedStyle(child).getPropertyValue('font-optical-sizing');
    const variationText = getComputedStyle(child).getPropertyValue('font-variation-settings');
    const optical = Object.freeze(map.get('font-optical-sizing'));
    const variation = Object.freeze(map.get('font-variation-settings'));
    child.style.font = '16px serif';
    computed('auto', 'normal');
    check(child.style.font === '16px serif' && getComputedStyle(child).font === '16px serif', 'font resets native values');
    check(String(optical) === 'none' && String(variation) === '"wght" 600', 'retained immutable values');
    check(opticalText === 'none' && variationText === '"wght" 600', 'retained CSSOM strings');
    const sheet = document.createElement('style');
    sheet.textContent = '@layer base { #font-target { font-optical-sizing:none;font-variation-settings:"wght" 400 } } #font-target { font-optical-sizing:auto;font-variation-settings:"wght" 550 }';
    document.head.append(sheet); child.id = 'font-target';
    child.style.cssText = 'font-optical-sizing:revert-layer;font-variation-settings:revert-layer';
    computed('auto', '"wght" 550');
    sheet.remove(); parent.remove();
    return true;
})()
