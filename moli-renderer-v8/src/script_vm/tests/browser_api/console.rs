use super::*;

#[test]
fn console_reporting_does_not_add_page_object_conversions() {
    let mut vm = new_storage_test_vm("https://console-reporting.test/");
    let result = vm.eval(r#"
(() => {
  const hits = [];
  let functionConversions = 0;
  const object = {
    get property() { hits.push('getter'); return 1; },
    toJSON() { hits.push('toJSON'); return {}; },
    toString() { hits.push('toString'); return 'object'; },
    [Symbol.toPrimitive]() { hits.push('toPrimitive'); return 'object'; }
  };
  const proxy = new Proxy({}, {
    get() { hits.push('Proxy.get'); },
    ownKeys() { hits.push('Proxy.ownKeys'); return []; },
    getOwnPropertyDescriptor() { hits.push('Proxy.getOwnPropertyDescriptor'); }
  });
  const fn = function named() {};
  fn.toString = () => { functionConversions++; return 'function'; };
  const error = new Error('console');
  Object.defineProperty(error, 'stack', {enumerable: true, get() { hits.push('stack'); return 'stack'; }});
  for (const method of ['log','info','warn','error','debug','trace']) {
    console[method](object, proxy, fn, error, Symbol('console'), 1n);
  }
  // Chromium's original V8 console converts a function once per call. The
  // renderer reporting copy must not add another conversion to that path.
  return JSON.stringify([hits, functionConversions]);
})()
"#).unwrap();
    assert_eq!(result, "[[],6]");
}

#[test]
fn console_reporting_cannot_turn_a_throwing_conversion_into_a_page_exception() {
    let mut vm = new_storage_test_vm("https://console-reporting.test/");
    assert_eq!(
        vm.eval(
            r#"
const object = {
  [Symbol.toPrimitive]() { throw new Error('conversion must not run'); },
  toJSON() { throw new Error('serialization must not run'); }
};
console.log(object);
'continued'
"#
        )
        .unwrap(),
        "continued"
    );
}

#[test]
fn console_reporting_does_not_read_a_page_prepare_stack_trace_getter() {
    let mut vm = new_storage_test_vm("https://console-reporting.test/");
    let result = vm
        .eval(
            r#"
(() => {
  let hits = 0;
  const original = Object.getOwnPropertyDescriptor(Error, 'prepareStackTrace');
  Object.defineProperty(Error, 'prepareStackTrace', {
    configurable: true,
    get() { hits++; return () => 'page stack'; }
  });
  try {
    console.log(new Error('console'));
    const afterConsole = hits;
    void new Error('explicit stack').stack;
    return `${afterConsole}|${hits}`;
  } finally {
    delete Error.prepareStackTrace;
    if (original) Object.defineProperty(Error, 'prepareStackTrace', original);
  }
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "0|1");
}
