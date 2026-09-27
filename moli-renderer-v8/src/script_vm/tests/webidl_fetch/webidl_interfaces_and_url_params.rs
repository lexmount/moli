use super::*;

#[test]
fn webidl_initializer_unions_observe_iterators_on_platform_objects() {
    let mut vm = new_storage_test_vm("https://initializer-union.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  const equal = (actual, expected, label) => check(
    JSON.stringify(actual) === JSON.stringify(expected), label + ': ' + JSON.stringify(actual));
  const factories = [
    ['Headers', () => new Headers([['x-original', 'original']])],
    ['URLSearchParams', () => new URLSearchParams([['x-original', 'original']])],
    ['FormData', () => {
      const form = new FormData();
      form.append('x-original', 'original');
      return form;
    }]
  ];
  const consumers = [
    ['Headers', input => new Headers(input)],
    ['URLSearchParams', input => new URLSearchParams(input)],
    ['Request', input => new Request('https://initializer-union.test/', {headers: input}).headers],
    ['Response', input => new Response(null, {headers: input}).headers]
  ];
  for (const [target, create] of consumers) {
    for (const [source, factory] of factories) {
      for (const mode of ['original', 'custom', 'undefined', 'null', 'throw', 'noncallable']) {
        const label = target + ' from ' + source + ' with ' + mode + ' iterator';
        const input = factory();
        const original = input[Symbol.iterator];
        const marker = {};
        const log = [];
        Object.defineProperty(input, 'x-record', {enumerable: true, get() {
          log.push('record');
          return 'record';
        }});
        Object.defineProperty(input, Symbol.iterator, {get() {
          log.push('iterator');
          if (mode === 'throw') throw marker;
          if (mode === 'noncallable') return 42;
          if (mode === 'undefined') return undefined;
          if (mode === 'null') return null;
          if (mode === 'original') return original;
          return function*() {
            check(this === input, label + ' receiver');
            log.push('iterate');
            yield ['x-custom', 'custom'];
          };
        }});
        if (mode === 'throw' || mode === 'noncallable') {
          let caught;
          try { create(input); } catch (error) { caught = error; }
          check(mode === 'throw' ? caught === marker : caught instanceof TypeError, label + ' exception');
        } else {
          const expected = mode === 'original' ? [['x-original', 'original']] :
            mode === 'custom' ? [['x-custom', 'custom']] : [['x-record', 'record']];
          equal(Array.from(create(input)), expected, label);
        }
        const expectedLog = ['iterator'];
        if (mode === 'custom') expectedLog.push('iterate');
        if (mode === 'undefined' || mode === 'null') expectedLog.push('record');
        equal(log, expectedLog, label + ' conversion order');
      }
    }
  }
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn webidl_initializer_records_reject_enumerable_symbol_keys() {
    let mut vm = new_storage_test_vm("https://initializer-symbol-record.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const consumers = [
    ['Headers', input => new Headers(input)],
    ['URLSearchParams', input => new URLSearchParams(input)],
    ['Request', input => new Request('https://initializer-symbol-record.test/', {headers: input})],
    ['Response', input => new Response(null, {headers: input})],
  ];
  const symbols = [
    ['Symbol.iterator', Symbol.iterator],
    ['ordinary symbol', Symbol('x')],
  ];
  for (const [target, create] of consumers) {
    for (const [description, symbol] of symbols) {
      const input = {};
      Object.defineProperty(input, symbol, {
        enumerable: true,
        value: undefined,
      });
      let caught;
      try { create(input); } catch (error) { caught = error; }
      if (!(caught instanceof TypeError)) {
        throw new Error(target + ' must convert its enumerable ' + description + ' record key');
      }
    }
  }
  return 'ok';
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn webidl_initializer_records_interleave_descriptor_and_value_access() {
    let mut vm = new_storage_test_vm("https://initializer-record-order.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const equal = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(label + ': ' + JSON.stringify(actual));
    }
  };
  const consumers = [
    ['Headers', input => Array.from(new Headers(input)).map(pair => pair.join('=')).join('&')],
    ['URLSearchParams', input => new URLSearchParams(input).toString()],
  ];
  for (const [target, create] of consumers) {
    const log = [];
    const record = {};
    Object.defineProperties(record, {
      a: {value: '1', enumerable: true, configurable: true},
      b: {value: '2', enumerable: true, configurable: true},
    });
    const input = new Proxy(record, {
      get(object, key, receiver) {
        log.push(['get', key === Symbol.iterator ? '@@iterator' : String(key)]);
        if (key === Symbol.iterator) return undefined;
        if (key === 'a') {
          Object.defineProperty(object, 'b', {enumerable: false});
        }
        return Reflect.get(object, key, receiver);
      },
      ownKeys(object) {
        log.push(['ownKeys']);
        return Reflect.ownKeys(object);
      },
      getOwnPropertyDescriptor(object, key) {
        log.push(['descriptor', String(key)]);
        return Reflect.getOwnPropertyDescriptor(object, key);
      },
    });
    equal(create(input), 'a=1', target + ' result');
    equal(log, [
      ['get', '@@iterator'],
      ['ownKeys'],
      ['descriptor', 'a'],
      ['get', 'a'],
      ['descriptor', 'b'],
    ], target + ' operation order');
  }
  return 'ok';
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn webidl_initializer_unions_convert_url_objects_as_records() {
    let mut vm = new_storage_test_vm("https://initializer-record.test/");
    let result = vm
        .eval(
            r#"
(() => {
  for (const Constructor of [Headers, URLSearchParams]) {
    const url = new URL('https://initializer-record.test/path?query=value');
    Object.defineProperty(url, Symbol.toPrimitive, {value() {
      throw new Error('URL object must use record conversion');
    }});
    if (Array.from(new Constructor(url)).length !== 0) throw new Error('empty URL record');
    url['x-record'] = 'record';
    if (JSON.stringify(Array.from(new Constructor(url))) !== '[["x-record","record"]]') {
      throw new Error(Constructor.name + ' must use own URL properties');
    }
  }
  return 'ok';
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn url_search_params_form_data_initializers_convert_file_values_through_the_iterator() {
    let mut vm = new_storage_test_vm("https://form-data-initializer.test/");
    let result = vm.eval(r#"
(() => {
  const file = new File(['contents'], 'entry.txt');
  const form = new FormData();
  form.append('a', 'first');
  form.append('file', file);
  form.append('a', 'second');
  const params = new URLSearchParams(form);
  form.append('later', 'ignored');
  if (JSON.stringify(Array.from(params)) !== '[["a","first"],["file","[object File]"],["a","second"]]') {
    throw new Error('FormData entries must be copied in iteration order with File stringification');
  }
  const marker = {};
  let conversions = 0;
  Object.defineProperty(file, Symbol.toPrimitive, {value(hint) {
    if (hint !== 'string') throw new Error('USVString must use the string hint');
    conversions++;
    throw marker;
  }});
  let caught;
  try { new URLSearchParams(form); } catch (error) { caught = error; }
  if (caught !== marker || conversions !== 1) throw new Error('File conversion exception must propagate');
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn headers_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://headers-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Headers();
  headers.append('X-A', '1');
  headers.append('X-A', '2');
  headers.set('X-B', '3');
  headers.set('X-Byte', '\u00ff');
  const seen = [];
  const thisArg = { marker: 'ctx' };
  headers.forEach(function(value, name, owner) {
    const displayValue = name === 'x-byte' ? value.charCodeAt(0) : value;
    seen.push([this.marker, name, displayValue, owner === headers].join(':'));
  }, thisArg);
  let invalidValue = 'missing';
  try {
    headers.set('X-Bad', '\u0100');
  } catch (error) {
    invalidValue = error && error.name;
  }
  const beforeDelete = [
    headers.get('x-a'),
    headers.has('x-b'),
    headers.get('x-byte').charCodeAt(0),
    invalidValue,
    seen.join(',')
  ].join('|');
  headers.delete('x-b');
  return beforeDelete + '|' + headers.has('x-b');
})()
"#,
        )
        .expect("Headers methods should parse WebIDL arguments");

    assert_eq!(
        result,
        "1, 2|true|255|TypeError|ctx:x-a:1, 2:true,ctx:x-b:3:true,ctx:x-byte:255:true|false"
    );
}

#[test]
fn headers_for_each_uses_webidl_callback_function_semantics() {
    let mut vm = new_storage_test_vm("https://headers-callback-function.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Headers([
    ['X-A', '1'],
    ['X-B', '2'],
    ['X-C', '3'],
  ]);
  const marker = { kind: 'abrupt' };
  const seen = [];
  const thisArg = { label: 'receiver' };
  const callback = new Proxy(function(value, name, owner) {
    seen.push(`${this.label}:${name}:${value}:${owner === headers}`);
    if (name === 'x-b') {
      throw marker;
    }
  }, {
    apply(target, receiver, arguments) {
      seen.push('proxy-apply');
      return Reflect.apply(target, receiver, arguments);
    }
  });
  let thrownIdentity = false;
  try {
    headers.forEach(callback, thisArg);
  } catch (error) {
    thrownIdentity = error === marker;
  }
  let nonCallable = 'accepted';
  try {
    headers.forEach({});
  } catch (error) {
    nonCallable = error?.name;
  }
  return JSON.stringify({ seen, thrownIdentity, nonCallable });
})()
"#,
        )
        .expect("Headers.forEach callback-function probe should evaluate");

    assert_eq!(
        result,
        r#"{"seen":["proxy-apply","receiver:x-a:1:true","proxy-apply","receiver:x-b:2:true"],"thrownIdentity":true,"nonCallable":"TypeError"}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn headers_for_each_visits_live_entries_in_window_and_worker() {
    let fixture = include_str!("../../../../tests/fixtures/headers-foreach.js");
    for worker in [false, true] {
        let loader = static_http_loader([]);
        let mut vm =
            new_page_task_executor_test_vm_with_loader("https://headers-foreach.test/", &loader);
        vm.eval("globalThis.headersForEachResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nheadersForEachProbe().then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                r#"
                const workerUrl = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
                const worker = new Worker(workerUrl);
                const finish = value => {{
                    headersForEachResult = value;
                    worker.terminate();
                    URL.revokeObjectURL(workerUrl);
                }};
                worker.onmessage = event => finish(event.data);
                worker.onerror = event => {{ finish({{error: event.message}}); event.preventDefault(); }};
                "#,
                serde_json::to_string(&source).unwrap()
            )
        } else {
            format!(
                "{fixture}\nheadersForEachProbe().then(value => {{ headersForEachResult = value; }}, error => {{ headersForEachResult = {{error: String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(headersForEachResult !== null)",
            "true",
            "Headers.forEach checks should finish",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(headersForEachResult)").unwrap())
                .unwrap();
        let checks = result["checks"]
            .as_array()
            .unwrap_or_else(|| panic!("worker={worker}: {result}"));
        let failures: Vec<_> = checks
            .iter()
            .filter(|check| check["pass"] != true)
            .collect();
        assert_eq!(result["state"], "pass", "worker={worker}: {failures:?}");
        assert_eq!(checks.len(), 36, "worker={worker}");
    }
}

#[tokio::test]
async fn headers_for_each_uses_callback_relevant_realm() {
    let mut vm = new_storage_test_vm("https://headers-callback-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__headersCallbackFrame = frame;
})()
"#,
    )
    .expect("Headers.forEach callback realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "Headers.forEach callback realm setup",
    )
    .await;
    let _ = materialize_single_child_default_realm_for_test(
        &mut vm,
        "Headers.forEach callback realm setup",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = __headersCallbackFrame.contentWindow;
  const headers = new Headers([['X-Realm', 'ok']]);
  child.__headersOwner = headers;
  child.__headersSeen = [];
  child.__headersRealmMarker = 'child';
  const callback = child.Function(
    'value',
    'name',
    'owner',
    `globalThis.__headersSeen.push([
      globalThis.__headersRealmMarker,
      this.receiverMarker,
      name,
      value,
      owner === globalThis.__headersOwner
    ].join(':'));
    if (name === 'x-realm') owner.append('x-tail', 'tail');`
  );
  headers.forEach(callback, { receiverMarker: 'parent-this' });
  return JSON.stringify({
    callbackRealm: Object.getPrototypeOf(callback) === child.Function.prototype,
    seen: child.__headersSeen,
  });
})()
"#,
        )
        .expect("cross-Realm Headers.forEach callback should evaluate");

    assert_eq!(
        result,
        r#"{"callbackRealm":true,"seen":["child:parent-this:x-realm:ok:true","child:parent-this:x-tail:tail:true"]}"#
    );
}

