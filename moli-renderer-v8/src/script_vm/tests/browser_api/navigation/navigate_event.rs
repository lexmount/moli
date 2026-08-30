use super::*;

#[test]
fn same_document_navigation_fires_navigate_event_before_mutation() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const seen = {};
              let capturedDestination = null;
              navigation.onnavigate = e => {
                capturedDestination = e.destination;
                seen.type = e.type;
                seen.navigationType = e.navigationType;
                seen.cancelable = e.cancelable;
                seen.canIntercept = e.canIntercept;
                seen.hashChange = e.hashChange;
                seen.destinationHash = new URL(e.destination.url).hash;
                seen.destinationState = e.destination.getState().ok;
                seen.destinationStateCloned = e.destination.getState() !== e.destination.getState();
                seen.destinationOwnSlots = Object.getOwnPropertyNames(e.destination)
                  .filter(name => name.startsWith("__lmNavigationDestination"))
                  .sort();
                Object.defineProperties(e.destination, {
                  __lmNavigationDestinationState: { value: { ok: 99 }, configurable: true },
                  __lmNavigationDestinationEntry: { value: null, configurable: true }
                });
                seen.destinationStateAfterSpoof = e.destination.getState().ok;
                seen.syntheticIntercept = (() => {
                  try {
                    new NavigateEvent("navigate", {
                      destination: e.destination,
                      signal: new AbortController().signal
                    }).intercept();
                    return "no throw";
                  } catch (error) {
                    return `${error.name}:${error instanceof DOMException}:${error.code}`;
                  }
                })();
                e.preventDefault();
              };
              const startLength = history.length;
              history.pushState({ ok: 7 }, "", "#blocked");
              return JSON.stringify({
                seen,
                hash: location.hash,
                state: history.state,
                lengthUnchanged: history.length === startLength,
                noInitThrows: (() => {
                  try {
                    new NavigateEvent("navigate");
                    return false;
                  } catch (error) {
                    return error.name === "TypeError";
                  }
                })(),
                missingSignalThrows: (() => {
                  try {
                    new NavigateEvent("navigate", { destination: capturedDestination });
                    return false;
                  } catch (error) {
                    return error.name === "TypeError";
                  }
                })()
              });
            })()
            "##,
        )
        .expect("same-document navigate event probe should evaluate");

    assert_eq!(
        result,
        r##"{"seen":{"type":"navigate","navigationType":"push","cancelable":true,"canIntercept":true,"hashChange":false,"destinationHash":"#blocked","destinationState":7,"destinationStateCloned":true,"destinationOwnSlots":[],"destinationStateAfterSpoof":7,"syntheticIntercept":"SecurityError:true:18"},"hash":"","state":null,"lengthUnchanged":true,"noInitThrows":true,"missingSignalThrows":true}"##
    );
}

#[test]
fn canceled_post_form_navigation_aborts_signal_without_synthetic_timer() {
    let mut vm = new_storage_test_vm("https://example.com/form-page");

    let setup = vm
        .eval(
            r##"
            (() => {
              const root = document.body || document.documentElement || document.appendChild(document.createElement("html"));
              if (!document.body && root === document.documentElement) {
                root.appendChild(document.createElement("body"));
              }
              const host = document.body || root;
              const form = document.createElement("form");
              form.action = "/submitted";
              form.method = "post";
              const input = document.createElement("input");
              input.name = "q";
              input.value = "value";
              form.appendChild(input);
              host.appendChild(form);
              globalThis.__lmCanceledFormNavigationLog = [];
              navigation.onnavigate = event => {
                __lmCanceledFormNavigationLog.push([
                  "navigate",
                  event.navigationType,
                  event.cancelable,
                  event.signal.aborted,
                  location.href
                ].join(":"));
                event.signal.addEventListener("abort", () => {
                  __lmCanceledFormNavigationLog.push([
                    "abort",
                    event.signal.reason.name,
                    location.href
                  ].join(":"));
                });
                event.preventDefault();
              };
              navigation.onnavigateerror = event => {
                __lmCanceledFormNavigationLog.push([
                  "error",
                  event.error.name,
                  location.href
                ].join(":"));
              };
              form.requestSubmit();
              return __lmCanceledFormNavigationLog.join("|");
            })()
            "##,
        )
        .expect("canceled form navigation setup should evaluate");

    assert_eq!(
        setup,
        "navigate:replace:true:false:https://example.com/form-page|abort:AbortError:https://example.com/form-page|error:AbortError:https://example.com/form-page"
    );
    assert!(
        !vm.has_ready_timeout(),
        "canceling a POST form navigation on the current event loop must not create a timer task"
    );
}

#[test]
fn cross_document_unload_lifecycle_orders_pagehide_before_unload_without_timer() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let lifecycle = vm
        .eval(
            r##"
            (() => {
              const log = [];
              addEventListener("beforeunload", event => log.push(`beforeunload:${event.isTrusted}`));
              addEventListener("pagehide", event => log.push([
                "pagehide",
                event instanceof PageTransitionEvent,
                event.persisted,
                event.bubbles,
                event.cancelable,
                event.isTrusted
              ].join(":")));
              addEventListener("unload", event => log.push(`unload:${event.isTrusted}`));
              navigation.navigate("/next-document");
              return log.join("|");
            })()
            "##,
        )
        .expect("cross-document unload lifecycle should evaluate");

    assert_eq!(
        lifecycle,
        "beforeunload:true|pagehide:true:false:true:true:true|unload:true"
    );
    assert!(
        !vm.has_ready_timeout(),
        "pagehide is part of the unload step and must not create an independent timer task"
    );
}

#[test]
fn before_unload_handler_coerces_its_result_while_window_event_is_current() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              let customCurrent = false;
              onbeforeunload = event => ({
                toString() {
                  customCurrent = window.event === event;
                  return "custom";
                }
              });
              const custom = new CustomEvent("beforeunload", { cancelable: true });
              const customDispatchResult = dispatchEvent(custom);

              let realEvent;
              const realSteps = [];
              onbeforeunload = event => {
                realEvent = event;
                realSteps.push([
                  event instanceof BeforeUnloadEvent,
                  event.cancelable,
                  event.returnValue,
                  window.event === event
                ]);
                return {
                  toString() {
                    realSteps.push(["coerce", window.event === event]);
                    return "leave";
                  }
                };
              };
              navigation.navigate("/next-document");

              return JSON.stringify({
                custom: [customCurrent, custom.defaultPrevented, customDispatchResult],
                realSteps,
                real: [realEvent.defaultPrevented, realEvent.returnValue]
              });
            })()
            "##,
        )
        .expect("beforeunload handler return processing should evaluate");

    assert_eq!(
        result,
        r#"{"custom":[true,false,true],"realSteps":[[true,true,"",true],["coerce",true]],"real":[true,"leave"]}"#
    );
}

