(() => {
    const check = (ok, message) => { if (!ok) throw new Error(message); };
    const same = (actual, expected, message) => {
        check(actual.length === expected.length && actual.every((value, i) => value === expected[i]), message);
    };
    const category = name => name.startsWith('--') ? 2 : name.startsWith('-') ? 1 : 0;
    const compare = (a, b) => category(a) - category(b) || (a < b ? -1 : a > b ? 1 : 0);
    const parent = document.createElement('div');
    parent.style.setProperty('--inherited', 'parent');
    const element = document.createElement('div');
    element.style.cssText = 'width:25%;overflow-wrap:anywhere;transform:translateX(1px);mask-position:10% 20%;--z:Z;--A:A;--a:a;transition-property:none;transition-duration:1s,2s;';
    parent.append(element); document.body.append(parent);
    const map = element.computedStyleMap();
    const names = [...getComputedStyle(element)].sort(compare);
    check(names.length > 250, 'complete computed declaration');
    same([...map.keys()], names, 'keys enumerate the computed declaration');
    check(map.size === names.length && new Set(names).size === names.length, 'unique size');
    for (const name of ['font-size', 'direction', 'inline-size', 'mask-position', 'unicode-bidi', 'zoom']) {
        check(names.includes(name), 'enabled longhand: ' + name);
    }
    check(String(map.get('mask-position')) === '10% 20%', 'public mask position value');
    for (const name of ['mask-position-x', 'mask-position-y', '-moz-default-appearance', '-moz-min-font-size-ratio', '-x-text-scale']) {
        check(!names.includes(name) && ![...map.keys()].includes(name), 'internal property excluded: ' + name);
    }
    for (const name of ['background', 'border', 'gap', 'overflow', 'word-wrap', '-webkit-transform']) {
        check(!names.includes(name) && ![...map.keys()].includes(name), 'query-only property: ' + name);
        check(map.has(name) && map.getAll(name).length > 0, 'query remains supported: ' + name);
    }
    check(String(map.get('word-wrap')) === String(map.get('overflow-wrap')), 'alias query');
    same([...map.keys()].filter(name => name.startsWith('--')), ['--A', '--a', '--inherited', '--z'], 'custom properties preserve case');
    const entries = [...map.entries()];
    same(entries.map(entry => entry[0]), names, 'entries order');
    same([...map].map(entry => entry[0]), names, 'default iterator order');
    const values = [...map.values()];
    check(values.length === names.length, 'values cardinality');
    entries.forEach(([name, list], i) => {
        check(list.length > 0 && list.every(value => value instanceof CSSStyleValue), 'typed entry: ' + name);
        same(list.map(String), map.getAll(name).map(String), 'getAll agrees: ' + name);
        same(values[i].map(String), list.map(String), 'values agree: ' + name);
    });
    const receiver = {};
    const visited = [];
    map.forEach(function (list, name, owner) {
        check(this === receiver && owner === map && list.length > 0, 'forEach arguments');
        visited.push(name);
    }, receiver);
    same(visited, names, 'forEach order');
    element.style.setProperty('--B', 'added');
    check(map.size === names.length + 1, 'held map observes added declaration');
    same([...map.keys()].filter(name => name.startsWith('--')), ['--A', '--B', '--a', '--inherited', '--z'], 'new declaration ordered');
    element.style.removeProperty('--B');
    parent.style.removeProperty('--inherited');
    check(map.size === names.length - 1 && ![...map.keys()].includes('--inherited'), 'inherited removal updates enumeration');
    const currentNames = [...map.keys()];
    const nativeComputedStyle = globalThis.getComputedStyle;
    const nativeItem = CSSStyleDeclaration.prototype.item;
    const nativeGet = CSSStyleDeclaration.prototype.getPropertyValue;
    try {
        globalThis.getComputedStyle = CSSStyleDeclaration.prototype.item = CSSStyleDeclaration.prototype.getPropertyValue = () => {
            throw new Error('author-overridden CSSOM method');
        };
        same([...map.keys()], currentNames, 'enumeration uses native declaration');
        check(map.size === currentNames.length, 'size uses native declaration');
        same([...map.entries()].map(entry => entry[0]), currentNames, 'entry values bypass public CSSOM');
    } finally {
        globalThis.getComputedStyle = nativeComputedStyle;
        CSSStyleDeclaration.prototype.item = nativeItem;
        CSSStyleDeclaration.prototype.getPropertyValue = nativeGet;
    }
    const other = document.getElementById('child').contentWindow;
    const child = other.document.createElement('div');
    child.style.setProperty('--child', 'child'); other.document.body.append(child);
    const childMap = child.computedStyleMap();
    const childNames = [...other.getComputedStyle(child)].sort(compare);
    same([...StylePropertyMapReadOnly.prototype.keys.call(childMap)], childNames, 'borrowed keys use owner declaration');
    check(childMap.size === childNames.length && childNames.includes('--child'), 'child map size');
    element.remove();
    check(map.size === 0 && [...map.keys()].length === 0 && [...map.entries()].length === 0 && [...map.values()].length === 0, 'disconnected map empty');
    map.forEach(() => { throw new Error('disconnected entry'); });
    parent.append(element);
    same([...map.keys()], currentNames, 'reconnected held map');
    parent.remove(); child.remove();
    return true;
})()