#[test]
fn form_data_and_url_search_params_for_each_use_live_webidl_callback_semantics() {
    let mut vm = new_storage_test_vm("https://pair-iterable-callback-function.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const makeFormData = () => {
    const collection = new FormData();
    collection.append('a', '1');
    collection.append('b', '2');
    collection.append('c', '3');
    return collection;
  };
  const makeSearchParams = () => new URLSearchParams('a=1&b=2&c=3');

  const probe = makeCollection => {
    const collection = makeCollection();
    const receiver = { label: 'receiver' };
    const seen = [];
    let applyCount = 0;
    const callback = new Proxy(function(value, key, owner) {
      seen.push([
        this.label,
        key,
        value,
        owner === collection,
        arguments.length
      ].join(':'));
      if (key === 'a') {
        owner.delete('b');
        owner.append('d', '4');
      }
    }, {
      apply(target, thisArg, args) {
        applyCount += 1;
        return Reflect.apply(target, thisArg, args);
      }
    });
    collection.forEach(callback, receiver);

    let omittedThisIsUndefined = false;
    makeCollection().forEach(function() {
      'use strict';
      omittedThisIsUndefined = this === undefined;
    });

    const marker = {};
    let abruptCount = 0;
    let abruptIdentity = false;
    try {
      makeCollection().forEach(() => {
        abruptCount += 1;
        throw marker;
      });
    } catch (error) {
      abruptIdentity = error === marker;
    }

    const revoked = Proxy.revocable(function() {}, {});
    revoked.revoke();
    let revokedError = '';
    try {
      makeCollection().forEach(revoked.proxy);
    } catch (error) {
      revokedError = error && error.name;
    }

    return {
      seen,
      applyCount,
      omittedThisIsUndefined,
      abruptCount,
      abruptIdentity,
      revokedError
    };
  };

  return JSON.stringify({
    formData: probe(makeFormData),
    searchParams: probe(makeSearchParams)
  });
})()
"#,
        )
        .expect("pair-iterable callback-function probe should evaluate");

    assert_eq!(
        result,
        r#"{"formData":{"seen":["receiver:a:1:true:3","receiver:c:3:true:3","receiver:d:4:true:3"],"applyCount":3,"omittedThisIsUndefined":true,"abruptCount":1,"abruptIdentity":true,"revokedError":"TypeError"},"searchParams":{"seen":["receiver:a:1:true:3","receiver:c:3:true:3","receiver:d:4:true:3"],"applyCount":3,"omittedThisIsUndefined":true,"abruptCount":1,"abruptIdentity":true,"revokedError":"TypeError"}}"#
    );
}

#[tokio::test]
async fn form_data_and_url_search_params_for_each_use_callback_relevant_realm() {
    let mut vm = new_storage_test_vm("https://pair-iterable-callback-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__pairIterableCallbackFrame = frame;
})()
"#,
    )
    .expect("pair-iterable callback realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "pair-iterable callback realm setup",
    )
    .await;
    let _ = materialize_single_child_default_realm_for_test(
        &mut vm,
        "pair-iterable callback realm setup",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = __pairIterableCallbackFrame.contentWindow;
  const formData = new FormData();
  formData.append('form', 'value');
  const searchParams = new URLSearchParams('search=value');
  child.__pairIterableOwners = { formData, searchParams };
  child.__pairIterableSeen = [];
  child.__pairIterableRealmMarker = 'child';

  const makeCallback = child.Function(
    'kind',
    `return function(value, key, owner) {
      globalThis.__pairIterableSeen.push([
        globalThis.__pairIterableRealmMarker,
        this.receiverMarker,
        kind,
        key,
        value,
        owner === globalThis.__pairIterableOwners[kind]
      ].join(':'));
    }`
  );
  const formCallback = makeCallback('formData');
  const searchCallback = makeCallback('searchParams');
  formData.forEach(formCallback, { receiverMarker: 'parent-this' });
  searchParams.forEach(searchCallback, { receiverMarker: 'parent-this' });

  return JSON.stringify({
    formCallbackRealm:
      Object.getPrototypeOf(formCallback) === child.Function.prototype,
    searchCallbackRealm:
      Object.getPrototypeOf(searchCallback) === child.Function.prototype,
    seen: child.__pairIterableSeen
  });
})()
"#,
        )
        .expect("cross-Realm pair-iterable callbacks should evaluate");

    assert_eq!(
        result,
        r#"{"formCallbackRealm":true,"searchCallbackRealm":true,"seen":["child:parent-this:formData:form:value:true","child:parent-this:searchParams:search:value:true"]}"#
    );
}

