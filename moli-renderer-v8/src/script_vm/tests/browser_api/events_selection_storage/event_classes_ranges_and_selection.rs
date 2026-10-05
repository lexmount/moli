use super::*;

#[test]
fn pointer_event_interface_sequences_preserve_backing_checks_and_getter_order() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-sequence-conversion.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    let result = vm.eval(r#"
        (() => {
          const assert = (ok, message) => { if (!ok) throw new Error(message); };
          const child = document.getElementById('child').contentWindow;
          const foreign = new child.PointerEvent('pointermove');
          Object.setPrototypeOf(foreign, null);
          const order = [];
          const event = new PointerEvent('pointermove', {
            get pointerType() {order.push('pointerType'); return 'mouse';},
            get coalescedEvents() {order.push('coalesced'); return new Set([foreign]);},
            get predictedEvents() {order.push('predicted'); return [foreign];},
            get bubbles() {order.push('bubbles'); return false;},
            get view() {order.push('view'); return null;}
          });
          assert(event.getCoalescedEvents()[0] === foreign && event.getPredictedEvents()[0] === foreign, 'cross realm backing identity');
          assert(order.indexOf('bubbles') < order.indexOf('pointerType') &&
            order.indexOf('pointerType') < order.indexOf('coalesced') &&
            order.indexOf('coalesced') < order.indexOf('predicted') &&
            order.indexOf('predicted') < order.indexOf('view'), 'sequence conversion position: ' + order);
          let reads = 0, closes = 0;
          let error;
          try {
            new child.PointerEvent('pointermove', {
              coalescedEvents: {[Symbol.iterator]() {return {
                next() {reads++; return {done: false, value: new Event('invalid')};},
                return() {closes++; return {done: true};}
              };}},
              get predictedEvents() {reads++; throw 'late predicted';},
              get view() {reads++; throw 'late view';}
            });
          } catch (caught) {error = caught;}
          assert(error instanceof child.TypeError && reads === 1 && closes === 0, 'backing rejection must stop before later fields');
          return 'ok';
        })()
    "#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn event_listener_exceptions_report_window_error_and_continue_dispatch() {
    let mut vm = new_storage_test_vm("https://event-listener-error-report.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const thrown = { name: "test" };
              const calls = [];
              const errors = [];
              window.onerror = function(message, source, line, column, error) {
                errors.push({
                  messageType: typeof message,
                  exact: error === thrown,
                  sourceType: typeof source,
                  lineType: typeof line,
                  columnType: typeof column
                });
                return true;
              };

              target.addEventListener("foo", {
                get handleEvent() {
                  calls.push("get");
                  throw thrown;
                }
              });
              target.addEventListener("foo", () => calls.push("after"));

              const returned = target.dispatchEvent(new Event("foo"));
              return JSON.stringify({ returned, calls, errors });
            })()
            "#,
        )
        .expect("event listener exception report probe should evaluate");

    assert_eq!(
        result,
        r#"{"returned":true,"calls":["get","after"],"errors":[{"messageType":"string","exact":true,"sourceType":"string","lineType":"number","columnType":"number"}]}"#
    );
}

#[test]
fn synthetic_error_event_uses_normal_window_event_handler_arguments() {
    let mut vm = new_storage_test_vm("https://synthetic-window-error-event.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const observations = {};
              document.body.onerror = (...args) => {
                observations.argumentCount = args.length;
                observations.receivedEvent = args[0] === event;
                return true;
              };
              const event = new Event("error", { bubbles: true, cancelable: true });
              document.body.dispatchEvent(event);
              observations.defaultPrevented = event.defaultPrevented;

              const errorEventObservations = {};
              document.body.onerror = (...args) => {
                errorEventObservations.argumentCount = args.length;
                errorEventObservations.message = args[0];
                errorEventObservations.source = args[1];
                errorEventObservations.errorMatches = args[4] === errorEvent.error;
                return true;
              };
              const errorEvent = new ErrorEvent("error", {
                cancelable: true,
                message: "boom",
                filename: "probe.js"
              });
              window.dispatchEvent(errorEvent);
              errorEventObservations.defaultPrevented = errorEvent.defaultPrevented;
              observations.errorEvent = errorEventObservations;
              return JSON.stringify(observations);
            })()
            "#,
        )
        .expect("synthetic error event handler probe should evaluate");

    assert_eq!(
        result,
        r#"{"argumentCount":1,"receivedEvent":true,"defaultPrevented":false,"errorEvent":{"argumentCount":5,"message":"boom","source":"probe.js","errorMatches":true,"defaultPrevented":true}}"#
    );
}

#[test]
fn window_onerror_null_assignment_supersedes_uncompiled_body_attribute() {
    let mut vm = new_storage_test_vm("https://window-onerror-null-override.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          globalThis.__bodyErrorAttributeRan = false;
          document.body.setAttribute("onerror", "globalThis.__bodyErrorAttributeRan = true");
          window.onerror = null;
          addEventListener("error", event => event.preventDefault(), { once: true });
        })()
        "#,
    )
    .expect("body onerror null override setup should evaluate");

    vm.report_window_script_failure_and_checkpoint_for_test(
        "window onerror null override probe",
        Some("https://window-onerror-null-override.test/probe.js"),
        None,
    );

    let result = vm
        .eval("JSON.stringify([globalThis.__bodyErrorAttributeRan, window.onerror === null])")
        .expect("body onerror null override result should evaluate");
    assert_eq!(result, "[false,true]");
}

#[test]
fn window_script_failure_report_does_not_infer_constructor_from_message_text() {
    let mut vm = new_storage_test_vm("https://window-script-failure-report.test/");
    vm.eval(
        r#"
        (() => {
          const OriginalError = Error;
          function FakeError() {}
          globalThis.Error = FakeError;
          globalThis.__windowScriptFailureReports = [];
          addEventListener("error", event => {
            __windowScriptFailureReports.push([
              event.error && event.error.constructor && event.error.constructor.name,
              event.error instanceof OriginalError,
              event.error instanceof FakeError,
              event.error instanceof SyntaxError,
              event.error instanceof WebAssembly.LinkError,
              event.message
            ].join("|"));
            event.preventDefault();
          });
          return "ready";
        })()
        "#,
    )
    .expect("window error listener setup should evaluate");

    vm.report_window_script_failure_and_checkpoint_for_test(
        "CompileError LinkError SyntaxError user-controlled text",
        Some("https://window-script-failure-report.test/script.js"),
        None,
    );

    let result = vm
        .eval("__windowScriptFailureReports.join(',')")
        .expect("window script failure report result should evaluate");
    assert_eq!(
        result,
        "Error|true|false|false|false|CompileError LinkError SyntaxError user-controlled text"
    );
}