#[test]
fn navigate_event_intercept_option_stringification_preserves_thrown_exception() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const seen = [];
              navigation.onnavigate = e => {
                for (const key of ["focusReset", "scroll"]) {
                  try {
                    e.intercept({
                      [key]: {
                        toString() {
                          throw new RangeError(`${key}:sentinel`);
                        }
                      }
                    });
                    seen.push(`${key}:no throw`);
                  } catch (error) {
                    seen.push(`${key}:${error.name}:${error.message}`);
                  }
                }
                e.preventDefault();
              };
              history.pushState(null, "", "#stringify-options");
              return seen.join("|");
            })()
            "##,
        )
        .expect("NavigateEvent intercept option probe should evaluate");

    assert_eq!(
        result,
        "focusReset:RangeError:focusReset:sentinel|scroll:RangeError:scroll:sentinel"
    );
}

#[test]
fn navigate_event_constructor_reflects_required_members_and_defaults() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const probeThrow = callback => {
                try {
                  callback();
                  return "no throw";
                } catch (error) {
                  return error.name;
                }
              };
              let destination = null;
              navigation.onnavigate = e => destination = e.destination;
              history.pushState({ statevar: "state" }, "", "#destination");
              const signal = new AbortController().signal;
              const info = { some: "object" };
              const formData = new FormData();
              const sourceElement = document.createElement("a");
              const methodNames = ["intercept", "deferPageSwap", "scroll"];
              const methodShape = (event, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(event, name);
                const value = descriptor && descriptor.value;
                return [
                  name,
                  typeof value,
                  value && value.name,
                  value && value.length,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.writable,
                  descriptor && descriptor.configurable,
                  Object.prototype.hasOwnProperty.call(event, name)
                ].join(":");
              };
              const full = new NavigateEvent("navigate", {
                navigationType: "replace",
                destination,
                canIntercept: true,
                userInitiated: true,
                hashChange: true,
                signal,
                formData,
                downloadRequest: "download",
                info,
                hasUAVisualTransition: true,
                sourceElement
              });
              const defaults = new NavigateEvent("navigate", { destination, signal });
              return JSON.stringify({
                noDictionary: probeThrow(() => new NavigateEvent("navigate")),
                noDestination: probeThrow(() => new NavigateEvent("navigate", {
                  signal,
                  canIntercept: false,
                  userInitiated: false,
                  hashChange: false
                })),
                noSignal: probeThrow(() => new NavigateEvent("navigate", { destination })),
                full: [
                  full.navigationType,
                  full.destination === destination,
                  full.canIntercept,
                  full.userInitiated,
                  full.hashChange,
                  full.signal === signal,
                  full.formData === formData,
                  full.downloadRequest,
                  full.info === info,
                  full.hasUAVisualTransition,
                  full.sourceElement === sourceElement
                ].join("|"),
                defaults: [
                  defaults.navigationType,
                  defaults.canIntercept,
                  defaults.userInitiated,
                  defaults.hashChange,
                  defaults.formData === null,
                  defaults.downloadRequest === null,
                  defaults.info === undefined,
                  defaults.sourceElement === null,
                  defaults.hasUAVisualTransition
                ].join("|"),
                methodKeys: Object.keys(full)
                  .filter(name => methodNames.includes(name))
                  .join(","),
                methods: methodNames.map(name => methodShape(full, name)).join("|")
              });
            })()
            "##,
        )
        .expect("NavigateEvent constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"noDictionary":"TypeError","noDestination":"TypeError","noSignal":"TypeError","full":"replace|true|true|true|true|true|true|download|true|true|true","defaults":"push|false|false|false|true|true|true|true|false","methodKeys":"intercept,deferPageSwap,scroll","methods":"intercept:function:intercept:0:true:true:true:true|deferPageSwap:function:deferPageSwap:0:true:true:true:true|scroll:function:scroll:0:true:true:true:true"}"#
    );
}

#[test]
fn navigation_transition_constructor_surface_is_present_but_illegal() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r#"
            (() => {
              const probeThrow = callback => {
                try {
                  callback();
                  return "no throw";
                } catch (error) {
                  return error.name;
                }
              };
              return JSON.stringify({
                type: typeof NavigationTransition,
                prototypeObject: typeof NavigationTransition.prototype,
                hasCommittedPrototypeMember: "committed" in NavigationTransition.prototype,
                initialTransition: navigation.transition,
                construct: probeThrow(() => new NavigationTransition())
              });
            })()
            "#,
        )
        .expect("NavigationTransition constructor surface should evaluate");

    assert_eq!(
        result,
        r#"{"type":"function","prototypeObject":"object","hasCommittedPrototypeMember":true,"initialTransition":null,"construct":"TypeError"}"#
    );
}

#[test]
fn intercepted_same_document_navigation_exposes_transition() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              globalThis.__lmInterceptTransitionLog = [];
              const record = name => globalThis.__lmInterceptTransitionLog.push({
                name,
                transitionObject: navigation.transition !== null,
                transitionBrand: navigation.transition instanceof NavigationTransition,
                fromMatches: navigation.transition?.from === from,
                navigationType: navigation.transition?.navigationType ?? null
              });
              navigation.addEventListener("navigate", () => record("navigate"));
              navigation.addEventListener("currententrychange", () => record("currententrychange"));
              navigation.addEventListener("navigatesuccess", () => record("navigatesuccess"));
              navigation.onnavigate = event => event.intercept({
                handler() { record("handler"); }
              });
              const result = navigation.navigate("#one");
              return JSON.stringify({
                log: globalThis.__lmInterceptTransitionLog,
                transitionAfterNavigate: navigation.transition !== null,
                committedPromise: typeof result.committed.then === "function",
                finishedPromise: typeof result.finished.then === "function"
              });
            })()
            "##,
        )
        .expect("intercepted transition probe should evaluate");

    assert_eq!(
        result,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"}],"transitionAfterNavigate":true,"committedPromise":true,"finishedPromise":true}"##
    );
    let after_microtask = vm
        .eval(
            r##"JSON.stringify({
              log: globalThis.__lmInterceptTransitionLog,
              transitionAfterNavigate: navigation.transition
            })"##,
        )
        .expect("intercepted transition microtask probe should evaluate");
    assert_eq!(
        after_microtask,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"},{"name":"navigatesuccess","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"push"}],"transitionAfterNavigate":null}"##
    );
}

#[test]
fn navigation_transition_object_keeps_declared_brand_and_members() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              let observed = "";
              navigation.onnavigate = event => event.intercept({
                handler() {
                  const transition = navigation.transition;
                  observed = [
                    transition instanceof NavigationTransition,
                    Object.prototype.toString.call(transition),
                    Object.keys(transition).join(","),
                    Object.getOwnPropertyNames(transition)
                      .filter(name => name.startsWith("__lmNavigationTransition"))
                      .join(","),
                    Object.hasOwn(transition, "from"),
                    Object.getOwnPropertyDescriptor(transition, "from").enumerable,
                    Object.getOwnPropertyDescriptor(transition, Symbol.toStringTag).writable,
                    transition.from === from,
                    transition.to !== null && typeof transition.to === "object",
                    transition.navigationType,
                    transition.committed instanceof Promise,
                    transition.finished instanceof Promise
                  ].join("|");
                }
              });
              navigation.navigate("#one");
              return observed;
            })()
            "##,
        )
        .expect("declared NavigationTransition probe should evaluate");

    assert_eq!(
        result,
        "true|[object NavigationTransition]|||true|false|false|true|true|push|true|true"
    );
}