#[test]
fn request_and_response_headers_share_intrinsic_prototype_methods() {
    let mut vm = new_storage_test_vm("https://headers-object-prototype.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const check = (value, label) => { if (!value) throw new Error(label); };
  const NativeHeaders = Headers;
  const prototype = NativeHeaders.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'Headers');
  const nativeGet = prototype.get;
  let lookups = 0;
  Object.defineProperty(globalThis, 'Headers', {configurable: true, get() {
    lookups++;
    throw new Error('public Headers lookup');
  }});
  try {
    const request = new Request('/request', {headers: {'x-test': 'request'}});
    const streamRequest = new Request('/stream', {method: 'POST', duplex: 'half',
      body: new ReadableStream({start(controller) {controller.close();}})});
    const response = new Response('body', {headers: {'x-test': 'response'}});
    const error = Response.error();
    const redirect = Response.redirect('/target');
    const holders = [request, request.clone(), streamRequest, streamRequest.clone(),
      response, response.clone(), error, error.clone(), redirect, redirect.clone()];
    const methods = ['get', 'has', 'getSetCookie', 'set', 'delete', 'append',
      'keys', 'values', 'entries', 'forEach', Symbol.iterator];
    for (const holder of holders) {
      const headers = holder.headers;
      check(headers === holder.headers, 'same Headers object');
      check(Object.getPrototypeOf(headers) === prototype && headers instanceof NativeHeaders,
        'intrinsic Headers prototype');
      check(Object.prototype.toString.call(headers) === '[object Headers]', 'Headers class');
      check(Reflect.ownKeys(headers).length === 0, 'no own methods');
      for (const key of methods) check(headers[key] === prototype[key], String(key));
    }
    check(nativeGet.call(request.headers, 'x-test') === 'request', 'request entries');
    check(nativeGet.call(response.headers, 'x-test') === 'response', 'response entries');
    prototype.get = function() { return this; };
    for (const holder of holders) check(holder.headers.get() === holder.headers,
      'prototype method changes must remain observable');
    const fresh = new Response('fresh').headers;
    check(fresh.get() === fresh, 'new objects share updated prototype methods');
    check(lookups === 0, 'factory must not consult replaced global');
    return 'ok';
  } finally {
    prototype.get = nativeGet;
    Object.defineProperty(globalThis, 'Headers', descriptor);
  }
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn response_headers_keep_receiver_realm_across_borrowed_getters_and_clones() {
    let mut vm = new_storage_test_vm("https://headers-owner-realm.test/");
    vm.eval(
        r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || root.appendChild(document.createElement('body'));
globalThis.headersFrame = document.createElement('iframe');
body.appendChild(headersFrame);
"#,
    )
    .unwrap();
    assert_initial_about_blank_child_completed_synchronously_for_test(&mut vm, "Headers realm")
        .await;
    let _ = materialize_single_child_default_realm_for_test(&mut vm, "Headers realm");
    let result = vm.eval(r#"
(() => {
  const check = (value, label) => { if (!value) throw new Error(label); };
  const child = headersFrame.contentWindow;
  const parentHeaders = Headers;
  const childHeaders = child.Headers;
  const parentResponse = new Response('parent', {headers: {'x-realm': 'parent'}});
  const childResponse = new child.Response('child', {headers: {'x-realm': 'child'}});
  const parentGetter = Object.getOwnPropertyDescriptor(Response.prototype, 'headers').get;
  const childGetter = Object.getOwnPropertyDescriptor(child.Response.prototype, 'headers').get;
  const descriptors = [globalThis, child].map(realm => Object.getOwnPropertyDescriptor(realm, 'Headers'));
  for (const realm of [globalThis, child]) Object.defineProperty(realm, 'Headers', {
    configurable: true, get() { throw new Error('public Headers lookup'); }
  });
  try {
    for (const [response, clone, ctor, getter, value] of [
      [parentResponse, child.Response.prototype.clone.call(parentResponse), parentHeaders, childGetter, 'parent'],
      [childResponse, Response.prototype.clone.call(childResponse), childHeaders, parentGetter, 'child']
    ]) {
      check(getter.call(response) === response.headers, 'borrowed getter returns associated Headers');
      for (const entry of [response, clone]) {
        const headers = entry.headers;
        check(Object.getPrototypeOf(headers) === ctor.prototype && headers instanceof ctor,
          'Headers must use the response realm');
        check(headers.get === ctor.prototype.get && Reflect.ownKeys(headers).length === 0,
          'Headers must share realm prototype methods');
        check(headers.get('x-realm') === value, 'cross-realm entries');
        check(ctor.prototype.get.call(headers, 'x-realm') === value, 'branded receiver');
      }
    }
    return 'ok';
  } finally {
    [globalThis, child].forEach((realm, index) => Object.defineProperty(realm, 'Headers', descriptors[index]));
  }
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn headers_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://headers-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Headers([['X-A', '1']]);
  headers.append('X-A', '2');
  headers.set('X-B', '3');
  const methods = [
    ['get', 1],
    ['has', 1],
    ['getSetCookie', 0],
    ['set', 2],
    ['delete', 1],
    ['append', 2],
    ['keys', 0],
    ['values', 0],
    ['entries', 0],
    ['forEach', 1],
  ];
  const descriptors = methods.map(([name, expectedLength]) => {
    const descriptor = Object.getOwnPropertyDescriptor(Headers.prototype, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      expectedLength,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable,
    ].join(':');
  });
  const iteratorDescriptor = Object.getOwnPropertyDescriptor(Headers.prototype, Symbol.iterator);
  const instanceOwn = methods
    .map(([name]) => name)
    .filter((name) => Object.hasOwn(headers, name))
    .join(',');
  const prototypeEnumerable = Object.keys(Headers.prototype)
    .filter((name) => methods.some(([methodName]) => methodName === name))
    .join(',');
  const iterated = Array.from(headers).map(([name, value]) => `${name}=${value}`).join(',');
  const forEachSeen = [];
  headers.forEach(function(value, name, owner) {
    forEachSeen.push([this.label, name, value, owner === headers].join(':'));
  }, { label: 'ctx' });
  return JSON.stringify({
    descriptors,
    iterator: [
      typeof iteratorDescriptor?.value,
      iteratorDescriptor?.value === Headers.prototype.entries,
      iteratorDescriptor?.value?.name,
      iteratorDescriptor?.value?.length,
      iteratorDescriptor?.enumerable,
      iteratorDescriptor?.writable,
      iteratorDescriptor?.configurable,
    ].join(':'),
    instanceOwn,
    prototypeEnumerable,
    behavior: [
      headers.get('x-a'),
      headers.has('x-b'),
      iterated,
      forEachSeen.join(','),
    ].join('|'),
  });
})()
"#,
        )
        .expect("Headers prototype methods descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["get:function:get:1:1:true:true:true","has:function:has:1:1:true:true:true","getSetCookie:function:getSetCookie:0:0:true:true:true","set:function:set:2:2:true:true:true","delete:function:delete:1:1:true:true:true","append:function:append:2:2:true:true:true","keys:function:keys:0:0:true:true:true","values:function:values:0:0:true:true:true","entries:function:entries:0:0:true:true:true","forEach:function:forEach:1:1:true:true:true"],"iterator":"function:true:entries:0:false:true:true","instanceOwn":"","prototypeEnumerable":"get,has,getSetCookie,set,delete,append,keys,values,entries,forEach","behavior":"1, 2|true|x-a=1, 2,x-b=3|ctx:x-a:1, 2:true,ctx:x-b:3:true"}"#
    );
}

#[test]
fn headers_methods_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://headers-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const fake = Object.create(Headers.prototype);
  const bare = {};
  const methods = [
    ['get', () => Headers.prototype.get.call(fake, 'x-test')],
    ['has', () => Headers.prototype.has.call(fake, 'x-test')],
    ['getSetCookie', () => Headers.prototype.getSetCookie.call(fake)],
    ['set', () => Headers.prototype.set.call(fake, 'x-test', '1')],
    ['delete', () => Headers.prototype.delete.call(fake, 'x-test')],
    ['append', () => Headers.prototype.append.call(fake, 'x-test', '1')],
    ['keys', () => Headers.prototype.keys.call(fake)],
    ['values', () => Headers.prototype.values.call(fake)],
    ['entries', () => Headers.prototype.entries.call(fake)],
    ['forEach', () => Headers.prototype.forEach.call(fake, () => {})],
    ['iterator', () => Headers.prototype[Symbol.iterator].call(fake)],
    ['bare', () => Headers.prototype.get.call(bare, 'x-test')],
  ];
  const failures = methods
    .map(([name, callback]) => `${name}:${probe(callback)}`)
    .join(',');
  const fakeInit = probe(() => new Headers(fake));
  const real = new Headers([['X-Test', '1']]);
  return [
    failures,
    fakeInit,
    real.get('x-test'),
    Array.from(real.keys()).join(','),
  ].join('|');
})()
"#,
        )
        .expect("Headers receiver brand checks should evaluate");

    assert_eq!(
        result,
        "get:TypeError,has:TypeError,getSetCookie:TypeError,set:TypeError,delete:TypeError,append:TypeError,keys:TypeError,values:TypeError,entries:TypeError,forEach:TypeError,iterator:TypeError,bare:TypeError|TypeError|1|x-test"
    );
}