#[test]
fn window_script_failure_error_event_is_trusted_without_trusting_synthetic_events() {
    let mut vm = new_storage_test_vm("https://window-error-trusted.test/");
    vm.eval(
        r#"
        (() => {
          globalThis.__windowErrorTrust = [];
          addEventListener("error", event => {
            __windowErrorTrust.push(event.isTrusted);
            event.preventDefault();
          });
        })()
        "#,
    )
    .expect("window error trust listener setup should evaluate");

    vm.report_window_error_body_best_effort(
        "trusted browser-generated error",
        Some("https://window-error-trusted.test/script.js"),
        None,
    );
    let result = vm
        .eval(
            r#"
            window.dispatchEvent(new ErrorEvent("error", { cancelable: true }));
            JSON.stringify(globalThis.__windowErrorTrust)
            "#,
        )
        .expect("window error trust result should evaluate");

    assert_eq!(result, "[true,false]");
}

#[test]
fn cross_realm_event_listener_object_errors_report_to_listener_window() {
    let mut vm = new_storage_test_vm("https://event-listener-cross-realm.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              frame.name = "eventListenerGlobalObject";
              (document.body || document.documentElement || document).appendChild(frame);
              const child = eventListenerGlobalObject;
              const target = new EventTarget();
              const missingHandleEvent = new child.Object();
              const events = [];

              child.addEventListener("error", event => {
                events.push({
                  targetIsChild: event.target === child,
                  errorIsChildTypeError: event.error &&
                    event.error.constructor === child.TypeError
                });
                event.preventDefault();
              });

              target.addEventListener("boom", missingHandleEvent);
              target.dispatchEvent(new Event("boom"));

              return JSON.stringify(events);
            })()
            "#,
        )
        .expect("cross-realm EventListener object error probe should evaluate");

    assert_eq!(
        result,
        r#"[{"targetIsChild":true,"errorIsChildTypeError":true}]"#
    );
}

#[test]
fn cross_realm_event_listener_error_cases_report_to_listener_window() {
    let mut vm = new_storage_test_vm("https://event-listener-cross-realm-cases.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              frame.name = "eventListenerGlobalObject";
              (document.body || document.documentElement || document).appendChild(frame);
              const child = eventListenerGlobalObject;
              const target = new EventTarget();
              const results = [];

              function record(name, expectedConstructor, listener) {
                function onerror(event) {
                  results.push([
                    name,
                    event.target === child,
                    !!event.error && event.error.constructor === expectedConstructor
                  ]);
                  event.preventDefault();
                }
                child.addEventListener("error", onerror);
                target.addEventListener(name, listener);
                target.dispatchEvent(new Event(name));
                child.removeEventListener("error", onerror);
              }

              record("missing", child.TypeError, new child.Object());

              const nonCallable = new child.Object();
              nonCallable.handleEvent = null;
              record("non-callable", child.TypeError, nonCallable);

              const revokedHandle = new child.Object();
              const handleProxy = child.Proxy.revocable(function() {}, {});
              revokedHandle.handleEvent = handleProxy.proxy;
              handleProxy.revoke();
              record("revoked-handle", child.TypeError, revokedHandle);

              const objectProxy = child.Proxy.revocable({}, {});
              objectProxy.revoke();
              record("revoked-object", child.TypeError, objectProxy.proxy);

              const functionProxy = child.Proxy.revocable(function() {}, {});
              functionProxy.revoke();
              record("revoked-function", child.TypeError, functionProxy.proxy);

              return JSON.stringify(results);
            })()
            "#,
        )
        .expect("cross-realm EventListener error matrix should evaluate");

    assert_eq!(
        result,
        r#"[["missing",true,true],["non-callable",true,true],["revoked-handle",true,true],["revoked-object",true,true],["revoked-function",true,true]]"#
    );
}

#[test]
fn child_window_listener_markers_avoid_public_getters_and_preserve_error_surface() {
    let mut vm = new_storage_test_vm("https://event-listener-child-marker.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              frame.name = "childMarkerWindow";
              (document.body || document.documentElement || document).appendChild(frame);
              const child = childMarkerWindow;
              const target = new EventTarget();
              const errors = [];
              const registration = [];

              const getterListener = {};
              Object.defineProperty(getterListener, "__moliCallbackErrorWindowHandle", {
                get() {
                  throw new Error("marker getter should not run");
                }
              });
              try {
                target.addEventListener("getter", getterListener);
                registration.push("ok");
              } catch (error) {
                registration.push(error.message);
              }
              try {
                matchMedia("(min-width: 0px)").addEventListener("change", getterListener);
                registration.push("mql-ok");
              } catch (error) {
                registration.push(`mql:${error.message}`);
              }

              child.onerror = function(message, source, line, column, error, extra) {
                errors.push({
                  argc: arguments.length,
                  messageType: typeof message,
                  sourceType: typeof source,
                  lineType: typeof line,
                  columnType: typeof column,
                  errorIsChildTypeError: !!error && error.constructor === child.TypeError,
                  extraIsUndefined: extra === undefined,
                  currentTargetIsChild: event.currentTarget === child
                });
                return true;
              };

              const created = child.Object.create(null);
              target.addEventListener("created", created);
              target.dispatchEvent(new Event("created"));

              let proxyCallName = "none";
              try {
                child.Proxy({}, {});
              } catch (error) {
                proxyCallName = error.name;
              }
              const proxyConstructWorks = !!new child.Proxy({}, {});

              return JSON.stringify({
                registration,
                errors,
                proxyCallName,
                proxyConstructWorks
              });
            })()
            "#,
        )
        .expect("child listener marker and error surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"registration":["ok","mql-ok"],"errors":[{"argc":5,"messageType":"string","sourceType":"string","lineType":"number","columnType":"number","errorIsChildTypeError":true,"extraIsUndefined":true,"currentTargetIsChild":true}],"proxyCallName":"TypeError","proxyConstructWorks":true}"#
    );
}

#[test]
fn child_window_forwarded_constructors_use_captured_native_intrinsics() {
    let mut vm = new_storage_test_vm("https://child-window-native-intrinsics.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const child = frame.contentWindow;
              const childDocument = frame.contentDocument;

              const object = new child.Object();
              const nullPrototypeObject = child.Object.create(null);
              const proxyTarget = {};
              const proxy = new child.Proxy(proxyTarget, {});
              const revocable = child.Proxy.revocable({}, {});
              const range = new child.Range();
              const staticRange = new child.StaticRange({
                startContainer: childDocument,
                startOffset: 0,
                endContainer: childDocument,
                endOffset: childDocument.childNodes.length
              });
              const readable = new child.ReadableStream();

              let proxyCallError = "none";
              try {
                child.Proxy({}, {});
              } catch (error) {
                proxyCallError = error.name;
              }
              revocable.revoke();

              return [
                Object.getPrototypeOf(object) === child.Object.prototype,
                Object.getPrototypeOf(nullPrototypeObject) === null,
                proxy !== proxyTarget,
                proxyCallError,
                range.startContainer === childDocument,
                range.endContainer === childDocument,
                staticRange.startContainer === childDocument,
                staticRange.endContainer === childDocument,
                typeof readable.getReader === "function"
              ].join("|");
            })()
            "#,
        )
        .expect("child window forwarded constructors should evaluate without recursion");

    assert_eq!(result, "true|true|true|TypeError|true|true|true|true|true");
}

