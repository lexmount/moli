use super::*;

#[tokio::test]
async fn popup_classic_script_custom_element_microtasks_wait_for_outer_javascript() {
    assert_popup_custom_element_microtask_order(false).await;
}

#[tokio::test]
async fn popup_javascript_url_custom_element_microtasks_wait_for_outer_javascript() {
    assert_popup_custom_element_microtask_order(true).await;
}

#[test]
fn custom_element_document_write_microtasks_wait_for_outer_javascript() {
    let mut vm = new_storage_test_vm("https://custom-element-microtasks.test/");

    let result = vm
        .eval(
            r#"
            globalThis.constructorMicrotaskLog = [];
            class WrittenElement extends HTMLElement {
              constructor() {
                super();
                constructorMicrotaskLog.push("constructor");
                Promise.resolve().then(() => {
                  constructorMicrotaskLog.push("constructor-microtask");
                  this.setAttribute("data-constructed", "yes");
                });
              }
            }
            customElements.define("microtask-written-element", WrittenElement);
            Promise.resolve().then(() => constructorMicrotaskLog.push("earlier-microtask"));
            document.write("<microtask-written-element></microtask-written-element>");
            constructorMicrotaskLog.push("after-write");
            const written = document.querySelector("microtask-written-element");
            JSON.stringify({
              log: constructorMicrotaskLog,
              custom: written instanceof WrittenElement,
              hasAttribute: written.hasAttribute("data-constructed")
            });
            "#,
        )
        .expect("document.write custom element should preserve the outer script boundary");

    assert_eq!(
        result,
        r#"{"log":["constructor","after-write"],"custom":true,"hasAttribute":false}"#
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({log: constructorMicrotaskLog, value: written.getAttribute('data-constructed')})"
        )
        .expect("constructor microtasks should run after the outer script"),
        r#"{"log":["constructor","after-write","earlier-microtask","constructor-microtask"],"value":"yes"}"#
    );
}

#[test]
fn custom_element_create_element_microtasks_wait_for_outer_javascript() {
    let mut vm = new_storage_test_vm("https://custom-element-microtasks.test/");

    let result = vm
        .eval(
            r#"
            globalThis.constructorMicrotaskLog = [];
            class CreatedElement extends HTMLElement {
              constructor() {
                super();
                constructorMicrotaskLog.push("constructor");
                queueMicrotask(() => {
                  constructorMicrotaskLog.push("microtask");
                  this.setAttribute("data-constructed", "yes");
                });
              }
            }
            customElements.define("microtask-created-element", CreatedElement);
            const created = document.createElement("microtask-created-element");
            constructorMicrotaskLog.push("after-create");
            JSON.stringify({
              log: constructorMicrotaskLog,
              custom: created instanceof CreatedElement,
              hasAttribute: created.hasAttribute("data-constructed")
            });
            "#,
        )
        .expect("createElement should preserve the outer script boundary");

    assert_eq!(
        result,
        r#"{"log":["constructor","after-create"],"custom":true,"hasAttribute":false}"#
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({log: constructorMicrotaskLog, value: created.getAttribute('data-constructed')})"
        )
        .expect("constructor microtasks should run after createElement's caller"),
        r#"{"log":["constructor","after-create","microtask"],"value":"yes"}"#
    );
}

#[test]
fn custom_elements_registry_shape_matches_chromium_probe() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const registry = window.customElements;
              const summarizeMethod = (name) => {
                const value = registry[name];
                const desc = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(registry), name);
                return {
                  string: String(value),
                  length: value.length,
                  desc: {
                    enumerable: !!desc?.enumerable,
                    configurable: !!desc?.configurable,
                    writable: !!desc?.writable,
                    hasGetter: typeof desc?.get === "function",
                    hasSetter: typeof desc?.set === "function",
                    valueType: typeof desc?.value
                  }
                };
              };
              return JSON.stringify({
                typeof: typeof registry,
                tag: Object.prototype.toString.call(registry),
                ctor: registry.constructor && registry.constructor.name,
                ownKeys: Object.keys(registry),
                ownNames: Object.getOwnPropertyNames(registry),
                protoKeys: Object.keys(Object.getPrototypeOf(registry)),
                protoCtor: Object.getPrototypeOf(registry)?.constructor?.name,
                define: summarizeMethod("define"),
                get: summarizeMethod("get"),
                getName: summarizeMethod("getName"),
                initialize: summarizeMethod("initialize"),
                upgrade: summarizeMethod("upgrade"),
                whenDefined: summarizeMethod("whenDefined")
              });
            })()
            "##,
        )
        .expect("customElements shape probe should evaluate");

    assert_eq!(
        result,
        r#"{"typeof":"object","tag":"[object CustomElementRegistry]","ctor":"CustomElementRegistry","ownKeys":[],"ownNames":[],"protoKeys":["define","get","getName","whenDefined","initialize","upgrade"],"protoCtor":"CustomElementRegistry","define":{"string":"function define() { [native code] }","length":2,"desc":{"enumerable":true,"configurable":true,"writable":true,"hasGetter":false,"hasSetter":false,"valueType":"function"}},"get":{"string":"function get() { [native code] }","length":1,"desc":{"enumerable":true,"configurable":true,"writable":true,"hasGetter":false,"hasSetter":false,"valueType":"function"}},"getName":{"string":"function getName() { [native code] }","length":1,"desc":{"enumerable":true,"configurable":true,"writable":true,"hasGetter":false,"hasSetter":false,"valueType":"function"}},"initialize":{"string":"function initialize() { [native code] }","length":1,"desc":{"enumerable":true,"configurable":true,"writable":true,"hasGetter":false,"hasSetter":false,"valueType":"function"}},"upgrade":{"string":"function upgrade() { [native code] }","length":1,"desc":{"enumerable":true,"configurable":true,"writable":true,"hasGetter":false,"hasSetter":false,"valueType":"function"}},"whenDefined":{"string":"function whenDefined() { [native code] }","length":1,"desc":{"enumerable":true,"configurable":true,"writable":true,"hasGetter":false,"hasSetter":false,"valueType":"function"}}}"#
    );
}

