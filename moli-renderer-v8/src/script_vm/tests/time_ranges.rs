use super::*;

#[test]
fn time_ranges_snapshots_preserve_native_brands_conversion_and_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://time-ranges.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("time_ranges.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn time_ranges_nonempty_native_snapshots_preserve_endpoints_and_uint32_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://time-ranges-native.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let intervals = [(-3.5, -1.0), (0.0, 0.0), (0.25, 1.75), (4.0, f64::INFINITY)];
        let ranges = crate::context_bootstrap::new_time_ranges_value(scope, &intervals);
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeRanges");
        assert_eq!(
            global.create_data_property(scope, key.into(), ranges.into()),
            Some(true)
        );
        // The endpoint backing array is inaccessible to author code. Register a
        // native Proxy to exercise the shared identity path independently.
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, ranges, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeRangesProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const snapshots = [nativeRanges, nativeRangesProxy];
      const realms = [globalThis, document.getElementById('child').contentWindow];
      const starts = [-3.5, 0, 0.25, 4], ends = [-1, 0, 1.75, Infinity];
      const equal = (a, b) => {if (!Object.is(a, b)) throw Error(`expected ${b}, got ${a}`);};
      const error = (realm, name, run) => {
        let caught; try {run();} catch(e) {caught = e;}
        if (!caught || caught.name !== name || !(caught instanceof (name === 'TypeError' ? realm.TypeError : realm.DOMException))) throw Error('wrong exception');
      };
      for (const realm of realms) for (const ranges of snapshots) {
        const p = realm.TimeRanges.prototype;
        equal(Object.getOwnPropertyDescriptor(p, 'length').get.call(ranges), 4);
        for (let i = 0; i < 4; i++) {equal(p.start.call(ranges, i), starts[i]); equal(p.end.call(ranges, i), ends[i]);}
        for (const [input, index] of [[NaN, 0], [Infinity, 0], [-Infinity, 0], [2**32, 0], [-(2**32), 0],
          [null, 0], [undefined, 0], ['2', 2], [1.9, 1], [-0.9, 0], [true, 1], [2**32 + 2, 2]]) {
          equal(p.start.call(ranges, input), starts[index]); equal(p.end.call(ranges, input), ends[index]);
        }
        for (const input of [-1, 4, 2**32 - 1]) for (const name of ['start', 'end']) error(realm, 'IndexSizeError', () => p[name].call(ranges, input));
        for (const input of [Symbol(), 0n]) for (const name of ['start', 'end']) error(realm, 'TypeError', () => p[name].call(ranges, input));
        let reads = 0;
        equal(p.start.call(ranges, {valueOf() {reads++; return 2;}}), 0.25); equal(reads, 1);
        const authorProxy = new Proxy(ranges, {get() {reads++;}});
        error(realm, 'TypeError', () => p.start.call(authorProxy, {valueOf() {reads++; return 0;}})); equal(reads, 1);
      }
      Object.defineProperty(nativeRanges, 'length', {value: 900}); nativeRanges.__moliTimeRanges = [100, 200];
      Object.setPrototypeOf(nativeRanges, null); Object.freeze(nativeRanges);
      equal(TimeRanges.prototype.start.call(nativeRanges, 2), 0.25);
      equal(TimeRanges.prototype.end.call(nativeRanges, 3), Infinity);
      return true;
    })()"#).unwrap(), "true");
}