#[test]
fn cross_realm_listener_throw_reports_listener_global_not_target_global() {
    let mut vm = new_storage_html_test_vm("https://event-listener-multiple-globals.test/");

    vm.eval(
        r#"
        (() => {
          const host = document.body || document.documentElement || document;
          const frameA = document.createElement("iframe");
          frameA.srcdoc = `<script>
            function listener() { throw new Error(); }
            objectListener = {};
          <\/script>`;
          const frameB = document.createElement("iframe");
          frameB.srcdoc = `<script>
            function handleEvent() { throw new Error(); }
          <\/script>`;
          const frameC = document.createElement("iframe");
          host.appendChild(frameA);
          host.appendChild(frameB);
          host.appendChild(frameC);
          globalThis.__crossRealmFrames = [frameA, frameB, frameC];
          return "ready";
        })()
        "#,
    )
    .expect("cross-realm multiple global frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const [frameA, frameB, frameC] = globalThis.__crossRealmFrames;
              const w = frameA.contentWindow;
              const w2 = frameB.contentWindow;
              const w3 = frameC.contentWindow;
              const results = [];

              function nextError(windows, callback) {
                function listener(event) {
                  results.push(callback(event));
                  event.preventDefault();
                }
                for (const current of windows) current.addEventListener("error", listener);
                return () => {
                  for (const current of windows) current.removeEventListener("error", listener);
                };
              }

              const functionTarget = new w2.EventTarget();
              functionTarget.addEventListener("party", w.listener);
              let cleanup = nextError([window, w, w2], event => [
                "function",
                event.target === w,
                !!event.error && event.error.constructor === w.Error
              ]);
              functionTarget.dispatchEvent(new Event("party"));
              results.push([
                "function-window-event-restored",
                typeof event === "undefined"
              ]);
              cleanup();

              const objectListener = w.objectListener;
              objectListener.handleEvent = w2.handleEvent;
              const objectTarget = new w3.EventTarget();
              objectTarget.addEventListener("party", objectListener);
              cleanup = nextError([window, w, w2, w3], event => [
                "object",
                event.target === w,
                !!event.error && event.error.constructor === w2.Error
              ]);
              objectTarget.dispatchEvent(new Event("party"));
              results.push([
                "object-window-event-restored",
                typeof event === "undefined"
              ]);
              cleanup();

              return JSON.stringify(results);
            })()
            "#,
        )
        .expect("cross-realm multiple global listener errors should evaluate");

    assert_eq!(
        result,
        r#"[["function",true,true],["function-window-event-restored",true],["object",true,true],["object-window-event-restored",true]]"#
    );
}

#[test]
fn event_timestamp_uses_performance_origin_and_safe_resolution() {
    let mut vm = new_storage_test_vm("https://event-timestamp.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const before = performance.now();
              const event = new MouseEvent("click");
              const after = performance.now();
              const descriptor = Object.getOwnPropertyDescriptor(Event.prototype, "timeStamp");
              const getterValue = descriptor.get.call(event);
              const delta = Math.round((new Event("b").timeStamp - event.timeStamp) * 1000);

              return JSON.stringify({
                hasGetter: typeof descriptor.get === "function",
                getterName: descriptor.get && descriptor.get.name,
                getterLength: descriptor.get && descriptor.get.length,
                enumerable: descriptor.enumerable,
                configurable: descriptor.configurable,
                hasOwnTimeStamp: Object.prototype.hasOwnProperty.call(event, "timeStamp"),
                withinNowRange: event.timeStamp >= before && event.timeStamp <= after,
                getterMatchesOwnValue: Object.is(getterValue, event.timeStamp),
                safeResolution: delta >= 0 && delta % 5 === 0
              });
            })()
            "#,
        )
        .expect("event timestamp probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasGetter":true,"getterName":"get timeStamp","getterLength":0,"enumerable":true,"configurable":true,"hasOwnTimeStamp":false,"withinNowRange":true,"getterMatchesOwnValue":true,"safeResolution":true}"#
    );
}

#[test]
fn ui_event_pseudo_target_is_declared_once_and_inherited() {
    let mut vm = new_storage_test_vm("https://ui-event-pseudo-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor =
                Object.getOwnPropertyDescriptor(UIEvent.prototype, "pseudoTarget");
              const getter = descriptor.get;
              const thrownName = receiver => {
                try {
                  getter.call(receiver);
                  return "none";
                } catch (error) {
                  return error && error.name;
                }
              };
              const textEvent = document.createEvent("TextEvent");

              return JSON.stringify({
                descriptor: [
                  getter.name,
                  getter.length,
                  typeof descriptor.set,
                  descriptor.enumerable,
                  descriptor.configurable
                ],
                placement: [
                  Object.prototype.hasOwnProperty.call(Event.prototype, "pseudoTarget"),
                  Object.prototype.hasOwnProperty.call(UIEvent.prototype, "pseudoTarget"),
                  Object.prototype.hasOwnProperty.call(MouseEvent.prototype, "pseudoTarget"),
                  "pseudoTarget" in MouseEvent.prototype
                ],
                values: [
                  new UIEvent("ui").pseudoTarget,
                  new MouseEvent("mouse").pseudoTarget,
                  getter.call(textEvent)
                ],
                illegalReceivers: [
                  thrownName(new Event("event")),
                  thrownName({})
                ]
              });
            })()
            "#,
        )
        .expect("UIEvent pseudoTarget prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":["get pseudoTarget",0,"undefined",true,true],"placement":[false,true,false,true],"values":[null,null,null],"illegalReceivers":["TypeError","TypeError"]}"#
    );
}

