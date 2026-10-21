use super::*;

#[test]
fn fullscreen_enabled_reflects_committed_document_policy() {
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-policy.test/",
        "<!doctype html><iframe id='child'></iframe>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = document.getElementById("child");
  child.allow = "sync-xhr 'none'";
  return JSON.stringify({
    enabled: [
      document.fullscreenEnabled,
      child.contentDocument.fullscreenEnabled,
      new Document().fullscreenEnabled
    ],
    allow: [child.allow, child.getAttribute("allow")]
  });
})()
"#,
        )
        .expect("fullscreen policy probe should evaluate");

    assert_eq!(
        result,
        r#"{"enabled":[true,true,false],"allow":["sync-xhr 'none'","sync-xhr 'none'"]}"#
    );
}

#[tokio::test]
async fn unsupported_fullscreen_requests_reject_and_queue_spec_shaped_error_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-unsupported.test/",
        "<!doctype html><html><body><div></div></body></html>",
    );

    let before = vm
        .eval(
            r#"
(() => {
  globalThis.__fullscreenEvents = [];
  globalThis.__fullscreenRejections = [];
  globalThis.__fullscreenConversionRejections = [];
  globalThis.__fullscreenReceiverRejections = [];
  globalThis.__fullscreenOptionReads = [];
  document.body.setAttribute("onfullscreenerror", "globalThis.__fullscreenAttributeRan = true");
  const contentAttributeInstalled = typeof document.body.onfullscreenerror === "function";
  document.onfullscreenerror = event => {
    __fullscreenEvents.push([
      event.type,
      event.target === document.body,
      event.bubbles,
      event.cancelable,
      event.composed
    ].join(":"));
  };
  const options = {
    get keyboardLock() {
      __fullscreenOptionReads.push("keyboardLock");
      return "none";
    },
    get navigationUI() {
      __fullscreenOptionReads.push("navigationUI");
      return "hide";
    }
  };
  const request = document.body.requestFullscreen(options);
  const exit = document.exitFullscreen();
  request.catch(error => __fullscreenRejections.push(`request:${error.name}`));
  exit.catch(error => __fullscreenRejections.push(`exit:${error.name}`));
  const getterError = new RangeError("navigationUI getter failed");
  const conversionInputs = [
    ["enum", { navigationUI: "invalid-value" }, null],
    ["string", "foo", null],
    ["number", 123, null],
    ["getter", {
      get navigationUI() {
        throw getterError;
      }
    }, getterError]
  ];
  const conversionPromiseShapes = conversionInputs.map(([label, input, expected]) => {
    try {
      const promise = document.body.requestFullscreen(input);
      promise.catch(error => __fullscreenConversionRejections.push(
        `${label}:${error.name}:${expected === null || error === expected}`
      ));
      return promise instanceof Promise;
    } catch (error) {
      return `throw:${error.name}`;
    }
  });
  const outcome = callback => {
    try {
      const promise = callback();
      promise.catch(error => __fullscreenReceiverRejections.push(error.name));
      return promise instanceof Promise;
    } catch (error) {
      return error.name;
    }
  };
  return JSON.stringify({
    requestShape: [
      typeof Element.prototype.requestFullscreen,
      Element.prototype.requestFullscreen.length,
      Object.hasOwn(Element.prototype, "onfullscreenchange"),
      Object.hasOwn(Element.prototype, "onfullscreenerror")
    ],
    documentShape: [
      typeof Document.prototype.exitFullscreen,
      Document.prototype.exitFullscreen.length,
      Object.hasOwn(Document.prototype, "onfullscreenchange"),
      Object.hasOwn(Document.prototype, "onfullscreenerror"),
      document.fullscreenEnabled
    ],
    promises: request instanceof Promise && exit instanceof Promise,
    conversionPromiseShapes,
    contentAttributeInstalled,
    optionReads: __fullscreenOptionReads,
    wrongReceivers: [
      outcome(() => Element.prototype.requestFullscreen.call({})),
      outcome(() => Document.prototype.exitFullscreen.call({}))
    ],
    events: __fullscreenEvents.length
  });
})()
"#,
        )
        .expect("fullscreen rejection setup should evaluate");

    assert_eq!(
        before,
        r#"{"requestShape":["function",0,true,true],"documentShape":["function",0,true,true,true],"promises":true,"conversionPromiseShapes":[true,true,true,true],"contentAttributeInstalled":true,"optionReads":["keyboardLock","navigationUI"],"wrongReceivers":[true,true],"events":0}"#,
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("fullscreen error task should drain");

    let after = vm
        .eval(
            r#"
JSON.stringify({
  events: __fullscreenEvents,
  rejections: __fullscreenRejections.sort(),
  conversionRejections: __fullscreenConversionRejections.sort(),
  receiverRejections: __fullscreenReceiverRejections,
  contentAttributeRan: globalThis.__fullscreenAttributeRan === true
})
"#,
        )
        .expect("fullscreen rejection result should evaluate");
    assert_eq!(
        after,
        r#"{"events":["fullscreenerror:true:true:false:true"],"rejections":["exit:TypeError","request:TypeError"],"conversionRejections":["enum:TypeError:true","getter:RangeError:true","number:TypeError:true","string:TypeError:true"],"receiverRejections":["TypeError","TypeError"],"contentAttributeRan":true}"#,
    );
}

