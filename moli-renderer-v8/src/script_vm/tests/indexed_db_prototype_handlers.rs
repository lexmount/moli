use super::*;

#[test]
fn indexed_db_event_handler_accessors_are_shared_on_prototypes() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-prototype-handlers.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const failures = [];
  const check = (name, value) => { if (!value) failures.push(name); };
  for (const [name, properties] of [
    ['IDBRequest', ['onsuccess', 'onerror']],
    ['IDBOpenDBRequest', ['onupgradeneeded', 'onblocked']],
    ['IDBTransaction', ['onabort', 'oncomplete', 'onerror']],
    ['IDBDatabase', ['onabort', 'onclose', 'onerror', 'onversionchange']],
  ]) {
    const prototype = globalThis[name].prototype;
    for (const property of properties) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      check(name + '/' + property + '/descriptor', descriptor && typeof descriptor.get === 'function' && typeof descriptor.set === 'function' && descriptor.enumerable && descriptor.configurable);
      if (!descriptor) continue;
      for (const receiver of [{}, Object.create(prototype)]) {
        for (const [accessor, args] of [[descriptor.get, []], [descriptor.set, [null]]]) {
          let error; try { Reflect.apply(accessor, receiver, args); } catch (caught) { error = caught; }
          check(name + '/' + property + '/receiver', error instanceof TypeError);
        }
      }
    }
  }
  const first = indexedDB.open('prototype-handlers-first');
  const second = indexedDB.open('prototype-handlers-second');
  for (const property of ['onsuccess', 'onerror', 'onupgradeneeded', 'onblocked']) {
    check(property + '/initial', first[property] === null && second[property] === null);
    check(property + '/inherited', !Object.hasOwn(first, property));
    const handler = () => {};
    first[property] = handler;
    check(property + '/per-instance', first[property] === handler && second[property] === null);
    check(property + '/no-own-shadow', !Object.hasOwn(first, property));
    first[property] = null;
    check(property + '/clear', first[property] === null);
  }
  globalThis.__indexedDbPrototypeHandlerFailures = failures;
  return failures.length === 0;
})()
"#).unwrap(), "true", "{}", vm.eval("JSON.stringify(__indexedDbPrototypeHandlerFailures)").unwrap());
}