#[test]
fn response_headers_inherit_declared_methods_and_iterator_alias() {
    let mut vm = new_storage_test_vm("https://headers-declared-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Response(null, { headers: [['X-A', '1']] }).headers;
  const prototype = Object.getPrototypeOf(headers);
  if (prototype !== Headers.prototype) throw new Error('Headers must use the intrinsic prototype');
  if (Reflect.ownKeys(headers).length !== 0) throw new Error('Headers methods must be inherited');
  const descriptors = [
    ['get', 1],
    ['has', 1],
    ['getSetCookie', 0],
    ['set', 2],
    ['delete', 1],
    ['append', 2],
    ['keys', 0],
    ['values', 0],
    ['entries', 0],
    ['forEach', 1],
  ].map(([name, expectedLength]) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      expectedLength,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable,
    ].join(':');
  });
  const iteratorDescriptor = Object.getOwnPropertyDescriptor(prototype, Symbol.iterator);
  const iterated = Array.from(headers).map(([name, value]) => `${name}=${value}`).join(',');
  return JSON.stringify({
    descriptors,
    iterator: [
      typeof iteratorDescriptor?.value,
      iteratorDescriptor?.value === headers.entries,
      iteratorDescriptor?.value?.name,
      iteratorDescriptor?.value?.length,
      iteratorDescriptor?.enumerable,
      iteratorDescriptor?.writable,
      iteratorDescriptor?.configurable,
      iterated,
    ].join(':'),
  });
})()
"#,
        )
        .expect("Headers declared methods descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["get:function:get:1:1:true:true:true","has:function:has:1:1:true:true:true","getSetCookie:function:getSetCookie:0:0:true:true:true","set:function:set:2:2:true:true:true","delete:function:delete:1:1:true:true:true","append:function:append:2:2:true:true:true","keys:function:keys:0:0:true:true:true","values:function:values:0:0:true:true:true","entries:function:entries:0:0:true:true:true","forEach:function:forEach:1:1:true:true:true"],"iterator":"function:true:entries:0:false:true:true:x-a=1"}"#
    );
}

