use super::*;

#[test]
fn idb_index_native_receivers_reject_forgery_before_conversion_in_the_callee_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://idb-index-native-receivers.test/");
    vm.eval(r#"
        document.body.innerHTML = '<iframe></iframe>';
        const other = document.querySelector('iframe').contentWindow;
        globalThis.receiverResult = "pending";
        const open = indexedDB.open("native-receivers", 1);
        open.onerror = () => {receiverResult = "open:" + open.error.name;};
        open.onupgradeneeded = () => {
        function checkReceivers() {
        const store = open.result.createObjectStore("values"); const index = store.createIndex("by-value", "value");
        let conversions = 0;
        let traps = 0;
        const poison = {
            [Symbol.toPrimitive]() { conversions++; return '3'; },
            get subtree() { conversions++; throw new Error('unexpected options conversion'); }
        };
        const cases = [
            ["IDBIndex", "count", "method", index, [], [poison]],
            ["IDBIndex", "getAll", "method", index, [], [poison]],
            ["IDBIndex", "getAllKeys", "method", index, [], [poison]],
            ["IDBIndex", "get", "method", index, ["missing"], [poison]],
            ["IDBIndex", "getKey", "method", index, ["missing"], [poison]],
            ["IDBIndex", "openCursor", "method", index, null, [poison, poison]],
            ["IDBIndex", "openKeyCursor", "method", index, null, [poison, poison]],
        ];
        for (const realm of [window, other]) {
            for (const [interfaceName, name, kind, value, validArgs, invalidArgs] of cases) {
                const prototype = realm[interfaceName].prototype;
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                const operation = kind === 'method' ? prototype[name] : descriptor && descriptor[kind];
                if (typeof operation !== 'function')
                    throw new Error(interfaceName + '.' + name + ': missing prototype member');
                const result = validArgs === null ? null : Reflect.apply(operation, value, validArgs);
                if (validArgs !== null && (!result || typeof result !== 'object'))
                    throw new Error(interfaceName + '.' + name + ': genuine request/object result');
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
        for (const realm of [window, other]) {
            for (const name of ['get', 'getKey', 'count', 'getAll', 'getAllKeys',
                                'openCursor', 'openKeyCursor']) {
                let error;
                try { realm.IDBIndex.prototype[name].call(index, Symbol()); }
                catch (caught) { error = caught; }
                if (!error || Object.getPrototypeOf(error) !== realm.DOMException.prototype ||
                    error.name !== 'DataError')
                    throw new Error('conversion realm: IDBIndex.' + name);
            }
        }
        if (conversions !== 0 || traps !== 0)
            throw new Error('receiver validation ran author code before rejection');
        return 'ok';
        }
        try { receiverResult = checkReceivers(); }
        catch (error) { receiverResult = String(error); }
        };
        "#).unwrap();
    let result = vm.eval_after_selected_page_tasks("receiverResult").unwrap();
    assert_eq!(result, "ok");
}
