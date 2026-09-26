use super::*;

#[test]
fn maplike_and_setlike_iterators_use_v8_intrinsics_after_public_tampering() {
    let mut vm = new_storage_test_vm("https://maplike-iterator-intrinsics.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const getPrototypeOf = Reflect.getPrototypeOf;
  const iteratorPrototype = getPrototypeOf(
    getPrototypeOf([][Symbol.iterator]())
  );
  const setIteratorPrototype = getPrototypeOf(new Set().values());
  const mapIteratorPrototype = getPrototypeOf(new Map().entries());
  const ArrayConstructor = Array;
  const SetConstructor = Set;
  const MapConstructor = Map;
  const originalObjectGetPrototypeOf = Object.getPrototypeOf;
  const originals = [
    [ArrayConstructor.prototype, "entries", ArrayConstructor.prototype.entries],
    [ArrayConstructor.prototype, "keys", ArrayConstructor.prototype.keys],
    [ArrayConstructor.prototype, "values", ArrayConstructor.prototype.values],
    [SetConstructor.prototype, "entries", SetConstructor.prototype.entries],
    [SetConstructor.prototype, "forEach", SetConstructor.prototype.forEach],
    [SetConstructor.prototype, "keys", SetConstructor.prototype.keys],
    [SetConstructor.prototype, "values", SetConstructor.prototype.values],
    [MapConstructor.prototype, "entries", MapConstructor.prototype.entries],
    [MapConstructor.prototype, "keys", MapConstructor.prototype.keys],
    [MapConstructor.prototype, "values", MapConstructor.prototype.values]
  ];

  class IntrinsicStateElement extends HTMLElement {
    constructor() {
      super();
      this.internals = this.attachInternals();
    }
  }
  customElements.define("intrinsic-state-element", IntrinsicStateElement);
  const stateElement = new IntrinsicStateElement();
  const styleTarget = document.createElement("div");
  styleTarget.style.display = "block";
  (document.body || document.documentElement || document).appendChild(styleTarget);
  const face = new FontFace("IntrinsicFace", "url(intrinsic.woff)");
  const fontSet = document.implementation.createHTMLDocument('').fonts;
  fontSet.add(face);

  const poisoned = function poisonedBuiltinIterator() {
    throw new Error("public collection builtin was observed");
  };
  for (const [prototype, name] of originals) {
    prototype[name] = poisoned;
  }
  Object.getPrototypeOf = function poisonedGetPrototypeOf() {
    throw new Error("public Object.getPrototypeOf was observed");
  };
  globalThis.Array = undefined;
  globalThis.Set = undefined;
  globalThis.Map = undefined;

  const failures = [];
  try {
    const states = stateElement.internals.states;
    states.add("--ready");
    const liveStateIterator = states.values();
    if (liveStateIterator.next().value !== "--ready") {
      failures.push("CustomStateSet:live-first");
    }
    states.delete("--ready");
    states.add("--late");
    const liveStateNext = liveStateIterator.next();
    if (liveStateNext.value !== "--late" || liveStateNext.done) {
      failures.push("CustomStateSet:live-add");
    }
    const stateForEach = [];
    states.forEach((value, key, owner) => {
      stateForEach.push([value, key, owner === states]);
    });
    if (
      stateForEach.length !== 1 ||
      stateForEach[0][0] !== "--late" ||
      stateForEach[0][1] !== "--late" ||
      stateForEach[0][2] !== true
    ) {
      failures.push("CustomStateSet:forEach");
    }
    const specs = [
      [
        "StylePropertyMapReadOnly",
        styleTarget.computedStyleMap().keys(),
        iteratorPrototype
      ],
      ["EventCounts", performance.eventCounts.entries(), mapIteratorPrototype],
      ["FontFaceSet", fontSet.values(), setIteratorPrototype],
      ["CustomStateSet", states.values(), setIteratorPrototype]
    ];
    if (
      StylePropertyMapReadOnly.prototype[Symbol.iterator] !==
      StylePropertyMapReadOnly.prototype.entries
    ) {
      failures.push("StylePropertyMapReadOnly:alias");
    }
    for (const [name, iterator, expectedParent] of specs) {
      const prototype = getPrototypeOf(iterator);
      const next = Object.getOwnPropertyDescriptor(prototype, "next");
      const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
      if (getPrototypeOf(prototype) !== expectedParent) {
        failures.push(`${name}:parent`);
      }
      if (iterator[Symbol.iterator]() !== iterator) {
        failures.push(`${name}:iterator`);
      }
      if (Object.hasOwn(iterator, "next") || Object.hasOwn(iterator, Symbol.iterator)) {
        failures.push(`${name}:own`);
      }
      if (Object.hasOwn(prototype, "constructor")) {
        failures.push(`${name}:constructor`);
      }
      if (
        !next ||
        next.enumerable !== true ||
        next.writable !== true ||
        next.configurable !== true ||
        next.value.length !== 0
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
      if (iterator.next().done !== false) {
        failures.push(`${name}:value`);
      }
    }
  } finally {
    for (const [prototype, name, original] of originals) {
      prototype[name] = original;
    }
    Object.getPrototypeOf = originalObjectGetPrototypeOf;
    globalThis.Array = ArrayConstructor;
    globalThis.Set = SetConstructor;
    globalThis.Map = MapConstructor;
  }
  return failures.join(",") || "ok";
})()
"#,
        )
        .expect("maplike/setlike intrinsic iterator probe should evaluate");

    assert_eq!(result, "ok");
}
#[test]
fn domcontentloaded_listener_watchdog_terminates_runaway_listener_and_recovers_isolate() {
    let _watchdog_timeout =
        crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::LifecycleEvent,
            std::time::Duration::from_millis(500),
        );
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        document.addEventListener("DOMContentLoaded", () => {
            while (true) {}
        });
        "#,
        None,
    )
    .expect("listener registration should succeed");

    let error = vm
        .dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect_err("runaway DOMContentLoaded listener should be terminated");
    assert!(error.to_string().contains("DOMContentLoaded"));

    let result = vm
        .eval("String(1 + 1)")
        .expect("isolate should recover after lifecycle event termination");
    assert_eq!(result, "2");
}
#[test]
fn removing_open_popover_clears_internal_open_state_before_reinsertion() {
    let mut vm = new_storage_test_vm("https://popover-removal-state.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const popover = document.createElement("div");
              popover.setAttribute("popover", "");
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              body.appendChild(popover);
              popover.showPopover();
              const before = popover.matches(":popover-open");
              popover.remove();
              const disconnected = popover.matches(":popover-open");
              body.appendChild(popover);
              const reinserted = popover.matches(":popover-open");
              return [before, disconnected, reinserted].join("|");
            })()
            "#,
        )
        .expect("popover removal state probe should evaluate");

    assert_eq!(result, "true|false|false");
}
#[test]
fn moving_open_popover_between_documents_clears_internal_open_state() {
    let mut vm = new_storage_test_vm("https://popover-cross-document.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const frame1 = document.createElement("iframe");
              const frame2 = document.createElement("iframe");
              body.append(frame1, frame2);
              const bodyFor = doc => doc.body || doc.appendChild(doc.createElement("body"));
              const popover = frame1.contentDocument.createElement("div");
              popover.setAttribute("popover", "");
              bodyFor(frame1.contentDocument).appendChild(popover);
              popover.showPopover();
              const before = popover.matches(":popover-open");
              bodyFor(frame2.contentDocument).appendChild(popover);
              const moved = popover.matches(":popover-open");
              popover.showPopover();
              const reshown = popover.matches(":popover-open");
              return [before, moved, reshown].join("|");
            })()
            "#,
        )
        .expect("popover cross-document move probe should evaluate");

    assert_eq!(result, "true|false|true");
}
#[test]
fn showing_autofocus_popover_uses_focus_delegation() {
    let mut vm = new_storage_test_vm("https://popover-autofocus-delegates.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const plain = document.createElement("div");
              const delegated = document.createElement("div");
              const passive = document.createElement("div");
              const shadowButton = document.createElement("button");
              const passiveShadowButton = document.createElement("button");
              plain.setAttribute("popover", "auto");
              plain.tabIndex = 0;
              plain.autofocus = true;
              delegated.setAttribute("popover", "auto");
              delegated.tabIndex = 0;
              delegated.autofocus = true;
              delegated.attachShadow({ mode: "open", delegatesFocus: true }).appendChild(shadowButton);
              passive.setAttribute("popover", "auto");
              passive.tabIndex = 0;
              passive.attachShadow({ mode: "open", delegatesFocus: true }).appendChild(passiveShadowButton);
              passiveShadowButton.autofocus = true;
              body.append(plain, delegated, passive);
              plain.showPopover();
              const plainActive = document.activeElement === plain;
              plain.hidePopover();
              delegated.showPopover();
              const delegatedDocumentActive = document.activeElement === delegated;
              const delegatedShadowActive = delegated.shadowRoot.activeElement === shadowButton;
              delegated.hidePopover();
              passive.showPopover();
              return [
                plainActive,
                delegatedDocumentActive,
                delegatedShadowActive,
                document.activeElement === body,
                passive.shadowRoot.activeElement === null
              ].join("|");
            })()
            "##,
        )
        .expect("popover autofocus delegation probe should evaluate");

    assert_eq!(result, "true|true|true|true|true");
}
#[test]
fn hiding_auto_popover_restores_prior_focus() {
    let mut vm = new_storage_test_vm("https://popover-focus-restore.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const prior = document.createElement("button");
              const auto = document.createElement("div");
              const manual = document.createElement("div");
              const autoInput = document.createElement("input");
              const manualInput = document.createElement("input");
              auto.setAttribute("popover", "auto");
              manual.setAttribute("popover", "manual");
              autoInput.autofocus = true;
              manualInput.autofocus = true;
              auto.appendChild(autoInput);
              manual.appendChild(manualInput);
              body.append(prior, auto, manual);
              prior.focus();
              auto.showPopover();
              const autoFocused = document.activeElement === autoInput;
              auto.hidePopover();
              const autoRestored = document.activeElement === prior;
              prior.focus();
              manual.showPopover();
              const manualFocused = document.activeElement === manualInput;
              manual.hidePopover();
              const manualRestored = document.activeElement === prior;
              return [autoFocused, autoRestored, manualFocused, manualRestored].join("|");
            })()
            "#,
        )
        .expect("popover focus restoration probe should evaluate");

    assert_eq!(result, "true|true|true|false");
}
#[test]
fn showing_popover_throws_when_force_close_removes_opening_popover() {
    let mut vm = new_storage_test_vm("https://popover-side-effect-removal.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const first = document.createElement("div");
              const second = document.createElement("div");
              first.popover = "auto";
              second.popover = "auto";
              body.append(first, second);
              first.showPopover();
              first.addEventListener("beforetoggle", () => second.remove());
              let thrown = "none";
              try {
                second.showPopover();
              } catch (error) {
                thrown = `${error.name}:${error instanceof DOMException}`;
              }
              return [
                thrown,
                second.matches(":popover-open"),
                document.body.contains(second)
              ].join("|");
            })()
            "#,
        )
        .expect("popover side-effect removal probe should evaluate");

    assert_eq!(result, "InvalidStateError:true|false|false");
}
#[test]
fn show_and_hide_popover_throw_on_redundant_state_changes() {
    let mut vm = new_storage_test_vm("https://popover-redundant-state.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const popover = document.createElement("div");
              popover.popover = "auto";
              body.append(popover);
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return `${error.name}:${error instanceof DOMException}`;
                }
              };
              const hiddenHide = probe(() => popover.hidePopover());
              popover.showPopover();
              const shownShow = probe(() => popover.showPopover());
              popover.hidePopover();
              return [hiddenHide, shownShow, popover.matches(":popover-open")].join("|");
            })()
            "#,
        )
        .expect("redundant popover state probe should evaluate");

    assert_eq!(
        result,
        "InvalidStateError:true|InvalidStateError:true|false"
    );
}
#[test]
fn opening_auto_popovers_preserves_flat_tree_ancestors() {
    let mut vm = new_storage_test_vm("https://popover-flat-tree-ancestors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const outer = document.createElement("div");
              const middle = document.createElement("div");
              const inner = document.createElement("div");
              const unrelated = document.createElement("div");
              for (const popover of [outer, middle, inner, unrelated]) {
                popover.popover = "auto";
              }
              outer.appendChild(middle).appendChild(inner);
              body.append(outer, unrelated);

              outer.showPopover();
              middle.showPopover();
              inner.showPopover();
              const nested = [outer, middle, inner].map(popover =>
                popover.matches(":popover-open")
              );

              unrelated.showPopover();
              const unrelatedClosesStack = [outer, middle, inner, unrelated].map(popover =>
                popover.matches(":popover-open")
              );

              unrelated.hidePopover();
              const sourceOwner = document.createElement("div");
              const sourceHost = document.createElement("span");
              const sourced = document.createElement("div");
              sourceOwner.popover = "auto";
              sourced.popover = "auto";
              const source = sourceHost
                .attachShadow({ mode: "open" })
                .appendChild(document.createElement("button"));
              sourceOwner.appendChild(sourceHost);
              body.append(sourceOwner, sourced);
              sourceOwner.showPopover();
              sourced.showPopover({ source });
              const shadowSource = [sourceOwner, sourced].map(popover =>
                popover.matches(":popover-open")
              );

              sourced.hidePopover();
              sourceOwner.hidePopover();
              const shadowHost = document.createElement("div");
              const unassigned = document.createElement("div");
              shadowHost.popover = "auto";
              unassigned.popover = "auto";
              shadowHost.appendChild(unassigned);
              shadowHost.attachShadow({ mode: "open" }).innerHTML = "<span></span>";
              body.appendChild(shadowHost);
              shadowHost.showPopover();
              unassigned.showPopover();
              const unassignedLightChild = [shadowHost, unassigned].map(popover =>
                popover.matches(":popover-open")
              );

              return JSON.stringify({
                nested,
                unrelatedClosesStack,
                shadowSource,
                unassignedLightChild
              });
            })()
            "#,
        )
        .expect("popover flat-tree ancestor probe should evaluate");

    assert_eq!(
        result,
        r#"{"nested":[true,true,true],"unrelatedClosesStack":[false,false,false,true],"shadowSource":[true,true],"unassignedLightChild":[false,true]}"#
    );
}
#[test]
fn input_button_popover_invokers_run_across_form_ownership() {
    let mut vm = new_storage_test_vm("https://input-button-popover.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.appendChild(document.createElement("html"));
              const body = html.appendChild(document.createElement("body"));
              const form = document.createElement("form");
              form.id = "owner";
              const popover = document.createElement("div");
              popover.id = "target";
              popover.popover = "auto";
              const makeInvoker = () => {
                const input = document.createElement("input");
                input.type = "button";
                input.setAttribute("popovertarget", "target");
                return input;
              };
              const ownedByAncestor = makeInvoker();
              const ownedByAttribute = makeInvoker();
              ownedByAttribute.setAttribute("form", "owner");
              const outsideForm = makeInvoker();
              const canceled = makeInvoker();
              const disabled = makeInvoker();
              disabled.disabled = true;
              const text = makeInvoker();
              text.type = "text";
              canceled.addEventListener("click", event => event.preventDefault());
              form.append(ownedByAncestor);
              body.append(form, ownedByAttribute, outsideForm, canceled, disabled, text, popover);
              const invoke = input => {
                input.click();
                const opened = popover.matches(":popover-open");
                if (opened) popover.hidePopover();
                return opened;
              };
              return [
                invoke(ownedByAncestor),
                invoke(ownedByAttribute),
                invoke(outsideForm),
                invoke(canceled),
                invoke(disabled),
                invoke(text)
              ].join("|");
            })()
            "#,
        )
        .expect("input button popover invoker probe should evaluate");

    assert_eq!(result, "true|true|true|false|false|false");
}
#[test]
fn url_hostname_setter_updates_href() {
    let mut vm = new_storage_test_vm("https://url-hostname-setter.test/");

    let result = vm
        .eval(
            r#"
const url = new URL("http://127.0.0.1:1234/path?x#y");
url.hostname = "localhost";
[
  url.href,
  url.hostname,
  typeof Object.getOwnPropertyDescriptor(URL.prototype, "hostname").set,
  Object.hasOwn(url, "hostname")
].join("|")
"#,
        )
        .expect("URL hostname setter probe should evaluate");

    assert_eq!(
        result,
        "http://localhost:1234/path?x#y|localhost|function|false"
    );
}
#[test]
fn permission_status_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://permission-status-slots.test/");

    vm.eval(
        r#"
            (() => {
              globalThis.__permissionStatusSlotProbe = "pending";
              navigator.permissions.query({ name: "geolocation" }).then((status) => {
                const internalNames = [
                  "__moliPermissionStatusName",
                  "__moliPermissionStatusState"
                ];
                const proto = Object.getPrototypeOf(status);
                const nameGetter = Object.getOwnPropertyDescriptor(proto, "name").get;
                const stateGetter = Object.getOwnPropertyDescriptor(proto, "state").get;
                const reflectedBefore = Object.getOwnPropertyNames(status)
                  .filter(name => internalNames.includes(name));
                const beforeName = status.name;
                const beforeState = status.state;
                Object.defineProperty(status, internalNames[0], {
                  value: "camera",
                  configurable: true
                });
                Object.defineProperty(status, internalNames[1], {
                  value: "granted",
                  configurable: true
                });
                const fakeReceiver = {};
                Object.defineProperty(fakeReceiver, internalNames[0], {
                  value: "microphone",
                  configurable: true
                });
                Object.defineProperty(fakeReceiver, internalNames[1], {
                  value: "denied",
                  configurable: true
                });
                const getterOutcome = getter => {
                  try {
                    return getter.call(fakeReceiver);
                  } catch (error) {
                    return `throw:${error && error.name}`;
                  }
                };
                globalThis.__permissionStatusSlotProbe = JSON.stringify({
                  reflectedBefore,
                  ownAfterSpoof: internalNames.every(name => Object.hasOwn(status, name)),
                  nameAfterSpoof: status.name,
                  stateAfterSpoof: status.state,
                  getterNameAfterSpoof: nameGetter.call(status),
                  getterStateAfterSpoof: stateGetter.call(status),
                  fakeName: getterOutcome(nameGetter),
                  fakeState: getterOutcome(stateGetter),
                  unchanged: status.name === beforeName && status.state === beforeState
                });
              });
            })()
            "#,
    )
    .expect("PermissionStatus private slot spoofing probe should evaluate");

    let result = vm
        .eval("String(globalThis.__permissionStatusSlotProbe)")
        .expect("PermissionStatus private slot promise should settle");

    assert_eq!(
        result,
        r#"{"reflectedBefore":[],"ownAfterSpoof":true,"nameAfterSpoof":"geolocation","stateAfterSpoof":"prompt","getterNameAfterSpoof":"geolocation","getterStateAfterSpoof":"prompt","fakeName":"throw:TypeError","fakeState":"throw:TypeError","unchanged":true}"#
    );
}
#[test]
fn screen_and_orientation_accessors_reject_fake_receivers() {
    let mut vm = new_storage_test_vm("https://screen-receiver-brand.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const orientation = screen.orientation;
              const fakeScreen = Object.create(screen);
              const fakeOrientation = Object.create(orientation);
              fakeScreen.__moliScreenBrand = true;
              fakeOrientation.__moliScreenOrientationBrand = true;

              const screenWidth = Object.getOwnPropertyDescriptor(
                Screen.prototype,
                "width"
              ).get;
              const screenOrientation = Object.getOwnPropertyDescriptor(
                Screen.prototype,
                "orientation"
              ).get;
              const orientationType = Object.getOwnPropertyDescriptor(
                ScreenOrientation.prototype,
                "type"
              ).get;
              const orientationOnchange = Object.getOwnPropertyDescriptor(
                ScreenOrientation.prototype,
                "onchange"
              );

              const outcome = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };

              return [
                outcome(() => screenWidth.call(fakeScreen)),
                outcome(() => screenOrientation.call(fakeScreen)),
                outcome(() => orientationType.call(fakeOrientation)),
                outcome(() => orientationOnchange.get.call(fakeOrientation)),
                outcome(() => orientationOnchange.set.call(fakeOrientation, null)),
                outcome(() => orientation.lock.call(fakeOrientation, "portrait-primary")),
                outcome(() => orientation.unlock.call(fakeOrientation)),
                outcome(() => screen.addEventListener.call(fakeScreen, "change", () => {})),
                outcome(() => screen.removeEventListener.call(fakeScreen, "change", () => {})),
                outcome(() => screen.dispatchEvent.call(fakeScreen, new Event("change"))),
                outcome(() => orientation.addEventListener.call(fakeOrientation, "change", () => {})),
                outcome(() => orientation.removeEventListener.call(fakeOrientation, "change", () => {})),
                outcome(() => orientation.dispatchEvent.call(fakeOrientation, new Event("change"))),
                Object.getOwnPropertyNames(screen)
                  .filter(name => name.startsWith("__moliScreen"))
                  .join(","),
                Object.getOwnPropertyNames(orientation)
                  .filter(name => name.startsWith("__moliScreen"))
                  .join(","),
                Object.getOwnPropertyNames(fakeScreen)
                  .filter(name => name.startsWith("__moliScreen"))
                  .join(","),
                Object.getOwnPropertyNames(fakeOrientation)
                  .filter(name => name.startsWith("__moliScreen"))
                  .join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("screen invalid receiver probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|||__moliScreenBrand|__moliScreenOrientationBrand"
    );
}
#[test]
fn utility_class_names_do_not_synthesize_app_shell_geometry() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = eval_with_layout_publications(&mut vm,
            r#"
            (function* () {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              document.body.innerHTML = "";
              const root = document.createElement("div");
              const app = document.createElement("div");
              app.className = "flex h-svh w-screen flex-col";
              const outer = document.createElement("div");
              outer.className = "relative z-0 flex min-h-0 w-full flex-1";
              const inner = document.createElement("div");
              inner.className = "relative flex min-h-0 w-full flex-1";
              const container = document.createElement("div");
              container.className = "relative flex min-w-0 flex-1 flex-col";
              const scrollRoot = document.createElement("div");
              scrollRoot.className = "group/scroll-root relative flex min-h-0 min-w-0 flex-1 flex-col";
              const main = document.createElement("main");
              const thread = document.createElement("div");
              thread.className = "flex flex-col min-h-full";
              const composer = document.createElement("div");
              composer.className = "flex flex-1 flex-col";
              const message = document.createElement("section");
              message.id = "message";
              composer.appendChild(message);
              thread.appendChild(composer);
              main.appendChild(thread);
              scrollRoot.appendChild(main);
              container.appendChild(scrollRoot);
              inner.appendChild(container);
              outer.appendChild(inner);
              app.appendChild(outer);
              root.appendChild(app);
              document.body.appendChild(root);
              yield; // Publish this scene before reading its geometry.
const outerRect = outer.getBoundingClientRect();
              const innerRect = inner.getBoundingClientRect();
              const scrollRootRect = scrollRoot.getBoundingClientRect();
              const threadRect = thread.getBoundingClientRect();
              const composerRect = composer.getBoundingClientRect();
              const messageRect = message.getBoundingClientRect();
              return JSON.stringify({
                outer: {
                  width: outerRect.width,
                  height: outerRect.height
                },
                inner: {
                  width: innerRect.width,
                  height: innerRect.height
                },
                scrollRoot: {
                  width: scrollRootRect.width,
                  height: scrollRootRect.height
                },
                thread: {
                  top: threadRect.top,
                  width: threadRect.width,
                  height: threadRect.height,
                  clientWidth: thread.clientWidth,
                  clientHeight: thread.clientHeight
                },
                composer: {
                  top: composerRect.top,
                  width: composerRect.width,
                  height: composerRect.height,
                  clientWidth: composer.clientWidth,
                  clientHeight: composer.clientHeight
                },
                message: {
                  top: messageRect.top,
                  width: messageRect.width,
                  height: messageRect.height,
                  offsetTop: message.offsetTop
                }
              });
            })()
            "#,
        )
        .expect("app shell geometry probe should evaluate");

    assert_eq!(
        result,
        r#"{"outer":{"width":1904,"height":0},"inner":{"width":1904,"height":0},"scrollRoot":{"width":1904,"height":0},"thread":{"top":8,"width":1904,"height":0,"clientWidth":1904,"clientHeight":0},"composer":{"top":8,"width":1904,"height":0,"clientWidth":1904,"clientHeight":0},"message":{"top":8,"width":1904,"height":0,"offsetTop":8}}"#
    );
}
#[test]
fn app_shell_geometry_does_not_match_site_specific_names() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
            (function* () {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              document.body.innerHTML = "";
              const main = document.createElement("main");
              const namedThread = document.createElement("div");
              namedThread.id = "thread";
              const namedComposer = document.createElement("div");
              namedComposer.className = "composer-parent";
              main.appendChild(namedThread);
              main.appendChild(namedComposer);
              document.body.appendChild(main);
              yield; // Publish this scene before reading its geometry.
const threadRect = namedThread.getBoundingClientRect();
              const composerRect = namedComposer.getBoundingClientRect();
              return JSON.stringify({
                thread: {width: threadRect.width, height: threadRect.height},
                composer: {width: composerRect.width, height: composerRect.height}
              });
            })()
            "#,
    )
    .expect("site-specific geometry probe should evaluate");

    assert_eq!(
        result,
        r#"{"thread":{"width":1904,"height":0},"composer":{"width":1904,"height":0}}"#
    );
}
#[test]
fn location_href_backing_slot_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/path?x=1#frag");

    let result = vm
        .eval(
            r#"
(() => {
  const internal = "__moliWindowLocationHref";
  const attributeNames = [
    "ancestorOrigins",
    "origin",
    "href",
    "hash",
    "search",
    "pathname",
    "protocol",
    "host",
    "hostname",
    "port"
  ];
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const stringify = value => value === undefined ? "undefined" : String(value);
  const descriptorShape = name => {
    const descriptor = Object.getOwnPropertyDescriptor(location, name);
    return [
      name,
      typeof descriptor.get,
      descriptor.get && descriptor.get.name,
      descriptor.get && descriptor.get.length,
      typeof descriptor.set,
      descriptor.set && descriptor.set.name,
      descriptor.set && descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(location, name)
    ].map(stringify).join(":");
  };
  const hrefDescriptor = Object.getOwnPropertyDescriptor(location, "href");
  const namesBefore = Object.getOwnPropertyNames(location)
    .filter(name => name.startsWith("__moliWindowLocation"))
    .sort()
    .join(",");
  const windowNamesBefore = Object.getOwnPropertyNames(window)
    .filter(name => name === "__moliWindowLocation")
    .sort()
    .join(",");
  const keys = Object.keys(location)
    .filter(name => attributeNames.includes(name))
    .join(",");
  const descriptors = attributeNames.map(descriptorShape).join("|");
  const deleteHref = delete location.href;

  Location.prototype[internal] = "https://proto-spoof.test/";
  Object.defineProperty(location, internal, {
    value: "https://own-spoof.test/",
    configurable: true
  });
  Object.defineProperty(window, "__moliWindowLocation", {
    value: { href: "https://window-spoof.test/" },
    configurable: true
  });
  const fakeLocation = Object.create(Location.prototype);
  Object.defineProperty(fakeLocation, internal, {
    value: "https://fake-spoof.test/",
    configurable: true
  });

  return JSON.stringify({
    namesBefore,
    windowNamesBefore,
    keys,
    descriptors,
    deleteHref,
    ownHref: Object.prototype.hasOwnProperty.call(location, "href"),
    ownInternal: Object.prototype.hasOwnProperty.call(location, internal),
    windowOwnInternal: Object.prototype.hasOwnProperty.call(window, "__moliWindowLocation"),
    windowLocationStable: window.location === location,
    hrefAfterSpoof: location.href,
    searchAfterSpoof: location.search,
    hashAfterSpoof: location.hash,
    fakeHref: probe(() => hrefDescriptor.get.call(fakeLocation))
  });
})()
"#,
        )
        .expect("Location href backing slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r##"{"namesBefore":"","windowNamesBefore":"","keys":"ancestorOrigins,origin,href,hash,search,pathname,protocol,host,hostname,port","descriptors":"ancestorOrigins:function:get ancestorOrigins:0:undefined:undefined:undefined:true:false:true|origin:function:get origin:0:undefined:undefined:undefined:true:false:true|href:function:get href:0:function:set href:1:true:false:true|hash:function:get hash:0:function:set hash:1:true:false:true|search:function:get search:0:function:set search:1:true:false:true|pathname:function:get pathname:0:function:set pathname:1:true:false:true|protocol:function:get protocol:0:function:set protocol:1:true:false:true|host:function:get host:0:function:set host:1:true:false:true|hostname:function:get hostname:0:function:set hostname:1:true:false:true|port:function:get port:0:function:set port:1:true:false:true","deleteHref":false,"ownHref":true,"ownInternal":true,"windowOwnInternal":true,"windowLocationStable":true,"hrefAfterSpoof":"https://example.com/path?x=1#frag","searchAfterSpoof":"?x=1","hashAfterSpoof":"#frag","fakeHref":"throw:TypeError"}"##
    );
}
#[test]
fn location_accessors_throw_on_invalid_receivers_without_navigation() {
    let mut vm = new_storage_test_vm("https://example.com/path?x=1");

    let result = vm
        .eval(
            r##"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const windowLocation = Object.getOwnPropertyDescriptor(window, "location");
    const locationHref = Object.getOwnPropertyDescriptor(location, "href");

  return [
    probe(() => Object.create(window).location),
    probe(() => Reflect.get(window, "location", {})),
    probe(() => windowLocation.get.call({})),
    probe(() => { Object.create(window).location = "#foo"; }),
    probe(() => Reflect.set(window, "location", "#foo", {})),
    probe(() => windowLocation.set.call({}, "#foo")),
    probe(() => { window.location = Symbol(); }),
    probe(() => Object.create(location).href),
    probe(() => Reflect.get(location, "href", {})),
    probe(() => locationHref.get.call({})),
    probe(() => { Object.create(location).href = "#foo"; }),
    probe(() => Reflect.set(location, "href", "#foo", {})),
    probe(() => locationHref.set.call({}, "#foo")),
    probe(() => { location.href = Symbol(); })
  ].join("|");
})()
"##,
        )
        .expect("location invalid receiver probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "invalid location receivers must not queue navigation"
    );

    let setup = vm
        .eval(
            r##"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><body>child</body>";
  (document.body || document.documentElement || document).appendChild(iframe);
  globalThis.__locationAccessorFrame = iframe;
  return "queued";
})()
"##,
        )
        .expect("same-origin child setup should evaluate");
    assert_eq!(setup, "queued");
    for _ in 0..4 {
        vm.drain_pending_child_frame_work_for_test();
    }

    let result = vm
        .eval(
            r##"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const childWindow = __locationAccessorFrame.contentWindow;
  const windowLocation = Object.getOwnPropertyDescriptor(childWindow, "location");
  const locationHref = Object.getOwnPropertyDescriptor(childWindow.location, "href");
  return [
    probe(() => Object.create(childWindow).location),
    probe(() => Reflect.get(childWindow, "location", {})),
    probe(() => { Object.create(childWindow).location = "#foo"; }),
    probe(() => Reflect.set(childWindow, "location", "#foo", {})),
    probe(() => { childWindow.location = Symbol(); }),
    probe(() => windowLocation.set.call(childWindow, Symbol())),
    probe(() => { childWindow.location.href = Symbol(); }),
    probe(() => locationHref.set.call(childWindow.location, Symbol()))
  ].join("|");
})()
"##,
        )
        .expect("same-origin child location invalid receiver probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn standalone_dialog_handler_without_page_residence_uses_headless_defaults() {
    let mut vm = new_storage_test_vm_without_page_residence("https://dialog-fallback.test/");
    vm.set_javascript_dialog_handler_enabled(true);

    let result = vm
        .eval(
            r#"
            [
              String(alert("standalone alert")),
              String(confirm("standalone confirm")),
              String(prompt("standalone prompt", "default"))
            ].join("|")
            "#,
        )
        .expect("standalone dialog fallback probe should evaluate");

    assert_eq!(result, "undefined|false|null");
    assert!(
        vm.take_pending_javascript_dialogs().is_empty(),
        "a realm without an exact Page residence must not claim a protocol dialog"
    );
}
#[tokio::test]
async fn sandboxed_child_without_allow_modals_uses_dialog_defaults_without_opening_one() {
    let mut vm = new_storage_test_vm("https://sandbox-dialogs.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "sandboxed-dialog-frame";
  frame.sandbox = "allow-scripts allow-same-origin";
  frame.srcdoc = `<script>
    function openDialogs() {
      return [
        String(alert("blocked alert")),
        String(confirm("blocked confirm")),
        String(prompt("blocked prompt", "default"))
      ].join("|");
    }
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("sandboxed dialog child setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "sandboxed dialog child should commit before its parser script",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "sandboxed dialog child script should run",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(&mut vm, "sandboxed dialog child").await;

    assert_eq!(
        vm.eval("document.getElementById('sandboxed-dialog-frame').contentWindow.openDialogs()")
            .expect("sandboxed child dialog defaults should evaluate"),
        "undefined|false|null"
    );
    assert!(
        vm.take_pending_javascript_dialogs().is_empty(),
        "a child without allow-modals must not publish a JavaScript dialog"
    );
}
#[tokio::test]
async fn sandboxed_child_with_allow_modals_can_open_a_dialog() {
    let mut vm = new_storage_test_vm("https://sandbox-dialogs.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "allowed-dialog-frame";
  frame.sandbox = "allow-scripts allow-same-origin allow-modals";
  frame.srcdoc = `<script>
    function openAlert() { return alert("allowed alert"); }
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("allowed dialog child setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "allowed dialog child should commit before its parser script",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "allowed dialog child script should run",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(&mut vm, "allowed dialog child").await;

    assert_eq!(
        vm.eval(
            "String(document.getElementById('allowed-dialog-frame').contentWindow.openAlert())"
        )
        .expect("allowed child dialog should evaluate"),
        "undefined"
    );
    let dialogs = vm.take_pending_javascript_dialogs();
    assert_eq!(dialogs.len(), 1);
    assert_eq!(dialogs[0].dialog_type(), "alert");
    assert_eq!(dialogs[0].message(), "allowed alert");
}
#[test]
fn notification_options_project_metadata_fields() {
    let mut vm = new_storage_test_vm("https://example.com/notification-metadata");

    let result = vm
        .eval(
            r#"
            (() => {
              const notification = new Notification("rich", {
                body: "Body",
                icon: "/icon.png",
                image: "/image.png",
                badge: "/badge.png",
                dir: "rtl",
                lang: "fr",
                vibrate: [1, 2],
                timestamp: 1234,
                renotify: true,
                silent: false,
                requireInteraction: true,
                data: { answer: 42 }
              });
              const single = new Notification("single", { vibrate: 9 });
              const defaults = new Notification("defaults");
              const descriptorShape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  Notification.prototype,
                  name
                );
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };

              return JSON.stringify({
                rich: [
                  notification.body,
                  notification.icon,
                  notification.image,
                  notification.badge,
                  notification.dir,
                  notification.lang,
                  Array.from(notification.vibrate).join("/"),
                  notification.timestamp,
                  notification.renotify,
                  notification.silent,
                  notification.requireInteraction,
                  notification.data.answer
                ].join("|"),
                singleVibrate: Array.from(single.vibrate).join("/"),
                defaults: [
                  defaults.body,
                  defaults.icon,
                  defaults.image,
                  defaults.badge,
                  defaults.dir,
                  defaults.lang,
                  defaults.vibrate.length,
                  defaults.timestamp,
                  defaults.renotify,
                  defaults.silent === null,
                  defaults.requireInteraction
                ].join("|"),
                descriptors: [
                  "body",
                  "icon",
                  "image",
                  "badge",
                  "dir",
                  "lang",
                  "vibrate",
                  "timestamp",
                  "renotify",
                  "silent",
                  "requireInteraction"
                ].map(descriptorShape).join(",")
              });
            })()
            "#,
        )
        .expect("Notification metadata projection should evaluate");

    assert_eq!(
        result,
        r#"{"rich":"Body|/icon.png|/image.png|/badge.png|rtl|fr|1/2|1234|true|false|true|42","singleVibrate":"9","defaults":"||||auto||0|0|false|true|false","descriptors":"body:function:get body:0:undefined:true:true,icon:function:get icon:0:undefined:true:true,image:function:get image:0:undefined:true:true,badge:function:get badge:0:undefined:true:true,dir:function:get dir:0:undefined:true:true,lang:function:get lang:0:undefined:true:true,vibrate:function:get vibrate:0:undefined:true:true,timestamp:function:get timestamp:0:undefined:true:true,renotify:function:get renotify:0:undefined:true:true,silent:function:get silent:0:undefined:true:true,requireInteraction:function:get requireInteraction:0:undefined:true:true"}"#
    );
}
#[test]
fn notification_permission_tracks_permission_overrides_and_request_permission() {
    fn notifications_override(
        setting: &str,
    ) -> crate::protocol_types::PermissionOverrideRegistration {
        crate::protocol_types::PermissionOverrideRegistration {
            permission: serde_json::Value::String("notifications".to_owned()),
            setting: setting.to_owned(),
            origin: None,
            embedded_origin: None,
        }
    }

    fn request_permission_result(vm: &mut ScriptVm) -> String {
        vm.eval(
            r#"
            (() => {
              globalThis.__notificationPermissionRequest = "pending";
              Notification.requestPermission().then(
                value => { globalThis.__notificationPermissionRequest = value; },
                error => {
                  globalThis.__notificationPermissionRequest = "error:" + error.name;
                }
              );
            })()
            "#,
        )
        .expect("Notification.requestPermission probe should evaluate");
        vm.eval("String(globalThis.__notificationPermissionRequest)")
            .expect("Notification.requestPermission result should evaluate")
    }

    let mut vm = new_storage_test_vm("https://example.com/notification-permission");
    assert_eq!(
        vm.eval("Notification.permission")
            .expect("Notification.permission should evaluate"),
        "default"
    );
    assert_eq!(request_permission_result(&mut vm), "default");

    vm.set_permission_overrides(&[notifications_override("granted")]);
    assert_eq!(
        vm.eval("Notification.permission")
            .expect("Notification.permission override should evaluate"),
        "granted"
    );
    assert_eq!(request_permission_result(&mut vm), "granted");

    vm.set_permission_overrides(&[notifications_override("denied")]);
    assert_eq!(
        vm.eval("Notification.permission")
            .expect("Notification.permission denied override should evaluate"),
        "denied"
    );
    assert_eq!(request_permission_result(&mut vm), "denied");

    let mut insecure_vm = new_storage_test_vm("http://insecure-notification.test/");
    insecure_vm.set_permission_overrides(&[notifications_override("granted")]);
    assert_eq!(
        insecure_vm
            .eval("Notification.permission")
            .expect("insecure Notification.permission should evaluate"),
        "denied"
    );
    assert_eq!(request_permission_result(&mut insecure_vm), "denied");
}