#[test]
fn custom_elements_define_options_extends_uses_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const probe = (callback) => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };

              let toStringCalls = 0;
              class BuiltInButton extends HTMLButtonElement {}
              customElements.define("wpt-webidl-extends-button", BuiltInButton, {
                extends: {
                  toString() {
                    toStringCalls++;
                    return "button";
                  }
                }
              });

              class UndefinedExtendsElement extends HTMLElement {}
              const undefinedExtends = probe(() => {
                customElements.define("wpt-webidl-extends-undefined", UndefinedExtendsElement, {
                  extends: undefined
                });
              });

              class SymbolExtendsElement extends HTMLElement {}
              const symbolExtends = probe(() => {
                customElements.define("wpt-webidl-extends-symbol", SymbolExtendsElement, {
                  extends: Symbol("button")
                });
              });

              class ThrowingExtendsElement extends HTMLElement {}
              const throwingExtends = probe(() => {
                customElements.define("wpt-webidl-extends-throwing", ThrowingExtendsElement, {
                  get extends() {
                    throw new RangeError("extends");
                  }
                });
              });

              const button = document.createElement("button", { is: "wpt-webidl-extends-button" });
              const auto = document.createElement("wpt-webidl-extends-undefined");
              return [
                toStringCalls,
                button instanceof BuiltInButton,
                undefinedExtends,
                auto instanceof UndefinedExtendsElement,
                symbolExtends,
                throwingExtends,
                customElements.get("wpt-webidl-extends-symbol") === undefined,
                customElements.get("wpt-webidl-extends-throwing") === undefined
              ].join("|");
            })()
            "##,
        )
        .expect("customElements.define options.extends WebIDL probe should evaluate");

    assert_eq!(result, "1|true|ok|true|TypeError|RangeError|true|true");
}

#[test]
fn custom_elements_define_rejects_unknown_builtin_extends_targets() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const probe = (name) => {
                try {
                  customElements.define(`wpt-unknown-extends-${name}`, class {}, { extends: name });
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };

              class BuiltInButton extends HTMLButtonElement {}
              customElements.define("wpt-known-extends-button", BuiltInButton, { extends: "button" });

              return [
                probe("bgsound"),
                probe("blink"),
                probe("isindex"),
                probe("multicol"),
                probe("nextid"),
                probe("spacer"),
                document.createElement("button", { is: "wpt-known-extends-button" }) instanceof BuiltInButton
              ].join("|");
            })()
            "##,
        )
        .expect("customElements.define built-in extends target validation should evaluate");

    assert_eq!(
        result,
        "NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|true"
    );
}

#[test]
fn custom_elements_define_validates_definition_inputs_in_spec_order() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const out = [];
              const probe = (callback) => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };

              out.push(probe(() => customElements.define("wpt-arrow-constructor", () => {})));
              out.push(probe(() => customElements.define("badmethod", ({ m() {} }).m)));
              out.push(probe(() => customElements.define("badname", () => {})));

              class BadCallbackElement extends HTMLElement {}
              BadCallbackElement.prototype.connectedCallback = null;
              out.push(probe(() => customElements.define("wpt-bad-callback", BadCallbackElement)));

              const thrown = { name: "expected-object" };
              class ThrowingCallbackElement extends HTMLElement {}
              Object.defineProperty(ThrowingCallbackElement.prototype, "disconnectedCallback", {
                get() {
                  throw thrown;
                }
              });
              out.push(probe(() => customElements.define("wpt-throwing-callback", ThrowingCallbackElement)));

              class NoAttributeCallbackElement extends HTMLElement {}
              NoAttributeCallbackElement.observedAttributes = 1;
              out.push(probe(() => customElements.define("wpt-no-attribute-callback", NoAttributeCallbackElement)));

              class BadObservedAttributesElement extends HTMLElement {
                attributeChangedCallback() {}
              }
              BadObservedAttributesElement.observedAttributes = 1;
              out.push(probe(() => customElements.define("wpt-bad-observed", BadObservedAttributesElement)));

              const constructorCalls = [];
              const ProxiedElement = new Proxy(class extends HTMLElement {}, {
                get(target, name) {
                  constructorCalls.push(String(name));
                  return target[name];
                }
              });
              customElements.define("wpt-proxy-definition", ProxiedElement);
              out.push(constructorCalls.join(","));

              function CallbackOrderElement() {}
              const callbackCalls = [];
              CallbackOrderElement.prototype = new Proxy(CallbackOrderElement.prototype, {
                get(target, name) {
                  callbackCalls.push(String(name));
                  return target[name];
                }
              });
              customElements.define("wpt-callback-order", CallbackOrderElement);
              out.push(String(callbackCalls.includes("connectedMoveCallback") === ("moveBefore" in Element.prototype)));

              customElements.define("wpt-duplicate-name", class extends HTMLElement {});
              const duplicateCalls = [];
              const DuplicateElement = new Proxy(class extends HTMLElement {}, {
                get(target, name) {
                  duplicateCalls.push(String(name));
                  return target[name];
                }
              });
              out.push(probe(() => customElements.define("wpt-duplicate-name", DuplicateElement)));
              out.push(String(duplicateCalls.length));

              const reentrantCalls = [];
              const ReentrantElement = new Proxy(class extends HTMLElement {}, {
                get(target, name) {
                  reentrantCalls.push(String(name));
                  if (name === "prototype") {
                    out.push(probe(() => {
                      customElements.define("wpt-inner-running-definition", class extends HTMLElement {});
                    }));
                  }
                  return target[name];
                }
              });
              out.push(probe(() => customElements.define("wpt-outer-running-definition", ReentrantElement)));
              out.push(reentrantCalls.join(","));

              return out.join("|");
            })()
            "##,
        )
        .expect("customElements.define validation probe should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|TypeError|TypeError|expected-object|ok|TypeError|prototype,disabledFeatures,formAssociated|true|NotSupportedError|0|NotSupportedError|ok|prototype,disabledFeatures,formAssociated"
    );
}

