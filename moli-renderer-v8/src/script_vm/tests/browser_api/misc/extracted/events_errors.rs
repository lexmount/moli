use super::*;

#[test]
fn error_events_and_rejection_events_carry_error_values_without_being_errors() {
    let mut vm = new_storage_test_vm("https://web-error-event-boundaries.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const thrown = new TypeError("boom");
  const errorEvent = new ErrorEvent("error", {
    message: "script boom",
    filename: "script.js",
    lineno: 7,
    colno: 3,
    error: thrown
  });
  const reason = new DOMException("rejected", "AbortError");
  const promise = Promise.reject(reason);
  promise.catch(() => {});
  const rejectionEvent = new PromiseRejectionEvent("unhandledrejection", {
    promise,
    reason
  });
  return [
    errorEvent instanceof Event,
    errorEvent instanceof Error,
    errorEvent instanceof DOMException,
    errorEvent.error === thrown,
    errorEvent.message,
    errorEvent.filename,
    errorEvent.lineno,
    errorEvent.colno,
    rejectionEvent instanceof Event,
    rejectionEvent instanceof Error,
    rejectionEvent instanceof DOMException,
    rejectionEvent.reason === reason,
    rejectionEvent.promise === promise,
    rejectionEvent.reason.name,
    rejectionEvent.reason.code
  ].join("|");
})()
"#,
        )
        .expect("error event boundary probe should evaluate");

    assert_eq!(
        result,
        "true|false|false|true|script boom|script.js|7|3|true|false|false|true|true|AbortError|20"
    );
}
#[test]
fn promise_rejection_event_constructor_parses_required_promise_and_optional_reason() {
    let mut vm = new_storage_test_vm("https://promise-rejection-event-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const promise = Promise.resolve("settled");
  const omitted = new PromiseRejectionEvent("unhandledrejection", { promise });
  const explicitUndefined = new PromiseRejectionEvent("unhandledrejection", {
    promise,
    reason: undefined
  });
  const explicitNull = new PromiseRejectionEvent("unhandledrejection", {
    promise,
    reason: null
  });
  const wrapped = new PromiseRejectionEvent("unhandledrejection", { promise: 42 });

  let missingPromiseThrows = false;
  try {
    new PromiseRejectionEvent("unhandledrejection", {});
  } catch (error) {
    missingPromiseThrows = error instanceof TypeError;
  }

  const marker = {};
  let getterErrorPreserved = false;
  try {
    new PromiseRejectionEvent("unhandledrejection", {
      get promise() { throw marker; }
    });
  } catch (error) {
    getterErrorPreserved = error === marker;
  }

  return [
    omitted.promise === promise,
    omitted.reason === undefined,
    explicitUndefined.reason === undefined,
    explicitNull.reason === null,
    wrapped.promise instanceof Promise,
    missingPromiseThrows,
    getterErrorPreserved
  ].join("|");
})()
"#,
        )
        .expect("PromiseRejectionEvent constructor probe should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|true");
}
#[test]
fn host_dispatched_events_ignore_user_replaced_event_constructor() {
    let mut vm = new_storage_test_vm("https://host-event-constructor.test/");

    vm.exec(
        r##"
        const IntrinsicEvent = Event;
        window.__hostEventProbe = [];
        window.Event = function Event() {
            window.__hostEventProbe.push("replacement-called");
            throw new Error("user Event constructor should not run");
        };
        document.addEventListener("DOMContentLoaded", event => {
            const methods = [
                "preventDefault",
                "stopPropagation",
                "stopImmediatePropagation",
                "composedPath"
            ];
            window.__hostEventProbe.push({
                type: event.type,
                trusted: event.isTrusted,
                target: event.target === document,
                currentTarget: event.currentTarget === document,
                bubbles: event.bubbles,
                cancelable: event.cancelable,
                intrinsicPrototype:
                    Object.getPrototypeOf(event) === IntrinsicEvent.prototype,
                intrinsicInstance: event instanceof IntrinsicEvent,
                ownMethods: methods.filter(name => Object.hasOwn(event, name)),
                inheritedMethods: methods.every(
                    name => event[name] === IntrinsicEvent.prototype[name]
                ),
                ownCancelBubble: Object.hasOwn(event, "cancelBubble"),
                ownReturnValue: Object.hasOwn(event, "returnValue"),
                ownTimeStamp: Object.hasOwn(event, "timeStamp"),
                ownIsTrusted: Object.hasOwn(event, "isTrusted"),
                keys: Object.keys(event)
            });
        });
        "##,
        None,
    )
    .expect("listener registration should succeed");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded dispatch should not call replaced Event constructor");

    let result = vm
        .eval("JSON.stringify(window.__hostEventProbe)")
        .expect("host event probe should be readable");

    assert_eq!(
        result,
        r#"[{"type":"DOMContentLoaded","trusted":true,"target":true,"currentTarget":true,"bubbles":true,"cancelable":false,"intrinsicPrototype":true,"intrinsicInstance":true,"ownMethods":[],"inheritedMethods":true,"ownCancelBubble":false,"ownReturnValue":false,"ownTimeStamp":false,"ownIsTrusted":true,"keys":["isTrusted"]}]"#
    );
}
#[test]
fn legacy_lenient_this_event_handlers_ignore_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://legacy-lenient-this.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const text = document.createTextNode("text");
  const invalidObject = {};
  const element = document.createElement("div");
  const shadow = element.attachShadow({mode: "open"});
  const invalidReceivers = [
    undefined, null, 1, invalidObject, text, shadow,
    Object.create(element), new Proxy(element, {}), new Proxy(document, {})
  ];
  const lenientDescriptors = [
    Object.getOwnPropertyDescriptor(HTMLElement.prototype, "onmouseenter"),
    Object.getOwnPropertyDescriptor(HTMLElement.prototype, "onmouseleave"),
    Object.getOwnPropertyDescriptor(Document.prototype, "onmouseenter"),
    Object.getOwnPropertyDescriptor(Document.prototype, "onmouseleave"),
    Object.getOwnPropertyDescriptor(Document.prototype, "onreadystatechange")
  ];
  const lenient = lenientDescriptors.every(descriptor =>
    invalidReceivers.every(receiver =>
      descriptor.get.call(receiver) === undefined &&
      descriptor.set.call(receiver) === undefined &&
      descriptor.set.call(receiver, undefined) === undefined &&
      descriptor.set.call(receiver, "ignored") === undefined
    )
  );

  const strict = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "onclick");
  const documentStrict = Object.getOwnPropertyDescriptor(Document.prototype, "onclick");
  const outcome = callback => {
    try {
      callback();
      return "return";
    } catch (error) {
      return error && error.name;
    }
  };

  let documentCalls = 0;
  const handler = () => documentCalls++;
  documentStrict.set.call(document, handler);
  const documentHandlerPreserved = documentStrict.get.call(document) === handler;
  document.dispatchEvent(new Event("click"));
  documentStrict.set.call(document, null);
  document.dispatchEvent(new Event("click"));

  return [
    lenient,
    outcome(() => strict.get.call({})),
    outcome(() => strict.set.call({})),
    outcome(() => strict.get.call(text)),
    outcome(() => strict.set.call(text)),
    outcome(() => strict.get.call(shadow)),
    outcome(() => strict.set.call(shadow)),
    outcome(() => documentStrict.get.call(element)),
    outcome(() => documentStrict.set.call(element)),
    outcome(() => documentStrict.get.call({})),
    outcome(() => documentStrict.set.call({})),
    Object.getOwnPropertyNames(invalidObject).length,
    documentHandlerPreserved,
    documentCalls
  ].join("|");
})()
"#,
        )
        .expect("LegacyLenientThis event handler probe should evaluate");

    assert_eq!(
        result,
        "true|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|0|true|1"
    );
}
#[test]
fn clipboard_event_constructor_applies_event_init_and_clipboard_data() {
    let mut vm = new_storage_test_vm("https://clipboard-event.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  const defaults = new ClipboardEvent("copy");
  const initialized = new ClipboardEvent("paste", {
    bubbles: true,
    cancelable: true,
    composed: true,
    clipboardData: transfer
  });
  return [
    defaults instanceof ClipboardEvent,
    defaults instanceof Event,
    defaults.clipboardData === null,
    defaults.composed,
    initialized.type,
    initialized.bubbles,
    initialized.cancelable,
    initialized.composed,
    initialized.clipboardData === transfer
  ].join("|");
})()
"#,
        )
        .expect("ClipboardEvent constructor probe should evaluate");

    assert_eq!(result, "true|true|true|false|paste|true|true|true|true");
}
#[test]
fn element_animate_autoplays_and_settles_finished_promise() {
    let mut vm = new_parsed_test_vm("https://animations.test/", "<!doctype html><body></body>");

    vm.eval(
        r#"
        (() => {
          globalThis.__animationProbe = [];
          const animation = document.body.animate(
            { opacity: [0, 1] },
            { duration: 1 }
          );
          globalThis.__animationProbe.push(animation.playState);
          animation.finished.then(() => {
            globalThis.__animationProbe.push(animation.playState);
          });
        })()
        "#,
    )
    .expect("Element.animate probe should evaluate");

    assert_eq!(
        vm.eval("globalThis.__animationProbe.join('|')")
            .expect("animation probe should be readable"),
        "running|finished"
    );
}
#[tokio::test]
async fn showing_popover_focuses_autofocus_descendant_before_toggle_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://popover-autofocus.test/");

    let before = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const button = document.createElement("button");
              const popover = document.createElement("div");
              const focusTarget = document.createElement("button");
              popover.setAttribute("popover", "");
              focusTarget.autofocus = true;
              popover.appendChild(focusTarget);
              body.append(button, popover);
              globalThis.__lmPopoverFocusEvents = [];
              popover.addEventListener("beforetoggle", e => globalThis.__lmPopoverFocusEvents.push(`${e.type}:${e.oldState}->${e.newState}`));
              popover.addEventListener("toggle", e => globalThis.__lmPopoverFocusEvents.push(`${e.type}:${e.oldState}->${e.newState}`));
              button.addEventListener("blur", () => globalThis.__lmPopoverFocusEvents.push("button blur"));
              focusTarget.addEventListener("focus", () => globalThis.__lmPopoverFocusEvents.push("focusTarget focus"));
              button.focus();
              popover.showPopover();
              return globalThis.__lmPopoverFocusEvents.join("|");
            })()
            "#,
        )
        .expect("popover autofocus setup should evaluate");

    assert_eq!(
        before,
        "beforetoggle:closed->open|button blur|focusTarget focus"
    );

    assert!(
        !vm.has_ready_callback_timer(),
        "popover toggle must not create a callback timer; focus rendering wakes are separate"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("queued popover toggle task should run")
    );

    let after = vm
        .eval(
            r#"
            (() => globalThis.__lmPopoverFocusEvents.join("|"))()
            "#,
        )
        .expect("popover autofocus event order should evaluate");

    assert_eq!(
        after,
        "beforetoggle:closed->open|button blur|focusTarget focus|toggle:closed->open"
    );
}
#[tokio::test]
async fn removing_open_popover_dispatches_forced_close_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://popover-removal-events.test/");

    let before = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const popover = document.createElement("div");
              popover.popover = "auto";
              body.append(popover);
              globalThis.__lmPopoverRemovalEvents = [];
              popover.addEventListener("beforetoggle", event => {
                globalThis.__lmPopoverRemovalEvents.push(
                  `${event.type}:${event.oldState}->${event.newState}:${event.cancelable}`
                );
              });
              popover.addEventListener("toggle", event => {
                globalThis.__lmPopoverRemovalEvents.push(
                  `${event.type}:${event.oldState}->${event.newState}:${event.cancelable}`
                );
              });
              popover.showPopover();
              return popover.matches(":popover-open");
            })()
            "#,
        )
        .expect("popover removal event setup should evaluate");

    assert_eq!(before, "true");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("initial popover show toggle task should run")
    );

    let removed = vm
        .eval(
            r#"
            (() => {
              const popover = document.querySelector("[popover]");
              globalThis.__lmPopoverRemovalEvents.length = 0;
              popover.remove();
              return [
                globalThis.__lmPopoverRemovalEvents.join("|"),
                popover.matches(":popover-open")
              ].join(";");
            })()
            "#,
        )
        .expect("popover removal should evaluate");

    assert_eq!(removed, "beforetoggle:open->closed:false;false");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("queued popover removal toggle task should run")
    );

    let after = vm
        .eval(
            r#"
            (() => globalThis.__lmPopoverRemovalEvents.join("|"))()
            "#,
        )
        .expect("popover removal event log should evaluate");

    assert_eq!(
        after,
        "beforetoggle:open->closed:false|toggle:open->closed:false"
    );
}
#[tokio::test]
async fn popover_toggle_events_coalesce_within_one_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://popover-toggle-coalesce.test/");

    let before = vm
        .eval(
            r#"
            (() => {
              const popover = document.createElement("div");
              popover.setAttribute("popover", "");
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              (document.body || html.appendChild(document.createElement("body"))).appendChild(popover);
              globalThis.__lmPopoverToggleEvents = [];
              popover.addEventListener("toggle", event => {
                globalThis.__lmPopoverToggleEvents.push(`${event.oldState}->${event.newState}`);
              });
              popover.showPopover();
              popover.hidePopover();
              popover.showPopover();
              return [
                popover.matches(":popover-open"),
                globalThis.__lmPopoverToggleEvents.length
              ].join("|");
            })()
            "#,
        )
        .expect("popover coalescing setup should evaluate");

    assert_eq!(before, "true|0");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("coalesced popover toggle task should run")
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("coalesced popover source should drain after one live task")
    );

    let after = vm
        .eval(
            r#"
            (() => globalThis.__lmPopoverToggleEvents.join("|"))()
            "#,
        )
        .expect("popover coalesced event probe should evaluate");

    assert_eq!(after, "closed->open");
}
#[test]
fn exec_with_script_url_sets_outer_error_stack_source_name() {
    let page_url = Url::parse("https://example.com/path/page.html").expect("page url");
    let mut vm = new_storage_test_vm(page_url.as_str());

    vm.exec(
        r#"
        (() => {
            try {
                throw new Error("outer-probe");
            } catch (error) {
                globalThis.__outerStackSourceProbe = String(error.stack);
            }
        })();
        "#,
        Some(&page_url),
    )
    .expect("script execution should succeed");

    let stack = vm
        .eval("globalThis.__outerStackSourceProbe")
        .expect("outer stack probe should evaluate");

    assert!(
        stack.contains(page_url.as_str()),
        "outer stack should include script resource name: {stack}"
    );
}
#[test]
fn console_debug_is_present_and_does_not_double_read_error_stack() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              let reads = 0;
              const err = new Error("probe");
              Object.defineProperty(err, "stack", {
                get() {
                  reads += 1;
                  return "stack";
                }
              });
              const slot = "__moliWindowConsole";
              const internalNames = () => Object.getOwnPropertyNames(window)
                .filter(name => name === slot)
                .join(",");
              const beforeAccess = internalNames();
              const firstConsole = window.console;
              const afterAccess = internalNames();
              Object.defineProperty(window, slot, {
                value: { spoof: true },
                configurable: true,
                writable: true
              });
              const afterSpoof = internalNames();
              const secondConsole = window.console;
              console.debug(err);
              void err.stack;
              try {
                return JSON.stringify({
                  beforeAccess,
                  afterAccess,
                  afterSpoof,
                  publicSpoof: window[slot].spoof === true,
                  sameConsole: firstConsole === secondConsole,
                  debugType: typeof secondConsole.debug,
                  reads
                });
              } finally {
                delete window[slot];
              }
            })()
            "#,
        )
        .expect("console.debug probe should evaluate");

    assert_eq!(
        result,
        r#"{"beforeAccess":"","afterAccess":"","afterSpoof":"__moliWindowConsole","publicSpoof":true,"sameConsole":true,"debugType":"function","reads":1}"#
    );
}
#[test]
fn console_does_not_invoke_error_prepare_stack_trace() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              let accessed = false;
              const originalPrepareStackTrace = Error.prepareStackTrace;
              try {
                Error.prepareStackTrace = () => {
                  accessed = true;
                  return "detected";
                };
                console.log(new Error(""));
                const afterConsole = accessed;
                void new Error("explicit stack access").stack;
                return `${afterConsole}|${accessed}`;
              } finally {
                Error.prepareStackTrace = originalPrepareStackTrace;
              }
            })()
            "#,
        )
        .expect("console Error.prepareStackTrace probe should evaluate");

    assert_eq!(result, "false|true");
}
#[test]
fn navigator_plain_object_promises_preserve_expected_shapes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              globalThis.__navigatorPlainObjectProbe = {};
              navigator.getBattery().then((battery) => {
                globalThis.__navigatorPlainObjectProbe.battery = [
                  Object.keys(battery).join(","),
                  battery.charging,
                  battery.chargingTime,
                  String(battery.dischargingTime),
                  battery.level,
                  typeof battery.addEventListener,
                  typeof battery.removeEventListener,
                  typeof battery.dispatchEvent,
                  Object.prototype.propertyIsEnumerable.call(battery, "addEventListener"),
                  battery.onchargingchange === null,
                  battery.onchargingtimechange === null,
                  battery.ondischargingtimechange === null,
                  battery.onlevelchange === null
                ].join("|");
                battery.addEventListener("chargingchange", () => {});
                battery.addEventListener("levelchange", () => {});
                battery.removeEventListener("levelchange", () => {});
              });
              navigator.permissions.query({ name: "geolocation" }).then((status) => {
                globalThis.__navigatorPlainObjectProbe.permission = [
                  Object.keys(status).join(","),
                  status.state,
                  status.onchange === null
                ].join("|");
              });
              navigator.storage.estimate().then((estimate) => {
                globalThis.__navigatorPlainObjectProbe.storage = [
                  Object.keys(estimate).join(","),
                  estimate.quota,
                  estimate.usage,
                  typeof estimate.usageDetails,
                  Object.keys(estimate.usageDetails).join(",")
                ].join("|");
              });
            })()
            "#,
    )
    .expect("navigator plain object promise probes should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__navigatorPlainObjectProbe)")
        .expect("navigator plain object promises should settle");

    assert_eq!(
        result,
        r#"{"battery":"charging,chargingTime,dischargingTime,level,onchargingchange,onchargingtimechange,ondischargingtimechange,onlevelchange|true|0|Infinity|1|function|function|function|false|true|true|true|true","permission":"|prompt|true","storage":"quota,usage,usageDetails|1073741824|0|object|"}"#
    );
}
#[test]
fn navigator_battery_status_declares_event_target_methods() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              globalThis.__navigatorBatteryMethodProbe = null;
              navigator.getBattery().then((battery) => {
                const fakeBattery = Object.create(battery);
                const summarize = name => {
                  const descriptor = Object.getOwnPropertyDescriptor(battery, name);
                  return [
                    !!descriptor,
                    descriptor && descriptor.enumerable,
                    descriptor && descriptor.configurable,
                    descriptor && descriptor.writable,
                    descriptor && typeof descriptor.value,
                    descriptor && descriptor.value.length,
                    descriptor && descriptor.value.name
                  ].join(":");
                };
                const methodOutcome = (name, ...args) => {
                  try {
                    return String(battery[name].call(fakeBattery, ...args));
                  } catch (error) {
                    return `throw:${error && error.name}`;
                  }
                };
                globalThis.__navigatorBatteryMethodProbe = [
                  Object.keys(battery).includes("addEventListener"),
                  summarize("addEventListener"),
                  summarize("removeEventListener"),
                  summarize("dispatchEvent"),
                  battery.dispatchEvent({ type: "chargingchange" }),
                  String(battery.addEventListener("chargingchange", () => {})),
                  String(battery.removeEventListener("chargingchange", () => {})),
                  methodOutcome("addEventListener", "chargingchange", () => {}),
                  methodOutcome("removeEventListener", "chargingchange", () => {}),
                  methodOutcome("dispatchEvent", { type: "chargingchange" })
                ].join("|");
              });
            })()
            "#,
    )
    .expect("navigator battery method probe should evaluate");

    let result = vm
        .eval("String(globalThis.__navigatorBatteryMethodProbe)")
        .expect("navigator battery method promise should settle");

    assert_eq!(
        result,
        "false|true:false:true:true:function:2:addEventListener|true:false:true:true:function:2:removeEventListener|true:false:true:true:function:1:dispatchEvent|true|undefined|undefined|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn location_conversion_hooks_and_prevent_extensions_match_location_exotic_semantics() {
    let mut vm = new_storage_test_vm("https://example.com/path");

    let result = vm
        .eval(
            r#"
            (() => {
              const describe = key => {
                const descriptor = Object.getOwnPropertyDescriptor(location, key);
                return {
                  own: Object.prototype.hasOwnProperty.call(location, key),
                  valueIsIntrinsic: key === "valueOf"
                    ? descriptor.value === Object.prototype.valueOf
                    : descriptor.value === undefined,
                  enumerable: descriptor.enumerable,
                  writable: descriptor.writable,
                  configurable: descriptor.configurable
                };
              };
              let objectPreventExtensions = "returned";
              try {
                Object.preventExtensions(location);
              } catch (error) {
                objectPreventExtensions = error && error.name;
              }
              const reflectPreventExtensions = Reflect.preventExtensions(location);
              location.afterPreventExtensions = 1;
              return JSON.stringify({
                valueOf: describe("valueOf"),
                toPrimitive: describe(Symbol.toPrimitive),
                objectPreventExtensions,
                reflectPreventExtensions,
                extensible: Object.isExtensible(location),
                remainsWritable: location.afterPreventExtensions === 1,
                href: String(location)
              });
            })()
            "#,
        )
        .expect("Location exotic semantics probe should evaluate");

    assert_eq!(
        result,
        r#"{"valueOf":{"own":true,"valueIsIntrinsic":true,"enumerable":false,"writable":false,"configurable":false},"toPrimitive":{"own":true,"valueIsIntrinsic":true,"enumerable":false,"writable":false,"configurable":false},"objectPreventExtensions":"TypeError","reflectPreventExtensions":false,"extensible":true,"remainsWritable":true,"href":"https://example.com/path"}"#
    );
}
#[test]
fn location_navigation_throws_syntax_error_for_unparseable_urls() {
    let mut vm = new_storage_test_vm("https://example.com/path?x=1#frag");

    let result = vm
        .eval(
            r#"
(() => {
  const originalHref = location.href;
  const probe = callback => {
    try {
      callback();
      return "returned";
    } catch (error) {
      return `${error.name}:${error.code}:${error instanceof DOMException}`;
    }
  };

  return JSON.stringify({
    assign: probe(() => location.assign("http://:")),
    replace: probe(() => location.replace("//")),
    href: probe(() => { location.href = "http://:"; }),
    windowLocation: probe(() => { window.location = "http://:"; }),
    documentLocation: probe(() => { document.location = "http://:"; }),
    unchanged: location.href === originalHref
  });
})()
"#,
        )
        .expect("invalid Location navigation probe should evaluate");

    assert_eq!(
        result,
        r#"{"assign":"SyntaxError:12:true","replace":"SyntaxError:12:true","href":"SyntaxError:12:true","windowLocation":"SyntaxError:12:true","documentLocation":"SyntaxError:12:true","unchanged":true}"#
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "invalid Location navigation must not queue navigation"
    );
}

