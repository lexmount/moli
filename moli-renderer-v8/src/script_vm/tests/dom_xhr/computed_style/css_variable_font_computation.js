(() => {
    const check = (ok, message) => { if (!ok) throw new Error(message); };
    const parent = document.createElement('div');
    const child = document.createElement('div');
    parent.style.cssText = 'font-optical-sizing:none;font-variation-settings:"wght" 700, "wdth" 90, "wght" 600;--weight:650';
    parent.append(child); document.body.append(parent);
    function computed(optical, variation) {
        const style = getComputedStyle(child);
        for (const [property, expected] of [['font-optical-sizing', optical], ['font-variation-settings', variation]]) {
            check(style.getPropertyValue(property) === expected, property + ': computed value');
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
    check(child.style.font === '', 'font cannot express non-initial reset-only values');
    const optical = getComputedStyle(child).getPropertyValue('font-optical-sizing');
    const variation = getComputedStyle(child).getPropertyValue('font-variation-settings');
    child.style.font = '16px serif';
    computed('auto', 'normal');
    check(child.style.font === '16px serif', 'font resets native values');
    check(String(optical) === 'none' && String(variation) === '"wght" 600', 'retained CSSOM strings');
    const sheet = document.createElement('style');
    sheet.textContent = '@layer base { #font-target { font-optical-sizing:none;font-variation-settings:"wght" 400 } } #font-target { font-optical-sizing:auto;font-variation-settings:"wght" 550 }';
    document.head.append(sheet); child.id = 'font-target';
    child.style.cssText = 'font-optical-sizing:revert-layer;font-variation-settings:revert-layer';
    computed('auto', '"wght" 550');
    sheet.remove(); parent.remove();
    return true;
})()