#[test]
fn custom_elements_define_reentrant_proxy_stops_before_constructor_properties() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              const InnerElement = new Proxy(class extends HTMLElement {}, {
                get(target, property, receiver) {
                  calls.push(`inner:${String(property)}`);
                  return Reflect.get(target, property, receiver);
                }
              });
              const OuterElement = new Proxy(class extends HTMLElement {}, {
                get(target, property, receiver) {
                  calls.push(`outer:${String(property)}`);
                  if (property === "prototype") {
                    customElements.define("wpt-reentrant-inner", InnerElement);
                  }
                  return Reflect.get(target, property, receiver);
                }
              });

              let errorName = "no-throw";
              try {
                customElements.define("wpt-reentrant-outer", OuterElement);
              } catch (error) {
                errorName = error.name;
              }

              class DefinitionAfterFailure extends HTMLElement {}
              customElements.define("wpt-after-reentrant-failure", DefinitionAfterFailure);

              return JSON.stringify({
                errorName,
                calls,
                innerRegistered: customElements.get("wpt-reentrant-inner") !== undefined,
                outerRegistered: customElements.get("wpt-reentrant-outer") !== undefined,
                flagCleared:
                  customElements.get("wpt-after-reentrant-failure") === DefinitionAfterFailure
              });
            })()
            "#,
        )
        .expect("reentrant customElements.define proxy probe should evaluate");

    assert_eq!(
        result,
        r#"{"errorName":"NotSupportedError","calls":["outer:prototype"],"innerRegistered":false,"outerRegistered":false,"flagCleared":true}"#
    );
}

#[test]
fn parent_node_replace_children_flushes_custom_element_reactions_after_operation() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const parent = document.createElement("div");
              parent.id = "ce-parent";
              body.appendChild(parent);

              const log = [];
              const children = () =>
                Array.from(parent.childNodes).map((node) => node.id || node.nodeName).join(",");

              class ScopedTreeElement extends HTMLElement {
                connectedCallback() {
                  log.push([
                    "connected",
                    this.id,
                    `new=${!!document.getElementById("ce-new")}`,
                    `old=${!!document.getElementById("ce-old")}`,
                    `children=${children()}`
                  ].join(":"));
                }
                disconnectedCallback() {
                  log.push([
                    "disconnected",
                    this.id,
                    `new=${!!document.getElementById("ce-new")}`,
                    `old=${!!document.getElementById("ce-old")}`,
                    `children=${children()}`
                  ].join(":"));
                }
              }
              customElements.define("wpt-tree-scope", ScopedTreeElement);

              const oldChild = document.createElement("wpt-tree-scope");
              oldChild.id = "ce-old";
              parent.appendChild(oldChild);
              log.length = 0;

              const newChild = document.createElement("wpt-tree-scope");
              newChild.id = "ce-new";
              parent.replaceChildren(newChild);

              return JSON.stringify({
                log,
                children: Array.from(parent.childNodes).map((node) => node.id || node.nodeName)
              });
            })()
            "#,
        )
        .expect("replaceChildren custom element reaction scope probe should evaluate");

    assert_eq!(
        result,
        r#"{"log":["disconnected:ce-old:new=true:old=false:children=ce-new","connected:ce-new:new=true:old=false:children=ce-new"],"children":["ce-new"]}"#
    );
}

#[test]
fn live_text_content_nested_reactions_follow_pending_disconnected_order() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const log = [];
              let second;

              class TextContentReactionElement extends HTMLElement {
                static get observedAttributes() { return ["data-state"]; }
                disconnectedCallback() {
                  log.push([
                    "disconnected",
                    this.id,
                    `children=${container.childNodes.length}`,
                    `text=${container.firstChild && container.firstChild.nodeValue}`
                  ].join(":"));
                  if (this.id === "first") {
                    second.setAttribute("data-state", "nested");
                  }
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push([
                    "attribute",
                    this.id,
                    oldValue,
                    newValue
                  ].join(":"));
                }
              }
              customElements.define("wpt-live-textcontent-reaction", TextContentReactionElement);

              const container = document.createElement("div");
              const first = document.createElement("wpt-live-textcontent-reaction");
              first.id = "first";
              second = document.createElement("wpt-live-textcontent-reaction");
              second.id = "second";
              container.append(first, second);
              body.append(container);
              log.length = 0;

              container.textContent = "fresh";

              return log.join("|");
            })()
            "#,
        )
        .expect("live textContent reaction ordering probe should evaluate");

    assert_eq!(
        result,
        "disconnected:first:children=1:text=fresh|disconnected:second:children=1:text=fresh|attribute:second::nested"
    );
}