#[test]
fn performance_entry_accessors_return_entries_sorted_by_start_time() {
    let mut vm = new_storage_test_vm("https://performance-entry-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              performance.measure("late", { start: 20, duration: 1 });
              performance.measure("early", { start: 0, duration: 1 });
              return [
                performance.getEntriesByType("measure").map(entry => entry.name).join(","),
                performance.getEntries().filter(entry => entry.entryType === "measure").map(entry => entry.name).join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("performance entry ordering probe should evaluate");

    assert_eq!(result, "early,late|early,late");
}

#[test]
fn legacy_event_init_methods_short_circuit_while_dispatching() {
    let mut vm = new_storage_test_vm("https://event-init-while-dispatching.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const event = new MouseEvent("x", {
                bubbles: false,
                cancelable: false,
                screenX: 7,
                clientX: 9,
                ctrlKey: false,
                button: 0
              });
              target.addEventListener("x", () => {
                event.initMouseEvent(
                  "changed",
                  true,
                  true,
                  window,
                  1,
                  2,
                  3,
                  4,
                  5,
                  true,
                  true,
                  true,
                  true,
                  1,
                  document
                );
              });
              target.dispatchEvent(event);

              const keyboard = new KeyboardEvent("key", { key: "A", repeat: false });
              target.addEventListener("key", () => {
                keyboard.initKeyboardEvent("changed", true, true, window, "B", 1, "", true, "");
              });
              target.dispatchEvent(keyboard);

              return JSON.stringify({
                mouseType: event.type,
                mouseBubbles: event.bubbles,
                mouseCancelable: event.cancelable,
                screenX: event.screenX,
                clientX: event.clientX,
                ctrlKey: event.ctrlKey,
                button: event.button,
                keyType: keyboard.type,
                key: keyboard.key,
                repeat: keyboard.repeat,
                location: keyboard.location
              });
            })()
            "#,
        )
        .expect("legacy event init while dispatching probe should evaluate");

    assert_eq!(
        result,
        r#"{"mouseType":"x","mouseBubbles":false,"mouseCancelable":false,"screenX":7,"clientX":9,"ctrlKey":false,"button":0,"keyType":"key","key":"A","repeat":false,"location":0}"#
    );
}

#[test]
fn legacy_event_init_methods_short_circuit_for_all_wpt_event_classes() {
    let mut vm = new_storage_test_vm("https://event-init-while-dispatching-all.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const failures = [];
              const cases = {
                KeyboardEvent: {
                  event: new KeyboardEvent("type", { key: "A" }),
                  init: event => event.initKeyboardEvent("type2", true, true, null, "a", 1, "", true, ""),
                  check: event => {
                    if (event.key !== "A") failures.push("KeyboardEvent:key");
                    if (event.repeat !== false) failures.push("KeyboardEvent:repeat");
                    if (event.location !== 0) failures.push("KeyboardEvent:location");
                  }
                },
                MouseEvent: {
                  event: new MouseEvent("type"),
                  init: event => event.initMouseEvent("type2", true, true, null, 0, 1, 1, 1, 1, true, true, true, true, 1, null),
                  check: event => {
                    for (const name of ["screenX", "screenY", "clientX", "clientY", "button"]) {
                      if (event[name] !== 0) failures.push(`MouseEvent:${name}:${event[name]}`);
                    }
                    for (const name of ["ctrlKey", "altKey", "shiftKey", "metaKey"]) {
                      if (event[name] !== false) failures.push(`MouseEvent:${name}`);
                    }
                  }
                },
                CustomEvent: {
                  event: new CustomEvent("type"),
                  init: event => event.initCustomEvent("type2", true, true, 1),
                  check: event => {
                    if (event.detail !== null) failures.push(`CustomEvent:detail:${event.detail}`);
                  }
                },
                UIEvent: {
                  event: new UIEvent("type"),
                  init: event => event.initUIEvent("type2", true, true, window, 1),
                  check: event => {
                    if (event.view !== null) failures.push("UIEvent:view");
                    if (event.detail !== 0) failures.push(`UIEvent:detail:${event.detail}`);
                  }
                },
                Event: {
                  event: new Event("type"),
                  init: event => event.initEvent("type2", true, true),
                  check: event => {
                    if (event.type !== "type") failures.push(`Event:type:${event.type}`);
                    if (event.bubbles !== false) failures.push("Event:bubbles");
                    if (event.cancelable !== false) failures.push("Event:cancelable");
                  }
                }
              };

              for (const [name, entry] of Object.entries(cases)) {
                const target = document.createElement("div");
                target.addEventListener("type", () => {
                  try {
                    entry.init(entry.event);
                    entry.check(entry.event);
                  } catch (error) {
                    failures.push(`${name}:throw:${error && error.name}`);
                  }
                });
                target.dispatchEvent(entry.event);
              }
              return failures.join("|") || "ok";
            })()
            "#,
        )
        .expect("all-class legacy init short-circuit probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn event_subclass_constructors_expose_legacy_keyboard_codes_and_validate_view() {
    let mut vm = new_storage_test_vm("https://event-subclass-constructors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const defaults = new KeyboardEvent("key");
              const nonDefaults = new KeyboardEvent("key", {
                charCode: 7,
                keyCode: 8,
                which: 9,
                view: window
              });
              let wrongViewName = "none";
              try {
                new UIEvent("x", { view: 7 });
              } catch (error) {
                wrongViewName = error && error.name;
              }
              return JSON.stringify({
                defaults: [defaults.charCode, defaults.keyCode, defaults.which],
                nonDefaults: [nonDefaults.charCode, nonDefaults.keyCode, nonDefaults.which],
                viewIsWindow: nonDefaults.view === window,
                wrongViewName,
                lengths: [
                  KeyboardEvent.prototype.initKeyboardEvent.length,
                  MouseEvent.prototype.initMouseEvent.length
                ]
              });
            })()
            "#,
        )
        .expect("event subclass constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":[0,0,0],"nonDefaults":[7,8,9],"viewIsWindow":true,"wrongViewName":"TypeError","lengths":[1,1]}"#
    );
}

#[test]
fn mouse_event_exposes_pointer_lock_movement_dictionary_members() {
    let mut vm = new_storage_test_vm("https://mouse-event-movement.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const defaults = new MouseEvent("default");
              const initialized = new MouseEvent("initialized", {
                movementX: 10,
                movementY: -11
              });
              const wheel = new WheelEvent("wheel", {
                movementX: 12,
                movementY: -13
              });
              const pointer = new PointerEvent("pointer", {
                movementX: 14,
                movementY: -15
              });
              return JSON.stringify({
                present: "movementX" in defaults && "movementY" in defaults,
                defaults: [defaults.movementX, defaults.movementY],
                initialized: [initialized.movementX, initialized.movementY],
                wheel: [wheel.movementX, wheel.movementY],
                pointer: [pointer.movementX, pointer.movementY]
              });
            })()
            "#,
        )
        .expect("MouseEvent movement member probe should evaluate");

    assert_eq!(
        result,
        r#"{"present":true,"defaults":[0,0],"initialized":[10,-11],"wheel":[12,-13],"pointer":[14,-15]}"#
    );
}