#[test]
fn precommit_transition_seed_is_not_script_writable() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              let observed = "";
              navigation.onnavigate = event => {
                event.__lmNavigateEventPrecommitTransitionFrom = {};
                event.__lmNavigateEventPrecommitTransitionDestination = null;
                event.__lmNavigateEventPrecommitTransitionType = "reload";
                event.intercept({
                  precommitHandler() {
                    observed = [
                      navigation.transition?.from === from,
                      navigation.transition?.navigationType,
                      navigation.transition?.finished instanceof Promise
                    ].join("|");
                  }
                });
              };
              navigation.navigate("#one");
              return observed;
            })()
            "##,
        )
        .expect("precommit transition private seed probe should evaluate");

    assert_eq!(result, "true|push|true");
}

#[test]
fn precommit_controller_methods_are_declared_and_slots_private() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const log = globalThis.__lmPrecommitControllerLog = [];
              let observed = "";
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler(controller) {
                    const initialNames = Object.getOwnPropertyNames(controller).sort();
                    Object.defineProperties(controller, {
                      __lmPrecommitControllerEvent: { value: null, configurable: true },
                      __lmPrecommitControllerActive: { value: false, configurable: true }
                    });
                    const addHandler = controller.addHandler;
                    const redirect = controller.redirect;
                    const addDescriptor = Object.getOwnPropertyDescriptor(controller, "addHandler");
                    const redirectDescriptor = Object.getOwnPropertyDescriptor(controller, "redirect");
                    controller.addHandler(() => log.push("added"));
                    controller.redirect("#declared");
                    const ownNames = Object.getOwnPropertyNames(controller).sort();
                    observed = JSON.stringify({
                      initialNames,
                      initialInternalNames: initialNames.filter(name => name.startsWith("__lm")),
                      ownNames,
                      keys: Object.keys(controller).sort(),
                      spoofedInternalNames: ownNames.filter(name => name.startsWith("__lm")),
                      addHandlerName: addHandler.name,
                      addHandlerLength: addHandler.length,
                      addHandlerEnumerable: addDescriptor.enumerable,
                      redirectName: redirect.name,
                      redirectLength: redirect.length,
                      redirectEnumerable: redirectDescriptor.enumerable,
                      destinationHash: new URL(event.destination.url).hash
                    });
                  },
                  handler() {
                    log.push(`handler:${location.hash}`);
                  }
                });
              };
              navigation.navigate("#one");
              return `${observed}|${log.join(",")}|${location.hash}`;
            })()
            "##,
        )
        .expect("precommit controller declared method probe should evaluate");

    assert_eq!(
        result,
        r##"{"initialNames":["addHandler","redirect"],"initialInternalNames":[],"ownNames":["__lmPrecommitControllerActive","__lmPrecommitControllerEvent","addHandler","redirect"],"keys":["addHandler","redirect"],"spoofedInternalNames":["__lmPrecommitControllerActive","__lmPrecommitControllerEvent"],"addHandlerName":"addHandler","addHandlerLength":1,"addHandlerEnumerable":true,"redirectName":"redirect","redirectLength":1,"redirectEnumerable":true,"destinationHash":"#declared"}||"##
    );
    assert_eq!(
        vm.eval("`${__lmPrecommitControllerLog.join(',')}|${location.hash}`")
            .expect("precommit controller handlers should settle"),
        "handler:#declared,added|#declared"
    );
}

#[test]
fn navigation_precommit_runs_after_complete_dispatch_and_before_commit_handlers() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let during_navigation = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmNavigationCallbackOrder = [];
              navigation.addEventListener("navigate", event => {
                __lmNavigationCallbackOrder.push("listener-1-before");
                event.intercept({
                  precommitHandler() {
                    __lmNavigationCallbackOrder.push("precommit");
                  },
                  handler() {
                    __lmNavigationCallbackOrder.push("handler");
                  }
                });
                __lmNavigationCallbackOrder.push("listener-1-after");
              });
              navigation.addEventListener("navigate", () => {
                __lmNavigationCallbackOrder.push("listener-2");
              });
              const result = navigation.navigate("#callback-order");
              result.committed.then(() => {
                __lmNavigationCallbackOrder.push("committed");
              });
              result.finished.then(() => {
                __lmNavigationCallbackOrder.push("finished");
              });
              return __lmNavigationCallbackOrder.join("|");
            })()
            "##,
        )
        .expect("Navigation callback ordering probe should evaluate");

    assert_eq!(
        during_navigation,
        "listener-1-before|listener-1-after|listener-2|precommit"
    );
    assert_eq!(
        vm.eval("__lmNavigationCallbackOrder.join('|')")
            .expect("Navigation callback Promise boundaries should settle"),
        "listener-1-before|listener-1-after|listener-2|precommit|handler|committed|finished"
    );
}