#[test]
fn location_protocol_setter_rejects_invalid_schemes_without_navigation() {
    let mut vm = new_storage_test_vm("https://example.com/path?x=1#frag");

    let result = vm
        .eval(
            r#"
(() => {
  const originalHref = location.href;
  const invalid = [
    "", "\u0000", "\u0001", "\u000c", " ", "!", "\u007f",
    "\u0080", "\u00ff", ":", "\u2020", "x!", "1http"
  ];
  const errors = invalid.map(value => {
    try {
      location.protocol = value;
      return "returned";
    } catch (error) {
      return `${error.name}:${error.code}:${error instanceof DOMException}`;
    }
  });
  return JSON.stringify({
    allSyntaxErrors: errors.every(value => value === "SyntaxError:12:true"),
    hrefUnchanged: location.href === originalHref
  });
})()
"#,
        )
        .expect("invalid Location protocol probe should evaluate");

    assert_eq!(result, r#"{"allSyntaxErrors":true,"hrefUnchanged":true}"#);
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "invalid Location protocols must not queue navigation"
    );
}

#[test]
fn location_protocol_setter_uses_scheme_state_override_navigation_rules() {
    let original = "http://example.com/path?x=1#frag";
    let mut same_vm = new_storage_test_vm(original);
    same_vm
        .eval(r#"location.protocol = "ht\ntp:gunk"; "queued""#)
        .expect("same Location protocol probe should evaluate");
    assert_eq!(
        same_vm
            .take_pending_location_navigation_with_seed()
            .expect("same protocol assignment should still navigate")
            .url
            .as_str(),
        original
    );

    let mut changed_vm = new_storage_test_vm(original);
    changed_vm
        .eval(r#"location.protocol = "https::::"; "queued""#)
        .expect("changed Location protocol probe should evaluate");
    assert_eq!(
        changed_vm
            .take_pending_location_navigation_with_seed()
            .expect("HTTP(S) protocol assignment should navigate")
            .url
            .as_str(),
        "https://example.com/path?x=1#frag"
    );
}

#[test]
fn location_protocol_setter_navigates_after_rejected_scheme_changes() {
    for (original, scheme) in [
        ("http://example.com/path?x=1#frag", "data"),
        ("http://example.com/path?x=1#frag", "x"),
        ("http://example.com/path?x=1#frag", "http+x"),
        ("https://example.com/path?x=1#frag", "data:gunk"),
        ("http://example.com:8080/path?x=1#frag", "file"),
        ("https://example.com:8443/path?x=1#frag", "file"),
        ("http://user@example.com/path?x=1#frag", "file"),
        ("http://:password@example.com/path?x=1#frag", "file"),
    ] {
        let mut vm = new_storage_test_vm(original);
        let result = vm
            .eval(&format!(
                "location.protocol = {}; location.href",
                serde_json::to_string(scheme).unwrap()
            ))
            .expect("incompatible scheme assignment should not throw");
        assert_eq!(result, original, "{original} -> {scheme}");
        let navigation = vm
            .take_pending_location_navigation_with_seed()
            .unwrap_or_else(|| panic!("{original} -> {scheme} must still navigate"));
        assert_eq!(navigation.url.as_str(), original);
    }
}

#[test]
fn location_protocol_setter_does_not_navigate_non_http_results() {
    for (original, scheme) in [
        ("http://example.com/path?x=1#frag", "ftp"),
        ("http://example.com/path?x=1#frag", "ws"),
        ("http://example.com/path?x=1#frag", "wss"),
        ("http://example.com/path?x=1#frag", "file"),
        ("http://:@example.com/path?x=1#frag", "file"),
        ("http://example.com:80/path?x=1#frag", "file"),
        ("https://example.com:443/path?x=1#frag", "file"),
        ("about:blank", "https"),
        ("file:///path", "https"),
    ] {
        let mut vm = new_storage_test_vm(original);
        vm.eval(&format!(
            "location.protocol = {}",
            serde_json::to_string(scheme).unwrap()
        ))
        .expect("valid scheme assignment should not throw");
        assert!(
            vm.take_pending_location_navigation_with_seed().is_none(),
            "{original} -> {scheme} must not navigate"
        );
    }
}

#[tokio::test]
async fn removing_popover_attribute_cancels_pending_toggle_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://popover-attribute-removal-toggle.test/",
        &loader,
    );

    let before = vm
        .eval(
            r#"
            (() => {
              const popover = document.createElement("div");
              popover.popover = "auto";
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              (document.body || html.appendChild(document.createElement("body"))).appendChild(popover);
              globalThis.__lmPopoverAttributeRemovalEvents = [];
              for (const type of ["beforetoggle", "toggle"]) {
                popover.addEventListener(type, event => {
                  globalThis.__lmPopoverAttributeRemovalEvents.push(
                    `${event.type}:${event.oldState}->${event.newState}`
                  );
                });
              }
              popover.showPopover();
              popover.hidePopover();
              popover.removeAttribute("popover");
              return JSON.stringify({
                events: globalThis.__lmPopoverAttributeRemovalEvents,
                open: popover.matches(":popover-open"),
                hasAttribute: popover.hasAttribute("popover")
              });
            })()
            "#,
        )
        .expect("popover attribute removal setup should evaluate");

    assert_eq!(
        before,
        r#"{"events":["beforetoggle:closed->open","beforetoggle:open->closed"],"open":false,"hasAttribute":false}"#
    );

    assert!(
        !vm.has_ready_timeout(),
        "popover toggle events must not create synthetic Page timers"
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("canceled popover toggle tasks should drain")
    );

    let after = vm
        .eval("JSON.stringify(__lmPopoverAttributeRemovalEvents)")
        .expect("popover attribute removal event log should evaluate");
    assert_eq!(
        after,
        r#"["beforetoggle:closed->open","beforetoggle:open->closed"]"#
    );
}