#[test]
fn text_like_element_replacements_keep_nested_reactions_behind_pending_disconnects() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const states = new WeakMap();
              const results = {};

              const summarize = (node) => {
                const first = node && node.firstChild;
                return [
                  node && node.childNodes.length,
                  first && first.nodeValue
                ].join(":");
              };

              class TextLikeReplacementElement extends HTMLElement {
                static get observedAttributes() { return ["data-state"]; }
                disconnectedCallback() {
                  const state = states.get(this);
                  state.log.push(`disconnected:${this.id}:${summarize(state.snapshot())}`);
                  if (this === state.first) {
                    state.second.setAttribute("data-state", "nested");
                  }
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  const state = states.get(this);
                  state.log.push(`attribute:${this.id}:${oldValue}:${newValue}`);
                }
              }
              customElements.define(
                "wpt-text-like-replacement-element",
                TextLikeReplacementElement
              );

              const run = (label, setup, mutate) => {
                const { target, snapshot } = setup();
                const log = [];
                const state = { log, snapshot };
                const first = document.createElement("wpt-text-like-replacement-element");
                first.id = `${label}-first`;
                const second = document.createElement("wpt-text-like-replacement-element");
                second.id = `${label}-second`;
                state.first = first;
                state.second = second;
                states.set(first, state);
                states.set(second, state);
                target.append(first, second);
                log.length = 0;

                mutate(target);
                results[label] = log.join("|");
              };

              const expected = (label) =>
                `disconnected:${label}-first:1:fresh|` +
                `disconnected:${label}-second:1:fresh|` +
                `attribute:${label}-second:null:nested`;

              run("innerText", () => {
                const target = document.createElement("div");
                body.append(target);
                return { target, snapshot: () => target };
              }, (target) => { target.innerText = "fresh"; });

              run("anchorText", () => {
                const target = document.createElement("a");
                body.append(target);
                return { target, snapshot: () => target };
              }, (target) => { target.text = "fresh"; });

              run("scriptText", () => {
                const target = document.createElement("script");
                body.append(target);
                return { target, snapshot: () => target };
              }, (target) => { target.text = "fresh"; });

              run("optionText", () => {
                const select = document.createElement("select");
                const target = document.createElement("option");
                select.append(target);
                body.append(select);
                return { target, snapshot: () => target };
              }, (target) => { target.text = "fresh"; });

              run("textareaDefaultValue", () => {
                const target = document.createElement("textarea");
                body.append(target);
                return { target, snapshot: () => target };
              }, (target) => { target.defaultValue = "fresh"; });

              run("outputValue", () => {
                const target = document.createElement("output");
                body.append(target);
                return { target, snapshot: () => target };
              }, (target) => { target.value = "fresh"; });

              run("outerText", () => {
                const parent = document.createElement("div");
                const target = document.createElement("span");
                parent.append(target);
                body.append(parent);
                return { target, snapshot: () => parent };
              }, (target) => { target.outerText = "fresh"; });

              const keys = [
                "innerText",
                "anchorText",
                "scriptText",
                "optionText",
                "textareaDefaultValue",
                "outputValue",
                "outerText"
              ];
              return keys.map((key) => {
                const wanted = expected(key);
                return results[key] === wanted ? `${key}:ok` : `${key}:${results[key]} != ${wanted}`;
              }).join("|");
            })()
            "#,
        )
        .expect("text-like element replacement reaction ordering probe should evaluate");

    assert_eq!(
        result,
        "innerText:ok|anchorText:ok|scriptText:ok|optionText:ok|textareaDefaultValue:ok|outputValue:ok|outerText:ok"
    );
}

