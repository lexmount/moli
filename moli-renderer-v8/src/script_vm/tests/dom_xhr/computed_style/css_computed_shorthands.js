(() => {
    const check = (ok, message) => {if (!ok) throw new Error(message);};
    const element = document.createElement('div');
    element.style.cssText = 'background:blue;mask:none;flex-flow:column wrap;transition:opacity 1s linear;';
    document.body.append(element);
    const map = element.computedStyleMap();
    for (const property of ['background', 'mask', 'flex-flow', 'transition']) {
        const value = map.get(property);
        check(Object.getPrototypeOf(value) === CSSStyleValue.prototype, property + ': opaque type');
        check(map.getAll(property).length === 1, property + ': one value');
        let rejected = false;
        try {element.attributeStyleMap.set('margin', value);} catch(e) {rejected = e instanceof TypeError;}
        check(rejected, property + ': property association');
        const expected = String(value);
        value.toString = () => {throw new Error('author stringifier');};
        element.attributeStyleMap.set(property, value);
        check(String(map.get(property)) === expected, property + ': native write');
    }
    const other = document.querySelector('iframe').contentWindow;
    const childElement = other.document.createElement('div');
    childElement.style.background = 'blue'; other.document.body.append(childElement);
    const childMap = childElement.computedStyleMap();
    const read = StylePropertyMapReadOnly.prototype.get;
    const value = read.call(childMap, 'background');
    check(value instanceof other.CSSStyleValue && !(value instanceof CSSStyleValue), 'map owner realm');
    const native = other.CSSStyleValue;
    other.CSSStyleValue = () => {throw new Error('author constructor');};
    check(read.call(childMap, 'background') instanceof native, 'intrinsic allocation');
    other.CSSStyleValue = native;
    childElement.remove(); element.remove();
    return true;
})()