#[test]
fn popover_show_rejects_owner_document_changes_during_toggle_steps() {
    let mut vm = new_storage_test_vm("https://popover-owner-document.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement || document.appendChild(
                document.createElement("html")
              );
              const body = document.body || root.appendChild(document.createElement("body"));
              const frame = document.createElement("iframe");
              body.appendChild(frame);
              const childDocument = frame.contentDocument;
              const childRoot = childDocument.documentElement || childDocument.appendChild(
                childDocument.createElement("html")
              );
              const childBody = childDocument.body || childRoot.appendChild(
                childDocument.createElement("body")
              );
              const invalidState = callback => {
                try {
                  callback();
                  return false;
                } catch (error) {
                  return error.name === "InvalidStateError";
                }
              };

              const movedWhileShowing = document.createElement("div");
              movedWhileShowing.popover = "auto";
              body.appendChild(movedWhileShowing);
              movedWhileShowing.addEventListener("beforetoggle", () => {
                childBody.appendChild(movedWhileShowing);
              }, { once: true });
              const showRejected = invalidState(() => movedWhileShowing.showPopover());

              const movedWhileHiding = document.createElement("div");
              movedWhileHiding.popover = "auto";
              body.appendChild(movedWhileHiding);
              movedWhileHiding.showPopover();
              movedWhileHiding.addEventListener("beforetoggle", event => {
                if (event.newState === "closed") childBody.appendChild(movedWhileHiding);
              }, { once: true });
              let hideThrew = false;
              try {
                movedWhileHiding.hidePopover();
              } catch (_) {
                hideThrew = true;
              }

              const parent = document.createElement("div");
              const openChild = document.createElement("div");
              const movedByDismiss = document.createElement("div");
              for (const popover of [parent, openChild, movedByDismiss]) {
                popover.popover = "auto";
              }
              parent.append(openChild, movedByDismiss);
              body.appendChild(parent);
              parent.showPopover();
              openChild.showPopover();
              openChild.addEventListener("beforetoggle", event => {
                if (event.newState === "closed") childBody.appendChild(movedByDismiss);
              });
              const dismissRejected = invalidState(() => movedByDismiss.showPopover());

              return JSON.stringify({
                showRejected,
                showStayedClosed: !movedWhileShowing.matches(":popover-open"),
                hideThrew,
                hideClosed: !movedWhileHiding.matches(":popover-open"),
                dismissRejected,
                dismissStayedClosed: !movedByDismiss.matches(":popover-open")
              });
            })()
            "#,
        )
        .expect("popover owner-document reentrancy should evaluate");

    assert_eq!(
        result,
        r#"{"showRejected":true,"showStayedClosed":true,"hideThrew":false,"hideClosed":true,"dismissRejected":true,"dismissStayedClosed":true}"#
    );
}