#[test]
fn pointer_event_converts_tilt_and_spherical_angles() {
    let mut vm = new_storage_test_vm("https://pointer-event-angles.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const defaults = new PointerEvent('pointer');
              const fromTilt = new PointerEvent('pointer', { tiltX: -45 });
              const fromSpherical = new PointerEvent('pointer', {
                azimuthAngle: 3 * Math.PI / 2,
                altitudeAngle: Math.PI / 4
              });
              const mixed = new PointerEvent('pointer', {
                tiltX: 45,
                azimuthAngle: Math.PI / 4
              });
              let tiltReads = 0;
              const observed = new PointerEvent('pointer', {
                get tiltX() {
                  tiltReads += 1;
                  return 45;
                }
              });
              let nonFinite = 'accepted';
              try {
                new PointerEvent('pointer', { altitudeAngle: Infinity });
              } catch (error) {
                nonFinite = error && error.name;
              }
              return JSON.stringify({
                defaults: [
                  defaults.tiltX,
                  defaults.tiltY,
                  defaults.azimuthAngle,
                  defaults.altitudeAngle
                ],
                fromTilt: [
                  fromTilt.tiltX,
                  fromTilt.tiltY,
                  fromTilt.azimuthAngle,
                  fromTilt.altitudeAngle
                ],
                fromSpherical: [
                  fromSpherical.tiltX,
                  fromSpherical.tiltY,
                  fromSpherical.azimuthAngle,
                  fromSpherical.altitudeAngle
                ],
                mixed: [
                  mixed.tiltX,
                  mixed.tiltY,
                  mixed.azimuthAngle,
                  mixed.altitudeAngle
                ],
                observed: [tiltReads, observed.tiltX, observed.altitudeAngle],
                nonFinite
              });
            })()
            "#,
        )
        .expect("PointerEvent angle conversion probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":[0,0,0,1.5707963267948966],"fromTilt":[-45,0,3.141592653589793,0.7853981633974483],"fromSpherical":[0,-45,4.71238898038469,0.7853981633974483],"mixed":[45,0,0.7853981633974483,1.5707963267948966],"observed":[1,45,0.7853981633974483],"nonFinite":"TypeError"}"#
    );
}

#[test]
fn pointer_event_sequences_preserve_identity_and_secure_context_exposure() {
    let mut secure_vm = new_storage_test_vm("https://pointer-event-sequences.test/");
    let secure = secure_vm
        .eval(
            r#"
            (() => {
              try {
                Object.defineProperty(globalThis, "isSecureContext", { value: false });
              } catch {}
              const publicSecureContextSpoofed = isSecureContext === false;
              const predicted = new PointerEvent("pointermove", { clientX: 20 });
              const coalesced = new PointerEvent("pointermove", { clientX: 5 });
              const event = new PointerEvent("pointermove", {
                predictedEvents: [predicted],
                coalescedEvents: [coalesced]
              });
              const firstPredicted = event.getPredictedEvents();
              const secondPredicted = event.getPredictedEvents();
              const firstCoalesced = event.getCoalescedEvents();
              const secondCoalesced = event.getCoalescedEvents();
              firstPredicted.length = 0;
              firstCoalesced.length = 0;
              let invalidEntry = "accepted";
              try {
                new PointerEvent("pointermove", { predictedEvents: [{}] });
              } catch (error) {
                invalidEntry = error && error.name;
              }
              let fakeReceiver = "accepted";
              try {
                PointerEvent.prototype.getPredictedEvents.call({});
              } catch (error) {
                fakeReceiver = error && error.name;
              }
              return JSON.stringify({
                methods: [
                  typeof event.getPredictedEvents,
                  typeof event.getCoalescedEvents,
                  event.getPredictedEvents.length,
                  event.getCoalescedEvents.length
                ],
                predictedIdentity: secondPredicted[0] === predicted,
                coalescedIdentity: secondCoalesced[0] === coalesced,
                arraysAreFresh: firstPredicted !== secondPredicted &&
                  firstCoalesced !== secondCoalesced,
                mutationIsolated: secondPredicted.length === 1 &&
                  secondCoalesced.length === 1,
                nestedDefaults: [
                  predicted.getPredictedEvents().length,
                  predicted.getCoalescedEvents().length
                ],
                secureBindingIndependent: !publicSecureContextSpoofed ||
                  "getCoalescedEvents" in event,
                invalidEntry,
                fakeReceiver
              });
            })()
            "#,
        )
        .expect("secure PointerEvent sequence probe should evaluate");
    assert_eq!(
        secure,
        r#"{"methods":["function","function",0,0],"predictedIdentity":true,"coalescedIdentity":true,"arraysAreFresh":true,"mutationIsolated":true,"nestedDefaults":[0,0],"secureBindingIndependent":true,"invalidEntry":"TypeError","fakeReceiver":"TypeError"}"#
    );

    let mut insecure_vm = new_storage_test_vm("http://pointer-event-sequences.test/");
    let insecure = insecure_vm
        .eval(
            r#"
            (() => {
              const originalSecureContext = isSecureContext;
              try {
                Object.defineProperty(globalThis, "isSecureContext", { value: true });
              } catch {}
              const publicSecureContextSpoofed = isSecureContext === true;
              const event = new PointerEvent("pointermove");
              return [
                originalSecureContext,
                typeof event.getPredictedEvents,
                "getCoalescedEvents" in event,
                !publicSecureContextSpoofed ||
                  !("getCoalescedEvents" in event)
              ].join("|");
            })()
            "#,
        )
        .expect("insecure PointerEvent exposure probe should evaluate");
    assert_eq!(insecure, "false|function|false|true");
}

#[test]
fn live_ranges_survive_initialization_of_child_and_sibling_realms() {
    let mut vm = new_storage_test_vm("https://range-realm-lifetime.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/range-realm-lifetime.js"
        ))
        .expect("new realms must preserve live ranges in existing documents"),
        ""
    );
}

