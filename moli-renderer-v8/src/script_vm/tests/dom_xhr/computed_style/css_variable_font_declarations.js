(() => {
    const check = (ok, message) => { if (!ok) throw new Error(message); };
    const properties = [
        ['font-optical-sizing', 'fontOpticalSizing', 'none'],
        ['font-variation-settings', 'fontVariationSettings', '"wght" 700, "wdth" 90, "wght" 600'],
    ];
    const documents = [document, document.implementation.createHTMLDocument('')];
    for (const [property, alias, value] of properties) {
        check(CSS.supports(property, value), property + ': native grammar');
        check(CSS.supports(property, 'var(--font-value)'), property + ': pending substitution');
        for (const doc of documents) {
            const style = doc.createElement('div').style;
            check(property in style && alias in style, property + ': accessors');
            for (const keyword of ['initial', 'inherit', 'unset', 'revert', 'revert-layer']) {
                check(CSS.supports(property, keyword), property + ': CSS-wide support');
                check(CSS.supports(`(${property}: ${keyword} !important)`), property + ': condition priority');
                style.setProperty(property, keyword, 'important');
                check(style.getPropertyValue(property) === keyword, property + ': stored keyword');
                check(style[alias] === keyword, property + ': keyword alias');
                check(style.getPropertyPriority(property) === 'important', property + ': keyword priority');
                style[alias] = keyword;
                check(style.getPropertyPriority(property) === '', property + ': alias clears priority');
            }
            style.setProperty(property, value, 'important');
            for (const invalid of ['unsupported-value', 'initial inherit', 'initial; color: red']) {
                check(!CSS.supports(property, invalid), property + ': invalid grammar');
                check(!CSS.supports(`(${property}: ${invalid})`), property + ': invalid condition');
                style.setProperty(property, invalid);
                check(style[alias] === value, property + ': rejected write preserves value');
                check(style.getPropertyPriority(property) === 'important', property + ': rejected write preserves priority');
            }
            check(style.removeProperty(property) === value, property + ': remove returns value');
            check(style[alias] === '', property + ': removal');
            style[alias] = '/**/IN\\69 TIAL/**/';
            check(style[property] === 'initial', property + ': tokenized CSS-wide value');
            check(CSS.supports(property, '/**/IN\\69 TIAL/**/'), property + ': tokenized support');
            check(CSS.supports(`(not (${property}: unsupported-value)) and (${property}: inherit)`), property + ': condition evaluation');
        }

    }
    return true;
})()