#[test]
fn clipboard_event_uses_a_branded_prototype_accessor_and_webidl_dictionary() {
    let mut vm = new_storage_test_vm("https://clipboard-event-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  const defaults = new ClipboardEvent("copy");
  const descriptor = Object.getOwnPropertyDescriptor(
    ClipboardEvent.prototype,
    "clipboardData"
  );
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };

  const order = [];
  const init = {};
  for (const [name, value] of [
    ["bubbles", true],
    ["cancelable", true],
    ["clipboardData", transfer],
    ["composed", true]
  ]) {
    Object.defineProperty(init, name, {
      get() {
        order.push(name);
        return value;
      }
    });
  }
  const initialized = new ClipboardEvent("paste", init);

  const sentinel = {};
  let preservesGetterException = false;
  try {
    new ClipboardEvent("copy", {
      get clipboardData() {
        throw sentinel;
      }
    });
  } catch (error) {
    preservesGetterException = error === sentinel;
  }

  return JSON.stringify({
    defaults: [
      defaults.clipboardData === null,
      Object.hasOwn(defaults, "clipboardData")
    ],
    initialized: [
      initialized.clipboardData === transfer,
      initialized.bubbles,
      initialized.cancelable,
      initialized.composed,
      Object.hasOwn(initialized, "clipboardData")
    ],
    descriptor: [
      typeof descriptor.get,
      descriptor.get.name,
      descriptor.get.length,
      typeof descriptor.set,
      descriptor.enumerable,
      descriptor.configurable
    ],
    order,
    errors: [
      errorName(() => descriptor.get.call({})),
      errorName(() => new ClipboardEvent("copy", {clipboardData: {}})),
      errorName(() => new ClipboardEvent("copy", 1))
    ],
    preservesGetterException
  });
})()
"#,
        )
        .expect("ClipboardEvent WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":[true,false],"initialized":[true,true,true,true,false],"descriptor":["function","get clipboardData",0,"undefined",true,true],"order":["bubbles","cancelable","composed","clipboardData"],"errors":["TypeError","TypeError","TypeError"],"preservesGetterException":true}"#
    );
}