#[test]
fn navigation_handlers_use_webidl_callback_realms_proxies_and_promise_errors() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        (() => {
          const root =
            document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body =
            document.body ||
            root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          frame.id = "navigation-callback-frame";
          body.appendChild(frame);
          return "created";
        })()
        "#,
    )
    .expect("Navigation callback child Realm should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "Navigation callback child Realm should materialize",
    );

    let during_navigation = vm
        .eval(
            r##"
            (() => {
              const frame = document.getElementById("navigation-callback-frame");
              const child = frame.contentWindow;
              globalThis.__lmNavigationCallbackFrame = frame;
              globalThis.__lmNavigationCallbackFacts = [];
              globalThis.__lmNavigationCallbackProxyCalls = [];
              globalThis.__lmNavigationCallbackError = null;

              const added = child.Function(`
                return new Proxy(
                  function() {
                    "use strict";
                    parent.__lmNavigationCallbackFacts.push({
                      name: "added",
                      callbackRealm:
                        globalThis === parent.__lmNavigationCallbackFrame.contentWindow,
                      receiver: this === parent.__lmNavigationCallbackEvent,
                      argumentCount: arguments.length
                    });
                  },
                  {
                    apply(target, receiver, args) {
                      parent.__lmNavigationCallbackProxyCalls.push("added");
                      return Reflect.apply(target, receiver, args);
                    }
                  }
                );
              `)();
              const precommit = child.Function("added", `
                return new Proxy(
                  function(controller) {
                    "use strict";
                    parent.__lmNavigationCallbackFacts.push({
                      name: "precommit",
                      callbackRealm:
                        globalThis === parent.__lmNavigationCallbackFrame.contentWindow,
                      receiver: this === parent.__lmNavigationCallbackEvent,
                      argumentCount: arguments.length
                    });
                    controller.addHandler(added);
                  },
                  {
                    apply(target, receiver, args) {
                      parent.__lmNavigationCallbackProxyCalls.push("precommit");
                      return Reflect.apply(target, receiver, args);
                    }
                  }
                );
              `)(added);
              const handler = child.Function(`
                return new Proxy(
                  function() {
                    "use strict";
                    parent.__lmNavigationCallbackFacts.push({
                      name: "handler",
                      callbackRealm:
                        globalThis === parent.__lmNavigationCallbackFrame.contentWindow,
                      receiver: this === parent.__lmNavigationCallbackEvent,
                      argumentCount: arguments.length
                    });
                    throw new Error("navigation-callback-realm-error");
                  },
                  {
                    apply(target, receiver, args) {
                      parent.__lmNavigationCallbackProxyCalls.push("handler");
                      return Reflect.apply(target, receiver, args);
                    }
                  }
                );
              `)();

              navigation.onnavigate = event => {
                globalThis.__lmNavigationCallbackEvent = event;
                event.intercept({
                  precommitHandler: precommit,
                  handler
                });
              };
              navigation.navigate("#callback-realm").finished.catch(error => {
                globalThis.__lmNavigationCallbackError = {
                  callbackRealm:
                    error instanceof child.Error && !(error instanceof Error),
                  message: error.message
                };
              });
              return JSON.stringify({
                facts: __lmNavigationCallbackFacts,
                proxyCalls: __lmNavigationCallbackProxyCalls,
                error: __lmNavigationCallbackError
              });
            })()
            "##,
        )
        .expect("Navigation callback-function semantics should queue");

    assert_eq!(
        during_navigation,
        r#"{"facts":[{"name":"precommit","callbackRealm":true,"receiver":true,"argumentCount":1}],"proxyCalls":["precommit"],"error":null}"#
    );
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              facts: __lmNavigationCallbackFacts,
              proxyCalls: __lmNavigationCallbackProxyCalls,
              error: __lmNavigationCallbackError
            })"#
        )
        .expect("Navigation callback-function Promise rejection should settle"),
        r#"{"facts":[{"name":"precommit","callbackRealm":true,"receiver":true,"argumentCount":1},{"name":"handler","callbackRealm":true,"receiver":true,"argumentCount":0},{"name":"added","callbackRealm":true,"receiver":true,"argumentCount":0}],"proxyCalls":["precommit","handler","added"],"error":{"callbackRealm":true,"message":"navigation-callback-realm-error"}}"#
    );
}

#[test]
fn navigation_handlers_skip_a_retired_callback_window_without_aborting_navigation() {
    let mut vm = new_storage_test_vm("https://example.com/base");
    vm.eval(
        r#"
        (() => {
          const root =
            document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body =
            document.body ||
            root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          frame.id = "retired-navigation-callback-frame";
          body.appendChild(frame);
          return "created";
        })()
        "#,
    )
    .expect("retired Navigation callback child Realm should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "retired Navigation callback child Realm should materialize",
    );

    let during_navigation = vm
        .eval(
            r##"
            (() => {
              const frame = document.getElementById("retired-navigation-callback-frame");
              const child = frame.contentWindow;
              globalThis.__lmRetiredNavigationCallbackRuns = [];
              globalThis.__lmRetiredNavigationSettlements = [];
              const precommit = child.Function(
                `parent.__lmRetiredNavigationCallbackRuns.push("precommit");`
              );
              const handler = child.Function(
                `parent.__lmRetiredNavigationCallbackRuns.push("handler");`
              );
              navigation.addEventListener("navigate", event => {
                event.intercept({ precommitHandler: precommit, handler });
              }, { once: true });
              navigation.addEventListener("navigate", () => {
                frame.remove();
              }, { once: true });
              const result = navigation.navigate("#retired-callback-realm");
              result.committed.then(
                () => __lmRetiredNavigationSettlements.push("committed"),
                error => __lmRetiredNavigationSettlements.push(`committed:${error.name}`)
              );
              result.finished.then(
                () => __lmRetiredNavigationSettlements.push("finished"),
                error => __lmRetiredNavigationSettlements.push(`finished:${error.name}`)
              );
              return JSON.stringify({
                runs: __lmRetiredNavigationCallbackRuns,
                settlements: __lmRetiredNavigationSettlements
              });
            })()
            "##,
        )
        .expect("retired Navigation callbacks should queue");

    assert_eq!(during_navigation, r#"{"runs":[],"settlements":[]}"#);
    assert_eq!(
        vm.eval(
            "JSON.stringify({ runs: __lmRetiredNavigationCallbackRuns, settlements: __lmRetiredNavigationSettlements })"
        )
        .expect("retired Navigation callbacks should settle without running"),
        r#"{"runs":[],"settlements":["committed","finished"]}"#
    );
}

#[test]
fn navigate_event_internal_flags_are_not_script_writable() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const destination = {
                url: location.href,
                key: "",
                id: "",
                index: 0,
                sameDocument: true,
                getState() { return null; }
              };
              const event = new NavigateEvent("navigate", {
                destination,
                signal: new AbortController().signal,
                canIntercept: true
              });
              const exposedBefore = "__lmNavigateEventSynthetic" in event;
              event.__lmNavigateEventSynthetic = false;
              try {
                event.intercept({ handler() {} });
                return "allowed";
              } catch (error) {
                return `${error.name}:${exposedBefore}`;
              }
            })()
            "##,
        )
        .expect("NavigateEvent private flag tamper probe should evaluate");

    assert_eq!(result, "SecurityError:false");
}

#[test]
fn navigate_event_dispatching_flag_is_not_script_writable_after_dispatch() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              let captured;
              navigation.onnavigate = event => { captured = event; };
              navigation.navigate("#one");
              const exposed = "__lmDispatching" in captured;
              captured.__lmDispatching = true;
              try {
                captured.intercept({ handler() {} });
                return "allowed";
              } catch (error) {
                return `${error.name}:${exposed}`;
              }
            })()
            "##,
        )
        .expect("NavigateEvent dispatching flag tamper probe should evaluate");

    assert_eq!(result, "InvalidStateError:false");
}

#[test]
fn navigation_scroll_and_focus_slots_are_not_script_writable() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              body.innerHTML = "<button id='before'>before</button><input id='after'><div id='target'></div>";
              before.focus();
              const log = [];
              navigation.onnavigate = event => {
                event.intercept({
                  handler() {
                    const exposed = [
                      "__lmNavigationFocusResetEpoch" in navigation,
                      "__lmNavigationActiveScrollEvent" in navigation,
                      "__lmNavigationScrollTargetHref" in navigation
                    ].join(",");
                    const forgedEpoch = navigation.__lmNavigationFocusResetEpoch;
                    after.focus();
                    navigation.__lmNavigationFocusResetEpoch = forgedEpoch;
                    navigation.__lmNavigationActiveScrollEvent = {};
                    navigation.__lmNavigationScrollTargetHref = "https://example.com/forged";
                    log.push(`${exposed}:${document.activeElement.id}`);
                  }
                });
              };
              navigation.navigate("#target");
              log.push(`after:${document.activeElement.id}`);
              return log.join("|");
            })()
            "##,
        )
        .expect("navigation private slot tamper probe should evaluate");

    assert_eq!(result, "false,false,false:after|after:after");
}