#[test]
fn host_tree_mutation_surfaces_keep_reactions_in_single_api_scope() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const states = new WeakMap();
              const dynamicStates = new Map();
              const failures = [];
              const tag = "wpt-host-tree-reaction";

              const stateFor = (element) =>
                states.get(element) || dynamicStates.get(element.getAttribute("data-case"));

              const record = (element, type) => {
                const state = stateFor(element);
                if (!state) {
                  return;
                }
                state.log.push(`${type}:${element.id}:${state.snapshot()}`);
                if (element.getAttribute("data-role") === "first") {
                  state.second().setAttribute("data-state", "nested");
                }
              };

              class HostTreeReactionElement extends HTMLElement {
                static get observedAttributes() { return ["data-state"]; }
                connectedCallback() { record(this, "connected"); }
                disconnectedCallback() { record(this, "disconnected"); }
                attributeChangedCallback(name, oldValue, newValue) {
                  const state = stateFor(this);
                  if (!state) {
                    return;
                  }
                  state.log.push(
                    `attribute:${this.id}:${oldValue}:${newValue}:${state.snapshot()}`
                  );
                }
              }
              customElements.define(tag, HostTreeReactionElement);

              const makePair = (parent, label, snapshot, log = []) => {
                const state = { log, snapshot, second: null };
                const first = document.createElement(tag);
                first.id = `${label}-first`;
                first.setAttribute("data-role", "first");
                const second = document.createElement(tag);
                second.id = `${label}-second`;
                second.setAttribute("data-role", "second");
                state.second = () => second;
                states.set(first, state);
                states.set(second, state);
                parent.append(first, second);
                return state;
              };

              const expect = (label, actual, expected) => {
                if (actual !== expected) {
                  failures.push(`${label}\nactual=${actual}\nexpected=${expected}`);
                }
              };
              const resetState = (state) => {
                state.second().removeAttribute("data-state");
                state.log.length = 0;
              };
              const connectedExpected = (label, snapshot) =>
                `connected:${label}-first:${snapshot}|` +
                `connected:${label}-second:${snapshot}|` +
                `attribute:${label}-second:null:nested:${snapshot}`;
              const disconnectedExpected = (label, snapshot) =>
                `disconnected:${label}-first:${snapshot}|` +
                `disconnected:${label}-second:${snapshot}|` +
                `attribute:${label}-second:null:nested:${snapshot}`;

              const runSelectRemoval = (label, mutate, snapshot) => {
                const select = document.createElement("select");
                const option = document.createElement("option");
                const state = makePair(option, label, snapshot(select, option));
                select.append(option);
                body.append(select);
                resetState(state);
                mutate(select, option);
                expect(label, state.log.join("|"), disconnectedExpected(label, snapshot(select, option)()));
                select.remove();
              };

              runSelectRemoval(
                "select-remove",
                (select) => { select.remove(0); },
                (select, option) => () =>
                  `options=${select.options.length}:optionParent=${option.parentNode === select}`
              );
              runSelectRemoval(
                "options-remove",
                (select) => { select.options.remove(0); },
                (select, option) => () =>
                  `options=${select.options.length}:optionParent=${option.parentNode === select}`
              );
              runSelectRemoval(
                "select-length",
                (select) => { select.length = 0; },
                (select, option) => () =>
                  `options=${select.options.length}:optionParent=${option.parentNode === select}`
              );
              runSelectRemoval(
                "options-length",
                (select) => { select.options.length = 0; },
                (select, option) => () =>
                  `options=${select.options.length}:optionParent=${option.parentNode === select}`
              );
              runSelectRemoval(
                "options-null",
                (select) => { select.options[0] = null; },
                (select, option) => () =>
                  `options=${select.options.length}:optionParent=${option.parentNode === select}`
              );

              {
                const label = "options-replace";
                const select = document.createElement("select");
                const oldOption = document.createElement("option");
                const replacement = document.createElement("option");
                const state = makePair(oldOption, label, () =>
                  `options=${select.options.length}:oldParent=${oldOption.parentNode === select}:replacementParent=${replacement.parentNode === select}`
                );
                select.append(oldOption);
                body.append(select);
                resetState(state);
                select.options[0] = replacement;
                expect(
                  label,
                  state.log.join("|"),
                  disconnectedExpected(label, "options=1:oldParent=false:replacementParent=true")
                );
                select.remove();
              }

              const runSelectAdd = (label, mutate) => {
                const select = document.createElement("select");
                const option = document.createElement("option");
                const state = makePair(option, label, () =>
                  `options=${select.options.length}:optionParent=${option.parentNode === select}`
                );
                body.append(select);
                state.log.length = 0;
                mutate(select, option);
                expect(
                  label,
                  state.log.join("|"),
                  connectedExpected(label, "options=1:optionParent=true")
                );
                select.remove();
              };
              runSelectAdd("select-add", (select, option) => { select.add(option); });
              runSelectAdd("options-add", (select, option) => { select.options.add(option); });

              {
                const label = "insert-adjacent-element";
                const host = document.createElement("div");
                const wrapper = document.createElement("span");
                const state = makePair(wrapper, label, () =>
                  `hostChildren=${host.childNodes.length}:wrapperParent=${wrapper.parentNode === host}`
                );
                body.append(host);
                state.log.length = 0;
                host.insertAdjacentElement("beforeend", wrapper);
                expect(
                  label,
                  state.log.join("|"),
                  connectedExpected(label, "hostChildren=1:wrapperParent=true")
                );
                host.remove();
              }

              {
                const label = "insert-adjacent-html";
                const host = document.createElement("div");
                const log = [];
                dynamicStates.set(label, {
                  log,
                  snapshot: () =>
                    `hostChildren=${host.childNodes.length}:wrapperParent=${document.getElementById(`${label}-wrapper`).parentNode === host}`,
                  second: () => document.getElementById(`${label}-second`)
                });
                body.append(host);
                host.insertAdjacentHTML(
                  "beforeend",
                  `<div id="${label}-wrapper"><${tag} id="${label}-first" data-case="${label}" data-role="first"></${tag}><${tag} id="${label}-second" data-case="${label}" data-role="second"></${tag}></div>`
                );
                // HTML fragment insertion upgrades the second element after the
                // first connected callback, so Chromium reports that nested
                // attribute mutation before the second connected callback.
                expect(
                  label,
                  log.join("|"),
                  `connected:${label}-first:hostChildren=1:wrapperParent=true|` +
                  `attribute:${label}-second:null:nested:hostChildren=1:wrapperParent=true|` +
                  `connected:${label}-second:hostChildren=1:wrapperParent=true`
                );
                host.remove();
              }

              {
                const table = document.createElement("table");
                const oldHead = document.createElement("thead");
                const newHead = document.createElement("thead");
                const log = [];
                const snapshot = () =>
                  `thead=${table.tHead === newHead ? "new" : table.tHead === oldHead ? "old" : "none"}:` +
                  `oldParent=${oldHead.parentNode === table}:newParent=${newHead.parentNode === table}`;
                const oldState = makePair(oldHead, "table-old", snapshot, log);
                body.append(table);
                table.tHead = oldHead;
                resetState(oldState);
                makePair(newHead, "table-new", snapshot, log);
                table.tHead = newHead;
                expect(
                  "table-slot",
                  log.join("|"),
                  "disconnected:table-old-first:thead=new:oldParent=false:newParent=true|" +
                  "disconnected:table-old-second:thead=new:oldParent=false:newParent=true|" +
                  "attribute:table-old-second:null:nested:thead=new:oldParent=false:newParent=true|" +
                  "connected:table-new-first:thead=new:oldParent=false:newParent=true|" +
                  "connected:table-new-second:thead=new:oldParent=false:newParent=true|" +
                  "attribute:table-new-second:null:nested:thead=new:oldParent=false:newParent=true"
                );
                table.remove();
              }

              return failures.length ? failures.join("\n---\n") : "ok";
            })()
            "#,
        )
        .expect("host tree mutation reaction boundary probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn set_attribute_node_flushes_reactions_after_attr_wrapper_is_attached() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const log = [];
              let attr;

              class AttrNodeReactionElement extends HTMLElement {
                static get observedAttributes() { return ["data-state"]; }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push([
                    name,
                    oldValue,
                    newValue,
                    `owner=${attr.ownerElement === this}`,
                    `cached=${this.getAttributeNode("data-state") === attr}`,
                    `value=${attr.value}`
                  ].join(":"));
                }
              }
              customElements.define("attr-node-reaction", AttrNodeReactionElement);

              const el = document.createElement("attr-node-reaction");
              body.appendChild(el);
              el.setAttribute = () => { throw new Error("public setAttribute should not be called"); };

              attr = document.createAttribute("data-state");
              attr.value = "ready";
              const old = el.setAttributeNode(attr);

              return JSON.stringify({
                log,
                old,
                owner: attr.ownerElement === el,
                cached: el.getAttributeNode("data-state") === attr,
                value: el.getAttribute("data-state")
              });
            })()
            "#,
        )
        .expect("setAttributeNode reaction scope probe should evaluate");

    assert_eq!(
        result,
        r#"{"log":["data-state::ready:owner=true:cached=true:value=ready"],"old":null,"owner":true,"cached":true,"value":"ready"}"#
    );
}