#[test]
fn headers_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://headers-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = (object) => Object.getOwnPropertyNames(object)
    .filter((name) => name.startsWith('__lmHeaders'))
    .sort()
    .join(',');
  const nullable = (value) => value === null ? 'null' : String(value);

  const headers = new Headers([
    ['X-Test', 'one'],
    ['Set-Cookie', 'a=1'],
  ]);
  const response = new Response('body', { headers: [['X-Safe', '1']] });
  const prototypeOwnBefore = internalNames(Headers.prototype);
  const initialOwn = [internalNames(headers), internalNames(response.headers)].join('|');

  Headers.prototype.__lmHeadersEntriesJson = '[["x-prototype","bad"]]';
  Headers.prototype.__lmHeadersImmutable = true;
  Headers.prototype.__lmHeadersGuard = 'request-no-cors';
  headers.__lmHeadersEntriesJson = '[["x-own","bad"]]';
  headers.__lmHeadersImmutable = true;
  headers.__lmHeadersGuard = 'request-no-cors';
  headers.append('X-Test', 'two');
  headers.append('X-Unsafe', 'bad');

  const fakeHeadersLike = Object.create(Headers.prototype);
  Object.defineProperty(fakeHeadersLike, '__lmHeadersEntriesJson', {
    value: '[["x-fake","bad"]]',
    enumerable: true,
  });
  const fakeInit = probe(() => {
    new Headers(fakeHeadersLike);
    return 'ok';
  });

  const record = { 'x-record': 'ok' };
  Object.defineProperty(record, '__lmHeadersEntriesJson', {
    value: '[["x-spoof","bad"]]',
    enumerable: false,
  });
  const recordCopy = new Headers(record);

  Headers.prototype.__lmHeadersGuard = 'none';
  response.headers.__lmHeadersGuard = 'none';
  response.headers.append('Set-Cookie', 'b=2');
  response.headers.append('X-Safe', '2');

  Headers.prototype.__lmHeadersImmutable = false;
  const errorHeaders = Response.error().headers;
  errorHeaders.__lmHeadersImmutable = false;
  const immutable = probe(() => {
    errorHeaders.append('X-Error', 'bad');
    return errorHeaders.get('X-Error');
  });

  return JSON.stringify({
    prototypeOwnBefore,
    initialOwn,
    real: [headers.get('X-Test'), headers.get('X-Unsafe'), headers.getSetCookie().join('|')].join('|'),
    fakeInit,
    record: [nullable(recordCopy.get('X-Record')), nullable(recordCopy.get('X-Spoof'))].join('|'),
    guarded: [response.headers.get('X-Safe'), response.headers.getSetCookie().join('|')].join('|'),
    immutable,
  });
})()
"#,
        )
        .expect("Headers private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"prototypeOwnBefore":"","initialOwn":"|","real":"one, two|bad|a=1","fakeInit":"throw:TypeError","record":"ok|null","guarded":"1, 2|","immutable":"throw:TypeError"}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn response_backing_slots_ignore_reflection_and_spoofing() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://response-slots.test/", &loader);

    vm.eval(
        r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = (object) => Object.getOwnPropertyNames(object)
    .filter((name) => name.startsWith('__lmResponse'))
    .sort()
    .join(',');
  const nullable = (value) => value === null ? 'null' : String(value);

  const response = new Response('payload', {
    status: 201,
    statusText: 'Created',
    headers: [['Content-Type', 'text/plain'], ['X-Original', '1']],
  });
  const initial = [
    internalNames(Response.prototype),
    internalNames(response),
    response.status,
    response.statusText,
    response.ok,
    response.type,
    response.url,
    response.redirected,
    response.headers.get('X-Original'),
    response.bodyUsed,
  ].join('|');

  const spoofEntries = {
    __lmResponseStatus: 599,
    __lmResponseStatusText: 'Spoofed',
    __lmResponseOk: false,
    __lmResponseUrl: 'https://spoofed.invalid/',
    __lmResponseRedirected: true,
    __lmResponseType: 'opaque',
    __lmResponseHeadersObject: new Headers([['X-Spoof', 'bad']]),
    __lmResponseBody: null,
    __lmResponseBodyUsed: true,
  };
  for (const [name, value] of Object.entries(spoofEntries)) {
    Object.defineProperty(Response.prototype, name, {
      value,
      configurable: true,
    });
    Object.defineProperty(response, name, {
      value,
      configurable: true,
    });
  }

  const clone = response.clone();
  const cloneInitial = [
    clone.status,
    clone.statusText,
    clone.ok,
    clone.type,
    clone.url,
    clone.redirected,
    clone.headers.get('X-Original'),
    nullable(clone.headers.get('X-Spoof')),
    clone.bodyUsed,
  ].join('|');

  const errorResponse = Response.error();
  errorResponse.__lmResponseStatus = 200;
  errorResponse.__lmResponseType = 'basic';
  errorResponse.__lmResponseBodyUsed = true;
  errorResponse.__lmResponseHeadersObject = new Headers([['X-Error-Spoof', 'bad']]);
  const errorClone = errorResponse.clone();
  const errorSurface = [
    errorResponse.status,
    errorResponse.ok,
    errorResponse.type,
    errorResponse.url,
    errorResponse.redirected,
    errorResponse.body === null,
    errorResponse.bodyUsed,
    nullable(errorResponse.headers.get('X-Error-Spoof')),
    errorClone.status,
    errorClone.type,
    errorClone.body === null,
    errorClone.bodyUsed,
  ].join('|');

  const fake = Object.create(Response.prototype);
  const bodyOutcome = async (prototype, method, receiver) => {
    let value;
    try {
      value = prototype[method].call(receiver);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
    const isPromise = value instanceof Promise;
    try {
      await value;
      return `resolved:${isPromise}`;
    } catch (error) {
      return `rejected:${isPromise}:${error && error.name}`;
    }
  };
  const fakeSurface = [
    probe(() => fake.status),
    probe(() => fake.headers),
    probe(() => fake.bodyUsed),
    probe(() => Response.prototype.clone.call(fake)),
  ].join('|');

  globalThis.__responseSlotProbe = {
    initial,
    cloneInitial,
    errorSurface,
    fakeSurface,
    consumed: null,
    fakeBodyMethods: null,
  };
  Promise.all([response.text(), clone.text()]).then(
    ([text, cloneText]) => {
      globalThis.__responseSlotProbe.consumed = [
        text,
        cloneText,
        response.bodyUsed,
        clone.bodyUsed,
        probe(() => response.clone()),
      ].join('|');
    },
    (error) => {
      globalThis.__responseSlotProbe.consumed = `reject:${error && error.name}`;
    }
  );
  Promise.all(["arrayBuffer", "blob", "bytes", "formData", "json", "text"]
    .map((method) => bodyOutcome(Response.prototype, method, fake))).then((outcomes) => {
      globalThis.__responseSlotProbe.fakeBodyMethods = outcomes.join("|");
    });
})()
"#,
    )
    .expect("Response private slot spoofing setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__responseSlotProbe.consumed !== null && __responseSlotProbe.fakeBodyMethods !== null)",
        "true",
        "response_backing_slots_ignore_reflection_and_spoofing",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__responseSlotProbe)")
        .expect("Response private slot spoofing probe result should evaluate");

    assert_eq!(
        result,
        r#"{"initial":"||201|Created|true|default||false|1|false","cloneInitial":"201|Created|true|default||false|1|null|false","errorSurface":"0|false|error||false|true|false|null|0|error|true|false","fakeSurface":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError","consumed":"payload|payload|true|true|throw:TypeError","fakeBodyMethods":"rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError"}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn request_backing_slots_ignore_reflection_and_spoofing() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://request-slots.test/", &loader);

    vm.eval(
        r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = (object) => Object.getOwnPropertyNames(object)
    .filter((name) => name.startsWith('__lmRequest') || name.startsWith('__lmNetworkBodySource'))
    .sort()
    .join(',');
  const nullable = (value) => value === null ? 'null' : String(value);

  const request = new Request('/source', {
    method: 'POST',
    body: 'payload',
    headers: [['X-Original', '1']],
    cache: 'reload',
    credentials: 'include',
    redirect: 'manual',
    duplex: 'half'
  });
  const initial = [
    internalNames(Request.prototype),
    internalNames(request),
    request.method,
    request.url,
    request.headers.get('X-Original'),
    request.cache,
    request.credentials,
    request.redirect,
    request.bodyUsed
  ].join('|');

  const spoofSource = {
    __lmNetworkBodySourceKind: 'bytes',
    __lmBody: 'spoof-body'
  };
  const spoofEntries = {
    __lmRequestMethod: 'GET',
    __lmRequestUrl: 'data:text/plain,spoof-url',
    __lmRequestHeaders: new Headers([['X-Spoof', 'bad']]),
    __lmRequestCache: 'only-if-cached',
    __lmRequestCredentials: 'omit',
    __lmRequestRedirect: 'error',
    __lmRequestBody: null,
    __lmRequestBodyUsed: true,
    __lmNetworkBodySource: spoofSource
  };
  for (const [name, value] of Object.entries(spoofEntries)) {
    Object.defineProperty(Request.prototype, name, {
      value,
      configurable: true
    });
    Object.defineProperty(request, name, {
      value,
      configurable: true
    });
  }

  const inherited = new Request(request);
  const clone = request.clone();
  const inheritedSurface = [
    inherited.method,
    inherited.url,
    inherited.headers.get('X-Original'),
    nullable(inherited.headers.get('X-Spoof')),
    inherited.cache,
    inherited.credentials,
    inherited.redirect,
    inherited.bodyUsed
  ].join('|');
  const cloneSurface = [
    clone.method,
    clone.url,
    clone.headers.get('X-Original'),
    nullable(clone.headers.get('X-Spoof')),
    clone.cache,
    clone.credentials,
    clone.redirect,
    clone.bodyUsed
  ].join('|');

  const fetchRequest = new Request('data:text/plain,fetch-ok');
  fetchRequest.__lmRequestUrl = 'data:text/plain,fetch-bad';
  fetchRequest.__lmNetworkBodySource = spoofSource;

  const fake = Object.create(Request.prototype);
  const bodyOutcome = async (prototype, method, receiver) => {
    let value;
    try {
      value = prototype[method].call(receiver);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
    const isPromise = value instanceof Promise;
    try {
      await value;
      return `resolved:${isPromise}`;
    } catch (error) {
      return `rejected:${isPromise}:${error && error.name}`;
    }
  };
  const fakeSurface = [
    probe(() => fake.method),
    probe(() => fake.headers),
    probe(() => fake.bodyUsed),
    probe(() => Request.prototype.clone.call(fake)),
  ].join('|');

  globalThis.__requestSlotProbe = {
    initial,
    inheritedSurface,
    cloneSurface,
    fakeSurface,
    consumed: null,
    fakeBodyMethods: null
  };
  Promise.all([
    request.text(),
    clone.text(),
    inherited.text(),
    fetch(fetchRequest).then((response) => response.text())
  ]).then(
    ([text, cloneText, inheritedText, fetchText]) => {
      globalThis.__requestSlotProbe.consumed = [
        text,
        cloneText,
        inheritedText,
        fetchText,
        request.bodyUsed,
        clone.bodyUsed,
        inherited.bodyUsed,
        probe(() => request.clone())
      ].join('|');
    },
    (error) => {
      globalThis.__requestSlotProbe.consumed = `reject:${error && error.name}`;
    }
  );
  Promise.all(["arrayBuffer", "blob", "bytes", "formData", "json", "text"]
    .map((method) => bodyOutcome(Request.prototype, method, fake))).then((outcomes) => {
      globalThis.__requestSlotProbe.fakeBodyMethods = outcomes.join("|");
    });
})()
"#,
    )
    .expect("Request private slot spoofing setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__requestSlotProbe.consumed !== null && __requestSlotProbe.fakeBodyMethods !== null)",
        "true",
        "request_backing_slots_ignore_reflection_and_spoofing",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__requestSlotProbe)")
        .expect("Request private slot spoofing probe result should evaluate");

    assert_eq!(
        result,
        r#"{"initial":"||POST|https://request-slots.test/source|1|reload|include|manual|false","inheritedSurface":"POST|https://request-slots.test/source|1|null|reload|include|manual|false","cloneSurface":"POST|https://request-slots.test/source|1|null|reload|include|manual|false","fakeSurface":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError","consumed":"payload|payload|payload|fetch-ok|true|true|true|throw:TypeError","fakeBodyMethods":"rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError"}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn request_constructor_rejects_used_or_locked_inherited_body() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://request-body.test/", &loader);

    vm.eval(
        r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const request = new Request("/used", {
    method: "POST",
    body: "payload",
    duplex: "half"
  });
  const cloneBefore = request.clone();
  const inheritedBefore = new Request(request);
  const locked = request.clone();
  locked.body.getReader();

  globalThis.__requestBodyUsedProbe = {
    bodyIsStream: request.body instanceof ReadableStream,
    bodyUsedBefore: request.bodyUsed,
    lockedNew: probe(() => new Request(locked)),
    lockedClone: probe(() => locked.clone()),
    done: null
  };

  Promise.all([
    request.text(),
    cloneBefore.text(),
    inheritedBefore.text(),
    request.text().then(
      value => `resolve:${value}`,
      error => `reject:${error && error.name}`
    )
  ]).then(
    ([requestText, cloneText, inheritedText, secondRead]) => {
      Object.assign(globalThis.__requestBodyUsedProbe, {
        requestText,
        cloneText,
        inheritedText,
        secondRead,
        bodyUsedAfter: request.bodyUsed,
        afterReplacement: probe(() => new Request(request, {
          body: "replacement",
          duplex: "half"
        }).bodyUsed),
        afterNew: probe(() => new Request(request)),
        afterClone: probe(() => request.clone()),
        done: "ok"
      });
    },
    error => {
      globalThis.__requestBodyUsedProbe.done = `reject:${error && error.name}`;
    }
  );
})()
"#,
    )
    .expect("Request bodyUsed probe setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__requestBodyUsedProbe.done !== null)",
        "true",
        "request_constructor_rejects_used_or_locked_inherited_body",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__requestBodyUsedProbe)")
        .expect("Request bodyUsed probe result should evaluate");

    assert_eq!(
        result,
        r#"{"bodyIsStream":true,"bodyUsedBefore":false,"lockedNew":"throw:TypeError","lockedClone":"throw:TypeError","done":"ok","requestText":"payload","cloneText":"payload","inheritedText":"payload","secondRead":"reject:TypeError","bodyUsedAfter":true,"afterReplacement":"false","afterNew":"throw:TypeError","afterClone":"throw:TypeError"}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_disturbs_the_input_request_body_before_network_completion() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://request-fetch-body-used.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const request = new Request("data:text/plain,done", {
    method: "POST",
    body: "payload"
  });
  const outcome = globalThis.__fetchRequestBodyUsed = {
    before: request.bodyUsed,
    immediate: null,
    clone: null,
    text: null
  };
  const promise = fetch(request);
  outcome.immediate = request.bodyUsed;
  try {
    request.clone();
    outcome.clone = "resolved";
  } catch (error) {
    outcome.clone = error && error.name;
  }
  promise.then(response => response.text()).then(text => {
    outcome.text = text;
  });
})()
"#,
    )
    .expect("fetch Request body disturbance probe should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__fetchRequestBodyUsed.text !== null)",
        "true",
        "fetch_disturbs_the_input_request_body_before_network_completion",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__fetchRequestBodyUsed)")
            .expect("fetch Request body disturbance result should evaluate"),
        r#"{"before":false,"immediate":true,"clone":"TypeError","text":"done"}"#
    );
}