#[test]
fn same_document_replace_dispatches_currententrychange_before_dispose() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const original = navigation.currentEntry;
              original.ondispose = () => {
                log.push(`dispose:${log.includes("currententrychange")}:${original.index}`);
              };
              navigation.oncurrententrychange = event => {
                log.push("currententrychange");
                log.push(`from:${event.from === original}:${event.from.index}:${event.navigationType}`);
              };
              navigation.navigate("#replace", { history: "replace" });
              return log.join("|");
            })()
            "##,
        )
        .expect("replace dispose ordering probe should evaluate");

    assert_eq!(
        result,
        "currententrychange|from:true:-1:replace|dispose:true:-1"
    );
}

#[tokio::test]
async fn reentrant_same_document_navigation_aborts_active_navigate_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmReentrantNavigationLog = [];
              let first = true;
              navigation.addEventListener("navigate", event => {
                globalThis.__lmReentrantNavigationLog.push(`navigate:${location.hash}`);
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmReentrantNavigationLog.push(`abort:${event.signal.reason.name}:${location.hash}`);
                });
                event.intercept({
                  handler: () => new Promise(resolve => {
                    setTimeout(() => {
                      globalThis.__lmReentrantNavigationLog.push(`handler:${location.hash}`);
                      resolve();
                    }, 0);
                  })
                });
                if (first) {
                  first = false;
                  const second = navigation.navigate("#two");
                  second.committed.then(
                    () => globalThis.__lmReentrantNavigationLog.push(`secondCommitted:${location.hash}`),
                    error => globalThis.__lmReentrantNavigationLog.push(`secondCommittedRejected:${error.name}`)
                  );
                  second.finished.then(
                    () => globalThis.__lmReentrantNavigationLog.push(`secondFinished:${location.hash}`),
                    error => globalThis.__lmReentrantNavigationLog.push(`secondFinishedRejected:${error.name}`)
                  );
                }
              });
              const firstResult = navigation.navigate("#one");
              firstResult.committed.then(
                () => globalThis.__lmReentrantNavigationLog.push("firstCommitted"),
                error => globalThis.__lmReentrantNavigationLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmReentrantNavigationLog.push("firstFinished"),
                error => globalThis.__lmReentrantNavigationLog.push(`firstFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${globalThis.__lmReentrantNavigationLog.join("|")}`;
            })()
            "##,
        )
        .expect("reentrant navigation setup should evaluate");

    assert_eq!(setup, "#two:navigate:|abort:AbortError:|navigate:");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("reentrant navigation handler should drain");
    let settled = vm
        .eval("globalThis.__lmReentrantNavigationLog.join('|')")
        .expect("reentrant navigation log should evaluate");

    assert_eq!(
        settled,
        "navigate:|abort:AbortError:|navigate:|secondCommitted:#two|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|handler:#two|secondFinished:#two"
    );
}

#[tokio::test]
async fn active_navigate_event_slot_is_not_script_writable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmActiveNavigateSlotLog = [];
              Object.defineProperty(Object.prototype, "__lmActiveNavigateEventEvent", {
                configurable: true,
                value: { cancelable: false }
              });
              Object.defineProperty(Object.prototype, "__lmActiveNavigateEventSignal", {
                configurable: true,
                value: { addEventListener() {
                  globalThis.__lmActiveNavigateSlotLog.push("prototype-signal");
                } }
              });
              Object.defineProperty(Object.prototype, "__lmActiveNavigateEventHref", {
                configurable: true,
                value: "#spoofed"
              });
              globalThis.__lmActiveNavigateSlotLog.push([
                Object.prototype.__lmActiveNavigateEventEvent.cancelable === false,
                typeof Object.prototype.__lmActiveNavigateEventSignal.addEventListener,
                Object.prototype.__lmActiveNavigateEventHref
              ].join(":"));
              let first = true;
              navigation.addEventListener("navigate", event => {
                globalThis.__lmActiveNavigateSlotLog.push(`navigate:${location.hash}`);
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmActiveNavigateSlotLog.push(`abort:${event.signal.reason.name}:${location.hash}`);
                });
                event.intercept({
                  handler: () => new Promise(resolve => {
                    setTimeout(() => {
                      globalThis.__lmActiveNavigateSlotLog.push(`handler:${location.hash}`);
                      resolve();
                    }, 0);
                  })
                });
                if (first) {
                  first = false;
                  globalThis.__lmActiveNavigateSlotLog.push(
                    `exposed:${"__lmNavigationActiveNavigateEvent" in navigation}`
                  );
                  navigation.__lmNavigationActiveNavigateEvent = null;
                  const second = navigation.navigate("#two");
                  second.committed.then(
                    () => globalThis.__lmActiveNavigateSlotLog.push(`secondCommitted:${location.hash}`),
                    error => globalThis.__lmActiveNavigateSlotLog.push(`secondCommittedRejected:${error.name}`)
                  );
                  second.finished.then(
                    () => globalThis.__lmActiveNavigateSlotLog.push(`secondFinished:${location.hash}`),
                    error => globalThis.__lmActiveNavigateSlotLog.push(`secondFinishedRejected:${error.name}`)
                  );
                }
              });
              const firstResult = navigation.navigate("#one");
              firstResult.committed.then(
                () => globalThis.__lmActiveNavigateSlotLog.push("firstCommitted"),
                error => globalThis.__lmActiveNavigateSlotLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmActiveNavigateSlotLog.push("firstFinished"),
                error => globalThis.__lmActiveNavigateSlotLog.push(`firstFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${globalThis.__lmActiveNavigateSlotLog.join("|")}`;
            })()
            "##,
        )
        .expect("active navigate event private slot probe should evaluate");

    assert_eq!(
        setup,
        "#two:true:function:#spoofed|navigate:|exposed:false|abort:AbortError:|navigate:"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("active navigate event private slot timers should drain");
    let settled = vm
        .eval("globalThis.__lmActiveNavigateSlotLog.join('|')")
        .expect("active navigate event private slot log should evaluate");

    assert_eq!(
        settled,
        "true:function:#spoofed|navigate:|exposed:false|abort:AbortError:|navigate:|secondCommitted:#two|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|handler:#two|secondFinished:#two"
    );
}

