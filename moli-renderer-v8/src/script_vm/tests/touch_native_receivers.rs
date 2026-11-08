use super::*;

#[test]
fn touch_native_receivers_reject_forgery_before_conversion_in_the_callee_realm() {
    let mut vm = new_storage_html_test_vm("https://touch-native-receivers.test/");
    let result = vm.eval(r#"
        document.body.innerHTML = '<iframe></iframe>';
        const other = document.querySelector('iframe').contentWindow;
        const touch = new Touch({identifier: 7, target: document.body});
        let conversions = 0;
        let traps = 0;
        const poison = {
            [Symbol.toPrimitive]() { conversions++; return '3'; },
            get subtree() { conversions++; throw new Error('unexpected options conversion'); }
        };
        const cases = [
            ["Touch", "identifier", "get", touch, [], [poison]],
            ["Touch", "target", "get", touch, [], [poison]],
            ["Touch", "screenX", "get", touch, [], [poison]],
            ["Touch", "screenY", "get", touch, [], [poison]],
            ["Touch", "clientX", "get", touch, [], [poison]],
            ["Touch", "clientY", "get", touch, [], [poison]],
            ["Touch", "pageX", "get", touch, [], [poison]],
            ["Touch", "pageY", "get", touch, [], [poison]],
            ["Touch", "radiusX", "get", touch, [], [poison]],
            ["Touch", "radiusY", "get", touch, [], [poison]],
            ["Touch", "rotationAngle", "get", touch, [], [poison]],
            ["Touch", "force", "get", touch, [], [poison]],
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

#[test]
fn touch_lists_protect_native_entries_and_validate_receivers() {
    let mut vm = new_storage_html_test_vm("https://touch-list-native.test/");
    let result = vm.eval(r#"
        document.body.innerHTML = '<iframe></iframe>';
        const other = document.querySelector('iframe').contentWindow;
        const first = new Touch({identifier: 1, target: document.body});
        const second = new Touch({identifier: 2, target: document.body});
        const source = [first, second];
        const list = new TouchEvent('touchstart', {touches: source}).touches;
        source.length = 0;
        const check = (condition, name) => {if (!condition) throw new Error(name);};
        for (const realm of [window, other]) {
            const item = realm.TouchList.prototype.item;
            const length = Object.getOwnPropertyDescriptor(realm.TouchList.prototype, 'length').get;
            check(length.call(list) === 2 && item.call(list, 1) === second, 'cross-realm state');
            let conversions = 0;
            for (const fake of [{}, Object.create(list), new Proxy(list, {})]) {
                for (const call of [() => length.call(fake), () => item.call(fake, {
                    valueOf() {conversions++; return 0;}
                })]) {
                    let error;
                    try {call();} catch (caught) {error = caught;}
                    check(error && Object.getPrototypeOf(error) === realm.TypeError.prototype,
                          'callee realm brand error');
                }
            }
            check(conversions === 0, 'brand check precedes conversion');
        }
        for (const index of [0, 1]) {
            const descriptor = Object.getOwnPropertyDescriptor(list, index);
            check(descriptor.value === [first, second][index],
                  'indexed value');
            check(!descriptor.writable && descriptor.enumerable && descriptor.configurable,
                  'indexed descriptor');
        }
        check(!Reflect.set(list, 0, second) && list[0] === first, 'read-only entry');
        check(!Reflect.deleteProperty(list, 0) && list.item(0) === first, 'supported index deletion');
        check(!Reflect.defineProperty(list, 0, {value: second}), 'entry definition');
        check(!Reflect.defineProperty(list, 4, {get() {return first;}}), 'index accessor definition');
        check(!Reflect.set(list, 4, first) && !Object.hasOwn(list, 4), 'unsupported index assignment');
        check(Reflect.deleteProperty(list, 4) && list[4] === undefined && list.item(4) === null,
              'unsupported index access');
        check([...list][0] === first && [...list][1] === second, 'iteration');
        check(Object.keys(list).join() === '0,1', 'enumeration');
        check(Reflect.set(list, 'note', 3) && list.note === 3, 'ordinary expando');
        'ok';
    "#).unwrap();
    assert_eq!(result, "ok");
}