#[test]
fn request_signal_init_inherited_and_clone_are_dependent() {
    let mut vm = new_storage_test_vm("https://request-signal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const controller = new AbortController();
  const request = new Request("/signal", { signal: controller.signal });
  const inherited = new Request(request);
  const clone = request.clone();
  const nullOverride = new Request(request, { signal: null });
  const preAborted = new Request("/pre-aborted", {
    signal: AbortSignal.abort("pre-aborted")
  });
  const events = [];
  request.signal.addEventListener("abort", () => events.push("request"));
  inherited.signal.addEventListener("abort", () => events.push("inherited"));
  clone.signal.addEventListener("abort", () => events.push("clone"));

  const before = {
    tag: Object.prototype.toString.call(request.signal),
    requestSignalDifferent: request.signal !== controller.signal,
    inheritedSignalDifferent: inherited.signal !== request.signal,
    cloneSignalDifferent: clone.signal !== request.signal,
    nullOverrideDifferent: nullOverride.signal !== request.signal,
    preAborted: preAborted.signal.aborted,
    preAbortedReason: String(preAborted.signal.reason),
    nullOverrideAborted: nullOverride.signal.aborted
  };
  controller.abort("request-abort");
  return JSON.stringify({
    before,
    after: {
      requestAborted: request.signal.aborted,
      requestReason: String(request.signal.reason),
      inheritedAborted: inherited.signal.aborted,
      inheritedReason: String(inherited.signal.reason),
      cloneAborted: clone.signal.aborted,
      cloneReason: String(clone.signal.reason),
      nullOverrideAborted: nullOverride.signal.aborted,
      events: events.sort()
    }
  });
})()
"#,
        )
        .expect("Request signal dependency probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":{"tag":"[object AbortSignal]","requestSignalDifferent":true,"inheritedSignalDifferent":true,"cloneSignalDifferent":true,"nullOverrideDifferent":true,"preAborted":true,"preAbortedReason":"pre-aborted","nullOverrideAborted":false},"after":{"requestAborted":true,"requestReason":"request-abort","inheritedAborted":true,"inheritedReason":"request-abort","cloneAborted":true,"cloneReason":"request-abort","nullOverrideAborted":false,"events":["clone","inherited","request"]}}"#
    );
}

#[test]
fn window_fetch_missing_input_rejects_type_error() {
    let mut vm = new_storage_test_vm("https://fetch-missing-input.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__fetchMissingInputResult = "pending";
  fetch().then(
    () => { globalThis.__fetchMissingInputResult = "resolved"; },
    (error) => { globalThis.__fetchMissingInputResult = error && error.name; }
  );
  return "scheduled";
})()
"#,
    )
    .expect("fetch missing input probe should evaluate");

    let result = vm
        .eval("String(globalThis.__fetchMissingInputResult)")
        .expect("fetch missing input rejection should settle");

    assert_eq!(result, "TypeError");
}