#[tokio::test]
async fn pending_precommit_navigation_slot_is_not_script_writable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmPendingPrecommitSlotLog = [];
              Object.defineProperties(Object.prototype, {
                __lmPrecommitCommitActive: { configurable: true, value: false },
                __lmPrecommitCommitCurrentHref: { configurable: true, value: "https://spoofed.invalid/current" },
                __lmPrecommitCommitEffectiveHref: { configurable: true, value: "https://spoofed.invalid/effective" },
                __lmPrecommitCommitKind: { configurable: true, value: "reload" },
                __lmPrecommitCommitSignal: {
                  configurable: true,
                  value: { addEventListener() {
                    globalThis.__lmPendingPrecommitSlotLog.push("prototype-signal");
                  } }
                }
              });
              globalThis.__lmPendingPrecommitSlotLog.push(
                `precommitSpoof:${Object.prototype.__lmPrecommitCommitActive}:${Object.prototype.__lmPrecommitCommitKind}`
              );
              let first = true;
              navigation.onnavigate = event => {
                globalThis.__lmPendingPrecommitSlotLog.push(`navigate:${location.hash}`);
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmPendingPrecommitSlotLog.push(`abort:${event.signal.reason.name}:${location.hash}`);
                });
                event.intercept({
                  precommitHandler: () => {
                    globalThis.__lmPendingPrecommitSlotLog.push(`precommit:${location.hash}`);
                    if (first) {
                      return new Promise(resolve => {
                        setTimeout(() => {
                          globalThis.__lmPendingPrecommitSlotLog.push(`first-precommit-timeout:${location.hash}`);
                          resolve();
                        }, 0);
                      });
                    }
                    return undefined;
                  },
                  handler: () => {
                    globalThis.__lmPendingPrecommitSlotLog.push(`handler:${location.hash}`);
                  }
                });
                if (first) {
                  first = false;
                }
              };
              const firstResult = navigation.navigate("#one");
              globalThis.__lmPendingPrecommitSlotLog.push(
                `exposed:${"__lmNavigationPendingPrecommitCommit" in navigation}`
              );
              navigation.__lmNavigationPendingPrecommitCommit = null;
              const second = navigation.navigate("#two");
              second.committed.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push(`secondCommitted:${location.hash}`),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`secondCommittedRejected:${error.name}`)
              );
              second.finished.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push(`secondFinished:${location.hash}`),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`secondFinishedRejected:${error.name}`)
              );
              firstResult.committed.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push("firstCommitted"),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmPendingPrecommitSlotLog.push("firstFinished"),
                error => globalThis.__lmPendingPrecommitSlotLog.push(`firstFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${globalThis.__lmPendingPrecommitSlotLog.join("|")}`;
            })()
            "##,
        )
        .expect("pending precommit private slot probe should evaluate");

    assert_eq!(
        setup,
        ":precommitSpoof:false:reload|navigate:|precommit:|exposed:false|abort:AbortError:|navigate:|precommit:"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("pending precommit private slot timers should drain");
    let settled = vm
        .eval("`${location.hash}:${globalThis.__lmPendingPrecommitSlotLog.join('|')}`")
        .expect("pending precommit private slot log should evaluate");

    assert_eq!(
        settled,
        "#two:precommitSpoof:false:reload|navigate:|precommit:|exposed:false|abort:AbortError:|navigate:|precommit:|handler:#two|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|secondCommitted:#two|secondFinished:#two"
    );
}

#[tokio::test]
async fn navigation_retirement_reentry_preserves_successor_window() {
    for pending_sibling in [false, true] {
        let server = StaticHttpServer::spawn(if pending_sibling { 2 } else { 1 }).await;
        let parent_url = server.base_url().join("parent").unwrap();
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        vm.eval(&format!("globalThis.pendingSibling = {pending_sibling};"))
            .unwrap();
        vm.eval(
            r##"
        globalThis.frame = document.createElement('iframe');
        frame.src = new URL('/child', location.href).href;
        globalThis.loaded = 0;
        frame.onload = () => loaded++;
        const container = document.body || document.documentElement || document;
        globalThis.group = document.createElement('div');
        globalThis.survivor = document.createElement('iframe');
        if (globalThis.pendingSibling) {
            survivor.src = new URL('/survivor', location.href).href;
            survivor.onload = () => loaded++;
        }
        group.appendChild(frame); group.appendChild(survivor);
        container.appendChild(group);
    "##,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loaded === (pendingSibling ? 2 : 1))",
            "true",
            "retirement child load",
        )
        .await;
        vm.eval(
        r##"
        const oldChild = frame.contentWindow;
        const oldNavigation = oldChild.navigation;
        globalThis.log = [];
        globalThis.settled = [];
        const oldSurvivor = survivor.contentWindow;
        globalThis.siblingSettled = [];
        globalThis.siblingErrors = 0;
        if (globalThis.pendingSibling) {
            oldSurvivor.navigation.addEventListener('navigateerror', () => siblingErrors++);
            oldSurvivor.navigation.addEventListener('navigate', event => {
                event.intercept({handler: () => new Promise(resolve => globalThis.releaseSibling = resolve)});
            }, {once: true});
            const pending = oldSurvivor.navigation.navigate(oldSurvivor.location.href + '#waiting');
            pending.committed.then(() => siblingSettled.push('committed'), e => siblingSettled.push(e.name));
            pending.finished.then(() => siblingSettled.push('finished'), e => siblingSettled.push(e.name));
        }
        oldNavigation.addEventListener('navigateerror', event => {
            log.push('error:' + event.error.name);
            survivor.remove();
            survivor.removeAttribute('src');
            container.appendChild(survivor);
            survivor.contentWindow.history.replaceState('successor', '');
            log.push('new-window:' + (survivor.contentWindow !== oldSurvivor));
        }, {once: true});
        oldNavigation.addEventListener('navigate', event => {
            event.signal.addEventListener('abort', () => log.push('abort'));
            event.intercept({handler() {
                log.push('handler');
                group.remove();
                log.push('after-remove');
            }});
        }, {once: true});
        const result = oldNavigation.navigate(oldChild.location.href + '#pending');
        result.committed.then(() => settled.push('committed'), e => settled.push(e.name));
        result.finished.then(() => settled.push('finished'), e => settled.push(e.name));
        log.push('after-navigate');
    "##,
    )
    .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(log)").unwrap(),
            r#"["handler","abort","error:AbortError","new-window:true","after-remove","after-navigate"]"#
        );
        assert_eq!(
        vm.eval(
            "JSON.stringify([survivor.isConnected, survivor.contentWindow.history.state, settled])"
        )
        .unwrap(),
        r#"[true,"successor",["committed","AbortError"]]"#
    );
        assert!(
            vm._context_host
                .borrow()
                .pending_history_traversal_admissions
                .is_empty()
        );
        if pending_sibling {
            assert_eq!(
                vm.eval("JSON.stringify([siblingSettled, siblingErrors])")
                    .unwrap(),
                r#"[["committed","AbortError"],1]"#
            );
            vm.eval("releaseSibling();").unwrap();
            assert_eq!(
                vm.eval("survivor.contentWindow.history.state").unwrap(),
                "successor"
            );
            assert_eq!(vm.eval("String(siblingErrors)").unwrap(), "1");
        }
    }
}