#[test]
fn character_data_setters_apply_replace_all_live_range_offsets() {
    let mut vm = new_storage_test_vm("https://character-data-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = document.createTextNode("abc");
              const dataRange = document.createRange();
              dataRange.setStart(text, 1);
              dataRange.setEnd(text, 2);
              text.data = "abc";
              const textContentRange = document.createRange();
              textContentRange.setStart(text, 1);
              textContentRange.setEnd(text, 3);
              text.textContent = "abc";
              const nodeValueRange = document.createRange();
              nodeValueRange.setStart(text, 1);
              nodeValueRange.setEnd(text, 3);
              text.nodeValue = "abc";

              const foreignDoc = document.implementation.createHTMLDocument("");
              const foreignText = foreignDoc.createTextNode("abc");
              const foreignRange = foreignDoc.createRange();
              foreignRange.setStart(foreignText, 0);
              foreignRange.setEnd(foreignText, 1);
              foreignText.textContent = "foo";

              const xmlDoc = document.implementation.createDocument(null, "root");
              const xmlComment = xmlDoc.createComment("abc");
              const xmlRange = xmlDoc.createRange();
              xmlRange.setStart(xmlComment, 1);
              xmlRange.setEnd(xmlComment, xmlComment.length);
              xmlComment.textContent = "foo";

              const detachedForeignText = foreignDoc.createTextNode("abcdef");
              const detachedForeignRange = foreignDoc.createRange();
              detachedForeignRange.setStart(detachedForeignText, 1);
              detachedForeignRange.setEnd(detachedForeignText, detachedForeignText.length);
              detachedForeignText.textContent += "foo";

              return [
                dataRange.startContainer === text,
                dataRange.endContainer === text,
                dataRange.startOffset,
                dataRange.endOffset,
                textContentRange.startOffset,
                textContentRange.endOffset,
                nodeValueRange.startOffset,
                nodeValueRange.endOffset,
                text.data,
                foreignRange.startContainer === foreignText,
                foreignRange.startOffset,
                foreignRange.endContainer === foreignText,
                foreignRange.endOffset,
                xmlRange.startContainer === xmlComment,
                xmlRange.startOffset,
                xmlRange.endContainer === xmlComment,
                xmlRange.endOffset,
                detachedForeignRange.startContainer === detachedForeignText,
                detachedForeignRange.startOffset,
                detachedForeignRange.endContainer === detachedForeignText,
                detachedForeignRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("character data setters should apply replace-all live range offsets");

    assert_eq!(
        result,
        "true|true|0|0|0|0|0|0|abc|true|0|true|0|true|0|true|0|true|0|true|0"
    );
}

#[test]
fn range_clone_contents_empty_character_data_range_returns_empty_fragment() {
    let mut vm = new_storage_test_vm("https://range-clone-empty-character-data.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = document.createTextNode("abc");
              const range = document.createRange();
              range.setStart(text, 0);
              range.setEnd(text, 0);
              const fragment = range.cloneContents();
              return [
                fragment.nodeType,
                fragment.childNodes.length,
                fragment.textContent,
                range.startContainer === text,
                range.endContainer === text,
                range.startOffset,
                range.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("empty CharacterData Range.cloneContents should evaluate");

    assert_eq!(result, "11|0||true|true|0|0");
}

#[test]
fn detached_character_data_edits_update_live_ranges() {
    let mut vm = new_storage_test_vm("https://detached-character-data-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");

              const deleteText = doc.createTextNode("abcdef");
              const deleteRange = doc.createRange();
              deleteRange.setStart(deleteText, 1);
              deleteRange.setEnd(deleteText, 4);
              deleteText.deleteData(1, 2);

              const insertText = doc.createTextNode("abcdef");
              const insertRange = doc.createRange();
              insertRange.setStart(insertText, 1);
              insertRange.setEnd(insertText, 4);
              insertText.insertData(2, "XYZ");

              const replaceText = doc.createTextNode("abcdef");
              const replaceRange = doc.createRange();
              replaceRange.setStart(replaceText, 1);
              replaceRange.setEnd(replaceText, 4);
              replaceText.replaceData(1, 2, "XYZ");

              const resetText = doc.createTextNode("abcdef");
              const resetRange = doc.createRange();
              resetRange.setStart(resetText, 1);
              resetRange.setEnd(resetText, 4);
              resetText.data = resetText.data;

              const nodeValueText = doc.createTextNode("abcdef");
              const nodeValueRange = doc.createRange();
              nodeValueRange.setStart(nodeValueText, 1);
              nodeValueRange.setEnd(nodeValueText, 4);
              nodeValueText.nodeValue = nodeValueText.nodeValue;

              const splitHost = doc.createElement("p");
              const splitText = doc.createTextNode("abcdef");
              splitHost.appendChild(splitText);
              doc.body.appendChild(splitHost);
              const splitTextRange = doc.createRange();
              splitTextRange.setStart(splitText, 1);
              splitTextRange.setEnd(splitText, 4);
              const splitParentRange = doc.createRange();
              splitParentRange.setStart(splitHost, 1);
              splitParentRange.setEnd(splitHost, 1);
              const splitNew = splitText.splitText(2);

              const detachedSplitText = doc.createTextNode("abcdef");
              const detachedSplitRange = doc.createRange();
              detachedSplitRange.setStart(detachedSplitText, 1);
              detachedSplitRange.setEnd(detachedSplitText, 4);
              detachedSplitText.splitText(2);

              return [
                deleteText.data,
                deleteRange.startOffset,
                deleteRange.endOffset,
                insertText.data,
                insertRange.startOffset,
                insertRange.endOffset,
                replaceText.data,
                replaceRange.startOffset,
                replaceRange.endOffset,
                resetRange.startOffset,
                resetRange.endOffset,
                nodeValueRange.startOffset,
                nodeValueRange.endOffset,
                splitText.data,
                splitNew.data,
                splitTextRange.startContainer === splitText,
                splitTextRange.startOffset,
                splitTextRange.endContainer === splitNew,
                splitTextRange.endOffset,
                splitParentRange.startOffset,
                splitParentRange.endOffset,
                detachedSplitText.data,
                detachedSplitRange.startContainer === detachedSplitText,
                detachedSplitRange.startOffset,
                detachedSplitRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("detached character data edits should update live ranges");

    assert_eq!(
        result,
        "adef|1|2|abXYZcdef|1|7|aXYZdef|1|5|0|0|0|0|ab|cdef|true|1|true|2|2|2|ab|true|1|2"
    );
}

#[test]
fn range_select_node_rejects_parentless_nodes_and_doctype_contents() {
    let mut vm = new_storage_test_vm("https://range-select-node-errors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const range = document.createRange();
              const detachedElement = document.createElement("div");
              const detachedText = document.createTextNode("abc");
              const fragment = document.createDocumentFragment();
              const docType = document.implementation.createDocumentType("html", "", "");
              const host = document.createElement("section");
              const child = document.createElement("span");
              host.appendChild(child);
              (document.body || document.documentElement || document).appendChild(host);

              function thrownName(callback) {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              }

              const parentlessElement = thrownName(() => range.selectNode(detachedElement));
              const parentlessText = thrownName(() => range.selectNode(detachedText));
              const parentlessFragment = thrownName(() => range.selectNode(fragment));
              const parentlessDocument = thrownName(() => range.selectNode(document));
              const doctypeContents = thrownName(() => range.selectNodeContents(docType));

              range.selectNode(child);
              const selectNodeState = [
                range.startContainer === host,
                range.startOffset,
                range.endContainer === host,
                range.endOffset
              ].join(",");

              range.selectNodeContents(child);
              const contentsState = [
                range.startContainer === child,
                range.startOffset,
                range.endContainer === child,
                range.endOffset
              ].join(",");

              return [
                parentlessElement,
                parentlessText,
                parentlessFragment,
                parentlessDocument,
                doctypeContents,
                selectNodeState,
                contentsState
              ].join("|");
            })()
            "#,
        )
        .expect("Range selectNode error checks should evaluate");

    assert_eq!(
        result,
        "InvalidNodeTypeError:24|InvalidNodeTypeError:24|InvalidNodeTypeError:24|InvalidNodeTypeError:24|InvalidNodeTypeError:24|true,0,true,1|true,0,true,0"
    );
}

#[test]
fn range_intersects_node_uses_exclusive_adjacent_boundaries() {
    let mut vm = new_storage_test_vm("https://range-intersects-node.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const a = document.createElement("a");
              const b = document.createElement("b");
              const c = document.createElement("c");
              host.append(a, b, c);
              (document.body || document.documentElement || document).appendChild(host);
              const range = document.createRange();
              range.setStart(host, 1);
              range.setEnd(host, 2);

              function thrownName(callback) {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error.name;
                }
              }

              return [
                range.intersectsNode(a),
                range.intersectsNode(b),
                range.intersectsNode(c),
                thrownName(() => range.intersectsNode()),
                thrownName(() => range.intersectsNode(null)),
                thrownName(() => range.intersectsNode({}))
              ].join("|");
            })()
            "#,
        )
        .expect("Range intersectsNode checks should evaluate");

    assert_eq!(result, "false|true|false|TypeError|TypeError|TypeError");
}