#[test]
fn window_fetch_document_csp_blocks_connect_src_and_dispatches_event() {
    let mut vm = new_storage_test_vm("https://fetch-connect-csp.test/");
    vm.set_response_content_security_policies(&[String::from("connect-src 'none'")]);

    vm.eval(
        r#"
(() => {
  globalThis.__fetchCspEvents = [];
  globalThis.__fetchCspResult = "pending";
  self.addEventListener("securitypolicyviolation", event => {
    __fetchCspEvents.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  fetch("data:text/plain,ok").then(
    () => { __fetchCspResult = "resolved"; },
    (error) => {
      __fetchCspResult = {
        name: error && error.name,
        csp: String(error && error.message).includes("Content Security Policy")
      };
    }
  );
  return "scheduled";
})()
"#,
    )
    .expect("fetch CSP block setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    let result = vm
        .eval(
            "JSON.stringify({ ...globalThis.__fetchCspResult, events: globalThis.__fetchCspEvents })",
        )
        .expect("fetch CSP rejection should settle");

    assert_eq!(
        result,
        r#"{"name":"TypeError","csp":true,"events":[{"blockedURI":"data","effectiveDirective":"connect-src","disposition":"enforce","instance":true}]}"#
    );
}

#[test]
fn window_fetch_document_csp_report_only_dispatches_without_blocking() {
    let mut vm = new_storage_test_vm("https://fetch-connect-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[String::from("connect-src 'none'")]);

    vm.eval(
        r#"
(() => {
  globalThis.__fetchReportOnlyEvents = [];
  globalThis.__fetchReportOnlyResult = "pending";
  self.addEventListener("securitypolicyviolation", event => {
    __fetchReportOnlyEvents.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  fetch("data:text/plain,ok").then(
    (response) => {
      __fetchReportOnlyResult = { status: response.status };
    },
    (error) => { __fetchReportOnlyResult = "rejected:" + (error && error.name); }
  );
  return "scheduled";
})()
"#,
    )
    .expect("fetch CSP report-only setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    let result = vm
        .eval(
            "JSON.stringify({ ...globalThis.__fetchReportOnlyResult, events: globalThis.__fetchReportOnlyEvents })",
        )
        .expect("fetch CSP report-only request should settle");

    assert_eq!(
        result,
        r#"{"status":200,"events":[{"blockedURI":"data","effectiveDirective":"connect-src","disposition":"report","instance":true}]}"#
    );
}

#[test]
fn request_redirect_init_uses_webidl_enum() {
    let mut vm = new_storage_test_vm("https://request-redirect-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const inherited = new Request("/source", { redirect: "manual" });
  let invalidName = "";
  try {
    new Request("/bad", { redirect: "invalid" });
  } catch (error) {
    invalidName = error && error.name;
  }
  return [
    new Request("/default").redirect,
    inherited.redirect,
    new Request(inherited, { redirect: "error" }).redirect,
    invalidName
  ].join("|");
})()
"#,
        )
        .expect("Request redirect init should evaluate");

    assert_eq!(result, "follow|manual|error|TypeError");
}

#[test]
fn request_mode_init_uses_webidl_enum_and_no_cors_method_gate() {
    let mut vm = new_storage_test_vm("https://request-mode-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const inherited = new Request("/source", { mode: "no-cors" });
  const probe = (callback) => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  return [
    new Request("/default").mode,
    inherited.mode,
    new Request(inherited, { mode: "same-origin" }).mode,
    probe(() => new Request("/bad", { mode: "invalid" })),
    probe(() => new Request("/nav", { mode: "navigate" })),
    probe(() => new Request("/put", { mode: "no-cors", method: "PUT" }))
  ].join("|");
})()
"#,
        )
        .expect("Request mode init should evaluate");

    assert_eq!(
        result,
        "cors|no-cors|same-origin|TypeError|TypeError|TypeError"
    );
}

#[test]
fn body_init_string_fallback_uses_webidl_usvstring_errors() {
    let mut vm = new_storage_test_vm("https://body-init-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const throwing = { toString() { throw new RangeError("body stringifier"); } };
  globalThis.__fetchBodySymbolResult = "pending";
  fetch("/body-symbol", { method: "POST", body: Symbol("body") }).then(
    () => { globalThis.__fetchBodySymbolResult = "resolved"; },
    (error) => { globalThis.__fetchBodySymbolResult = error && error.name; }
  );
  return JSON.stringify({
    requestSymbol: probe(() => new Request("/body-symbol", { method: "POST", body: Symbol("body") })),
    requestThrowing: probe(() => new Request("/body-throwing", { method: "POST", body: throwing })),
    responseSymbol: probe(() => new Response(Symbol("body"))),
    responseThrowing: probe(() => new Response(throwing)),
  });
})()
"#,
        )
        .expect("BodyInit conversion probe should evaluate");

    let fetch_result = vm
        .eval("String(globalThis.__fetchBodySymbolResult)")
        .expect("fetch body conversion rejection should settle");

    assert_eq!(
        result,
        r#"{"requestSymbol":"TypeError","requestThrowing":"RangeError","responseSymbol":"TypeError","responseThrowing":"RangeError"}"#
    );
    assert_eq!(fetch_result, "TypeError");
}

#[test]
fn url_search_params_delete_preserves_opaque_path_trailing_space() {
    let mut vm = new_storage_test_vm("https://url-opaque-path-query-removal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const first = new URL('data:space    ?test');
  first.searchParams.delete('test');
  const second = new URL('data:space    ?test#test');
  second.searchParams.delete('test');
  return JSON.stringify({
    firstPathname: first.pathname,
    firstHref: first.href,
    secondPathname: second.pathname,
    secondHref: second.href,
  });
})()
"#,
        )
        .expect("URL opaque path query removal probe should evaluate");

    assert_eq!(
        result,
        r#"{"firstPathname":"space   %20","firstHref":"data:space   %20","secondPathname":"space   %20","secondHref":"data:space   %20#test"}"#
    );
}

#[test]
fn url_search_params_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://url-search-params-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const params = new URLSearchParams('a=1&a=2&b=null');
  const seen = [];
  const thisArg = { marker: 'ctx' };
  params.append(null, undefined);
  params.set('c', 3);
  params.delete('a', 1);
  params.forEach(function(value, name, owner) {
    seen.push(`${this.marker}:${name}:${value}:${owner === params}`);
  }, thisArg);
  return JSON.stringify({
    allA: params.getAll('a').join(','),
    hasANull: params.has('a', null),
    hasBNull: params.has('b', null),
    nullValue: params.get(null),
    serialized: params.toString(),
    seen,
    getSymbol: probe(() => params.get(Symbol())),
    appendMissingValue: probe(() => params.append('x')),
    deleteSymbolValue: probe(() => params.delete('a', Symbol())),
    hasSymbolName: probe(() => params.has(Symbol())),
    forEachMissing: probe(() => params.forEach())
  });
})()
"#,
        )
        .expect("URLSearchParams WebIDL argument conversion probe should run");

    assert_eq!(
        result,
        r#"{"allA":"2","hasANull":false,"hasBNull":true,"nullValue":"undefined","serialized":"a=2&b=null&null=undefined&c=3","seen":["ctx:a:2:true","ctx:b:null:true","ctx:null:undefined:true","ctx:c:3:true"],"getSymbol":"throw:TypeError","appendMissingValue":"throw:TypeError","deleteSymbolValue":"throw:TypeError","hasSymbolName":"throw:TypeError","forEachMissing":"throw:TypeError"}"#
    );
}

#[test]
fn form_data_and_url_search_params_template_methods_have_webidl_descriptors() {
    let mut vm = new_storage_test_vm("https://template-method-descriptors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodDescriptor = (Constructor, key) => {
    const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, key);
    return [
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(':');
  };
  const iteratorAliasShape = Constructor => {
    const entries = Object.getOwnPropertyDescriptor(Constructor.prototype, 'entries');
    const iterator = Object.getOwnPropertyDescriptor(Constructor.prototype, Symbol.iterator);
    return [
      entries.value === iterator.value,
      iterator.enumerable,
      iterator.writable,
      iterator.configurable,
      iterator.value.name,
      iterator.value.length
    ].join(':');
  };
  return JSON.stringify({
    uspAppend: methodDescriptor(URLSearchParams, 'append'),
    uspForEach: methodDescriptor(URLSearchParams, 'forEach'),
    uspIterator: iteratorAliasShape(URLSearchParams),
    fdAppend: methodDescriptor(FormData, 'append'),
    fdForEach: methodDescriptor(FormData, 'forEach'),
    fdIterator: iteratorAliasShape(FormData)
  });
})()
"#,
        )
        .expect("template method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"uspAppend":"function:append:2:true:true:true","uspForEach":"function:forEach:1:true:true:true","uspIterator":"true:false:true:true:entries:0","fdAppend":"function:append:2:true:true:true","fdForEach":"function:forEach:1:true:true:true","fdIterator":"true:false:true:true:entries:0"}"#
    );
}

#[test]
fn url_search_params_iterators_share_a_webidl_iterator_prototype() {
    let mut vm = new_storage_test_vm("https://url-search-params-iterator-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const params = new URLSearchParams("a=1");
  const entries = params.entries();
  const keys = params.keys();
  const prototype = Object.getPrototypeOf(entries);
  const intrinsicIteratorPrototype = Object.getPrototypeOf(
    Object.getPrototypeOf([][Symbol.iterator]())
  );
  const next = Object.getOwnPropertyDescriptor(prototype, "next");
  const outcome = callback => {
    try { callback(); return "return"; }
    catch (error) { return error && error.name; }
  };
  return JSON.stringify({
    shared: prototype === Object.getPrototypeOf(keys),
    base: Object.getPrototypeOf(prototype) === intrinsicIteratorPrototype,
    tag: Object.prototype.toString.call(entries),
    ownTag: Object.hasOwn(entries, Symbol.toStringTag),
    next: [typeof next.value, next.enumerable, next.writable, next.configurable].join(":"),
    invalid: [
      outcome(() => prototype.next()),
      outcome(() => prototype.next.call(new Headers().entries()))
    ].join(",")
  });
})()
"#,
        )
        .expect("URLSearchParams iterator prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"shared":true,"base":true,"tag":"[object URLSearchParams Iterator]","ownTag":false,"next":"function:true:true:true","invalid":"TypeError,TypeError"}"#
    );
}