#[tokio::test]
async fn navigation_intercept_handlers_preserve_cancellation_and_committed_entry() {
    for api in [
        "location-fragment",
        "navigate-fragment",
        "navigate-cross-document",
        "reload",
        "precommit",
    ] {
        for cause in ["none", "stop", "detach", "throw"] {
            let server = StaticHttpServer::spawn(1).await;
            let parent_url = server.base_url().join("parent").unwrap();
            let loader = static_http_loader([]);
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
            let script = include_str!("../../../../../tests/fixtures/navigation-interception.js");
            vm.eval(&format!(
                "{script}\n\
                 globalThis.interceptionResult = 'pending';\n\
                 navigationInterceptionProbe({api:?}, {cause:?}).then(\n\
                   value => interceptionResult = value,\n\
                   error => interceptionResult = String(error));"
            ))
            .unwrap();
            let context = format!("{api}/{cause}");
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(interceptionResult !== 'pending')",
                "true",
                &context,
            )
            .await;
            let result: serde_json::Value =
                serde_json::from_str(&vm.eval("JSON.stringify(interceptionResult)").unwrap())
                    .unwrap();
            let (log, settlement) = match cause {
                "none" => (
                    vec!["navigate", "first", "first-after", "second", "success"],
                    "fulfilled",
                ),
                "throw" => (
                    vec!["navigate", "first", "second", "abort", "error:Error"],
                    "Error",
                ),
                _ => (
                    vec![
                        "navigate",
                        "first",
                        "abort",
                        "error:AbortError",
                        "first-after",
                        "second",
                    ],
                    "AbortError",
                ),
            };
            let promises = if api.starts_with("location-") {
                serde_json::json!([])
            } else {
                serde_json::json!(["fulfilled", settlement])
            };
            assert_eq!(
                result,
                serde_json::json!({
                    "log": log,
                    "entryChanges": 1,
                    "defaultPrevented": false,
                    "aborted": cause != "none",
                    "transition": ["fulfilled", settlement],
                    "promises": promises,
                    "committedEntry": true,
                    "sameReason": true,
                    "errorKind": true,
                    "transitionCleared": true
                }),
                "{context}"
            );
            assert_eq!(server.finish_targets().await, ["/child"], "{context}");
        }
    }
}

#[tokio::test]
async fn navigation_intercept_reentrant_handlers_preserve_replacement_navigation() {
    for throws_after_replacement in [false, true] {
        let server = StaticHttpServer::spawn(1).await;
        let parent_url = server.base_url().join("parent").unwrap();
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        let script = include_str!("../../../../../tests/fixtures/navigation-interception.js");
        vm.eval(&format!(
            "{script}\n\
             globalThis.interceptionResult = 'pending';\n\
             navigationReentrantInterceptionProbe({throws_after_replacement}).then(\n\
               value => interceptionResult = value,\n\
               error => interceptionResult = String(error));"
        ))
        .unwrap();
        let context = format!("throws after replacement: {throws_after_replacement}");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(interceptionResult !== 'pending')",
            "true",
            &context,
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(interceptionResult)").unwrap()).unwrap();
        let mut log = vec!["first", "abort", "replacement-handler"];
        if !throws_after_replacement {
            log.push("first-after");
        }
        log.push("second");
        assert_eq!(
            result,
            serde_json::json!({
                "before": {
                    "activeTransition": true,
                    "promises": ["fulfilled", "AbortError"],
                    "transition": ["fulfilled", "AbortError"],
                    "committedEntry": true,
                    "sameReason": true,
                    "errors": 1,
                    "successes": 0
                },
                "log": log,
                "replacementEntry": true,
                "transitionCleared": true,
                "errors": 1,
                "successes": 1
            }),
            "{context}"
        );
        assert_eq!(server.finish_targets().await, ["/child"], "{context}");
    }
}

#[tokio::test]
async fn nested_same_document_navigation_marks_outer_event_canceled() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmNestedNavigationCancelLog = [];
              let secondResult;
              navigation.onnavigate = event => {
                globalThis.__lmNestedNavigationCancelLog.push(`navigate:${event.info}:${event.defaultPrevented}:${location.hash}`);
                if (event.info === 1) {
                  secondResult = navigation.navigate("#two", { info: 2, history: "push" });
                  globalThis.__lmNestedNavigationCancelLog.push(`outerCanceled:${event.defaultPrevented}`);
                }
              };
              const firstResult = navigation.navigate("#one", { info: 1, history: "push" });
              firstResult.committed.then(
                () => globalThis.__lmNestedNavigationCancelLog.push("firstCommitted"),
                error => globalThis.__lmNestedNavigationCancelLog.push(`firstCommittedRejected:${error.name}`)
              );
              firstResult.finished.then(
                () => globalThis.__lmNestedNavigationCancelLog.push("firstFinished"),
                error => globalThis.__lmNestedNavigationCancelLog.push(`firstFinishedRejected:${error.name}`)
              );
              secondResult.committed.then(
                entry => globalThis.__lmNestedNavigationCancelLog.push(`secondCommitted:${new URL(entry.url).hash}`),
                error => globalThis.__lmNestedNavigationCancelLog.push(`secondCommittedRejected:${error.name}`)
              );
              secondResult.finished.then(
                entry => globalThis.__lmNestedNavigationCancelLog.push(`secondFinished:${new URL(entry.url).hash}`),
                error => globalThis.__lmNestedNavigationCancelLog.push(`secondFinishedRejected:${error.name}`)
              );
              return `${location.hash}:${navigation.entries().map(entry => new URL(entry.url).hash).join(",")}:${globalThis.__lmNestedNavigationCancelLog.join("|")}`;
            })()
            "##,
        )
        .expect("nested navigation cancellation setup should evaluate");
    assert_eq!(
        setup,
        "#two:,#two:navigate:1:false:|navigate:2:false:|outerCanceled:true"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("nested navigation cancellation should drain");
    let settled = vm
        .eval("globalThis.__lmNestedNavigationCancelLog.join('|')")
        .expect("nested navigation cancellation log should evaluate");
    assert_eq!(
        settled,
        "navigate:1:false:|navigate:2:false:|outerCanceled:true|firstCommittedRejected:AbortError|firstFinishedRejected:AbortError|secondCommitted:#two|secondFinished:#two"
    );
}