#[test]
fn static_range_constructor_sets_immutable_abstract_range_boundaries() {
    let mut vm = new_storage_test_vm("https://static-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              host.id = "host";
              host.append("a", document.createElement("span"), "b");
              (document.body || document.documentElement || document).append(host);
              const text = host.firstChild;
              const doctype = document.implementation.createDocumentType("html", "", "");
              const range = new StaticRange({
                startContainer: text,
                startOffset: 0,
                endContainer: host,
                endOffset: 3
              });
              host.insertBefore(document.createTextNode("x"), text);
              const attrError = (() => {
                try {
                  new StaticRange({
                    startContainer: host.getAttributeNode("id"),
                    startOffset: 0,
                    endContainer: host,
                    endOffset: 0
                  });
                } catch (error) {
                  return error.name;
                }
              })();
              const doctypeError = (() => {
                try {
                  new StaticRange({
                    startContainer: doctype,
                    startOffset: 0,
                    endContainer: doctype,
                    endOffset: 0
                  });
                } catch (error) {
                  return error.name;
                }
              })();
              const missingError = (() => {
                try {
                  new StaticRange({ startOffset: 0, endContainer: host, endOffset: 0 });
                } catch (error) {
                  return error.name;
                }
              })();
              return JSON.stringify({
                ctor: typeof StaticRange,
                abstract: range instanceof AbstractRange,
                staticRange: range instanceof StaticRange,
                startSame: range.startContainer === text,
                startOffset: range.startOffset,
                endSame: range.endContainer === host,
                endOffset: range.endOffset,
                collapsed: range.collapsed,
                attrError,
                doctypeError,
                missingError
              });
            })()
            "#,
        )
        .expect("StaticRange constructor checks should evaluate");

    assert_eq!(
        result,
        r#"{"ctor":"function","abstract":true,"staticRange":true,"startSame":true,"startOffset":0,"endSame":true,"endOffset":3,"collapsed":false,"attrError":"InvalidNodeTypeError","doctypeError":"InvalidNodeTypeError","missingError":"TypeError"}"#
    );
}

#[test]
fn range_uses_native_record_storage_and_static_range_keeps_boundary_slots() {
    let mut vm = new_storage_test_vm("https://range-declared-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const accessorDescriptor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  descriptor?.enumerable,
                  typeof descriptor?.set,
                  descriptor?.configurable
                ].join(":");
              };
              const host = document.createElement("div");
              host.append("abc");
              (document.body || document.documentElement || document).append(host);
              const text = host.firstChild;
              const constructed = new Range();
              const created = document.createRange();
              const staticRange = new StaticRange({
                startContainer: text,
                startOffset: 1,
                endContainer: text,
                endOffset: 2
              });
              text.insertData(0, "z");
              return JSON.stringify({
                rangeTag: Object.prototype.toString.call(constructed),
                rangeCtor: constructed.constructor && constructed.constructor.name,
                rangeKeys: Object.keys(constructed).join(","),
                rangeOwnInternalSlotCount: Object.getOwnPropertyNames(constructed)
                  .filter((name) => name.startsWith("__moli")).length,
                rangeStartIsDocument: constructed.startContainer === document,
                rangeStartOffset: constructed.startOffset,
                rangeEndIsDocument: constructed.endContainer === document,
                rangeEndOffset: constructed.endOffset,
                rangeCollapsed: constructed.collapsed,
                rangeStartEnumerable: Object.prototype.propertyIsEnumerable.call(constructed, "startContainer"),
                createdTag: Object.prototype.toString.call(created),
                createdStartIsDocument: created.startContainer === document,
                createdOwnInternalSlotCount: Object.getOwnPropertyNames(created)
                  .filter((name) => name.startsWith("__moli")).length,
                staticTag: Object.prototype.toString.call(staticRange),
                staticCtor: staticRange.constructor && staticRange.constructor.name,
                staticAbstract: staticRange instanceof AbstractRange,
                staticKeys: Object.keys(staticRange).join(","),
                staticOwnInternalSlotCount: Object.getOwnPropertyNames(staticRange)
                  .filter((name) => name.startsWith("__moli")).length,
                staticStartSame: staticRange.startContainer === text,
                staticStartOffset: staticRange.startOffset,
                staticEndSame: staticRange.endContainer === text,
                staticEndOffset: staticRange.endOffset,
                staticCollapsed: staticRange.collapsed,
                staticStartEnumerable: Object.prototype.propertyIsEnumerable.call(staticRange, "startContainer"),
                abstractAccessors: [
                  accessorDescriptor(AbstractRange.prototype, "startContainer"),
                  accessorDescriptor(AbstractRange.prototype, "startOffset"),
                  accessorDescriptor(AbstractRange.prototype, "endContainer"),
                  accessorDescriptor(AbstractRange.prototype, "endOffset"),
                  accessorDescriptor(AbstractRange.prototype, "collapsed"),
                  accessorDescriptor(AbstractRange.prototype, "commonAncestorContainer")
                ]
              });
            })()
            "#,
        )
        .expect("Range native record storage probe should evaluate");

    assert_eq!(
        result,
        r#"{"rangeTag":"[object Range]","rangeCtor":"Range","rangeKeys":"","rangeOwnInternalSlotCount":0,"rangeStartIsDocument":true,"rangeStartOffset":0,"rangeEndIsDocument":true,"rangeEndOffset":0,"rangeCollapsed":true,"rangeStartEnumerable":false,"createdTag":"[object Range]","createdStartIsDocument":true,"createdOwnInternalSlotCount":0,"staticTag":"[object StaticRange]","staticCtor":"StaticRange","staticAbstract":true,"staticKeys":"","staticOwnInternalSlotCount":0,"staticStartSame":true,"staticStartOffset":1,"staticEndSame":true,"staticEndOffset":2,"staticCollapsed":false,"staticStartEnumerable":false,"abstractAccessors":["startContainer:function:get startContainer:0:true:undefined:true","startOffset:function:get startOffset:0:true:undefined:true","endContainer:function:get endContainer:0:true:undefined:true","endOffset:function:get endOffset:0:true:undefined:true","collapsed:function:get collapsed:0:true:undefined:true","commonAncestorContainer:function:get commonAncestorContainer:0:true:undefined:true"]}"#
    );
}

