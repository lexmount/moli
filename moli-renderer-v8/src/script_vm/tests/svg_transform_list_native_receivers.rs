use super::*;

#[test]
fn svg_transform_list_native_receivers_reject_forgery_before_conversion_in_the_callee_realm() {
    let mut vm = new_storage_html_test_vm("https://svg-transform-list-native-receivers.test/");
    let result = vm.eval(r#"
        document.body.innerHTML = '<iframe></iframe>';
        const other = document.querySelector('iframe').contentWindow;
        const ns = 'http://www.w3.org/2000/svg';
        const svg = document.createElementNS(ns, 'svg'); document.body.append(svg);
        const createSvg = name => {const node=document.createElementNS(ns,name); svg.append(node); return node;};
        const list = createSvg('rect').transform.baseVal; const item = svg.createSVGTransform();
        let conversions = 0;
        let traps = 0;
        const poison = {
            [Symbol.toPrimitive]() { conversions++; return '3'; },
            get subtree() { conversions++; throw new Error('unexpected options conversion'); }
        };
        const cases = [
            ["SVGTransformList", "length", "get", list, [], [poison]],
            ["SVGTransformList", "numberOfItems", "get", list, [], [poison]],
            ["SVGTransformList", "clear", "method", list, [], [poison]],
            ["SVGTransformList", "appendItem", "method", list, [item], [poison]],
        ];
        for (const realm of [window, other]) {
            for (const [interfaceName, name, kind, value, validArgs, invalidArgs] of cases) {
                const prototype = realm[interfaceName].prototype;
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                const operation = kind === 'method' ? prototype[name] : descriptor && descriptor[kind];
                if (typeof operation !== 'function')
                    throw new Error(interfaceName + '.' + name + ': missing prototype member');
                const result = Reflect.apply(operation, value, validArgs);
                if (kind === 'get' && result === undefined)
                    throw new Error(interfaceName + '.' + name + ': genuine receiver lost native state');
                const author = new Proxy(value, {
                    get() { traps++; throw new Error('unexpected receiver get'); },
                    getPrototypeOf() { traps++; throw new Error('unexpected receiver prototype'); }
                });
                const revoked = Proxy.revocable(value, {}); revoked.revoke();
                for (const receiver of [null, undefined, {}, Object.create(value),
                                        Object.create(prototype), author, revoked.proxy]) {
                    let error;
                    try { Reflect.apply(operation, receiver, invalidArgs); }
                    catch (caught) { error = caught; }
                    if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype)
                        throw new Error(interfaceName + '.' + name + ': expected callee TypeError');
                }
            }
        }
        if (conversions !== 0 || traps !== 0)
            throw new Error('receiver validation ran author code before rejection');
        'ok';
    "#).unwrap();
    assert_eq!(result, "ok");
}