#[tokio::test]
async fn interrupted_intercepted_same_document_navigation_rejects_first_finished() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmDoubleInterceptLog = [];
              const record = name => {
                globalThis.__lmDoubleInterceptLog.push(`${name}:${location.hash}:${navigation.transition ? navigation.transition.navigationType : "null"}`);
              };
              Object.defineProperties(Object.prototype, {
                __lmInterceptSettlementActive: { configurable: true, value: false },
                __lmInterceptSettlementFilename: { configurable: true, value: "spoof.js" },
                __lmInterceptSettlementSignal: {
                  configurable: true,
                  value: { addEventListener() { record("prototype-signal"); } }
                },
                __lmInterceptSettlementCommittedResolve: {
                  configurable: true,
                  value() { record("prototype-committed"); }
                },
                __lmInterceptSettlementResolve: {
                  configurable: true,
                  value() { record("prototype-resolve"); }
                },
                __lmInterceptSettlementReject: {
                  configurable: true,
                  value() { record("prototype-reject"); }
                },
                __lmInterceptSettlementValue: { configurable: true, value: "spoofed" }
              });
              record(`interceptSpoof:${Object.prototype.__lmInterceptSettlementActive}:${Object.prototype.__lmInterceptSettlementFilename}`);
              navigation.addEventListener("navigate", event => record("navigate"));
              navigation.addEventListener("currententrychange", event => record("currententrychange"));
              navigation.addEventListener("navigatesuccess", event => record("navigatesuccess"));
              navigation.addEventListener("navigateerror", event => record(`navigateerror:${event.error?.name}`));
              navigation.addEventListener("navigate", event => {
                event.signal.addEventListener("abort", () => record(`abort:${event.signal.reason?.name}`));
                event.intercept({
                  handler: () => new Promise(resolve => {
                    record("handler");
                    setTimeout(() => {
                      record("handler-timeout");
                      resolve();
                    }, 1);
                  })
                });
              });
              const first = navigation.navigate("#one");
              first.committed.then(() => record("committed1"), error => record(`committed1-rejected:${error.name}`));
              first.finished.then(() => record("finished1"), error => record(`finished1-rejected:${error.name}`));
              navigation.transition?.finished.then(() => record("transition1"), error => record(`transition1-rejected:${error.name}`));
              const second = navigation.navigate("#two");
              second.committed.then(() => record("committed2"), error => record(`committed2-rejected:${error.name}`));
              second.finished.then(() => record("finished2"), error => record(`finished2-rejected:${error.name}`));
              navigation.transition?.finished.then(() => record("transition2"), error => record(`transition2-rejected:${error.name}`));
              Promise.resolve().then(() => record("microtask"));
              return globalThis.__lmDoubleInterceptLog.join("|");
            })()
            "##,
        )
        .expect("double intercept setup should evaluate");
    assert_eq!(
        setup,
        "interceptSpoof:false:spoof.js::null|navigate::null|currententrychange:#one:push|handler:#one:push|abort:AbortError:#one:push|navigateerror:AbortError:#one:push|navigate:#one:null|currententrychange:#two:push|handler:#two:push"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("double intercept timers should drain");
    let settled = vm
        .eval("globalThis.__lmDoubleInterceptLog.join('|')")
        .expect("double intercept log should evaluate");
    assert_eq!(
        settled,
        "interceptSpoof:false:spoof.js::null|navigate::null|currententrychange:#one:push|handler:#one:push|abort:AbortError:#one:push|navigateerror:AbortError:#one:push|navigate:#one:null|currententrychange:#two:push|handler:#two:push|committed1:#two:push|finished1-rejected:AbortError:#two:push|transition1-rejected:AbortError:#two:push|committed2:#two:push|microtask:#two:push|handler-timeout:#two:push|handler-timeout:#two:push|navigatesuccess:#two:push|finished2:#two:null|transition2:#two:null"
    );
}

#[tokio::test]
async fn location_href_double_intercept_cancels_first_settlement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/start", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmLocationDoubleLog = [];
              const record = name => {
                globalThis.__lmLocationDoubleLog.push(`${name}:${location.href}:${navigation.transition ? navigation.transition.navigationType : "null"}`);
              };
              Object.defineProperties(Object.prototype, {
                __lmLocationInterceptActive: { configurable: true, value: false },
                __lmLocationInterceptFilename: { configurable: true, value: "spoof-location.js" },
                __lmLocationInterceptSignal: {
                  configurable: true,
                  value: { addEventListener() { record("prototype-signal"); } }
                }
              });
              record(`locationInterceptSpoof:${Object.prototype.__lmLocationInterceptActive}:${Object.prototype.__lmLocationInterceptFilename}`);
              navigation.addEventListener("navigate", event => {
                record("navigate");
                event.signal.addEventListener("abort", () => record(`abort:${event.signal.reason?.name}`));
                event.intercept({ handler() {
                  record("handler");
                  return new Promise(resolve => setTimeout(() => {
                    record("handler-timeout");
                    resolve();
                  }, 1));
                }});
              });
              navigation.addEventListener("currententrychange", () => record("currententrychange"));
              navigation.addEventListener("navigateerror", event => {
                record(`navigateerror:${event.error?.name}`);
                navigation.transition?.finished.then(
                  () => record("transition-finished"),
                  error => record(`transition-rejected:${error.name}`)
                );
              });
              navigation.addEventListener("navigatesuccess", () => {
                record("navigatesuccess");
                navigation.transition?.finished.then(
                  () => record("transition-finished"),
                  error => record(`transition-rejected:${error.name}`)
                );
              });
              location.href = "/common/blank.html#1";
              location.href = "/common/blank.html#2";
              Promise.resolve().then(() => record("microtask"));
              return globalThis.__lmLocationDoubleLog.join("|");
            })()
            "##,
        )
        .expect("location double setup should evaluate");
    assert_eq!(
        setup,
        "locationInterceptSpoof:false:spoof-location.js:https://example.com/start:null|navigate:https://example.com/start:null|currententrychange:https://example.com/common/blank.html#1:push|handler:https://example.com/common/blank.html#1:push|abort:AbortError:https://example.com/common/blank.html#1:push|navigateerror:AbortError:https://example.com/common/blank.html#1:push|navigate:https://example.com/common/blank.html#1:null|currententrychange:https://example.com/common/blank.html#2:replace|handler:https://example.com/common/blank.html#2:replace"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("location double timers should drain");
    let settled = vm
        .eval("globalThis.__lmLocationDoubleLog.join('|')")
        .expect("location double log should evaluate");
    assert_eq!(
        settled,
        "locationInterceptSpoof:false:spoof-location.js:https://example.com/start:null|navigate:https://example.com/start:null|currententrychange:https://example.com/common/blank.html#1:push|handler:https://example.com/common/blank.html#1:push|abort:AbortError:https://example.com/common/blank.html#1:push|navigateerror:AbortError:https://example.com/common/blank.html#1:push|navigate:https://example.com/common/blank.html#1:null|currententrychange:https://example.com/common/blank.html#2:replace|handler:https://example.com/common/blank.html#2:replace|transition-rejected:AbortError:https://example.com/common/blank.html#2:replace|microtask:https://example.com/common/blank.html#2:replace|handler-timeout:https://example.com/common/blank.html#2:replace|handler-timeout:https://example.com/common/blank.html#2:replace|navigatesuccess:https://example.com/common/blank.html#2:replace|transition-finished:https://example.com/common/blank.html#2:null"
    );
}