#[test]
fn named_node_map_attribute_mutations_do_not_call_shadowed_element_methods() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const log = [];

              class NamedNodeMapReactionElement extends HTMLElement {
                static get observedAttributes() { return ["data-map"]; }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push(`${name}:${oldValue}:${newValue}:has=${this.hasAttribute("data-map")}`);
                }
              }
              customElements.define("named-node-map-reaction", NamedNodeMapReactionElement);

              const el = document.createElement("named-node-map-reaction");
              body.appendChild(el);
              el.setAttribute = () => { throw new Error("public setAttribute should not be called"); };
              el.setAttributeNode = () => { throw new Error("public setAttributeNode should not be called"); };
              el.removeAttribute = () => { throw new Error("public removeAttribute should not be called"); };
              el.removeAttributeNode = () => { throw new Error("public removeAttributeNode should not be called"); };

              const attr = document.createAttribute("data-map");
              attr.value = "one";
              const old = el.attributes.setNamedItem(attr);
              const removed = el.attributes.removeNamedItem("data-map");

              return JSON.stringify({
                log,
                old,
                removed: removed === attr,
                owner: attr.ownerElement,
                value: el.getAttribute("data-map")
              });
            })()
            "#,
        )
        .expect("NamedNodeMap attribute mutation probe should evaluate");

    assert_eq!(
        result,
        r#"{"log":["data-map:null:one:has=true","data-map:one:null:has=false"],"old":null,"removed":true,"owner":null,"value":null}"#
    );
}

#[test]
fn attr_value_setter_on_live_attr_uses_reaction_scope_without_public_element_methods() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const log = [];
              let attr;

              class AttrValueReactionElement extends HTMLElement {
                static get observedAttributes() { return ["data-value"]; }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push(`${name}:${oldValue}:${newValue}:attr=${attr ? attr.value : "unset"}`);
                }
              }
              customElements.define("attr-value-reaction", AttrValueReactionElement);

              const el = document.createElement("attr-value-reaction");
              body.appendChild(el);
              el.setAttribute("data-value", "before");
              attr = el.getAttributeNode("data-value");
              log.length = 0;

              const nativeGetAttribute = el.getAttribute.bind(el);
              el.getAttribute = () => { throw new Error("public getAttribute should not be called"); };
              el.getAttributeNS = () => { throw new Error("public getAttributeNS should not be called"); };
              el.setAttribute = () => { throw new Error("public setAttribute should not be called"); };
              el.setAttributeNS = () => { throw new Error("public setAttributeNS should not be called"); };
              attr.value = "after";

              return JSON.stringify({
                log,
                value: nativeGetAttribute("data-value"),
                attrValue: attr.value
              });
            })()
            "#,
        )
        .expect("Attr.value live reaction probe should evaluate");

    assert_eq!(
        result,
        r#"{"log":["data-value:before:after:attr=after"],"value":"after","attrValue":"after"}"#
    );
}

#[test]
fn html_constructor_uses_receiver_prototype_without_second_newtarget_lookup() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              class SomeCustomElement extends HTMLElement {}
              let getCount = 0;
              const countingProxy = new Proxy(SomeCustomElement, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    ++getCount;
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              customElements.define("wpt-html-constructor-prototype-count", countingProxy);

              getCount = 0;
              const direct = new countingProxy();
              const directCount = getCount;

              getCount = 0;
              const reflected = Reflect.construct(HTMLElement, [], countingProxy);
              const reflectedCount = getCount;

              return JSON.stringify({
                directCount,
                reflectedCount,
                directInstance: direct instanceof countingProxy,
                reflectedInstance: reflected instanceof countingProxy,
                directLocalName: direct.localName,
                reflectedLocalName: reflected.localName
              });
            })()
            "#,
        )
        .expect("HTMLConstructor prototype lookup count probe should evaluate");

    assert_eq!(
        result,
        r#"{"directCount":1,"reflectedCount":1,"directInstance":true,"reflectedInstance":true,"directLocalName":"wpt-html-constructor-prototype-count","reflectedLocalName":"wpt-html-constructor-prototype-count"}"#
    );
}

#[test]
fn html_constructor_registered_newtarget_without_html_inheritance_returns_receiver() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              class SomeCustomElement {}
              let getCount = 0;
              const countingProxy = new Proxy(SomeCustomElement, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    ++getCount;
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              customElements.define("wpt-html-constructor-no-inheritance", countingProxy);

              getCount = 0;
              const instance = Reflect.construct(HTMLElement, [], countingProxy);
              return JSON.stringify({
                getCount,
                customInstance: instance instanceof countingProxy,
                baseInstance: instance instanceof SomeCustomElement,
                htmlInstance: instance instanceof HTMLElement,
                localNameIsUndefined: instance.localName === undefined,
                nodeNameIsUndefined: instance.nodeName === undefined,
                ownNames: Object.getOwnPropertyNames(instance)
              });
            })()
            "#,
        )
        .expect("HTMLConstructor no-inheritance NewTarget probe should evaluate");

    assert_eq!(
        result,
        r#"{"getCount":1,"customInstance":true,"baseInstance":true,"htmlInstance":false,"localNameIsUndefined":true,"nodeNameIsUndefined":true,"ownNames":[]}"#
    );
}

#[test]
fn html_constructor_non_object_newtarget_prototype_uses_html_interface_fallback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              let returnNonObject = false;
              function AutonomousElement() {
                return Reflect.construct(HTMLElement, [], new.target);
              }
              const AutonomousProxy = new Proxy(AutonomousElement, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    return returnNonObject ? 5 : {};
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              customElements.define(
                "wpt-html-constructor-fallback-autonomous",
                AutonomousProxy
              );

              function BuiltinElement() {
                return Reflect.construct(HTMLParagraphElement, [], new.target);
              }
              const BuiltinProxy = new Proxy(BuiltinElement, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    return returnNonObject ? undefined : {};
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              customElements.define(
                "wpt-html-constructor-fallback-builtin",
                BuiltinProxy,
                { extends: "p" }
              );

              returnNonObject = true;
              const autonomous = new AutonomousProxy();
              const builtin = new BuiltinProxy();
              return JSON.stringify({
                autonomousPrototype:
                  Object.getPrototypeOf(autonomous) === HTMLElement.prototype,
                autonomousElement: autonomous instanceof Element,
                autonomousHtml: autonomous instanceof HTMLElement,
                autonomousLocalName: autonomous.localName,
                builtinPrototype:
                  Object.getPrototypeOf(builtin) === HTMLParagraphElement.prototype,
                builtinElement: builtin instanceof Element,
                builtinHtml: builtin instanceof HTMLElement,
                builtinParagraph: builtin instanceof HTMLParagraphElement,
                builtinLocalName: builtin.localName,
                builtinIs: builtin.getAttribute("is")
              });
            })()
            "#,
        )
        .expect("HTMLConstructor fallback prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"autonomousPrototype":true,"autonomousElement":true,"autonomousHtml":true,"autonomousLocalName":"wpt-html-constructor-fallback-autonomous","builtinPrototype":true,"builtinElement":true,"builtinHtml":true,"builtinParagraph":true,"builtinLocalName":"p","builtinIs":null}"#
    );
}