#[test]
fn selection_record_handle_ignores_legacy_slot_name_property() {
    let mut vm = new_storage_test_vm("https://selection-record-internal-field.test/");

    vm.eval(
        r#"
            (() => {
              const container = document.body || document.documentElement || document;
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              container.appendChild(host);

globalThis.__readFixture = () => {
              const selection = getSelection();
              selection.__moliSelectionRecordId = 0n;
              selection.setBaseAndExtent(text, 1, text, 3);
              const range = selection.getRangeAt(0);

              return [
                selection.__moliSelectionRecordId === 0n,
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset,
                range.startContainer === text,
                range.startOffset,
                range.endContainer === text,
                range.endOffset,
                selection.toString()
              ].join("|");
            };
})()
            "#,
    )
    .expect("Selection native record handle should ignore public legacy-slot-name spoofing");
    vm.publish_layout_for_test()
        .expect("publish prepared fixture");
    let result = vm
        .eval("__readFixture()")
        .expect("Selection native record handle should ignore public legacy-slot-name spoofing");

    assert_eq!(result, "true|true|1|true|3|true|1|true|3|bc");
}

#[test]
fn document_create_range_declared_method_keeps_descriptor_and_behavior() {
    let mut vm = new_storage_test_vm("https://document-create-range-method.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "createRange");
              const range = document.createRange();
              return JSON.stringify({
                descriptor: [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":"),
                prototypeKeys: Object.keys(Document.prototype).includes("createRange"),
                ownOnDocument: Object.hasOwn(document, "createRange"),
                tag: Object.prototype.toString.call(range),
                instance: range instanceof Range,
                startIsDocument: range.startContainer === document,
                collapsed: range.collapsed
              });
            })()
            "#,
        )
        .expect("Document.createRange method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":"true:function:createRange:0:true:true:true","prototypeKeys":true,"ownOnDocument":false,"tag":"[object Range]","instance":true,"startIsDocument":true,"collapsed":true}"#
    );
}

#[test]
fn range_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://range-prototype-methods.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
            (function* () {
              const methods = [
                ["setStart", 2],
                ["setEnd", 2],
                ["selectNodeContents", 1],
                ["cloneContents", 0],
                ["collapse", 0],
                ["selectNode", 1],
                ["setStartBefore", 1],
                ["setStartAfter", 1],
                ["setEndBefore", 1],
                ["setEndAfter", 1],
                ["cloneRange", 0],
                ["toString", 0],
                ["comparePoint", 2],
                ["isPointInRange", 2],
                ["intersectsNode", 1],
                ["compareBoundaryPoints", 2],
                ["insertNode", 1],
                ["createContextualFragment", 1],
                ["deleteContents", 0],
                ["extractContents", 0],
                ["surroundContents", 1],
                ["getBoundingClientRect", 0],
                ["getClientRects", 0],
                ["detach", 0]
              ];
              const range = document.createRange();
              const descriptors = methods.map(([name, length]) => {
                const descriptor = Object.getOwnPropertyDescriptor(Range.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable,
                  Object.hasOwn(range, name),
                  descriptor?.value === range[name],
                  descriptor?.value?.length === length
                ].join(":");
              });
              const enumerableMethods = Object.keys(Range.prototype)
                .filter((name) => methods.some(([method]) => method === name))
                .join(",");

              const host = document.createElement("div");
              host.append("abcdef");
              (document.body || document.documentElement || document).append(host);
              const text = host.firstChild;
              range.setStart(text, 1);
              range.setEnd(text, 4);
              const clone = range.cloneRange();
              yield; // Publish this scene before reading its geometry.
const rect = range.getBoundingClientRect();
              const rects = range.getClientRects();
              const behavior = [
                range.toString(),
                clone.toString(),
                range.comparePoint(text, 2),
                range.isPointInRange(text, 2),
                range.intersectsNode(text),
                rect && rect.constructor && rect.constructor.name,
                rects.length
              ].join(":");
              range.collapse(true);
              range.detach();
              return JSON.stringify({
                descriptors,
                enumerableMethods,
                behavior,
                afterCollapse: [
                  range.collapsed,
                  range.startContainer === text,
                  range.startOffset,
                  range.endContainer === text,
                  range.endOffset
                ].join(":")
              });
            })()
            "#,
    )
    .expect("Range prototype method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["setStart:true:function:setStart:2:true:true:true:false:true:true","setEnd:true:function:setEnd:2:true:true:true:false:true:true","selectNodeContents:true:function:selectNodeContents:1:true:true:true:false:true:true","cloneContents:true:function:cloneContents:0:true:true:true:false:true:true","collapse:true:function:collapse:0:true:true:true:false:true:true","selectNode:true:function:selectNode:1:true:true:true:false:true:true","setStartBefore:true:function:setStartBefore:1:true:true:true:false:true:true","setStartAfter:true:function:setStartAfter:1:true:true:true:false:true:true","setEndBefore:true:function:setEndBefore:1:true:true:true:false:true:true","setEndAfter:true:function:setEndAfter:1:true:true:true:false:true:true","cloneRange:true:function:cloneRange:0:true:true:true:false:true:true","toString:true:function:toString:0:true:true:true:false:true:true","comparePoint:true:function:comparePoint:2:true:true:true:false:true:true","isPointInRange:true:function:isPointInRange:2:true:true:true:false:true:true","intersectsNode:true:function:intersectsNode:1:true:true:true:false:true:true","compareBoundaryPoints:true:function:compareBoundaryPoints:2:true:true:true:false:true:true","insertNode:true:function:insertNode:1:true:true:true:false:true:true","createContextualFragment:true:function:createContextualFragment:1:true:true:true:false:true:true","deleteContents:true:function:deleteContents:0:true:true:true:false:true:true","extractContents:true:function:extractContents:0:true:true:true:false:true:true","surroundContents:true:function:surroundContents:1:true:true:true:false:true:true","getBoundingClientRect:true:function:getBoundingClientRect:0:true:true:true:false:true:true","getClientRects:true:function:getClientRects:0:true:true:true:false:true:true","detach:true:function:detach:0:true:true:true:false:true:true"],"enumerableMethods":"setStart,setEnd,selectNodeContents,cloneContents,collapse,selectNode,setStartBefore,setStartAfter,setEndBefore,setEndAfter,cloneRange,toString,comparePoint,isPointInRange,intersectsNode,compareBoundaryPoints,insertNode,createContextualFragment,deleteContents,extractContents,surroundContents,getBoundingClientRect,getClientRects,detach","behavior":"bcd:bcd:0:true:true:DOMRect:1","afterCollapse":"true:true:1:true:1"}"#
    );
}