#[test]
fn webidl_iterator_prototypes_use_v8_intrinsics_after_public_tampering() {
    let mut vm = new_storage_test_vm("https://webidl-iterator-intrinsics.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ArrayConstructor = Array;
  const originalArrayIterator = ArrayConstructor.prototype[Symbol.iterator];
  const originalObjectGetPrototypeOf = Object.getPrototypeOf;
  const getPrototypeOf = Reflect.getPrototypeOf;
  const iteratorPrototype = getPrototypeOf(
    getPrototypeOf([][Symbol.iterator]())
  );
  const asyncIteratorPrototype = getPrototypeOf(
    getPrototypeOf(async function*() {}).prototype
  );
  const specs = [
    ["URLSearchParams", () => new URLSearchParams("a=1").entries()],
    ["Headers", () => new Headers({ "x-a": "1" }).entries()],
    ["FormData", () => new FormData().entries()]
  ];
  const failures = [];
  const poisoned = function poisonedIterator() {
    throw new Error("public Array iterator was observed");
  };
  ArrayConstructor.prototype[Symbol.iterator] = poisoned;
  Object.getPrototypeOf = function poisonedGetPrototypeOf() {
    throw new Error("public Object.getPrototypeOf was observed");
  };
  globalThis.Array = undefined;
  try {
    for (let index = 0; index < specs.length; index += 1) {
      const name = specs[index][0];
      const iterator = specs[index][1]();
      const prototype = getPrototypeOf(iterator);
      const next = Object.getOwnPropertyDescriptor(prototype, "next");
      const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
      if (getPrototypeOf(prototype) !== iteratorPrototype) {
        failures.push(`${name}:parent`);
      }
      if (iterator[Symbol.iterator]() !== iterator) {
        failures.push(`${name}:iterator`);
      }
      if (Object.hasOwn(iterator, Symbol.iterator)) {
        failures.push(`${name}:own-iterator`);
      }
      if (Object.hasOwn(prototype, "constructor")) {
        failures.push(`${name}:constructor`);
      }
      if (
        !next ||
        next.enumerable !== true ||
        next.writable !== true ||
        next.configurable !== true
      ) {
        failures.push(`${name}:next`);
      }
      if (
        !tag ||
        tag.value !== `${name} Iterator` ||
        tag.enumerable !== false ||
        tag.writable !== false ||
        tag.configurable !== true
      ) {
        failures.push(`${name}:tag`);
      }
    }
    const asyncIterator = new ReadableStream({
      start(controller) {
        controller.close();
      }
    }).values();
    const asyncPrototype = getPrototypeOf(asyncIterator);
    const asyncTag = Object.getOwnPropertyDescriptor(
      asyncPrototype,
      Symbol.toStringTag
    );
    if (getPrototypeOf(asyncPrototype) !== asyncIteratorPrototype) {
      failures.push("ReadableStream:parent");
    }
    if (asyncIterator[Symbol.asyncIterator]() !== asyncIterator) {
      failures.push("ReadableStream:iterator");
    }
    if (Object.hasOwn(asyncIterator, Symbol.asyncIterator)) {
      failures.push("ReadableStream:own-iterator");
    }
    if (Object.hasOwn(asyncPrototype, "constructor")) {
      failures.push("ReadableStream:constructor");
    }
    if (
      !asyncTag ||
      asyncTag.value !== "ReadableStream AsyncIterator" ||
      asyncTag.enumerable !== false ||
      asyncTag.writable !== false ||
      asyncTag.configurable !== true
    ) {
      failures.push("ReadableStream:tag");
    }
  } finally {
    globalThis.Array = ArrayConstructor;
    ArrayConstructor.prototype[Symbol.iterator] = originalArrayIterator;
    Object.getPrototypeOf = originalObjectGetPrototypeOf;
  }
  return failures.join("|");
})()
"#,
        )
        .expect("WebIDL iterator prototypes should use V8 intrinsics");

    assert_eq!(result, "");
}

#[test]
fn url_search_params_subclasses_retain_their_webidl_brand() {
    let mut vm = new_storage_test_vm("https://url-search-params-subclass.test/");

    let result = vm
        .eval(
            r#"
(() => {
  class ReadonlyURLSearchParams extends URLSearchParams {
    append() { throw new Error('readonly'); }
    delete() { throw new Error('readonly'); }
    set() { throw new Error('readonly'); }
    sort() { throw new Error('readonly'); }
  }

  const params = new ReadonlyURLSearchParams('loc=fr&loc=de&empty=');
  const seen = [];
  params.forEach((value, name, owner) => {
    seen.push(`${name}:${value}:${owner === params}`);
  });

  return JSON.stringify({
    directPrototypeIsSubclass:
      Object.getPrototypeOf(params) === ReadonlyURLSearchParams.prototype,
    get: params.get('loc'),
    getAll: params.getAll('loc').join(','),
    has: params.has('empty'),
    size: params.size,
    entries: Array.from(params.entries()).map(pair => pair.join(':')).join(','),
    seen,
    serialized: params.toString()
  });
})()
"#,
        )
        .expect("URLSearchParams subclass WebIDL operations should evaluate");

    assert_eq!(
        result,
        r#"{"directPrototypeIsSubclass":true,"get":"fr","getAll":"fr,de","has":true,"size":3,"entries":"loc:fr,loc:de,empty:","seen":["loc:fr:true","loc:de:true","empty::true"],"serialized":"loc=fr&loc=de&empty="}"#
    );
}

#[tokio::test]
async fn response_headers_keep_receiver_realm_across_borrowed_getters_and_clones() {
    let mut vm = new_storage_test_vm("https://headers-owner-realm.test/");
    vm.eval(
        r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || root.appendChild(document.createElement('body'));
globalThis.headersFrame = document.createElement('iframe');
body.appendChild(headersFrame);
"#,
    )
    .unwrap();
    assert_initial_about_blank_child_completed_synchronously_for_test(&mut vm, "Headers realm")
        .await;
    let _ = materialize_single_child_default_realm_for_test(&mut vm, "Headers realm");
    let result = vm.eval(r#"
(() => {
  const check = (value, label) => { if (!value) throw new Error(label); };
  const child = headersFrame.contentWindow;
  const parentHeaders = Headers;
  const childHeaders = child.Headers;
  const parentResponse = new Response('parent', {headers: {'x-realm': 'parent'}});
  const childResponse = new child.Response('child', {headers: {'x-realm': 'child'}});
  const parentGetter = Object.getOwnPropertyDescriptor(Response.prototype, 'headers').get;
  const childGetter = Object.getOwnPropertyDescriptor(child.Response.prototype, 'headers').get;
  const descriptors = [globalThis, child].map(realm => Object.getOwnPropertyDescriptor(realm, 'Headers'));
  for (const realm of [globalThis, child]) Object.defineProperty(realm, 'Headers', {
    configurable: true, get() { throw new Error('public Headers lookup'); }
  });
  try {
    for (const [response, clone, ctor, getter, value] of [
      [parentResponse, child.Response.prototype.clone.call(parentResponse), parentHeaders, childGetter, 'parent'],
      [childResponse, Response.prototype.clone.call(childResponse), childHeaders, parentGetter, 'child']
    ]) {
      check(getter.call(response) === response.headers, 'borrowed getter returns associated Headers');
      for (const entry of [response, clone]) {
        const headers = entry.headers;
        check(Object.getPrototypeOf(headers) === ctor.prototype && headers instanceof ctor,
          'Headers must use the response realm');
        check(headers.get === ctor.prototype.get && Reflect.ownKeys(headers).length === 0,
          'Headers must share realm prototype methods');
        check(headers.get('x-realm') === value, 'cross-realm entries');
        check(ctor.prototype.get.call(headers, 'x-realm') === value, 'branded receiver');
      }
    }
    return 'ok';
  } finally {
    [globalThis, child].forEach((realm, index) => Object.defineProperty(realm, 'Headers', descriptors[index]));
  }
})()
"#).unwrap();
    assert_eq!(result, "ok");
}