#[test]
fn html_constructor_child_window_uses_newtarget_realm_fallback_prototype() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              let returnNonObject = false;

              function AutonomousElement() {
                return Reflect.construct(w.HTMLElement, [], new.target);
              }
              const AutonomousProxy = new Proxy(AutonomousElement, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    return returnNonObject ? null : {};
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              w.customElements.define(
                "wpt-html-constructor-child-fallback-autonomous",
                AutonomousProxy
              );

              function BuiltinElement() {
                return Reflect.construct(w.HTMLParagraphElement, [], new.target);
              }
              const BuiltinProxy = new w.Proxy(BuiltinElement, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    return returnNonObject ? "fallback" : {};
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              w.customElements.define(
                "wpt-html-constructor-child-fallback-builtin",
                BuiltinProxy,
                { extends: "p" }
              );

              returnNonObject = true;
              const autonomous = new AutonomousProxy();
              const builtin = new BuiltinProxy();
              return JSON.stringify({
                childHTMLElementPrototypeSplit:
                  HTMLElement.prototype !== w.HTMLElement.prototype,
                childParagraphPrototypeSplit:
                  HTMLParagraphElement.prototype !== w.HTMLParagraphElement.prototype,
                autonomousMainPrototype:
                  Object.getPrototypeOf(autonomous) === HTMLElement.prototype,
                autonomousChildPrototype:
                  Object.getPrototypeOf(autonomous) === w.HTMLElement.prototype,
                autonomousChildElement: autonomous instanceof w.HTMLElement,
                autonomousMainElement: autonomous instanceof HTMLElement,
                autonomousLocalName: autonomous.localName,
                builtinMainPrototype:
                  Object.getPrototypeOf(builtin) === HTMLParagraphElement.prototype,
                builtinChildPrototype:
                  Object.getPrototypeOf(builtin) === w.HTMLParagraphElement.prototype,
                builtinChildParagraph: builtin instanceof w.HTMLParagraphElement,
                builtinMainParagraph: builtin instanceof HTMLParagraphElement,
                builtinLocalName: builtin.localName,
                builtinIs: builtin.getAttribute("is")
              });
            })()
            "#,
        )
        .expect("child window HTMLConstructor fallback prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"childHTMLElementPrototypeSplit":true,"childParagraphPrototypeSplit":true,"autonomousMainPrototype":true,"autonomousChildPrototype":false,"autonomousChildElement":false,"autonomousMainElement":true,"autonomousLocalName":"wpt-html-constructor-child-fallback-autonomous","builtinMainPrototype":true,"builtinChildPrototype":false,"builtinChildParagraph":false,"builtinMainParagraph":true,"builtinLocalName":"p","builtinIs":null}"#
    );
}

#[test]
fn child_document_parser_upgrade_uses_existing_child_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              doc.write('<test-element id="first-element"><test-element id="second-element">');

              const element1 = doc.getElementById("first-element");
              const element2 = doc.getElementById("second-element");
              const log = [];
              const entry = (type, element, args = []) => [
                type,
                element === element1,
                element === element2,
                element.id,
                ...args
              ].join(":");

              class TestElement extends w.HTMLElement {
                constructor() {
                  super();
                  log.push(entry("constructed", this));
                }
                connectedCallback() {
                  log.push(entry("connected", this));
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push(entry("attributeChanged", this, [name, oldValue, newValue]));
                }
                static get observedAttributes() { return ["id"]; }
              }
              w.customElements.define("test-element", TestElement);

              return JSON.stringify({
                firstPrototype: Object.getPrototypeOf(element1) === TestElement.prototype,
                secondPrototype: Object.getPrototypeOf(element2) === TestElement.prototype,
                log
              });
            })()
            "#,
        )
        .expect("child document parser custom element upgrade identity probe should evaluate");

    assert_eq!(
        result,
        r#"{"firstPrototype":true,"secondPrototype":true,"log":["constructed:true:false:first-element","attributeChanged:true:false:first-element:id::first-element","connected:true:false:first-element","constructed:false:true:second-element","attributeChanged:false:true:second-element:id::second-element","connected:false:true:second-element"]}"#
    );
}

#[test]
fn child_custom_elements_upgrade_accepts_child_document_node_root() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              const element = doc.createElement("test-element");
              const log = [];

              class TestElement extends w.HTMLElement {
                constructor() {
                  super();
                  log.push(`constructed:${this === element}:${this.ownerDocument === doc}`);
                }
                connectedCallback() {
                  log.push("connected");
                }
              }
              w.customElements.define("test-element", TestElement);
              w.customElements.upgrade(element);

              return JSON.stringify({
                prototype: Object.getPrototypeOf(element) === TestElement.prototype,
                log
              });
            })()
            "#,
        )
        .expect("child customElements.upgrade Node root probe should evaluate");

    assert_eq!(
        result,
        r#"{"prototype":true,"log":["constructed:true:true"]}"#
    );
}

