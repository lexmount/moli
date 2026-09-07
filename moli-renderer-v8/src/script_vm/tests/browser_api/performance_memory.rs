use super::*;

#[test]
fn performance_memory_exposes_readonly_branded_heap_snapshots_without_a_constructor() {
    let mut vm = new_storage_test_vm("https://performance-memory.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const descriptor = Object.getOwnPropertyDescriptor(Performance.prototype, 'memory');
        const first = performance.memory;
        const second = performance.memory;
        const prototype = Object.getPrototypeOf(first);
        const names = ['totalJSHeapSize', 'usedJSHeapSize', 'jsHeapSizeLimit'];
        const rejects = callback => {
          try { callback(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const checks = [
          descriptor.get.name === 'get memory', descriptor.get.length === 0,
          descriptor.set === undefined, descriptor.enumerable, descriptor.configurable,
          !Object.hasOwn(performance, 'memory'), !('MemoryInfo' in globalThis),
          first !== second, Object.getPrototypeOf(second) === prototype,
          Object.getPrototypeOf(prototype) === Object.prototype,
          !Object.hasOwn(prototype, 'constructor'),
          Object.prototype.toString.call(first) === '[object MemoryInfo]',
          Object.getOwnPropertyNames(first).length === 0,
          Object.getOwnPropertyNames(prototype).sort().join() === names.slice().sort().join(),
          first.totalJSHeapSize >= first.usedJSHeapSize,
          first.jsHeapSizeLimit >= first.usedJSHeapSize,
          rejects(() => descriptor.get.call(Object.create(performance))),
          rejects(() => descriptor.get.call({}))
        ];
        for (const name of names) {
          const property = Object.getOwnPropertyDescriptor(prototype, name);
          const value = first[name];
          checks.push(Number.isSafeInteger(value), value >= 10000000, value % 100000 === 0,
            property.get.name === 'get ' + name, property.get.length === 0,
            property.set === undefined, property.enumerable, property.configurable);
          first[name] = 0;
          checks.push(first[name] === value, second[name] === value,
            rejects(() => { 'use strict'; first[name] = 0; }));
          for (const fake of [{}, prototype, Object.create(first), performance]) {
            checks.push(rejects(() => property.get.call(fake)));
          }
        }
        return checks.every(Boolean);
      })()
    "#,
        )
        .expect("legacy memory snapshots should have branded readonly attributes");
    assert_eq!(result, "true");
}

#[test]
fn performance_memory_uses_the_receiver_realm_and_accepts_genuine_foreign_snapshots() {
    let mut vm = new_parsed_test_vm(
        "https://performance-memory-realms.test/",
        "<!doctype html><iframe></iframe>",
    );
    let result = vm.eval(r#"
      (() => {
        const child = document.querySelector('iframe').contentWindow;
        const getter = Object.getOwnPropertyDescriptor(Performance.prototype, 'memory').get;
        const childGetter = Object.getOwnPropertyDescriptor(child.Performance.prototype, 'memory').get;
        const localPrototype = Object.getPrototypeOf(performance.memory);
        const childPrototype = Object.getPrototypeOf(child.performance.memory);
        const foreign = getter.call(child.performance);
        return [
          childPrototype !== localPrototype,
          Object.getPrototypeOf(childPrototype) === child.Object.prototype,
          Object.getPrototypeOf(foreign) === childPrototype,
          Object.getPrototypeOf(childGetter.call(performance)) === localPrototype,
          Object.getOwnPropertyDescriptor(localPrototype, 'usedJSHeapSize').get.call(foreign)
            === foreign.usedJSHeapSize,
          !('MemoryInfo' in child)
        ].every(Boolean);
      })()
    "#).expect("memory snapshots should use their Performance object's realm");
    assert_eq!(result, "true");
}