#[test]
fn fullscreen_members_use_native_receivers_and_callee_realm_promise_rejections() {
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-native-bindings.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(include_str!(
        "../../../../tests/fixtures/fullscreen-native-bindings.js"
    ))
    .expect("Fullscreen binding matrix should evaluate");
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1361");
    assert_eq!(
        vm.eval("__uiEventResults.passed").unwrap(),
        "1361",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn fullscreen_members_accept_registered_native_proxies_without_author_traps() {
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(r#"
      globalThis.fullscreenProxyRecords = [];
      globalThis.fullscreenProxyTraps = 0;
      const other = document.querySelector('#child').contentWindow;
      for (const owner of [globalThis, other]) for (const flavor of ['live', 'html', 'xml']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('fullscreen') :
          owner.document.implementation.createDocument(null, 'r');
        const element = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        const root = element.attachShadow({mode: 'open'});
        for (const [kind, target] of [['Document', doc], ['Element', element], ['ShadowRoot', root]]) {
          const handler = Object.create(null);
          handler.get = () => {fullscreenProxyTraps++; throw Error('get');};
          handler.getPrototypeOf = () => {fullscreenProxyTraps++; throw Error('getPrototypeOf');};
          fullscreenProxyRecords.push({kind, target, handler, callee: owner === globalThis ? other : globalThis});
        }
      }
      true;
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "fullscreenProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 18);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "target").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(value).unwrap();
            let target = moli_webapi_declare::web_api_object_target(scope, target).unwrap();
            let context = target.get_creation_context(scope).unwrap();
            let key = v8::String::new(scope, "handler").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let handler = v8::Local::<v8::Object>::try_from(value).unwrap();
            let proxy = {
                let scope = &mut v8::ContextScope::new(scope, context);
                let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                proxy
            };
            let key = v8::String::new(scope, "proxy").unwrap();
            assert!(record.set(scope, key.into(), proxy.into()).unwrap());
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"(() => {
      const jobs = [];
      for (const [index, {kind, target, proxy, callee}] of fullscreenProxyRecords.entries()) {
        const check = (passed, name) => {if (!passed) throw Error(`${index}/${kind}: ${name}`);};
        const typeError = error => Object.getPrototypeOf(error) === callee.TypeError.prototype;
        const author = new Proxy(proxy, {});
        const proto = callee[kind].prototype;
        const members = kind === 'Document' ? ['fullscreen', 'fullscreenElement', 'fullscreenEnabled'] :
          kind === 'ShadowRoot' ? ['fullscreenElement'] : [];
        for (const member of members) {
          const descriptor = Object.getOwnPropertyDescriptor(proto, member);
          const before = descriptor.get.call(target);
          check(descriptor.get.call(proxy) === before, member + ' native getter');
          descriptor.set.call(proxy, {toString() {throw Error('unused value');}});
          check(descriptor.get.call(target) === before, member + ' lenient setter');
          for (const callback of [() => descriptor.get.call(author), () => descriptor.set.call(author, 0)]) {
            let caught;
            try {callback();} catch(error) {caught = error;}
            check(caught && typeError(caught), member + ' author proxy');
          }
        }
        if (kind === 'ShadowRoot') continue;
        const method = proto[kind === 'Document' ? 'exitFullscreen' : 'requestFullscreen'];
        const marker = {};
        let converted = 0;
        for (const receiver of [proxy, author]) {
          const genuine = receiver === proxy;
          const promise = method.call(receiver, {get navigationUI() {converted++; throw marker;}});
          check(Object.getPrototypeOf(promise) === callee.Promise.prototype, 'callee Promise');
          jobs.push(promise.then(() => {throw Error('unexpected success');}, error => {
            check(kind === 'Element' && genuine ? error === marker : typeError(error), 'rejection identity');
          }));
          check(converted === (kind === 'Element' ? 1 : 0), 'native conversion only');
        }
        check(fullscreenProxyTraps === 0, 'no traps');
      }
      return Promise.all(jobs).then(() => {
        if (fullscreenProxyTraps !== 0) throw Error('late proxy trap');
        globalThis.fullscreenProxyComplete = true;
      });
    })()"#).unwrap();
    assert_eq!(vm.eval("fullscreenProxyComplete").unwrap(), "true");
}