#[test]
fn clipboard_change_event_exposes_frozen_types_and_bigint_change_id() {
    let mut vm = new_storage_test_vm("http://clipboard-change-event.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const defaults = new ClipboardChangeEvent("clipboardchange");
  const order = [];
  const sourceTypes = [
    {
      toString() {
        order.push("type item");
        return "text/plain";
      }
    },
    "text/html",
    "\uD800"
  ];
  const expectedChangeId = -(1n << 100n) + 123n;
  const init = {};
  for (const [name, value] of [
    ["bubbles", true],
    ["cancelable", true],
    ["changeId", expectedChangeId],
    ["composed", true],
    ["types", sourceTypes]
  ]) {
    Object.defineProperty(init, name, {
      get() {
        order.push(name);
        return value;
      }
    });
  }
  const initialized = new ClipboardChangeEvent("clipboardchange", init);
  sourceTypes[0] = "changed";
  sourceTypes.push("image/png");

  const typesDescriptor = Object.getOwnPropertyDescriptor(
    ClipboardChangeEvent.prototype,
    "types"
  );
  const changeIdDescriptor = Object.getOwnPropertyDescriptor(
    ClipboardChangeEvent.prototype,
    "changeId"
  );
  const accessor = descriptor => [
    typeof descriptor.get,
    descriptor.get.name,
    descriptor.get.length,
    typeof descriptor.set,
    descriptor.enumerable,
    descriptor.configurable
  ];
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };

  return JSON.stringify({
    constructor: [
      typeof ClipboardChangeEvent,
      ClipboardChangeEvent.name,
      ClipboardChangeEvent.length,
      ClipboardChangeEvent.prototype instanceof Event,
      isSecureContext
    ],
    defaults: [
      defaults instanceof ClipboardChangeEvent,
      defaults instanceof Event,
      defaults.types.length,
      Object.isFrozen(defaults.types),
      defaults.types === defaults.types,
      defaults.changeId === 0n,
      Object.hasOwn(defaults, "types"),
      Object.hasOwn(defaults, "changeId"),
      Object.prototype.toString.call(defaults)
    ],
    initialized: [
      initialized.bubbles,
      initialized.cancelable,
      initialized.composed,
      initialized.types.slice(0, 2).join(","),
      initialized.types.length,
      initialized.types[2].charCodeAt(0) === 0xD800,
      initialized.types === initialized.types,
      Object.isFrozen(initialized.types),
      String(initialized.changeId),
      initialized.changeId === expectedChangeId
    ],
    accessors: [accessor(typesDescriptor), accessor(changeIdDescriptor)],
    keys: Object.keys(ClipboardChangeEvent.prototype),
    order,
    booleanBigInt: new ClipboardChangeEvent("x", {changeId: true}).changeId === 1n,
    errors: [
      errorName(() => ClipboardChangeEvent("x")),
      errorName(() => new ClipboardChangeEvent()),
      errorName(() => new ClipboardChangeEvent("x", 1)),
      errorName(() => new ClipboardChangeEvent("x", {changeId: 1})),
      errorName(() => new ClipboardChangeEvent("x", {types: null})),
      errorName(() => typesDescriptor.get.call({})),
      errorName(() => changeIdDescriptor.get.call({}))
    ]
  });
})()
"#,
        )
        .expect("ClipboardChangeEvent WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructor":["function","ClipboardChangeEvent",1,true,false],"defaults":[true,true,0,true,true,true,false,false,"[object ClipboardChangeEvent]"],"initialized":[true,true,true,"text/plain,text/html",3,true,true,true,"-1267650600228229401496703205253",true],"accessors":[["function","get types",0,"undefined",true,true],["function","get changeId",0,"undefined",true,true]],"keys":["types","changeId"],"order":["bubbles","cancelable","composed","changeId","types","type item"],"booleanBigInt":true,"errors":["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn clipboard_event_dictionaries_convert_event_init_before_derived_members() {
    let mut vm = new_storage_test_vm("https://clipboard-event-dictionary-order.test/");
    let result = vm.eval(r#"
      (() => {
        const checks = [];
        for (const [Ctor, derivedMember] of [
          [ClipboardEvent, 'clipboardData'], [ClipboardChangeEvent, 'changeId']
        ]) {
          for (const baseMember of ['bubbles', 'cancelable', 'composed']) {
            const baseError = new RangeError('base getter');
            const derivedError = new TypeError('derived getter');
            const reads = [];
            const init = {};
            Object.defineProperty(init, baseMember, {get() {
              reads.push(baseMember);
              throw baseError;
            }});
            Object.defineProperty(init, derivedMember, {get() {
              reads.push(derivedMember);
              throw derivedError;
            }});
            try { new Ctor('change', init); checks.push(false); }
            catch (error) { checks.push(error === baseError); }
            checks.push(reads.length === 1, reads[0] === baseMember);
          }
        }
        const transfer = new DataTransfer();
        const clipboardInit = {
          clipboardData: {},
          get composed() { this.clipboardData = transfer; return true; }
        };
        const clipboard = new ClipboardEvent('copy', clipboardInit);
        checks.push(clipboard.composed, clipboard.clipboardData === transfer);
        const changeInit = {
          changeId: 1,
          get composed() { this.changeId = 42n; return true; },
          types: ['text/plain']
        };
        const change = new ClipboardChangeEvent('clipboardchange', changeInit);
        checks.push(change.composed, change.changeId === 42n, change.types[0] === 'text/plain');
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("Clipboard dictionary conversion should stop on base errors and observe base getter mutations first");
    assert_eq!(result, "true");
}
