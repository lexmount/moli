use super::*;

#[test]
fn performance_timing_interface_exposure_interfaces_have_realm_local_constructors_and_exposure() {
    for url in [
        "https://interface-exposure.test/",
        "http://interface-exposure.test/",
    ] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(vm.eval(r#"
(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const names = ["LargestContentfulPaint", "PerformanceEventTiming", "PerformancePaintTiming", "PerformanceServerTiming"];
  const parents = {"LargestContentfulPaint": "PerformanceEntry", "PerformanceEventTiming": "PerformanceEntry", "PerformancePaintTiming": "PerformanceEntry", "PerformanceServerTiming": null};
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const child = document.createElement('iframe');
  document.body.appendChild(child);
  const other = child.contentWindow;
  for (const realm of [window, other]) {
    for (const name of names) {
      const C = realm[name];
      assert(typeof C === 'function' && C.name === name && C.length === 0, name + ' constructor');
      assert(C.prototype.constructor === C, name + ' prototype identity');
      assert(Object.getPrototypeOf(C.prototype) === (parents[name] ? realm[parents[name]].prototype : realm.Object.prototype), name + ' prototype inheritance');
      assert(realm[name] === C, name + ' stable lazy materialization');
      for (const invoke of [() => C(), () => new C(), () => Reflect.construct(C, [], function Subclass() {})]) {
        let error;
        try { invoke(); } catch (caught) { error = caught; }
        assert(error instanceof realm.TypeError, name + ' rejects construction in callee realm');
      }
      assert(C !== (realm === window ? other[name] : window[name]), name + ' distinct realm constructors');
    }
  }
  return 'passed';
})()
"#).unwrap(), "passed", "{url}");
    }
}

#[test]
fn performance_timing_interface_exposure_interfaces_worker_exposure_follows_realm_and_security_rules()
 {
    use crate::context_bootstrap::exposed_interfaces::RealmKind;
    use std::pin::pin;
    crate::ensure_v8_for_test();
    for realm in [
        RealmKind::DedicatedWorker,
        RealmKind::SharedWorker,
        RealmKind::ServiceWorker,
    ] {
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        for secure in [false, true] {
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);
            crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
                scope, global, realm, secure,
            )
            .expect("worker interfaces");
            let name = "PerformanceServerTiming";
            let expected = true;
            let key = crate::util::v8str(scope, name);
            assert_eq!(
                global.has_own_property(scope, key.into()),
                Some(expected),
                "{realm:?}/{secure}/{name}"
            );
            if expected {
                assert!(
                    global.get(scope, key.into()).unwrap().is_function(),
                    "{name} materialization"
                );
            }
            for name in [
                "LargestContentfulPaint",
                "PerformanceEventTiming",
                "PerformancePaintTiming",
            ] {
                let key = crate::util::v8str(scope, name);
                assert_eq!(
                    global.has_own_property(scope, key.into()),
                    Some(false),
                    "{realm:?}/{name}"
                );
            }
        }
    }
}
