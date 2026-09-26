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
        r#"[{"type":"DOMContentLoaded","trusted":true,"target":true,"currentTarget":true,"bubbles":true,"cancelable":false,"intrinsicPrototype":true,"intrinsicInstance":true,"ownMethods":[],"inheritedMethods":true,"ownCancelBubble":false,"ownReturnValue":false,"ownTimeStamp":false,"ownIsTrusted":true,"keys":["type","target","srcElement","currentTarget","defaultPrevented","bubbles","cancelable","isTrusted","composed","eventPhase"]}]"#
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
    Object.getOwnPropertyDescriptor(Document.prototype, "onmouseleave")
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
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
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

    assert!(!vm.has_ready_timeout());
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
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
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
              const html = document.appendChild(document.createElement("html"));
              html.appendChild(document.createElement("body")).appendChild(popover);
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
