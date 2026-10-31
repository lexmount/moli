use super::*;

#[test]
fn data_transfer_refresh_preserves_live_lists_item_identity_and_removed_views() {
    let mut vm = new_storage_html_test_vm("https://data-transfer-refresh.test/");
    assert_eq!(vm.eval(r#"(() => {
        const transfer = new DataTransfer(), items = transfer.items, files = transfer.files;
        const file = new File(['body'], 'same.txt', {type:'text/plain'});
        const views = Array.from({length:128}, () => items.add(file));
        const string = items.add('original', 'text/plain'), types = transfer.types;
        transfer.setData('text/plain', 'updated');
        if (items !== transfer.items || files !== transfer.files || items[128] !== string ||
            transfer.getData('text/plain') !== 'updated' || new Set(views).size !== views.length)
            throw Error('list or record identity');
        const check = expected => {
            if (items.length !== expected.length+1 || files.length !== expected.length ||
                expected.some((view,i) => items[i] !== view || files[i] !== file || view.getAsFile() !== file) ||
                items[expected.length] !== string) throw Error('live order or file identity');
        };
        check(views);
        items.remove(63);
        const removed = views.splice(63,1)[0];
        if (removed.kind !== '' || removed.type !== '' || removed.getAsFile() !== null)
            throw Error('removed view stayed live');
        check(views);
        items.add(file);
        const added = items[128];
        if (views.includes(added) || added === removed || added.getAsFile() !== file)
            throw Error('new record reused an old view');
        items.remove(128);
        check(views);
        if (Array.from(types).join(',') !== 'text/plain,Files') throw Error('old type snapshot');
        items.clear();
        if (items.length || files.length || views.some(view => view.kind !== '') || string.kind !== '' ||
            transfer.types.length || Array.from(types).join(',') !== 'text/plain,Files')
            throw Error('clear or frozen snapshot');
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn data_transfer_native_receivers_reject_forgery_before_conversion_in_the_callee_realm() {
    let mut vm = new_storage_html_test_vm("https://data-transfer-native-receivers.test/");
    let result = vm.eval(r#"
        document.body.innerHTML = '<iframe></iframe>';
        const other = document.querySelector('iframe').contentWindow;
        const transfer = new DataTransfer(); const items = transfer.items; const item = items.add('hello', 'text/plain');
        let conversions = 0;
        let traps = 0;
        const poison = {
            [Symbol.toPrimitive]() { conversions++; return '3'; },
            get subtree() { conversions++; throw new Error('unexpected options conversion'); }
        };
        const cases = [
            ["DataTransfer", "types", "get", transfer, [], [poison]],
            ["DataTransfer", "items", "get", transfer, [], [poison]],
            ["DataTransfer", "dropEffect", "set", transfer, ['copy'], [poison]],
            ["DataTransfer", "setData", "method", transfer, ['text/plain', 'hello'], [poison, poison]],
            ["DataTransfer", "getData", "method", transfer, ['text/plain'], [poison]],
            ["DataTransferItem", "kind", "get", item, [], [poison]],
            ["DataTransferItem", "type", "get", item, [], [poison]],
            ["DataTransferItem", "getAsFile", "method", item, [], [poison]],
            ["DataTransferItemList", "length", "get", items, [], [poison]],
            ["DataTransferItemList", "clear", "method", items, [], [poison]],
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