#[test]
fn custom_element_constructor_selected_prototypes_survive_wrapper_lookup() {
    let mut vm = new_storage_test_vm("https://ce-selected-prototypes.test/");
    let result = vm.eval(r#"
        (() => {
          window.addEventListener("error", event => event.preventDefault());
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const frame = body.appendChild(document.createElement("iframe"));
          const failures = [];
          let cases = 0;
          for (const [realm, w] of [["main", window], ["child", frame.contentWindow]]) {
            const doc = w.document;
            const root = doc.body || doc.documentElement || doc;
            for (const mode of ["create", "upgrade", "failed-upgrade"]) {
              for (const kind of ["null", "object", "inherited", "base", "same-name", "html-name", "getter"]) {
                const name = `selected-${mode}-${kind}`;
                let selected;
                let constructorReads = 0;
                let sameDuring = false;
                let prototypeDuring = false;
                const callbacks = [];
                class SelectedElement extends w.HTMLElement {
                  constructor() {
                    super();
                    Object.setPrototypeOf(this, selected);
                    const lookedUp = mode === "create" ? doc.adoptNode(this) : doc.querySelector(name);
                    sameDuring = lookedUp === this;
                    prototypeDuring = Object.getPrototypeOf(lookedUp) === selected;
                    if (mode === "failed-upgrade") throw new Error("after super");
                  }
                  connectedCallback() { callbacks.push(Object.getPrototypeOf(this) === selected); }
                }
                switch (kind) {
                  case "null": selected = null; break;
                  case "object": selected = {}; break;
                  case "inherited": selected = Object.create(SelectedElement.prototype); break;
                  case "base": selected = w.HTMLElement.prototype; break;
                  case "same-name": selected = (class SelectedElement extends w.HTMLElement {}).prototype; break;
                  case "html-name": selected = (class HTMLReplacementElement extends w.HTMLElement {}).prototype; break;
                  case "getter":
                    selected = Object.create(w.HTMLElement.prototype);
                    Object.defineProperty(selected, "constructor", {get() {
                      constructorReads++;
                      throw new Error("prototype.constructor must not be read");
                    }});
                    break;
                }
                let element;
                if (mode !== "create") {
                  element = doc.createElement(name);
                  root.appendChild(element);
                }
                w.customElements.define(name, SelectedElement);
                if (mode === "create") {
                  element = doc.createElement(name);
                  root.appendChild(element);
                }
                const lookedUp = doc.querySelector(name);
                const after = Object.getPrototypeOf(element) === selected;
                const afterLookup = Object.getPrototypeOf(lookedUp) === selected;
                const sameAfter = lookedUp === element;
                const callbacksCorrect = JSON.stringify(callbacks) ===
                    (mode === "failed-upgrade" ? "[]" : "[true]");
                if (!sameDuring || !prototypeDuring || !after || !afterLookup || !sameAfter ||
                    constructorReads !== 0 || !callbacksCorrect) {
                  failures.push({realm, mode, kind, sameDuring, prototypeDuring, after, afterLookup,
                                 sameAfter, constructorReads, callbacks});
                }
                cases++;
              }
            }
          }
          return JSON.stringify({cases, failures});
        })()
    "#).expect("constructor-selected prototypes should remain observable");
    assert_eq!(result, r#"{"cases":42,"failures":[]}"#);
}

#[test]
fn custom_element_constructor_selected_prototype_survives_prevent_extensions() {
    let mut vm = new_storage_test_vm("https://ce-non-extensible-prototype.test/");
    let result = vm.eval(r#"
        (() => {
          window.addEventListener("error", event => event.preventDefault());
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const frame = body.appendChild(document.createElement("iframe"));
          const failures = [];
          for (const [realm, w] of [["main", window], ["child", frame.contentWindow]]) {
            const doc = w.document;
            const root = doc.body || doc.documentElement || doc;
            for (const mode of ["create", "upgrade", "parser"]) {
              const name = `non-extensible-${mode}-${realm}`;
              const selected = Object.create(w.HTMLElement.prototype);
              let constructed;
              class SelectedElement extends w.HTMLElement {
                constructor() {
                  super();
                  constructed = this;
                  Object.setPrototypeOf(this, selected);
                  Object.preventExtensions(this);
                }
              }
              let element;
              if (mode === "upgrade") {
                element = doc.createElement(name);
                root.appendChild(element);
              }
              w.customElements.define(name, SelectedElement);
              if (mode === "create") {
                element = doc.createElement(name);
                root.appendChild(element);
              } else if (mode === "parser") {
                const container = root.appendChild(doc.createElement("div"));
                container.innerHTML = `<${name}></${name}>`;
                element = container.firstChild;
              }
              const lookedUp = doc.querySelector(name);
              if (element !== constructed || lookedUp !== element || Object.isExtensible(element) ||
                  Object.getPrototypeOf(element) !== selected ||
                  Object.getPrototypeOf(lookedUp) !== selected) {
                failures.push({realm, mode, same: element === constructed,
                  lookup: lookedUp === element, extensible: Object.isExtensible(element),
                  selected: Object.getPrototypeOf(element) === selected,
                  lookupSelected: Object.getPrototypeOf(lookedUp) === selected});
              }
            }
          }
          return JSON.stringify(failures);
        })()
    "#).expect("non-extensible custom elements should retain their selected prototype");
    assert_eq!(result, "[]");
}

#[test]
fn child_failed_custom_element_creation_preserves_original_and_fallback_prototypes() {
    let mut vm = new_storage_test_vm("https://ce-fallback-prototype.test/");
    let result = vm
        .eval(
            r#"
        (() => {
          window.addEventListener("error", event => event.preventDefault());
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const frame = body.appendChild(document.createElement("iframe"));
          const w = frame.contentWindow;
          let constructed;
          class InvalidElement extends w.HTMLElement {
            constructor() {
              super();
              constructed = this;
              this.setAttribute("invalid", "yes");
              Object.setPrototypeOf(this, null);
            }
          }
          w.customElements.define("invalid-created-element", InvalidElement);
          const element = w.document.createElement("invalid-created-element");
          (w.document.body || w.document.documentElement).appendChild(element);
          const lookedUp = w.document.querySelector("invalid-created-element");
          const original = w.document.adoptNode(constructed);
          return [element instanceof w.HTMLUnknownElement,
                  Object.getPrototypeOf(element) === w.HTMLUnknownElement.prototype,
                  element instanceof InvalidElement, element.hasAttribute("invalid"),
                  lookedUp === element, element !== constructed, original === constructed,
                  Object.getPrototypeOf(original) === null].join(":");
        })()
    "#,
        )
        .expect("failed construction should preserve the original and initialize its fallback");
    assert_eq!(result, "true:true:false:false:true:true:true:true");
}
