use super::*;
use crate::custom_elements::CustomElementRegistryKey;

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
fn detached_custom_elements_upgrade_uses_owner_document_registry() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement || document.appendChild(document.createElement("html"));
              const element = document.createElement("wpt-detached-upgrade");
              const log = [];
              class DetachedUpgradeElement extends HTMLElement {
                constructor() {
                  super();
                  root.appendChild(this);
                  log.push("constructor");
                }
                connectedCallback() {
                  log.push("connected");
                }
              }
              customElements.define("wpt-detached-upgrade", DetachedUpgradeElement);
              customElements.upgrade(element);
              return JSON.stringify({
                ownerIsDocument: element.ownerDocument === document,
                registryType: String(element.customElementRegistry),
                registryIsGlobal: element.customElementRegistry === customElements,
                getMatches: customElements.get("wpt-detached-upgrade") === DetachedUpgradeElement,
                definedBeforeUpgrade: element.matches(":defined"),
                localName: element.localName,
                namespaceURI: element.namespaceURI,
                upgraded: Object.getPrototypeOf(element) === DetachedUpgradeElement.prototype,
                definedAfterUpgrade: element.matches(":defined"),
                connected: element.isConnected,
                log
              });
            })()
            "#,
        )
        .expect("detached customElements.upgrade probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownerIsDocument":true,"registryType":"[object CustomElementRegistry]","registryIsGlobal":true,"getMatches":true,"definedBeforeUpgrade":true,"localName":"wpt-detached-upgrade","namespaceURI":"http://www.w3.org/1999/xhtml","upgraded":true,"definedAfterUpgrade":true,"connected":true,"log":["constructor"]}"#
    );
}

#[test]
fn child_detached_custom_elements_upgrade_skips_connected_during_constructor() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              const element = doc.createElement("wpt-child-detached-upgrade");
              const log = [];
              class ChildDetachedUpgradeElement extends w.HTMLElement {
                constructor() {
                  super();
                  doc.documentElement.appendChild(this);
                  log.push("constructor");
                }
                connectedCallback() {
                  log.push("connected");
                }
              }
              w.customElements.define("wpt-child-detached-upgrade", ChildDetachedUpgradeElement);
              w.customElements.upgrade(element);
              return JSON.stringify({
                upgraded: Object.getPrototypeOf(element) === ChildDetachedUpgradeElement.prototype,
                connected: element.isConnected,
                log
              });
            })()
            "#,
        )
        .expect("child detached customElements.upgrade probe should evaluate");

    assert_eq!(
        result,
        r#"{"upgraded":true,"connected":true,"log":["constructor"]}"#
    );
}

#[test]
fn create_element_allows_constructor_adopted_back_to_owner_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const childDocument = frame.contentDocument;
              class AdoptedBackElement extends HTMLElement {
                constructor() {
                  super();
                  childDocument.adoptNode(this);
                  document.adoptNode(this);
                }
              }
              customElements.define("wpt-adopted-back-element", AdoptedBackElement);
              let error = null;
              let instance = null;
              try {
                instance = document.createElement("wpt-adopted-back-element");
              } catch (caught) {
                error = caught && caught.name;
              }
              return JSON.stringify({
                error,
                instanceOf: instance instanceof AdoptedBackElement,
                unknown: instance instanceof HTMLUnknownElement,
                ownerBack: instance && instance.ownerDocument === document,
                localName: instance && instance.localName
              });
            })()
            "#,
        )
        .expect("adopted-back createElement probe should evaluate");

    assert_eq!(
        result,
        r#"{"error":null,"instanceOf":true,"unknown":false,"ownerBack":true,"localName":"wpt-adopted-back-element"}"#
    );
}

#[test]
fn child_document_upgrade_constructor_mutations_do_not_enqueue_attribute_reactions() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              doc.write("<test-element></test-element>");

              const element = doc.querySelector("test-element");
              const log = [];
              const steps = [];
              const errors = [];
              w.onerror = function(message, url, line, column, error) {
                errors.push(error && error.name || String(message));
                return true;
              };

              class TestElement extends w.HTMLElement {
                constructor() {
                  super();
                  steps.push(`after-super:${this === element}`);
                  this.id = "foo";
                  steps.push(`after-id:${this.getAttribute("id")}`);
                  this.setAttribute("id", "foo");
                  steps.push(`after-set:${this.getAttribute("id")}`);
                  this.removeAttribute("id");
                  steps.push(`after-remove:${this.getAttribute("id")}`);
                  steps.push(`style-type:${typeof this.style}`);
                  this.style.fontSize = "10px";
                  steps.push(`after-style:${this.getAttribute("style")}`);
                  log.push(`constructed:${this === element}:${this.getAttribute("style")}`);
                }
                connectedCallback() {
                  log.push(`connected:${this === element}`);
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push(`attribute:${this === element}:${name}:${oldValue}:${newValue}`);
                }
                static get observedAttributes() { return ["id", "style"]; }
              }
              w.customElements.define("test-element", TestElement);

              return JSON.stringify({
                prototype: Object.getPrototypeOf(element) === TestElement.prototype,
                id: element.getAttribute("id"),
                style: element.getAttribute("style"),
                steps,
                log,
                errors
              });
            })()
            "#,
        )
        .expect("child document upgrade constructor mutation probe should evaluate");

    assert_eq!(
        result,
        r#"{"prototype":true,"id":null,"style":"font-size: 10px;","steps":["after-super:true","after-id:foo","after-set:foo","after-remove:null","style-type:object","after-style:font-size: 10px;"],"log":["constructed:true:font-size: 10px;","connected:true"],"errors":[]}"#
    );
}

#[test]
fn document_reaction_entrypoints_use_dom_mutation_owner() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              const log = [];

              class TestElement extends w.HTMLElement {
                constructor() {
                  super();
                  log.push("constructed");
                }
                connectedCallback() {
                  log.push("connected");
                }
                disconnectedCallback() {
                  log.push("disconnected");
                }
              }
              w.customElements.define("doc-reaction-element", TestElement);

              const template = doc.createElement("template");
              template.innerHTML = "<doc-reaction-element></doc-reaction-element>";
              const afterTemplateInnerHTML = log.splice(0);
              doc.importNode(template.content, true);
              const afterTemplateImport = log.splice(0);

              doc.title = "";
              const title = doc.querySelector("title");
              const titleElement = doc.createElement("doc-reaction-element");
              title.appendChild(titleElement);
              titleElement.textContent = "hello";
              const titleSetup = log.splice(0);
              title.text = "world";
              const afterTitleText = log.splice(0);

              const oldBody = doc.body;
              oldBody.innerHTML = "<doc-reaction-element>hello</doc-reaction-element>";
              const bodyRemovalSetup = log.splice(0);
              doc.body = doc.createElement("body");
              const afterBodyRemoval = log.splice(0);

              const inserted = doc.createElement("doc-reaction-element");
              const replacementBody = doc.createElement("body");
              replacementBody.appendChild(inserted);
              const bodyInsertionSetup = log.splice(0);
              doc.body = replacementBody;
              const afterBodyInsertion = log.splice(0);

              return JSON.stringify({
                afterTemplateInnerHTML,
                afterTemplateImport,
                titleSetup,
                titleHTML: title.innerHTML,
                afterTitleText,
                bodyRemovalSetup,
                afterBodyRemoval,
                bodyInsertionSetup,
                afterBodyInsertion
              });
            })()
            "#,
        )
        .expect("Document custom-element reaction entrypoint probe should evaluate");

    assert_eq!(
        result,
        r#"{"afterTemplateInnerHTML":[],"afterTemplateImport":["constructed"],"titleSetup":["constructed","connected"],"titleHTML":"world","afterTitleText":["disconnected"],"bodyRemovalSetup":["constructed","connected"],"afterBodyRemoval":["disconnected"],"bodyInsertionSetup":["constructed"],"afterBodyInsertion":["connected"]}"#
    );
}

#[test]
fn child_document_replacement_entrypoints_disconnect_old_custom_elements() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const run = (method) => {
                const frame = document.createElement("iframe");
                (document.body || document.documentElement || document).appendChild(frame);
                const w = frame.contentWindow;
                const doc = frame.contentDocument;
                const log = [];
                class TestElement extends w.HTMLElement {
                  constructor() {
                    super();
                    log.push("constructed");
                  }
                  connectedCallback() {
                    log.push("connected");
                  }
                  disconnectedCallback() {
                    log.push("disconnected");
                  }
                }
                w.customElements.define("doc-replacement-element", TestElement);
                doc.body.innerHTML = "<doc-replacement-element></doc-replacement-element>";
                const setup = log.splice(0);
                if (method === "open") {
                  doc.open();
                } else if (method === "write") {
                  doc.write("");
                } else {
                  doc.writeln("");
                }
                return [method, setup.join(","), log.join(",")].join(":");
              };
              return [run("open"), run("write"), run("writeln")].join("|");
            })()
            "#,
        )
        .expect("Document replacement custom-element reaction probe should evaluate");

    assert_eq!(
        result,
        "open:constructed,connected:disconnected|write:constructed,connected:disconnected|writeln:constructed,connected:disconnected"
    );
}

#[test]
fn child_document_exec_command_delete_disconnects_custom_element() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              const log = [];
              class TestElement extends w.HTMLElement {
                constructor() {
                  super();
                  log.push("constructed");
                }
                connectedCallback() {
                  log.push("connected");
                }
                disconnectedCallback() {
                  log.push("disconnected");
                }
              }
              w.customElements.define("doc-command-element", TestElement);
              const instance = doc.createElement("doc-command-element");
              const container = doc.createElement("div");
              container.contentEditable = true;
              container.appendChild(instance);
              doc.body.appendChild(container);
              const setup = log.splice(0);

              container.focus();
              doc.getSelection().collapse(container, 1);
              const returned = doc.execCommand("delete", false, null);

              return JSON.stringify({
                setup,
                returned,
                remaining: container.childNodes.length,
                log
              });
            })()
            "#,
        )
        .expect("Document.execCommand delete custom-element reaction probe should evaluate");

    assert_eq!(
        result,
        r#"{"setup":["constructed","connected"],"returned":true,"remaining":0,"log":["disconnected"]}"#
    );
}

#[test]
fn detached_document_adopted_callback_nested_mutation_drains_existing_target_queue() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const w = frame.contentWindow;
              const doc = frame.contentDocument;
              const log = [];
              let element1;
              let element2;
              let element3;

              const label = (element) =>
                element === element1 ? "one" :
                element === element2 ? "two" :
                element === element3 ? "three" : "other";

              class TestElement extends w.HTMLElement {
                constructor() {
                  super();
                  log.push(`constructed:${label(this)}`);
                }
                adoptedCallback() {
                  log.push(`adopted:${label(this)}`);
                  if (this === element1) {
                    element3.setAttribute("id", "foo");
                  }
                }
                connectedCallback() {
                  log.push(`connected:${label(this)}`);
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  log.push(`attribute:${label(this)}:${name}:${oldValue}:${newValue}`);
                }
                static get observedAttributes() { return ["id", "class"]; }
              }
              w.customElements.define("test-element", TestElement);

              element1 = doc.createElement("test-element");
              element2 = doc.createElement("test-element");
              element3 = doc.createElement("test-element");
              log.length = 0;

              const container = doc.createElement("div");
              container.appendChild(element1);
              container.appendChild(element2);
              container.appendChild(element3);

              const anotherDocument = document.implementation.createHTMLDocument();
              anotherDocument.documentElement.appendChild(container);
              return JSON.stringify(log);
            })()
            "#,
        )
        .expect("detached document adopted nested mutation probe should evaluate");

    assert_eq!(
        result,
        r#"["adopted:one","adopted:three","connected:three","attribute:three:id:null:foo","connected:one","adopted:two","connected:two"]"#
    );
}

#[test]
fn html_constructor_invalid_newtarget_must_not_read_prototype_before_sanity_checks() {
    let mut vm = new_storage_test_vm("https://example.com/");

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

              class Unregistered extends HTMLElement {}
              let unregisteredGetCount = 0;
              const unregisteredProxy = new Proxy(Unregistered, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    ++unregisteredGetCount;
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              const unregisteredError = probe(() => {
                Reflect.construct(HTMLElement, [], unregisteredProxy);
              });

              class MismatchedBuiltin extends HTMLParagraphElement {}
              let mismatchedGetCount = 0;
              const mismatchedProxy = new Proxy(MismatchedBuiltin, {
                get(target, prop, receiver) {
                  if (prop === "prototype") {
                    ++mismatchedGetCount;
                  }
                  return Reflect.get(target, prop, receiver);
                }
              });
              customElements.define("wpt-html-constructor-mismatch", mismatchedProxy);
              mismatchedGetCount = 0;
              const mismatchedError = probe(() => {
                Reflect.construct(HTMLParagraphElement, [], mismatchedProxy);
              });

              return JSON.stringify({
                unregisteredError,
                unregisteredGetCount,
                mismatchedError,
                mismatchedGetCount
              });
            })()
            "#,
        )
        .expect("HTMLConstructor invalid NewTarget timing probe should evaluate");

    assert_eq!(
        result,
        r#"{"unregisteredError":"TypeError","unregisteredGetCount":0,"mismatchedError":"TypeError","mismatchedGetCount":0}"#
    );
}

#[test]
fn upgrade_construction_stack_reentry_and_return_validation_report_type_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                root.appendChild(document.createElement("body"));
              const errors = [];
              window.addEventListener("error", event => {
                event.preventDefault();
                errors.push(event.error && event.error.name || String(event.message));
              });

              body.innerHTML =
                "<wpt-reenter-after></wpt-reenter-after>" +
                "<wpt-reenter-before></wpt-reenter-before>" +
                "<wpt-upgrade-return-other></wpt-upgrade-return-other>";

              class ReenterAfter extends HTMLElement {
                constructor(skip) {
                  super();
                  if (!skip) {
                    new ReenterAfter(true);
                  }
                }
              }
              customElements.define("wpt-reenter-after", ReenterAfter);

              class ReenterBefore extends HTMLElement {
                constructor(skip) {
                  if (!skip) {
                    new ReenterBefore(true);
                  }
                  super();
                }
              }
              customElements.define("wpt-reenter-before", ReenterBefore);

              class ReturnOther extends HTMLElement {
                constructor() {
                  super();
                  return document.createElement("span");
                }
              }
              customElements.define("wpt-upgrade-return-other", ReturnOther);

              return JSON.stringify(errors);
            })()
            "#,
        )
        .expect("upgrade construction stack validation probe should evaluate");

    assert_eq!(result, r#"["TypeError","TypeError","TypeError"]"#);
}

#[test]
fn custom_elements_registry_isolated_for_child_window_definition() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const documentTarget = document.body || document.documentElement || document;
              documentTarget.appendChild(frame);

              class MainDuplicate extends HTMLElement {}
              class ChildDuplicate extends frame.contentWindow.HTMLElement {}
              customElements.define("wpt-cross-global-duplicate", MainDuplicate);
              frame.contentWindow.customElements.define("wpt-cross-global-duplicate", ChildDuplicate);

              const InnerCustomElement = class extends frame.contentWindow.HTMLElement {};
              const calls = [];
              const proxy = new Proxy(class extends HTMLElement {}, {
                get(target, name) {
                  calls.push(String(name));
                  if (name === "prototype") {
                    frame.contentWindow.customElements.define(
                      "wpt-child-global-during-prototype",
                      InnerCustomElement
                    );
                  }
                  return target[name];
                }
              });
              customElements.define("wpt-main-global-during-prototype", proxy);

              const childDuplicate =
                frame.contentDocument.createElement("wpt-cross-global-duplicate");
              const mainDuplicate =
                document.createElement("wpt-cross-global-duplicate");
              const childDuringPrototype =
                frame.contentDocument.createElement("wpt-child-global-during-prototype");

              return JSON.stringify({
                distinctRegistry: frame.contentWindow.customElements !== customElements,
                mainDuplicate: customElements.get("wpt-cross-global-duplicate") === MainDuplicate,
                childDuplicate: frame.contentWindow.customElements.get("wpt-cross-global-duplicate") === ChildDuplicate,
                mainDoesNotSeeChild: customElements.get("wpt-child-global-during-prototype") === undefined,
                childDuplicateInstance: childDuplicate instanceof ChildDuplicate,
                mainDuplicateInstance: mainDuplicate instanceof MainDuplicate,
                childDuringPrototypeInstance: childDuringPrototype instanceof InnerCustomElement,
                calls
              });
            })()
            "#,
        )
        .expect("child customElements registry isolation probe should evaluate");

    assert_eq!(
        result,
        r#"{"distinctRegistry":true,"mainDuplicate":true,"childDuplicate":true,"mainDoesNotSeeChild":true,"childDuplicateInstance":true,"mainDuplicateInstance":true,"childDuringPrototypeInstance":true,"calls":["prototype","disabledFeatures","formAssociated"]}"#
    );
}

#[test]
fn detached_iframe_document_keeps_content_window_custom_elements_registry() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const documentTarget = document.body || document.documentElement || document;
              documentTarget.appendChild(frame);
              const doc = frame.contentDocument;
              const registry = frame.contentWindow.customElements;
              const connected = doc.customElementRegistry === registry;
              frame.remove();
              return JSON.stringify({
                connected,
                detached: doc.customElementRegistry === registry
              });
            })()
            "#,
        )
        .expect("detached iframe customElementRegistry probe should evaluate");

    assert_eq!(result, r#"{"connected":true,"detached":true}"#);
}

#[test]
fn iframe_document_navigation_replaces_custom_elements_registry() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let setup = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const documentTarget = document.body || document.documentElement || document;
              documentTarget.appendChild(frame);
              window.__cePerDocumentFrame = frame;
              window.__cePerDocumentBefore = frame.contentWindow.customElements;
              frame.srcdoc = "<title>child</title>";
              return frame.contentWindow.customElements === window.__cePerDocumentBefore;
            })()
            "#,
        )
        .expect("iframe customElements navigation setup should evaluate");

    assert_eq!(
        setup, "true",
        "setting the first srcdoc must not eagerly replace the initial-empty LocalWindow registry"
    );
    vm.drain_pending_child_frame_work_for_test();

    let first_commit = vm
        .eval(
            r#"
            (() => {
              const frame = window.__cePerDocumentFrame;
              const childWindow = frame.contentWindow;
              const afterFirstNavigation = childWindow.customElements;
              window.__ceFirstCommittedRegistry = afterFirstNavigation;
              window.__ceRetainedChildWindow = childWindow;
              const firstNavigationReplaced =
                afterFirstNavigation !== window.__cePerDocumentBefore;
              childWindow.document.open();
              const afterOpen = childWindow.customElements === afterFirstNavigation;
              childWindow.document.close();
              frame.srcdoc = "<title>later child</title>";
              return JSON.stringify({firstNavigationReplaced, afterOpen});
            })()
            "#,
        )
        .expect("iframe customElements srcdoc result should evaluate");

    assert_eq!(
        first_commit,
        r#"{"firstNavigationReplaced":true,"afterOpen":true}"#
    );
    vm.drain_pending_child_frame_work_for_test();

    let later_navigation = vm
        .eval(
            r#"
            (() => {
              const frame = window.__cePerDocumentFrame;
              const childWindow = window.__ceRetainedChildWindow;
              const afterLaterNavigation = childWindow.customElements;
              const laterNavigationReplaced =
                afterLaterNavigation !== window.__ceFirstCommittedRegistry;
              frame.remove();
              const afterRemove = childWindow.customElements === afterLaterNavigation;
              return JSON.stringify({laterNavigationReplaced, afterRemove});
            })()
            "#,
        )
        .expect("later iframe customElements navigation result should evaluate");
    assert_eq!(
        later_navigation,
        r#"{"laterNavigationReplaced":true,"afterRemove":true}"#
    );
}

#[test]
fn child_document_open_installs_replacement_stream_before_disconnected_reactions() {
    let mut vm = new_storage_test_vm("https://child-open-ce-reaction.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const target = document.body || document.documentElement || document;
              target.appendChild(frame);
              const { contentWindow, contentDocument } = frame;
              const log = [];

              class ReentrantWrite extends contentWindow.HTMLElement {
                disconnectedCallback() {
                  log.push("disconnected");
                  contentDocument.write("<p id='reaction-write'>reaction</p>");
                  log.push(
                    contentDocument.getElementById("reaction-write").textContent
                  );
                }
              }
              contentWindow.customElements.define(
                "child-open-reentrant-write",
                ReentrantWrite
              );
              contentDocument.body.appendChild(new ReentrantWrite());

              contentDocument.open();
              contentDocument.write("<p id='caller-write'>caller</p>");
              contentDocument.close();

              return JSON.stringify({
                log,
                reaction: contentDocument.getElementById("reaction-write").textContent,
                caller: contentDocument.getElementById("caller-write").textContent,
              });
            })()
            "#,
        )
        .expect("child document.open custom-element reaction probe should evaluate");

    assert_eq!(
        result, r#"{"log":["disconnected","reaction"],"reaction":"reaction","caller":"caller"}"#,
        "Document.open [CEReactions] must flush only after the replacement owner and parser stream are coherent",
    );
}

#[test]
fn custom_elements_define_skips_closed_popup_documents() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const name = "x-popup-closed-skip";
              const popup = open("about:blank");
              const host = popup.document.createElement("div");
              const shadow = host.attachShadow({
                mode: "open",
                registry: window.customElements
              });
              const node = shadow.appendChild(popup.document.createElement(name));
              (popup.document.body || popup.document.documentElement || popup.document)
                .appendChild(host);
              popup.close();
              class ClosedPopupElement extends HTMLElement {}
              customElements.define(name, ClosedPopupElement);
              return JSON.stringify({
                closed: popup.closed,
                upgraded: node instanceof ClosedPopupElement
              });
            })()
            "#,
        )
        .expect("closed popup custom elements probe should evaluate");

    assert_eq!(result, r#"{"closed":true,"upgraded":false}"#);
}

#[test]
fn scoped_registry_define_upgrades_open_popup_documents() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const name = "x-popup-open-upgrade";
              const popup = open("about:blank");
              const host = popup.document.createElement("div");
              const shadow = host.attachShadow({
                mode: "open",
                customElementRegistry: registry
              });
              shadow.innerHTML = `<${name}></${name}>`;
              const node = shadow.querySelector(name);
              popup.document.body.appendChild(host);
              class OpenPopupElement extends HTMLElement {}
              registry.define(name, OpenPopupElement);
              const owns = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
              const deleteDefaultView = delete popup.document.defaultView;
              const deleteParentWindow = delete popup.document.parentWindow;
              return JSON.stringify({
                closed: popup.closed,
                shadowRegistry: shadow.customElementRegistry === registry,
                nodeRegistry: node.customElementRegistry === registry,
                defined: node.matches(":defined"),
                defaultView: popup.document.defaultView === popup,
                parentWindowMissing: !("parentWindow" in popup.document) && popup.document.parentWindow === undefined,
                ownDefaultView: owns(popup.document, "defaultView"),
                ownParentWindow: owns(popup.document, "parentWindow"),
                deleteDefaultView,
                deleteParentWindow,
                upgraded: node instanceof OpenPopupElement
              });
            })()
            "#,
        )
        .expect("open popup scoped custom elements probe should evaluate");

    assert_eq!(
        result,
        r#"{"closed":false,"shadowRegistry":true,"nodeRegistry":true,"defined":true,"defaultView":true,"parentWindowMissing":true,"ownDefaultView":false,"ownParentWindow":false,"deleteDefaultView":true,"deleteParentWindow":true,"upgraded":true}"#
    );
}

#[test]
fn scoped_registry_upgrade_order_follows_cross_document_shadow_tree_adoption() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const documentTarget = document.body || document.documentElement || document;
              const frame1 = documentTarget.appendChild(document.createElement("iframe"));
              const frame2 = documentTarget.appendChild(document.createElement("iframe"));
              const childBody = frame => {
                if (!frame.contentDocument.body) {
                  frame.contentDocument.body = frame.contentDocument.createElement("body");
                }
                return frame.contentDocument.body;
              };

              const host1 = document.createElement("div");
              const shadow1 = host1.attachShadow({
                mode: "open",
                customElementRegistry: registry
              });
              shadow1.innerHTML = '<x-adopt-order id="a"></x-adopt-order>';
              documentTarget.appendChild(host1);

              const host2 = document.createElement("div");
              const shadow2 = host2.attachShadow({
                mode: "open",
                customElementRegistry: registry
              });
              shadow2.innerHTML = '<x-adopt-order id="b"></x-adopt-order>';
              documentTarget.appendChild(host2);

              childBody(frame1).appendChild(host2);
              childBody(frame2).appendChild(host1);

              const upgrades = [];
              registry.define("x-adopt-order", class extends HTMLElement {
                constructor() {
                  super();
                  upgrades.push(this.id);
                }
              });
              return JSON.stringify(upgrades);
            })()
            "#,
        )
        .expect("cross-document scoped registry upgrade order probe should evaluate");

    assert_eq!(result, r#"["b","a"]"#);
}

#[test]
fn child_document_write_declarative_shadow_script_exposes_host_named_property() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = (document.body || document.documentElement || document)
                .appendChild(document.createElement("iframe"));
              frame.contentDocument.open();
              frame.contentDocument.write(`<!doctype html><html><body><div id="host"><template shadowrootmode="open" shadowrootcustomelementregistry><script>
                window.registry = new CustomElementRegistry;
                window.scriptHostType = typeof host;
                window.scriptHostHasShadow = !!(typeof host !== "undefined" && host.shadowRoot);
                try { registry.initialize(host.shadowRoot); window.initializeOk = true; }
                catch (error) { window.initializeError = error && error.message; }
                registry.define("constructor-throws-exception", class extends HTMLElement { constructor() { super(); throw TypeError; } });
                registry.define("constructor-returns-different-element", class extends HTMLElement { constructor() { super(); return document.createElement("span"); } });
              <\/script><constructor-throws-exception></constructor-throws-exception><constructor-returns-different-element></constructor-returns-different-element></template></div></body></html>`);
              frame.contentDocument.close();
              const throwsElement = frame.contentWindow.host.shadowRoot.querySelector("constructor-throws-exception");
              const returnsElement = frame.contentWindow.host.shadowRoot.querySelector("constructor-returns-different-element");
              return JSON.stringify({
                winHostType: typeof frame.contentWindow.host,
                winRegistryType: typeof frame.contentWindow.registry,
                hostRegistryType: typeof (frame.contentWindow.host &&
                  frame.contentWindow.host.shadowRoot &&
                  frame.contentWindow.host.shadowRoot.customElementRegistry),
                winRegistryAfterHostRegistry:
                  frame.contentWindow.registry ===
                  frame.contentWindow.host.shadowRoot.customElementRegistry,
                throwsRegistryIsWindowRegistry:
                  throwsElement.customElementRegistry === frame.contentWindow.registry,
                returnsRegistryIsWindowRegistry:
                  returnsElement.customElementRegistry === frame.contentWindow.registry,
                scriptHostType: frame.contentWindow.scriptHostType,
                scriptHostHasShadow: frame.contentWindow.scriptHostHasShadow,
                initializeOk: frame.contentWindow.initializeOk === true,
                initializeError: frame.contentWindow.initializeError || null,
                shadowScriptCount: frame.contentWindow.host.shadowRoot.querySelectorAll("script").length,
                shadowScriptTextIncludesRegistry:
                  frame.contentWindow.host.shadowRoot.innerHTML.includes("window.registry"),
                documentHost: !!frame.contentDocument.getElementById("host"),
                documentHostShadow: !!(frame.contentDocument.getElementById("host") &&
                  frame.contentDocument.getElementById("host").shadowRoot)
              });
            })()
            "#,
        )
        .expect("child document declarative shadow host probe should evaluate");

    assert_eq!(
        result,
        r#"{"winHostType":"object","winRegistryType":"object","hostRegistryType":"object","winRegistryAfterHostRegistry":true,"throwsRegistryIsWindowRegistry":true,"returnsRegistryIsWindowRegistry":true,"scriptHostType":"object","scriptHostHasShadow":true,"initializeOk":true,"initializeError":null,"shadowScriptCount":1,"shadowScriptTextIncludesRegistry":true,"documentHost":true,"documentHostShadow":true}"#
    );
}

#[test]
fn child_shadow_script_named_property_uses_actual_host_name_only() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = (document.body || document.documentElement || document)
                .appendChild(document.createElement("iframe"));
              frame.contentDocument.open();
              frame.contentDocument.write(`<!doctype html><html><body><div id="actualHost"><template shadowrootmode="open"><script>
                window.scriptActualHostType = typeof actualHost;
                window.scriptActualHostHasShadow =
                  !!(typeof actualHost !== "undefined" && actualHost.shadowRoot);
                window.scriptHardcodedHostType = typeof host;
              <\/script></template></div></body></html>`);
              frame.contentDocument.close();
              return JSON.stringify({
                winActualHost:
                  frame.contentWindow.actualHost ===
                  frame.contentDocument.getElementById("actualHost"),
                winHostType: typeof frame.contentWindow.host,
                scriptActualHostType: frame.contentWindow.scriptActualHostType,
                scriptActualHostHasShadow: frame.contentWindow.scriptActualHostHasShadow,
                scriptHardcodedHostType: frame.contentWindow.scriptHardcodedHostType
              });
            })()
            "#,
        )
        .expect("child shadow script host named property probe should evaluate");

    assert_eq!(
        result,
        r#"{"winActualHost":true,"winHostType":"undefined","scriptActualHostType":"object","scriptActualHostHasShadow":true,"scriptHardcodedHostType":"undefined"}"#
    );
}

#[test]
fn custom_elements_registry_constructor_creates_scoped_registry_store() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              class GlobalOnly extends HTMLElement {}
              class ScopedOnly extends HTMLElement {}
              class GlobalShared extends HTMLElement {}
              class Shared extends HTMLElement {}
              customElements.define("wpt-global-only", GlobalOnly);
              customElements.define("wpt-shared-name", GlobalShared);
              registry.define("wpt-scoped-only", ScopedOnly);
              registry.define("wpt-shared-name", Shared);
              const scopedElement = document.createElement(
                "wpt-scoped-only",
                { customElementRegistry: registry }
              );
              const scopedCandidate = document.createElement(
                "wpt-unresolved-scoped",
                { customElementRegistry: registry }
              );
              const nullRegistryElement = document.createElement(
                "wpt-global-only",
                { customElementRegistry: null }
              );

              let scopedOnlyDirectConstruction = "not-thrown";
              try {
                new ScopedOnly();
              } catch (error) {
                scopedOnlyDirectConstruction = error.constructor.name;
              }
              const globalConstructed = new GlobalOnly();

              return JSON.stringify({
                constructable: registry instanceof CustomElementRegistry,
                tag: Object.prototype.toString.call(registry),
                globalDoesNotSeeScoped: customElements.get("wpt-scoped-only") === undefined,
                scopedDoesNotSeeGlobal: registry.get("wpt-global-only") === undefined,
                globalShared: customElements.get("wpt-shared-name") === GlobalShared,
                scopedShared: registry.get("wpt-shared-name") === Shared,
                scopedGetNameGlobal: registry.getName(GlobalOnly),
                scopedGetNameScoped: registry.getName(ScopedOnly),
                scopedElementInstance: scopedElement instanceof ScopedOnly,
                scopedElementRegistry: scopedElement.customElementRegistry === registry,
                scopedCandidateRegistry: scopedCandidate.customElementRegistry === registry,
                nullRegistry: nullRegistryElement.customElementRegistry,
                globalConstructedName: globalConstructed.localName,
                scopedOnlyDirectConstruction
              });
            })()
            "#,
        )
        .expect("scoped customElements registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructable":true,"tag":"[object CustomElementRegistry]","globalDoesNotSeeScoped":true,"scopedDoesNotSeeGlobal":true,"globalShared":true,"scopedShared":true,"scopedGetNameGlobal":null,"scopedGetNameScoped":"wpt-scoped-only","scopedElementInstance":true,"scopedElementRegistry":true,"scopedCandidateRegistry":true,"nullRegistry":null,"globalConstructedName":"wpt-global-only","scopedOnlyDirectConstruction":"TypeError"}"#
    );
}

#[test]
fn scoped_registry_compaction_removes_orphaned_store() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          const registry = new CustomElementRegistry();
          class ScopedElement extends HTMLElement {}
          registry.define("wpt-scoped-compact", ScopedElement);
        })()
        "#,
    )
    .expect("scoped registry setup should evaluate");

    vm.with_default_context_scope_and_checkpoint_for_test(|_scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        assert_eq!(
            host.scoped_custom_element_registry_wrapper_count_for_test(),
            1
        );
        assert_eq!(host.scoped_custom_elements_store_count_for_test(), 1);

        host.remove_scoped_custom_element_registry_wrapper_for_test(1);
        host.compact_scoped_custom_element_registry_wrappers_for_test();

        assert_eq!(
            host.scoped_custom_element_registry_wrapper_count_for_test(),
            0
        );
        assert_eq!(host.scoped_custom_elements_store_count_for_test(), 0);
        Ok(())
    })
    .expect("scoped registry compaction probe should run");
}

#[test]
fn scoped_registry_lookup_cleanup_removes_orphaned_store() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          const registry = new CustomElementRegistry();
          class ScopedElement extends HTMLElement {}
          registry.define("wpt-scoped-lookup", ScopedElement);
        })()
        "#,
    )
    .expect("scoped registry setup should evaluate");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        assert_eq!(
            host.scoped_custom_element_registry_wrapper_count_for_test(),
            1
        );
        assert_eq!(host.scoped_custom_elements_store_count_for_test(), 1);

        host.remove_scoped_custom_element_registry_wrapper_for_test(1);
        assert!(
            host.custom_element_registry_object_for_key(scope, CustomElementRegistryKey::Scoped(1))
                .is_none()
        );

        assert_eq!(
            host.scoped_custom_element_registry_wrapper_count_for_test(),
            0
        );
        assert_eq!(host.scoped_custom_elements_store_count_for_test(), 0);
        Ok(())
    })
    .expect("scoped registry lookup cleanup probe should run");
}

#[test]
fn scoped_registry_direct_constructor_reentry_keeps_consumed_upgrade() {
    let mut vm = new_storage_test_vm("https://example.com/");

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
              const host = document.createElement("div");
              document.body.appendChild(host);

              const beforeRegistry = new CustomElementRegistry();
              const beforeShadow =
                host.attachShadow({ mode: "open", customElementRegistry: beforeRegistry });
              let beforeNeedsNested = true;
              let beforeNested;
              class BeforeReentry extends HTMLElement {
                constructor() {
                  if (beforeNeedsNested) {
                    beforeNeedsNested = false;
                    beforeNested = new BeforeReentry();
                  }
                  super();
                }
              }
              customElements.define("wpt-before-global", BeforeReentry);
              beforeRegistry.define("wpt-before-scoped", BeforeReentry);
              beforeShadow.innerHTML = "<wpt-before-scoped></wpt-before-scoped>";
              const beforeElement = beforeShadow.firstChild;

              const afterHost = document.createElement("div");
              document.body.appendChild(afterHost);
              const afterRegistry = new CustomElementRegistry();
              const afterShadow =
                afterHost.attachShadow({ mode: "open", customElementRegistry: afterRegistry });
              let afterNeedsNested = true;
              let afterNested;
              let afterNestedError;
              class AfterReentry extends HTMLElement {
                constructor() {
                  super();
                  if (afterNeedsNested) {
                    afterNeedsNested = false;
                    try {
                      afterNested = new AfterReentry();
                    } catch (error) {
                      afterNestedError = error && error.name;
                    }
                  }
                }
              }
              customElements.define("wpt-after-global", AfterReentry);
              afterRegistry.define("wpt-after-scoped", AfterReentry);
              afterShadow.innerHTML = "<wpt-after-scoped></wpt-after-scoped>";
              const afterElement = afterShadow.firstChild;

              let sameDefinitionError;
              window.onerror = function(message, url, line, column, error) {
                sameDefinitionError = error && error.name;
                return true;
              };
              class SameDefinitionReentry extends HTMLElement {
                constructor(skip) {
                  super();
                  if (!skip) {
                    new SameDefinitionReentry(true);
                  }
                }
              }
              customElements.define("wpt-same-definition-reentry", SameDefinitionReentry);
              document.createElement("wpt-same-definition-reentry")
                .cloneNode(false);

              const sameNameHost = document.createElement("div");
              document.body.appendChild(sameNameHost);
              const sameNameRegistry = new CustomElementRegistry();
              const sameNameShadow =
                sameNameHost.attachShadow({ mode: "open", customElementRegistry: sameNameRegistry });
              let sameNameNeedsNested = true;
              let sameNameNested;
              class SameNameReentry extends HTMLElement {
                constructor() {
                  if (sameNameNeedsNested) {
                    sameNameNeedsNested = false;
                    sameNameNested = new SameNameReentry();
                  }
                  super();
                }
              }
              customElements.define("wpt-same-name-reentry", SameNameReentry);
              sameNameRegistry.define("wpt-same-name-reentry", SameNameReentry);
              sameNameShadow.innerHTML = "<wpt-same-name-reentry></wpt-same-name-reentry>";
              const sameNameElement = sameNameShadow.firstChild;

              return JSON.stringify({
                beforeInstance: beforeElement instanceof BeforeReentry,
                beforeLocalName: beforeElement.localName,
                beforeNestedSame: beforeNested === beforeElement,
                beforeNestedInstance: beforeNested instanceof BeforeReentry,
                afterInstance: afterElement instanceof AfterReentry,
                afterLocalName: afterElement.localName,
                afterNestedInstance: afterNested instanceof AfterReentry,
                afterNestedError,
                sameDefinitionError,
                sameNameInstance: sameNameElement instanceof SameNameReentry,
                sameNameLocalName: sameNameElement.localName,
                sameNameNestedSame: sameNameNested === sameNameElement
              });
            })()
            "#,
        )
        .expect("scoped registry direct constructor re-entry probe should evaluate");

    assert_eq!(
        result,
        r#"{"beforeInstance":true,"beforeLocalName":"wpt-before-scoped","beforeNestedSame":true,"beforeNestedInstance":true,"afterInstance":true,"afterLocalName":"wpt-after-scoped","afterNestedInstance":false,"afterNestedError":"TypeError","sameDefinitionError":"TypeError","sameNameInstance":true,"sameNameLocalName":"wpt-same-name-reentry","sameNameNestedSame":true}"#
    );
}

#[test]
fn custom_elements_initialize_sets_scoped_registry_associations() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const doc = new Document();
              const root = doc.createElement("wpt-init-root");
              const child = doc.createElement("wpt-init-child");
              root.appendChild(child);
              doc.appendChild(root);
              const fragment = doc.createDocumentFragment();
              const fragmentChild = doc.createElement("wpt-init-fragment-child");
              fragment.appendChild(fragmentChild);

              let globalDocumentError = null;
              try {
                customElements.initialize(new Document());
              } catch (error) {
                globalDocumentError = error.name;
              }

              const before = {
                doc: doc.customElementRegistry,
                root: root.customElementRegistry,
                fragmentType: typeof fragment.customElementRegistry
              };
              registry.initialize(doc);
              registry.initialize(fragment);
              const createdAfterInitialize = doc.createElement("wpt-init-after");

              return JSON.stringify({
                initializeType: typeof registry.initialize,
                globalDocumentError,
                beforeDocRegistry: before.doc,
                beforeRootRegistry: before.root,
                fragmentGetterType: before.fragmentType,
                docRegistry: doc.customElementRegistry === registry,
                rootRegistry: root.customElementRegistry === registry,
                childRegistry: child.customElementRegistry === registry,
                fragmentChildRegistry: fragmentChild.customElementRegistry === registry,
                createdAfterInitializeRegistry:
                    createdAfterInitialize.customElementRegistry === registry
              });
            })()
            "#,
        )
        .expect("custom element registry initialize probe should evaluate");

    assert_eq!(
        result,
        r#"{"initializeType":"function","globalDocumentError":"NotSupportedError","beforeDocRegistry":null,"beforeRootRegistry":null,"fragmentGetterType":"undefined","docRegistry":true,"rootRegistry":true,"childRegistry":true,"fragmentChildRegistry":true,"createdAfterInitializeRegistry":true}"#
    );
}

#[test]
fn attach_shadow_default_registry_uses_owner_document_default() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const scopedRegistry = new CustomElementRegistry();
              const scopedHost = document.createElement(
                "wpt-scoped-host",
                { customElementRegistry: scopedRegistry }
              );
              const defaultRoot = scopedHost.attachShadow({ mode: "open" });

              const nullRoot = document
                .createElement("wpt-null-host")
                .attachShadow({ mode: "open", customElementRegistry: null });
              const nestedHost = document.createElement("wpt-nested-host");
              nullRoot.appendChild(nestedHost);
              const nestedDefaultRoot = nestedHost.attachShadow({ mode: "open" });

              const explicitNullRoot = document
                .createElement("wpt-explicit-null-host")
                .attachShadow({ mode: "open", customElementRegistry: null });

              return JSON.stringify({
                scopedHostRegistry: scopedHost.customElementRegistry === scopedRegistry,
                defaultRootGlobal: defaultRoot.customElementRegistry === customElements,
                nestedHostNull: nestedHost.customElementRegistry,
                nestedDefaultRootGlobal:
                  nestedDefaultRoot.customElementRegistry === customElements,
                explicitNullRootRegistry: explicitNullRoot.customElementRegistry
              });
            })()
            "#,
        )
        .expect("attachShadow default custom element registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"scopedHostRegistry":true,"defaultRootGlobal":true,"nestedHostNull":null,"nestedDefaultRootGlobal":true,"explicitNullRootRegistry":null}"#
    );
}

#[test]
fn removing_from_shadow_tree_preserves_non_default_registry_association() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const scopedRoot = document
                .createElement("wpt-scoped-root")
                .attachShadow({ mode: "open", customElementRegistry: registry });
              const scopedWrapper = document.createElement("div");
              const scopedChild = document.createElement("wpt-scoped-child");
              scopedWrapper.appendChild(scopedChild);
              scopedRoot.appendChild(scopedWrapper);

              const nullRoot = document
                .createElement("wpt-null-root")
                .attachShadow({ mode: "open", customElementRegistry: null });
              const nullWrapper = document.createElement("section");
              const nullChild = document.createElement("wpt-null-child");
              nullWrapper.appendChild(nullChild);
              nullRoot.appendChild(nullWrapper);

              const beforeScoped = scopedChild.customElementRegistry === registry;
              const beforeNull = nullChild.customElementRegistry;
              scopedWrapper.remove();
              nullWrapper.remove();

              return JSON.stringify({
                beforeScoped,
                afterScoped: scopedChild.customElementRegistry === registry,
                beforeNull,
                afterNull: nullChild.customElementRegistry
              });
            })()
            "#,
        )
        .expect("shadow removal custom element registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"beforeScoped":true,"afterScoped":true,"beforeNull":null,"afterNull":null}"#
    );
}

#[test]
fn custom_element_registry_upgrade_filters_by_receiver_registry() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry1 = new CustomElementRegistry();
              const registry2 = new CustomElementRegistry();
              class RegistryOneElement extends HTMLElement {}
              class RegistryTwoElement extends HTMLElement {}
              registry1.define("wpt-upgrade-filter", RegistryOneElement);
              registry2.define("wpt-upgrade-filter", RegistryTwoElement);

              const element = document.createElement(
                "wpt-upgrade-filter",
                { customElementRegistry: registry1 }
              );
              const beforeUpgradeDefined = element.matches(":defined");
              registry2.upgrade(element);
              const afterWrongRegistryDefined = element.matches(":defined");
              registry1.upgrade(element);

              const root = document
                .createElement("wpt-upgrade-host")
                .attachShadow({ mode: "open", customElementRegistry: registry1 });
              const scopedChild = document.createElement(
                "wpt-upgrade-filter",
                { customElementRegistry: registry1 }
              );
              const nullChild = document.createElement(
                "wpt-upgrade-filter",
                { customElementRegistry: null }
              );
              root.append(scopedChild, nullChild);
              registry1.upgrade(root);

              return JSON.stringify({
                beforeUpgradeDefined,
                afterWrongRegistryDefined,
                afterRightRegistry: element instanceof RegistryOneElement,
                scopedChildUpgraded: scopedChild instanceof RegistryOneElement,
                nullChildUpgraded: nullChild instanceof RegistryOneElement
              });
            })()
            "#,
        )
        .expect("CustomElementRegistry.upgrade registry filter probe should evaluate");

    assert_eq!(
        result,
        r#"{"beforeUpgradeDefined":true,"afterWrongRegistryDefined":true,"afterRightRegistry":true,"scopedChildUpgraded":true,"nullChildUpgraded":false}"#
    );
}

#[test]
fn custom_element_registry_initialize_overwrites_null_descendant_associations() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const root = document
                .createElement("wpt-init-null-root")
                .attachShadow({ mode: "open", customElementRegistry: null });
              root.innerHTML = "<wpt-init-null-child></wpt-init-null-child>";
              const child = root.querySelector("wpt-init-null-child");
              const before = child.customElementRegistry;
              registry.initialize(root);

              return JSON.stringify({
                rootRegistry: root.customElementRegistry === registry,
                before,
                childRegistry: child.customElementRegistry === registry
              });
            })()
            "#,
        )
        .expect("CustomElementRegistry.initialize null subtree probe should evaluate");

    assert_eq!(
        result,
        r#"{"rootRegistry":true,"before":null,"childRegistry":true}"#
    );
}

#[test]
fn global_registry_options_reject_other_document_targets() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const other = document.implementation.createHTMLDocument();
              const own = document.implementation.createHTMLDocument();
              const otherElement = other.createElement("div");
              const ownRegistry = new CustomElementRegistry();
              const probe = (callback) => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };

              return [
                probe(() => customElements.initialize(other.createElement("x-cross-init"))),
                probe(() => other.createElement("div", { customElementRegistry: customElements })),
                probe(() => other.createElementNS("http://www.w3.org/1999/xhtml", "div", {
                  customElementRegistry: customElements
                })),
                probe(() => otherElement.attachShadow({
                  mode: "closed",
                  customElementRegistry: customElements
                })),
                probe(() => other.importNode(otherElement, {
                  customElementRegistry: customElements
                })),
                probe(() => own.createElement("x-own-scoped", {
                  customElementRegistry: ownRegistry
                })).replace(/ok/, String(
                  own.createElement("x-own-scoped", {
                    customElementRegistry: ownRegistry
                  }).customElementRegistry === ownRegistry
                ))
              ].join("|");
            })()
            "#,
        )
        .expect("global registry cross-document validation should evaluate");

    assert_eq!(
        result,
        "NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|true"
    );
}

#[test]
fn scoped_registry_define_upgrades_associated_connected_nodes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const movingRegistry = new CustomElementRegistry();
              const otherRegistry = new CustomElementRegistry();
              const direct = document.createElement(
                "wpt-scoped-define",
                { customElementRegistry: registry }
              );
              const host = document.createElement("div");
              const shadow = host.attachShadow({
                mode: "open",
                customElementRegistry: registry
              });
              shadow.innerHTML = "<wpt-scoped-define></wpt-scoped-define>";
              const shadowElement = shadow.querySelector("wpt-scoped-define");
              const documentElement = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const target = document.body ||
                documentElement.appendChild(document.createElement("body"));
              target.appendChild(direct);
              target.appendChild(host);
              const movingHost = target.appendChild(document.createElement("div"));
              const movingShadow = movingHost.attachShadow({
                mode: "open",
                customElementRegistry: movingRegistry
              });
              movingShadow.innerHTML = "<wpt-scoped-move></wpt-scoped-move>";
              const moved = movingShadow.querySelector("wpt-scoped-move");
              const otherHost = target.appendChild(document.createElement("div"));
              const otherShadow = otherHost.attachShadow({
                mode: "open",
                customElementRegistry: otherRegistry
              });
              otherShadow.appendChild(moved);

              const calls = [];
              class ScopedDefineElement extends HTMLElement {
                connectedCallback() {
                  calls.push(this === direct ? "direct" : "shadow");
                }
              }
              class ScopedMovedElement extends HTMLElement {}
              registry.define("wpt-scoped-define", ScopedDefineElement);
              movingRegistry.define("wpt-scoped-move", ScopedMovedElement);

              return [
                direct instanceof ScopedDefineElement,
                shadowElement instanceof ScopedDefineElement,
                moved instanceof ScopedMovedElement,
                direct.customElementRegistry === registry,
                shadowElement.customElementRegistry === registry,
                moved.customElementRegistry === movingRegistry,
                calls.sort().join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("scoped registry define upgrade probe should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|direct,shadow");
}

#[test]
fn fragment_html_skips_redundant_default_registry_associations() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let setup = vm
        .eval(
            r#"
            (() => {
              const container = document.createElement("div");
              window.__moliDefaultRegistryContainer = container;
              return container.customElementRegistry === customElements;
            })()
            "#,
        )
        .expect("default registry fragment setup should evaluate");
    assert_eq!(setup, "true");
    let associations_before = vm.custom_element_registry_association_count_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const container = window.__moliDefaultRegistryContainer;
              container.innerHTML = "<div></div>".repeat(128);
              return [
                container.children.length,
                Array.from(container.children).every(
                  child => child.customElementRegistry === customElements
                )
              ].join("|");
            })()
            "#,
        )
        .expect("default registry fragment should evaluate");

    assert_eq!(result, "128|true");
    assert_eq!(
        vm.custom_element_registry_association_count_for_test(),
        associations_before,
        "fragment roots using their owner document's default registry should not need explicit associations"
    );
}

#[test]
fn fragment_html_uses_context_custom_element_registry_association() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const documentElement = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                documentElement.appendChild(document.createElement("body"));

              const registry = new CustomElementRegistry();
              const otherRegistry = new CustomElementRegistry();

              class GlobalSharedElement extends HTMLElement {}
              class ScopedSharedElement extends HTMLElement {}
              class OtherSharedElement extends HTMLElement {}
              customElements.define("wpt-fragment-shared", GlobalSharedElement);
              registry.define("wpt-fragment-shared", ScopedSharedElement);
              otherRegistry.define("wpt-fragment-shared", OtherSharedElement);

              class WrongNullElement extends HTMLElement {}
              customElements.define("wpt-fragment-null", WrongNullElement);

              const constructionErrors = [];
              window.addEventListener("error", event => {
                event.preventDefault();
                constructionErrors.push(String(event.error || event.message));
              });
              registry.define(
                "wpt-fragment-throws",
                class extends HTMLElement {
                  constructor() {
                    super();
                    throw TypeError;
                  }
                }
              );
              registry.define(
                "wpt-fragment-returns",
                class extends HTMLElement {
                  constructor() {
                    super();
                    return document.createElement("span");
                  }
                }
              );

              const scopedContainer = document.createElement("div", {
                customElementRegistry: registry
              });
              scopedContainer.innerHTML =
                "<wpt-fragment-shared id='scoped-root'>" +
                  "<span id='scoped-span'>" +
                    "<wpt-fragment-shared id='scoped-nested'></wpt-fragment-shared>" +
                  "</span>" +
                "</wpt-fragment-shared>";
              const scopedRoot = scopedContainer.querySelector("#scoped-root");
              const scopedSpan = scopedContainer.querySelector("#scoped-span");
              const scopedNested = scopedContainer.querySelector("#scoped-nested");

              const otherHost = document.createElement("div");
              body.appendChild(otherHost);
              const otherShadow = otherHost.attachShadow({
                mode: "open",
                customElementRegistry: otherRegistry
              });
              otherShadow.appendChild(scopedContainer);
              scopedContainer.innerHTML =
                "<wpt-fragment-shared id='moved-root'></wpt-fragment-shared>";
              const movedRoot = scopedContainer.querySelector("#moved-root");
              scopedContainer.innerHTML =
                "<wpt-fragment-throws id='failed-throws'></wpt-fragment-throws>" +
                "<wpt-fragment-returns id='failed-returns'></wpt-fragment-returns>";
              const failedThrows = scopedContainer.querySelector("#failed-throws");
              const failedReturns = scopedContainer.querySelector("#failed-returns");

              const nullHost = document.createElement("div");
              body.appendChild(nullHost);
              const nullShadow = nullHost.attachShadow({
                mode: "open",
                customElementRegistry: null
              });
              const nullContainer = document.createElement("div");
              nullShadow.appendChild(nullContainer);
              nullContainer.innerHTML =
                "<wpt-fragment-null id='null-root'>" +
                  "<wpt-fragment-null id='null-nested'></wpt-fragment-null>" +
                "</wpt-fragment-null>";
              const nullRoot = nullContainer.querySelector("#null-root");
              const nullNested = nullContainer.querySelector("#null-nested");

              const anchor = document.createElement("span");
              nullShadow.appendChild(anchor);
              anchor.insertAdjacentHTML(
                "afterend",
                "<wpt-fragment-null id='null-adjacent'></wpt-fragment-null>"
              );
              const nullAdjacent = nullShadow.querySelector("#null-adjacent");

              const scopedSiblingTarget = document.createElement("div", {
                customElementRegistry: registry
              });
              body.appendChild(scopedSiblingTarget);
              scopedSiblingTarget.insertAdjacentHTML(
                "beforebegin",
                "<wpt-fragment-shared id='before-sibling'></wpt-fragment-shared>"
              );
              scopedSiblingTarget.insertAdjacentHTML(
                "afterend",
                "<wpt-fragment-shared id='after-sibling'></wpt-fragment-shared>"
              );
              const beforeSibling = document.querySelector("#before-sibling");
              const afterSibling = document.querySelector("#after-sibling");

              const outerTarget = document.createElement("div", {
                customElementRegistry: registry
              });
              body.appendChild(outerTarget);
              outerTarget.outerHTML =
                "<wpt-fragment-shared id='outer-replacement'></wpt-fragment-shared>";
              const outerReplacement = document.querySelector("#outer-replacement");

              return JSON.stringify({
                scopedRoot: scopedRoot instanceof ScopedSharedElement,
                scopedRootNotGlobal: !(scopedRoot instanceof GlobalSharedElement),
                scopedSpanRegistry: scopedSpan.customElementRegistry === registry,
                scopedNested: scopedNested instanceof ScopedSharedElement,
                movedRoot: movedRoot instanceof ScopedSharedElement,
                movedRootNotOther: !(movedRoot instanceof OtherSharedElement),
                movedRootRegistry: movedRoot.customElementRegistry === registry,
                failedThrowsRegistry:
                  failedThrows.customElementRegistry === registry,
                failedReturnsRegistry:
                  failedReturns.customElementRegistry === registry,
                failedConstructionErrors: constructionErrors.length >= 1,
                nullRootRegistry: nullRoot.customElementRegistry === null,
                nullRootNotWrong: !(nullRoot instanceof WrongNullElement),
                nullNestedRegistry: nullNested.customElementRegistry === null,
                nullNestedNotWrong: !(nullNested instanceof WrongNullElement),
                nullAdjacentRegistry: nullAdjacent.customElementRegistry === null,
                nullAdjacentNotWrong: !(nullAdjacent instanceof WrongNullElement),
                beforeSiblingRegistry:
                  beforeSibling.customElementRegistry === customElements,
                beforeSiblingGlobal: beforeSibling instanceof GlobalSharedElement,
                beforeSiblingNotScoped: !(beforeSibling instanceof ScopedSharedElement),
                afterSiblingRegistry:
                  afterSibling.customElementRegistry === customElements,
                afterSiblingGlobal: afterSibling instanceof GlobalSharedElement,
                afterSiblingNotScoped: !(afterSibling instanceof ScopedSharedElement),
                outerReplacementRegistry:
                  outerReplacement.customElementRegistry === customElements,
                outerReplacementGlobal:
                  outerReplacement instanceof GlobalSharedElement,
                outerReplacementNotScoped:
                  !(outerReplacement instanceof ScopedSharedElement)
              });
            })()
            "##,
        )
        .expect("fragment HTML scoped registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"scopedRoot":true,"scopedRootNotGlobal":true,"scopedSpanRegistry":true,"scopedNested":true,"movedRoot":true,"movedRootNotOther":true,"movedRootRegistry":true,"failedThrowsRegistry":true,"failedReturnsRegistry":true,"failedConstructionErrors":true,"nullRootRegistry":true,"nullRootNotWrong":true,"nullNestedRegistry":true,"nullNestedNotWrong":true,"nullAdjacentRegistry":true,"nullAdjacentNotWrong":true,"beforeSiblingRegistry":true,"beforeSiblingGlobal":true,"beforeSiblingNotScoped":true,"afterSiblingRegistry":true,"afterSiblingGlobal":true,"afterSiblingNotScoped":true,"outerReplacementRegistry":true,"outerReplacementGlobal":true,"outerReplacementNotScoped":true}"#
    );
}

#[test]
fn fragment_html_custom_element_constructor_sees_connected_token_attributes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const documentElement = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                documentElement.appendChild(document.createElement("body"));
              const host = document.createElement("div");
              host.id = "host";
              body.appendChild(host);

              const events = [];
              class FragmentAttrElement extends HTMLElement {
                constructor() {
                  super();
                  events.push([
                    "ctor",
                    this.hasAttribute("data-token"),
                    this.getAttribute("data-token"),
                    this.isConnected,
                    this.parentElement && this.parentElement.id
                  ].join("|"));
                }
                connectedCallback() {
                  events.push([
                    "connected",
                    this.isConnected,
                    this.parentElement && this.parentElement.id
                  ].join("|"));
                }
              }
              customElements.define("wpt-fragment-attr", FragmentAttrElement);
              host.innerHTML =
                "<wpt-fragment-attr data-token='owned'></wpt-fragment-attr>";
              const element = host.firstElementChild;
              return [
                events.join(","),
                element instanceof FragmentAttrElement,
                element.getAttribute("data-token")
              ].join("||");
            })()
            "#,
        )
        .expect("fragment HTML custom element timing probe should evaluate");

    assert_eq!(
        result,
        "ctor|true|owned|true|host,connected|true|host||true||owned"
    );
}

#[test]
fn fragment_html_custom_element_constructor_sees_disconnected_parent() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              host.id = "host";

              const events = [];
              class FragmentDetachedElement extends HTMLElement {
                constructor() {
                  super();
                  events.push([
                    "ctor",
                    this.hasAttribute("data-token"),
                    this.getAttribute("data-token"),
                    this.isConnected,
                    this.parentElement && this.parentElement.id
                  ].join("|"));
                }
                connectedCallback() {
                  events.push("connected");
                }
              }
              customElements.define("wpt-fragment-detached", FragmentDetachedElement);
              host.innerHTML =
                "<wpt-fragment-detached data-token='owned'></wpt-fragment-detached>";
              const element = host.firstElementChild;
              return [
                events.join(","),
                element instanceof FragmentDetachedElement,
                element.isConnected,
                element.getAttribute("data-token")
              ].join("||");
            })()
            "#,
        )
        .expect("detached fragment HTML custom element timing probe should evaluate");

    assert_eq!(result, "ctor|true|owned|false|host||true||false||owned");
}

#[test]
fn fragment_html_outer_and_adjacent_constructors_see_inserted_tree_position() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const documentElement = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                documentElement.appendChild(document.createElement("body"));
              body.textContent = "";
              const before = document.createElement("div");
              before.id = "before";
              const target = document.createElement("div");
              target.id = "target";
              body.append(before, target);

              const events = [];
              class FragmentOuterElement extends HTMLElement {
                constructor() {
                  super();
                  events.push([
                    "outer-ctor",
                    this.hasAttribute("data-token"),
                    this.getAttribute("data-token"),
                    this.isConnected,
                    this.parentElement && this.parentElement.nodeName,
                    this.previousElementSibling && this.previousElementSibling.id
                  ].join("|"));
                }
                connectedCallback() {
                  events.push([
                    "outer-connected",
                    this.isConnected,
                    this.parentElement && this.parentElement.nodeName
                  ].join("|"));
                }
              }
              class FragmentAdjacentElement extends HTMLElement {
                constructor() {
                  super();
                  events.push([
                    "adjacent-ctor",
                    this.hasAttribute("data-token"),
                    this.getAttribute("data-token"),
                    this.isConnected,
                    this.parentElement && this.parentElement.nodeName,
                    this.previousElementSibling && this.previousElementSibling.id
                  ].join("|"));
                }
                connectedCallback() {
                  events.push([
                    "adjacent-connected",
                    this.isConnected,
                    this.parentElement && this.parentElement.nodeName
                  ].join("|"));
                }
              }
              customElements.define("wpt-fragment-outer", FragmentOuterElement);
              customElements.define("wpt-fragment-adjacent", FragmentAdjacentElement);

              target.outerHTML =
                "<wpt-fragment-outer data-token='owned'></wpt-fragment-outer>";
              before.insertAdjacentHTML(
                "afterend",
                "<wpt-fragment-adjacent data-token='owned'></wpt-fragment-adjacent>"
              );

              return events.join(",");
            })()
            "#,
        )
        .expect("outerHTML/insertAdjacentHTML custom element timing probe should evaluate");

    assert_eq!(
        result,
        "outer-ctor|true|owned|true|BODY|before,outer-connected|true|BODY,adjacent-ctor|true|owned|true|BODY|before,adjacent-connected|true|BODY"
    );
}

#[test]
fn range_contextual_fragment_uses_context_custom_element_registry_association() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const registry = new CustomElementRegistry();

              class WrongRangeElement extends HTMLElement {}
              class ScopedRangeElement extends HTMLElement {}
              customElements.define("wpt-range-context", WrongRangeElement);
              registry.define("wpt-range-context", ScopedRangeElement);

              const template = document.createElement("template", {
                customElementRegistry: registry
              });
              const templateRange = document.createRange();
              templateRange.selectNodeContents(template);
              const templateFragment = templateRange.createContextualFragment(
                "<wpt-range-context id='template-root'>" +
                  "<wpt-range-context id='template-nested'></wpt-range-context>" +
                "</wpt-range-context>"
              );
              const templateRoot =
                templateFragment.querySelector("#template-root");
              const templateNested =
                templateFragment.querySelector("#template-nested");

              const div = document.createElement("div", {
                customElementRegistry: registry
              });
              const divRange = document.createRange();
              divRange.selectNodeContents(div);
              const divFragment = divRange.createContextualFragment(
                "<wpt-range-context id='div-root'>" +
                  "<wpt-range-context id='div-nested'></wpt-range-context>" +
                "</wpt-range-context>"
              );
              const divRoot = divFragment.querySelector("#div-root");
              const divNested = divFragment.querySelector("#div-nested");

              return JSON.stringify({
                templateRegistry: template.customElementRegistry === registry,
                templateRootRegistry: templateRoot.customElementRegistry === null,
                templateRootNotWrong:
                  !(templateRoot instanceof WrongRangeElement),
                templateRootNotScoped:
                  !(templateRoot instanceof ScopedRangeElement),
                templateNestedRegistry:
                  templateNested.customElementRegistry === null,
                templateNestedNotWrong:
                  !(templateNested instanceof WrongRangeElement),
                templateNestedNotScoped:
                  !(templateNested instanceof ScopedRangeElement),
                divRootRegistry: divRoot.customElementRegistry === registry,
                divRootScoped: divRoot instanceof ScopedRangeElement,
                divRootNotWrong: !(divRoot instanceof WrongRangeElement),
                divNestedRegistry:
                  divNested.customElementRegistry === registry,
                divNestedScoped: divNested instanceof ScopedRangeElement,
                divNestedNotWrong: !(divNested instanceof WrongRangeElement)
              });
            })()
            "##,
        )
        .expect("Range.createContextualFragment scoped registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"templateRegistry":true,"templateRootRegistry":true,"templateRootNotWrong":true,"templateRootNotScoped":true,"templateNestedRegistry":true,"templateNestedNotWrong":true,"templateNestedNotScoped":true,"divRootRegistry":true,"divRootScoped":true,"divRootNotWrong":true,"divNestedRegistry":true,"divNestedScoped":true,"divNestedNotWrong":true}"#
    );
}

#[test]
fn range_contextual_fragment_custom_element_constructor_sees_fragment_parent() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const range = document.createRange();
              range.selectNodeContents(host);

              const events = [];
              class RangeParentElement extends HTMLElement {
                constructor() {
                  super();
                  events.push([
                    "ctor",
                    this.hasAttribute("data-token"),
                    this.getAttribute("data-token"),
                    this.isConnected,
                    this.parentNode && this.parentNode.nodeType,
                    this.parentNode && this.parentNode.nodeName
                  ].join("|"));
                }
              }
              customElements.define("wpt-range-parent", RangeParentElement);
              const fragment = range.createContextualFragment(
                "<wpt-range-parent data-token='owned'></wpt-range-parent>"
              );
              const element = fragment.firstElementChild;
              return [
                events.join(","),
                element instanceof RangeParentElement,
                element.isConnected,
                element.parentNode && element.parentNode.nodeType,
                element.parentNode && element.parentNode.nodeName,
                element.getAttribute("data-token")
              ].join("||");
            })()
            "#,
        )
        .expect("Range.createContextualFragment parent timing probe should evaluate");

    assert_eq!(
        result,
        "ctor|true|owned|false|11|#document-fragment||true||false||11||#document-fragment||owned"
    );
}

#[test]
fn scoped_registry_initialize_upgrades_existing_associated_nodes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const registry = new CustomElementRegistry();
              const doc = new Document();
              const container = doc.createElementNS("http://www.w3.org/1999/xhtml", "div");
              container.innerHTML =
                "<wpt-scoped-init id='a'></wpt-scoped-init>" +
                "<wpt-scoped-init id='b'></wpt-scoped-init>";
              const elements = Array.from(container.querySelectorAll("wpt-scoped-init"));
              const explicit = document.createElement(
                "wpt-scoped-init",
                { customElementRegistry: registry }
              );

              const constructed = [];
              const constructorThisMatches = [];
              class ScopedInitElement extends HTMLElement {
                constructor() {
                  super();
                  constructed.push(this.id || "explicit");
                  constructorThisMatches.push(
                    this === explicit ? "explicit" : String(elements.indexOf(this))
                  );
                }
              }
              registry.define("wpt-scoped-init", ScopedInitElement);
              registry.initialize(container);
              registry.initialize(explicit);

              return [
                elements[0] instanceof ScopedInitElement,
                elements[1] instanceof ScopedInitElement,
                explicit instanceof ScopedInitElement,
                elements[0].customElementRegistry === registry,
                elements[1].customElementRegistry === registry,
                explicit.customElementRegistry === registry,
                constructed.join(","),
                constructorThisMatches.join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("scoped registry initialize upgrade probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|a,b,explicit|0,1,explicit"
    );
}

#[test]
fn custom_element_registry_associations_survive_tree_mutations_and_adoption() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const documentTarget = document.body || document.documentElement || document;
              documentTarget.appendChild(frame);
              const frameTarget = frame.contentDocument.body ||
                frame.contentDocument.appendChild(frame.contentDocument.createElement("body"));
              if (frameTarget === null)
                return "frameTarget:null";

              const implicitGlobal = document.createElement("div");
              frameTarget.appendChild(implicitGlobal);

              const explicitGlobal = document.createElement(
                "div",
                { customElementRegistry: customElements }
              );
              frameTarget.appendChild(explicitGlobal);

              const scoped = new CustomElementRegistry();
              const scopedElement = document.createElement(
                "div",
                { customElementRegistry: scoped }
              );
              if (scopedElement === null)
                return "scopedElement:null";
              frameTarget.appendChild(scopedElement);

              const sameDocumentScoped = new CustomElementRegistry();
              const sameDocumentHost = document.createElement("div");
              documentTarget.appendChild(sameDocumentHost);
              const sameDocumentShadow = sameDocumentHost.attachShadow({
                mode: "open",
                customElementRegistry: sameDocumentScoped
              });
              if (sameDocumentShadow === null)
                return "sameDocumentShadow:null";
              const sameDocumentGlobal = document.createElement("div");
              documentTarget.appendChild(sameDocumentGlobal);
              sameDocumentShadow.appendChild(sameDocumentGlobal);

              const shadowGlobalHost = document.createElement("div");
              const shadowGlobal = shadowGlobalHost.attachShadow({ mode: "closed" });
              frameTarget.appendChild(shadowGlobalHost);

              const shadowScopedRegistry = new CustomElementRegistry();
              const shadowScopedHost = document.createElement("div");
              const shadowScoped = shadowScopedHost.attachShadow({
                mode: "closed",
                customElementRegistry: shadowScopedRegistry
              });
              frameTarget.appendChild(shadowScopedHost);

              return JSON.stringify({
                implicitGlobalRetargeted:
                  implicitGlobal.customElementRegistry === frame.contentWindow.customElements,
                explicitGlobalRetargeted:
                  explicitGlobal.customElementRegistry === frame.contentWindow.customElements,
                scopedPreserved:
                  scopedElement.customElementRegistry === scoped,
                sameDocumentGlobalPreserved:
                  sameDocumentGlobal.customElementRegistry === customElements,
                shadowGlobalRetargeted:
                  shadowGlobal.customElementRegistry === frame.contentWindow.customElements,
                shadowScopedOptionPreserved:
                  shadowScoped.customElementRegistry === shadowScopedRegistry
              });
            })()
            "#,
        )
        .expect("custom element registry adoption probe should evaluate");

    assert_eq!(
        result,
        r#"{"implicitGlobalRetargeted":true,"explicitGlobalRetargeted":true,"scopedPreserved":true,"sameDocumentGlobalPreserved":true,"shadowGlobalRetargeted":true,"shadowScopedOptionPreserved":true}"#
    );
}

#[test]
fn document_import_node_options_and_registry_fallback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const fallback = new CustomElementRegistry();
              const scoped = new CustomElementRegistry();
              class GlobalElement extends HTMLElement {}
              class FallbackElement extends HTMLElement {}
              class ScopedElement extends HTMLElement {}
              customElements.define("wpt-import-shared", GlobalElement);
              fallback.define("wpt-import-shared", FallbackElement);
              scoped.define("wpt-import-shared", ScopedElement);

              const root = document.createElement("div");
              root.appendChild(document.createElement("span"));
              const defaultClone = document.importNode(root);
              const dictClone = document.importNode(root, {});
              const selfOnlyClone = document.importNode(root, { selfOnly: true });
              let nullRegistryError = "none";
              try {
                document.importNode(root, { customElementRegistry: null });
              } catch (error) {
                nullRegistryError = error && error.name;
              }

              const detached = document.implementation
                .createHTMLDocument()
                .createElement("wpt-import-shared");
              const fallbackClone = document.importNode(detached, {
                customElementRegistry: fallback
              });
              const globalSource = document.createElement("wpt-import-shared");
              const globalClone = document.importNode(globalSource, {
                customElementRegistry: fallback
              });

              const container = document.createElement("div", {
                customElementRegistry: null
              });
              const child = document.createElement("wpt-import-shared", {
                customElementRegistry: scoped
              });
              const grandchild = document.createElement("wpt-import-shared", {
                customElementRegistry: null
              });
              child.appendChild(grandchild);
              container.appendChild(child);
              const imported = document.importNode(container, {
                customElementRegistry: fallback
              });
              const importedChild = imported.firstElementChild;
              const importedGrandchild = importedChild.firstElementChild;

              return JSON.stringify({
                defaultCloneHasChildren: defaultClone.hasChildNodes(),
                dictCloneHasChildren: dictClone.hasChildNodes(),
                selfOnlyCloneHasChildren: selfOnlyClone.hasChildNodes(),
                nullRegistryError,
                fallbackCloneRegistry: fallbackClone.customElementRegistry === fallback,
                fallbackCloneInstance: fallbackClone instanceof FallbackElement,
                globalCloneRegistry: globalClone.customElementRegistry === customElements,
                globalCloneInstance: globalClone instanceof GlobalElement,
                importedRegistry: imported.customElementRegistry === fallback,
                importedChildRegistry: importedChild.customElementRegistry === scoped,
                importedGrandchildRegistry:
                  importedGrandchild.customElementRegistry === fallback,
                importedGrandchildInstance: importedGrandchild instanceof FallbackElement
              });
            })()
            "#,
        )
        .expect("Document.importNode registry fallback probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaultCloneHasChildren":false,"dictCloneHasChildren":true,"selfOnlyCloneHasChildren":false,"nullRegistryError":"TypeError","fallbackCloneRegistry":true,"fallbackCloneInstance":true,"globalCloneRegistry":true,"globalCloneInstance":true,"importedRegistry":true,"importedChildRegistry":true,"importedGrandchildRegistry":true,"importedGrandchildInstance":true}"#
    );
}

#[test]
fn document_import_node_retargets_cross_document_registry_associations() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const target = document.body || document.documentElement || document;
              target.appendChild(frame);
              const childDocument = frame.contentDocument;
              const ChildHTMLElement = frame.contentWindow.HTMLElement;
              class ChildElement extends ChildHTMLElement {}
              childDocument.defaultView.customElements.define(
                "wpt-import-child",
                ChildElement
              );

              const scoped = new CustomElementRegistry();
              const implicitGlobal = document.createElement("div");
              const explicitGlobal = document.createElement("div", {
                customElementRegistry: customElements
              });
              const scopedElement = document.createElement("div", {
                customElementRegistry: scoped
              });

              const childImplicit = childDocument.importNode(implicitGlobal);
              const childExplicit = childDocument.importNode(explicitGlobal);
              const childScoped = childDocument.importNode(scopedElement);

              const shadowGlobalHost = document.createElement("div");
              shadowGlobalHost.attachShadow({ mode: "open", clonable: true });
              const shadowScopedHost = document.createElement("div");
              shadowScopedHost.attachShadow({
                mode: "open",
                clonable: true,
                customElementRegistry: scoped
              });
              const childShadowGlobal = childDocument.importNode(shadowGlobalHost);
              const childShadowScoped = childDocument.importNode(shadowScopedHost);

              const nullDocument = document.implementation.createHTMLDocument();
              const nullGlobal = nullDocument.importNode(implicitGlobal);
              const nullScoped = nullDocument.importNode(scopedElement);
              const nullShadowGlobal = nullDocument.importNode(shadowGlobalHost);
              const nullShadowScoped = nullDocument.importNode(shadowScopedHost);

              return JSON.stringify({
                childImplicit:
                  childImplicit.customElementRegistry === frame.contentWindow.customElements,
                childExplicit:
                  childExplicit.customElementRegistry === frame.contentWindow.customElements,
                childScoped: childScoped.customElementRegistry === scoped,
                childShadowGlobal:
                  childShadowGlobal.shadowRoot.customElementRegistry ===
                    frame.contentWindow.customElements,
                childShadowScoped:
                  childShadowScoped.shadowRoot.customElementRegistry === scoped,
                nullGlobal: nullGlobal.customElementRegistry === null,
                nullScoped: nullScoped.customElementRegistry === scoped,
                nullShadowGlobal:
                  nullShadowGlobal.shadowRoot.customElementRegistry === null,
                nullShadowScoped:
                  nullShadowScoped.shadowRoot.customElementRegistry === scoped
              });
            })()
            "#,
        )
        .expect("Document.importNode cross-document registry probe should evaluate");

    assert_eq!(
        result,
        r#"{"childImplicit":true,"childExplicit":true,"childScoped":true,"childShadowGlobal":true,"childShadowScoped":true,"nullGlobal":true,"nullScoped":true,"nullShadowGlobal":true,"nullShadowScoped":true}"#
    );
}

#[test]
fn custom_element_registry_associations_survive_with_retained_detached_nodes() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let initial_associations = vm.custom_element_registry_association_count_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const documentTarget = document.body || document.documentElement || document;
              documentTarget.appendChild(frame);
              const frameTarget = frame.contentDocument.body ||
                frame.contentDocument.appendChild(frame.contentDocument.createElement("body"));

              const adopted = frame.contentDocument.createElement(
                "div",
                { customElementRegistry: null }
              );
              frameTarget.appendChild(adopted);
              window.__moliRegistryCleanupFrame = frame;
              window.__moliRegistryCleanupAdopted = adopted;
              return adopted.customElementRegistry === null;
            })()
            "#,
        )
        .expect("custom element registry child teardown setup should evaluate");

    assert_eq!(result, "true");
    assert!(
        vm.custom_element_registry_association_count_for_test() > initial_associations,
        "child document element should create an explicit registry association"
    );

    let retained_registry = vm
        .eval(
            r#"
            (() => {
              const frame = window.__moliRegistryCleanupFrame;
              frame.parentNode.removeChild(frame);
              return window.__moliRegistryCleanupAdopted.customElementRegistry === null;
            })()
            "#,
        )
        .expect("custom element registry child teardown removal should evaluate");

    assert_eq!(retained_registry, "true");

    assert_eq!(
        vm.custom_element_registry_association_count_for_test(),
        initial_associations + 2,
        "the detached document registry and retained element's explicit null registry must survive"
    );
}

#[test]
fn custom_elements_upgrade_fails_when_definition_disables_existing_shadow() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              let errorName = null;
              window.addEventListener("error", event => {
                event.preventDefault();
                errorName = event.error && event.error.name;
              }, { once: true });

              class ShadowDisabledElement extends HTMLElement {
                static get disabledFeatures() { return ["shadow"]; }
              }
              const element = document.createElement("wpt-shadow-disabled-upgrade");
              element.attachShadow({ mode: "open" });
              customElements.define("wpt-shadow-disabled-upgrade", ShadowDisabledElement);
              customElements.upgrade(element);

              return [
                element instanceof ShadowDisabledElement,
                errorName
              ].join("|");
            })()
            "#,
        )
        .expect("disabled shadow custom element upgrade probe should evaluate");

    assert_eq!(result, "false|NotSupportedError");
}

#[test]
fn custom_elements_do_not_upgrade_in_documents_without_browsing_context() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              class DetachedPredefined extends HTMLElement {}
              customElements.define("wpt-detached-predefined", DetachedPredefined);

              const template = document.createElement("template");
              const templateDoc = template.content.ownerDocument;
              if (!templateDoc.documentElement)
                templateDoc.appendChild(templateDoc.createElement("html"));
              const htmlDoc = document.implementation.createHTMLDocument("");

              return [
                templateDoc.createElement("wpt-detached-predefined") instanceof DetachedPredefined,
                htmlDoc.createElement("wpt-detached-predefined") instanceof DetachedPredefined
              ].join("|");
            })()
            "#,
        )
        .expect("detached document custom element upgrade probe should evaluate");

    assert_eq!(result, "false|false");
}

#[test]
fn unresolved_custom_elements_in_detached_documents_use_current_realm_prototypes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const XHTML_NS = "http://www.w3.org/1999/xhtml";
              const plain = new Document();
              plain.appendChild(plain.createElement("html"));
              const html = document.implementation.createHTMLDocument("");
              const xhtml = document.implementation.createDocument(XHTML_NS, "html", null);

              const plainElement = plain.createElement("wpt-detached-plain");
              const htmlElement = html.createElement("wpt-detached-html");
              const xhtmlElement = xhtml.createElement("wpt-detached-xhtml");
              const plainProto = Object.getPrototypeOf(plainElement);
              const htmlProto = Object.getPrototypeOf(htmlElement);
              const xhtmlProto = Object.getPrototypeOf(xhtmlElement);

              return [
                plain.defaultView === null,
                html.defaultView === null,
                xhtml.defaultView === null,
                plainProto === Element.prototype,
                htmlProto === HTMLElement.prototype,
                xhtmlProto === HTMLElement.prototype,
                Object.prototype.hasOwnProperty.call(htmlElement, "appendChild"),
                Object.prototype.hasOwnProperty.call(htmlElement, "setAttribute"),
                typeof htmlElement.appendChild,
                typeof htmlElement.setAttribute
              ].join("|");
            })()
            "#,
        )
        .expect("detached document prototype probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|false|false|function|function"
    );
}

#[test]
fn unresolved_custom_elements_adopted_from_detached_documents_upgrade_in_live_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const XHTML_NS = "http://www.w3.org/1999/xhtml";
              const html = document.implementation.createHTMLDocument("");
              const xhtml = document.implementation.createDocument(XHTML_NS, "html", null);
              const htmlElement = html.createElement("wpt-adopted-html");
              const xhtmlElement = xhtml.createElement("wpt-adopted-xhtml");

              class AdoptedHtml extends HTMLElement {}
              class AdoptedXhtml extends HTMLElement {}
              customElements.define("wpt-adopted-html", AdoptedHtml);
              customElements.define("wpt-adopted-xhtml", AdoptedXhtml);

              const target = document.body || document.documentElement || document;
              target.appendChild(htmlElement);
              target.appendChild(xhtmlElement);

              return [
                htmlElement.customElementRegistry === customElements,
                xhtmlElement.customElementRegistry === customElements,
                htmlElement instanceof AdoptedHtml,
                xhtmlElement instanceof AdoptedXhtml,
                Object.getPrototypeOf(htmlElement) === AdoptedHtml.prototype,
                Object.getPrototypeOf(xhtmlElement) === AdoptedXhtml.prototype
              ].join("|");
            })()
            "#,
        )
        .expect("detached document adoption upgrade probe should evaluate");

    assert_eq!(result, "true|true|true|true|true|true");
}

#[test]
fn non_html_namespace_custom_elements_do_not_upgrade_after_adoption() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const plain = new Document();
              plain.appendChild(plain.createElement("html"));
              const element = plain.createElement("wpt-non-html-upgrade");
              let constructed = 0;

              class NonHtmlUpgrade extends HTMLElement {
                constructor() {
                  super();
                  constructed += 1;
                }
              }
              customElements.define("wpt-non-html-upgrade", NonHtmlUpgrade);

              (document.body || document.documentElement || document).appendChild(element);

              return [
                element.namespaceURI === null,
                Object.getPrototypeOf(element) === Element.prototype,
                element instanceof NonHtmlUpgrade,
                constructed
              ].join("|");
            })()
            "#,
        )
        .expect("non-HTML namespace custom element probe should evaluate");

    assert_eq!(result, "true|true|false|0");
}

#[test]
fn unresolved_custom_elements_in_iframe_documents_use_child_realm_prototypes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const doc = frame.contentDocument;
              const win = frame.contentWindow;
              const element = doc.createElement("wpt-frame-prototype");
              const errors = [];
              window.addEventListener("error", (event) => {
                errors.push(event.error && event.error.name + ":" + event.error.message);
              });

              let constructed = 0;
              class FrameElement extends win.HTMLElement {
                constructor() {
                  super();
                  constructed += 1;
                }
              }
              win.customElements.define("wpt-frame-prototype", FrameElement);
              const beforeInsert = Object.getPrototypeOf(element) === win.HTMLElement.prototype;
              const registryBeforeInsert = element.customElementRegistry === win.customElements;
              const definedBeforeInsert =
                win.customElements.get("wpt-frame-prototype") === FrameElement;
              doc.documentElement.appendChild(element);
              const upgradedAfterInsert =
                element instanceof FrameElement &&
                Object.getPrototypeOf(element) === FrameElement.prototype;

              return [
                beforeInsert,
                registryBeforeInsert,
                definedBeforeInsert,
                element.parentNode === doc.documentElement,
                doc.documentElement.lastChild === element,
                element.isConnected,
                upgradedAfterInsert,
                element instanceof FrameElement,
                Object.getPrototypeOf(element) === FrameElement.prototype,
                constructed,
                errors.join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("iframe document prototype probe should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|true|true|true|1|");
}

#[test]
fn iframe_html_table_element_reactions_use_child_realm_table_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const doc = frame.contentDocument;
              const win = frame.contentWindow;
              const calls = [];

              class TableChild extends win.HTMLElement {
                constructor() {
                  super();
                  calls.push("constructed");
                }
                connectedCallback() { calls.push("connected"); }
                disconnectedCallback() { calls.push("disconnected"); }
              }
              win.customElements.define("wpt-frame-table-child", TableChild);

              doc.body.innerHTML = "<table></table>";
              const table = doc.querySelector("table");
              const captionDescriptor =
                Object.getOwnPropertyDescriptor(Object.getPrototypeOf(table), "caption");

              const caption = doc.createElement("caption");
              caption.innerHTML = "<wpt-frame-table-child>cap</wpt-frame-table-child>";
              const captionConstructed = calls.join("|");
              calls.length = 0;
              table.caption = caption;
              const captionConnected = calls.join("|");
              const captionIdentity = table.caption === caption;
              calls.length = 0;
              table.deleteCaption();
              const captionDisconnected = calls.join("|");
              calls.length = 0;

              const thead = doc.createElement("thead");
              thead.innerHTML =
                "<tr><td><wpt-frame-table-child>head</wpt-frame-table-child></td></tr>";
              const theadInnerHTML = thead.innerHTML;
              const theadConstructed = calls.join("|");
              calls.length = 0;
              table.tHead = thead;
              const theadConnected = calls.join("|");
              const rowState = `${table.rows.length}:${table.rows[0] === thead.firstElementChild}`;
              calls.length = 0;
              table.deleteRow(0);
              const rowRemoved = table.rows.length;
              const rowDisconnected = calls.join("|");

              return [
                typeof captionDescriptor.get,
                typeof captionDescriptor.set,
                table.caption === null,
                caption.innerHTML,
                captionConstructed,
                captionConnected,
                captionIdentity,
                captionDisconnected,
                theadInnerHTML,
                theadConstructed,
                theadConnected,
                rowState,
                rowRemoved,
                rowDisconnected
              ].join("||");
            })()
            "#,
        )
        .expect("iframe HTMLTableElement reaction probe should evaluate");

    assert_eq!(
        result,
        "function||function||true||<wpt-frame-table-child>cap</wpt-frame-table-child>||constructed||connected||true||disconnected||<tr><td><wpt-frame-table-child>head</wpt-frame-table-child></td></tr>||constructed||connected||1:true||0||disconnected"
    );
}

#[test]
fn iframe_html_table_row_and_section_reactions_use_child_realm_table_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const doc = frame.contentDocument;
              const win = frame.contentWindow;
              const calls = [];

              class TableChild extends win.HTMLElement {
                constructor() {
                  super();
                  calls.push("constructed");
                }
                connectedCallback() { calls.push("connected"); }
                disconnectedCallback() { calls.push("disconnected"); }
              }
              win.customElements.define("wpt-frame-table-detail", TableChild);

              doc.body.innerHTML = `
                <table id="row-table">
                  <tr><td><wpt-frame-table-detail>cell</wpt-frame-table-detail></td></tr>
                </table>
                <table id="head-table">
                  <thead><tr><td><wpt-frame-table-detail>head</wpt-frame-table-detail></td></tr></thead>
                </table>
                <table id="foot-table">
                  <tfoot><tr><td><wpt-frame-table-detail>foot</wpt-frame-table-detail></td></tr></tfoot>
                </table>
              `;

              const row = doc.querySelector("#row-table").rows[0];
              const td = row.cells[0];
              const rowSurface = [
                typeof win.HTMLTableRowElement,
                row instanceof win.HTMLTableRowElement,
                Object.getPrototypeOf(row) === win.HTMLTableRowElement.prototype,
                typeof row.cells,
                row.cells.length,
                row.cells[0] === td,
                td.cellIndex,
                typeof row.insertCell,
                typeof row.deleteCell
              ].join(":");
              calls.length = 0;
              row.deleteCell(0);
              const rowDelete = `${row.cells.length}:${calls.join("|")}`;

              const thead = doc.querySelector("#head-table").tHead;
              const headSurface = [
                typeof win.HTMLTableSectionElement,
                thead instanceof win.HTMLTableSectionElement,
                Object.getPrototypeOf(thead) === win.HTMLTableSectionElement.prototype,
                thead.rows.length,
                typeof thead.insertRow,
                typeof thead.deleteRow
              ].join(":");
              calls.length = 0;
              thead.deleteRow(0);
              const headDelete = `${thead.rows.length}:${calls.join("|")}`;

              const tfoot = doc.querySelector("#foot-table").tFoot;
              calls.length = 0;
              tfoot.deleteRow(0);
              const footDelete = `${tfoot.rows.length}:${calls.join("|")}`;

              return JSON.stringify({
                rowSurface,
                rowDelete,
                headSurface,
                headDelete,
                footDelete
              });
            })()
            "##,
        )
        .expect("iframe table row/section reaction probe should evaluate");

    assert_eq!(
        result,
        r#"{"rowSurface":"function:true:true:object:1:true:0:function:function","rowDelete":"0:disconnected","headSurface":"function:true:true:1:function:function","headDelete":"0:disconnected","footDelete":"0:disconnected"}"#
    );
}

#[test]
fn range_extract_contents_disconnects_custom_element_moved_to_fragment() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              customElements.define("wpt-range-extract-child", class extends HTMLElement {
                connectedCallback() { calls.push(`connected:${this.isConnected}`); }
                disconnectedCallback() { calls.push(`disconnected:${this.isConnected}`); }
              });
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const host = document.createElement("div");
              document.body.appendChild(host);
              const child = document.createElement("wpt-range-extract-child");
              host.appendChild(child);
              const initial = calls.join("|");
              calls.length = 0;

              const range = document.createRange();
              range.selectNode(child);
              const fragment = range.extractContents();

              return [
                initial,
                calls.join("|"),
                fragment.firstChild === child,
                host.childNodes.length,
                child.isConnected
              ].join("||");
            })()
            "#,
        )
        .expect("range extract custom element lifecycle probe should evaluate");

    assert_eq!(result, "connected:true||disconnected:false||true||0||false");
}

#[test]
fn selection_delete_from_document_disconnects_custom_element() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              customElements.define("wpt-selection-delete-child", class extends HTMLElement {
                connectedCallback() { calls.push(`connected:${this.isConnected}`); }
                disconnectedCallback() { calls.push(`disconnected:${this.isConnected}`); }
              });
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const host = document.createElement("div");
              document.body.appendChild(host);
              const child = document.createElement("wpt-selection-delete-child");
              host.appendChild(child);
              host.prepend(document.createTextNode("start"));
              host.append(document.createTextNode("end"));
              const initial = calls.join("|");
              calls.length = 0;

              const selection = getSelection();
              selection.selectAllChildren(host);
              const range = selection.getRangeAt(0);
              selection.deleteFromDocument();
              const collapsedRange = selection.getRangeAt(0);

              return [
                initial,
                calls.join("|"),
                host.childNodes.length,
                child.isConnected,
                collapsedRange === range,
                selection.anchorNode === host,
                selection.anchorOffset,
                selection.focusNode === host,
                selection.focusOffset
              ].join("||");
            })()
            "#,
        )
        .expect("selection delete custom element lifecycle probe should evaluate");

    assert_eq!(
        result,
        "connected:true||disconnected:false||0||false||true||true||0||true||0"
    );
}

#[test]
fn animation_commit_styles_enqueues_style_attribute_reactions() {
    let mut vm = new_storage_test_vm("https://example.com/");

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

              const observedCalls = [];
              customElements.define("wpt-animation-style-child", class extends HTMLElement {
                static get observedAttributes() { return ["style"]; }
                connectedCallback() { observedCalls.push("connected"); }
                attributeChangedCallback(name, oldValue, newValue, namespace) {
                  observedCalls.push(`${name}:${oldValue}:${newValue}:${namespace}`);
                }
              });
              const observed = document.createElement("wpt-animation-style-child");
              document.body.appendChild(observed);
              const observedSetup = observedCalls.splice(0).join("|");

              const firstAnimation = observed.animate([{borderColor: "rgb(0, 0, 255)"}], 1);
              const afterAnimate = observedCalls.splice(0).join("|");
              firstAnimation.commitStyles();
              const afterFirstCommit = observedCalls.splice(0).join("|");
              const firstStyle = observed.getAttribute("style");

              const secondAnimation = observed.animate([{borderColor: "rgb(0, 255, 0)"}], 1);
              const afterSecondAnimate = observedCalls.splice(0).join("|");
              secondAnimation.commitStyles();
              const afterSecondCommit = observedCalls.splice(0).join("|");
              const secondStyle = observed.getAttribute("style");

              const unobservedCalls = [];
              customElements.define("wpt-animation-unobserved-child", class extends HTMLElement {
                connectedCallback() { unobservedCalls.push("connected"); }
                attributeChangedCallback(name) { unobservedCalls.push(name); }
              });
              const unobserved = document.createElement("wpt-animation-unobserved-child");
              document.body.appendChild(unobserved);
              const unobservedSetup = unobservedCalls.splice(0).join("|");
              unobserved.animate([{borderColor: "rgb(0, 0, 255)"}], 1).commitStyles();
              const unobservedAfterCommit = unobservedCalls.splice(0).join("|");

              return JSON.stringify({
                observedSetup,
                afterAnimate,
                afterFirstCommit,
                firstStyle,
                afterSecondAnimate,
                afterSecondCommit,
                secondStyle,
                unobservedSetup,
                unobservedAfterCommit,
                unobservedStyle: unobserved.getAttribute("style")
              });
            })()
            "#,
        )
        .expect("animation commitStyles custom-element reaction probe should evaluate");

    assert_eq!(
        result,
        r#"{"observedSetup":"connected","afterAnimate":"","afterFirstCommit":"style:null:border-color: rgb(0, 0, 255);:null","firstStyle":"border-color: rgb(0, 0, 255);","afterSecondAnimate":"","afterSecondCommit":"style:border-color: rgb(0, 0, 255);:border-color: rgb(0, 255, 0);:null","secondStyle":"border-color: rgb(0, 255, 0);","unobservedSetup":"connected","unobservedAfterCommit":"","unobservedStyle":"border-color: rgb(0, 0, 255);"}"#
    );
}

#[test]
fn range_clone_and_contextual_fragment_deliver_initial_attribute_reactions() {
    let mut vm = new_storage_test_vm("https://example.com/");

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

              const cloneCalls = [];
              class CloneChild extends HTMLElement {
                static get observedAttributes() { return ["id"]; }
                constructor() {
                  super();
                  cloneCalls.push("constructed");
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  cloneCalls.push(`${name}:${oldValue}:${newValue}`);
                }
              }
              customElements.define("wpt-range-clone-child", CloneChild);
              const cloneHost = document.createElement("div");
              document.body.appendChild(cloneHost);
              const source = document.createElement("wpt-range-clone-child");
              cloneHost.appendChild(source);
              source.id = "source";
              cloneCalls.length = 0;
              const cloneRange = document.createRange();
              cloneRange.selectNode(source);
              const cloned = cloneRange.cloneContents().firstChild;

              const fragmentCalls = [];
              class FragmentChild extends HTMLElement {
                static get observedAttributes() { return ["id"]; }
                constructor() {
                  super();
                  fragmentCalls.push("constructed");
                }
                attributeChangedCallback(name, oldValue, newValue) {
                  fragmentCalls.push(`${name}:${oldValue}:${newValue}`);
                }
              }
              customElements.define("wpt-range-fragment-child", FragmentChild);
              const fragmentRange = document.createRange();
              fragmentRange.selectNodeContents(document.body);
              const fragment = fragmentRange.createContextualFragment(
                '<wpt-range-fragment-child id="fragment"></wpt-range-fragment-child>'
              );
              const parsed = fragment.firstChild;

              return [
                cloneCalls.join("|"),
                cloned instanceof CloneChild,
                cloned.id,
                fragmentCalls.join("|"),
                parsed instanceof FragmentChild,
                parsed.id
              ].join("||");
            })()
            "#,
        )
        .expect("range clone/contextual fragment initial attributes should evaluate");

    assert_eq!(
        result,
        "constructed|id:null:source||true||source||constructed|id:null:fragment||true||fragment"
    );
}

#[test]
fn range_partial_clone_extract_constructs_custom_elements_in_tree_order() {
    let mut vm = new_storage_test_vm("https://example.com/");

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

              document.body.innerHTML = `
                <c-e id="root">
                  <c-e id="root-0">
                    <c-e id="root-0-0">
                      <c-e id="root-0-0-0"></c-e>
                      <span id="start"></span>
                    </c-e>
                  </c-e>
                  <c-e id="root-1"></c-e>
                  <span id="end"></span>
                </c-e>`;

              const logs = [];
              class CE extends HTMLElement {
                constructor() {
                  super();
                  logs.push(this.id);
                }
              }
              customElements.define("c-e", CE);

              function getRange() {
                const range = new Range();
                range.setStart(document.getElementById("start"), 0);
                range.setEnd(document.getElementById("end"), 0);
                return range;
              }

              logs.length = 0;
              getRange().cloneContents();
              const cloneLog = logs.join(",");

              logs.length = 0;
              getRange().extractContents();
              const extractLog = logs.join(",");

              return `${cloneLog}|${extractLog}`;
            })()
            "#,
        )
        .expect("Range partial custom element construction probe should evaluate");

    assert_eq!(result, "root-0,root-0-0,root-1|root-0,root-0-0");
}

#[test]
fn range_insert_node_into_detached_document_dispatches_adoption_lifecycle() {
    let mut vm = new_storage_test_vm("https://example.com/");

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
              const calls = [];
              const targetDoc = document.implementation.createHTMLDocument("");
              customElements.define("wpt-range-adopt-child", class extends HTMLElement {
                connectedCallback() { calls.push(`connected:${this.ownerDocument === targetDoc}`); }
                disconnectedCallback() { calls.push(`disconnected:${this.isConnected}`); }
                adoptedCallback(oldDocument, newDocument) {
                  calls.push(`adopted:${oldDocument === document}:${newDocument === targetDoc}`);
                }
              });
              const child = document.createElement("wpt-range-adopt-child");
              document.body.appendChild(child);
              calls.length = 0;

              const range = document.createRange();
              range.selectNodeContents(targetDoc.documentElement);
              range.insertNode(child);

              return [
                calls.join("|"),
                child.ownerDocument === targetDoc,
                targetDoc.documentElement.firstChild === child
              ].join("||");
            })()
            "#,
        )
        .expect("range insertNode detached document adoption probe should evaluate");

    assert_eq!(
        result,
        "disconnected:true|adopted:true:true|connected:true||true||true"
    );
}

#[test]
fn defined_pseudo_tracks_autonomous_upgrade_and_style_invalidation() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.head) {
                document.documentElement.appendChild(document.createElement("head"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const style = document.createElement("style");
              style.textContent = `
                wpt-defined-later:defined { color: rgb(255, 165, 0); }
                wpt-defined-later:not(:defined) { color: rgb(0, 0, 255); }
              `;
              document.head.appendChild(style);

              const during = [];
              const element = document.createElement("wpt-defined-later");
              document.body.appendChild(element);
              const before = [
                element.matches(":defined"),
                getComputedStyle(element).color
              ].join("|");

              customElements.define("wpt-defined-later", class extends HTMLElement {
                constructor() {
                  during.push(element.matches(":defined"));
                  super();
                  during.push(this.matches(":defined"));
                }
              });

              return [
                before,
                during.join("|"),
                element.matches(":defined"),
                getComputedStyle(element).color
              ].join("||");
            })()
            "#,
        )
        .expect("defined pseudo autonomous probe should evaluate");

    assert_eq!(
        result,
        "false|rgb(0, 0, 255)||false|false||true||rgb(255, 165, 0)"
    );
}

#[test]
fn defined_pseudo_tracks_customized_builtin_candidates() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.head) {
                document.documentElement.appendChild(document.createElement("head"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const style = document.createElement("style");
              style.textContent = `
                p:defined { color: rgb(255, 165, 0); }
                p:not(:defined) { color: rgb(0, 0, 255); }
              `;
              document.head.appendChild(style);

              const direct = document.createElement("p", { is: "wpt-defined-p" });
              const ns = document.createElementNS(
                "http://www.w3.org/1999/xhtml",
                "p",
                { is: "wpt-defined-ns-p" }
              );
              const empty = document.createElement("p", { is: "" });
              document.body.appendChild(direct);
              document.body.appendChild(ns);
              document.body.appendChild(empty);
              const before = [
                direct.getAttribute("is"),
                direct.matches(":defined"),
                ns.getAttribute("is"),
                ns.matches(":defined"),
                empty.matches(":defined"),
                getComputedStyle(direct).color
              ].join("|");

              customElements.define("wpt-defined-p", class extends HTMLElement {}, { extends: "p" });
              customElements.define("wpt-defined-ns-p", class extends HTMLElement {}, { extends: "p" });

              return [
                before,
                direct.matches(":defined"),
                ns.matches(":defined"),
                empty.matches(":defined"),
                getComputedStyle(direct).color
              ].join("||");
            })()
            "#,
        )
        .expect("defined pseudo customized built-in probe should evaluate");

    assert_eq!(
        result,
        "|false||false|false|rgb(0, 0, 255)||true||true||false||rgb(255, 165, 0)"
    );
}

#[test]
fn document_create_element_customized_builtin_options_are_internal_state() {
    let mut vm = new_storage_test_vm("https://example.com/");

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
              const xhtml = "http://www.w3.org/1999/xhtml";

              class SuperP extends HTMLParagraphElement {}
              customElements.define("wpt-string-super-p", SuperP, { extends: "p" });
              const stringP = document.createElement("p", "wpt-string-super-p");
              const stringNsP = document.createElementNS(xhtml, "p", "wpt-string-super-p");

              const late = document.createElement("div", { is: "wpt-late-div" });
              const lateBefore = [
                late instanceof HTMLDivElement,
                late.hasAttribute("is"),
                late.matches(":defined")
              ].join("/");
              class LateDiv extends HTMLDivElement {}
              customElements.define("wpt-late-div", LateDiv, { extends: "div" });
              document.body.appendChild(late);

              let prefixDuringConstructor;
              let tagDuringConstructor;
              class PrefixTimingEl extends HTMLElement {
                constructor() {
                  super();
                  prefixDuringConstructor = this.prefix;
                  tagDuringConstructor = this.tagName;
                }
              }
              customElements.define("wpt-prefix-timing-el", PrefixTimingEl);
              const prefixed = document.createElementNS(
                xhtml,
                "p:wpt-prefix-timing-el"
              );

              let innerPrefix;
              let outerPrefixDuringReentrantConstructor;
              class ReentrantPrefixEl extends HTMLElement {
                static callCount = 0;
                constructor() {
                  super();
                  if (ReentrantPrefixEl.callCount++ === 0) {
                    const inner = new ReentrantPrefixEl();
                    innerPrefix = inner.prefix;
                  }
                  outerPrefixDuringReentrantConstructor = this.prefix;
                }
              }
              customElements.define("wpt-reentrant-prefix-el", ReentrantPrefixEl);
              const reentrantPrefixed = document.createElementNS(
                xhtml,
                "r:wpt-reentrant-prefix-el"
              );

              class BuiltinAddress extends HTMLElement {}
              customElements.define(
                "wpt-built-address",
                BuiltinAddress,
                { extends: "address" }
              );
              const built = document.createElementNS(
                xhtml,
                "q:address",
                { is: "wpt-built-address" }
              );

              return JSON.stringify({
                stringOptionsIgnored: [
                  stringP instanceof HTMLParagraphElement,
                  stringP instanceof SuperP,
                  stringP.hasAttribute("is"),
                  stringNsP instanceof HTMLParagraphElement,
                  stringNsP instanceof SuperP,
                  stringNsP.hasAttribute("is")
                ].join("/"),
                lateBefore,
                lateAfter: [
                late instanceof LateDiv,
                late.hasAttribute("is"),
                late.matches(":defined")
              ].join("/"),
                lateOuterHTML: late.outerHTML,
                prefixDuringConstructor,
                tagDuringConstructor,
                prefixedAfter: [prefixed.prefix, prefixed.tagName].join("/"),
                reentrantPrefix: [
                  outerPrefixDuringReentrantConstructor,
                  innerPrefix,
                  reentrantPrefixed.prefix
                ],
                built: [
                  built instanceof BuiltinAddress,
                  built.prefix,
                  built.localName,
                  built.hasAttribute("is")
                ].join("/")
              });
            })()
            "#,
        )
        .expect("customized built-in createElement options probe should evaluate");

    assert_eq!(
        result,
        r#"{"stringOptionsIgnored":"true/false/false/true/false/false","lateBefore":"true/false/false","lateAfter":"true/false/true","lateOuterHTML":"<div is=\"wpt-late-div\"></div>","prefixDuringConstructor":null,"tagDuringConstructor":"WPT-PREFIX-TIMING-EL","prefixedAfter":"p/P:WPT-PREFIX-TIMING-EL","reentrantPrefix":[null,null,"r"],"built":"true/q/address/false"}"#
    );
}

#[test]
fn defined_pseudo_tracks_detached_scoped_registry_initialize() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const plain = new Document();
              const uncustomized = plain.createElement("blah");
              const registry = new CustomElementRegistry();
              registry.define("wpt-defined-scoped", class extends HTMLElement {});
              const candidate = plain.createElementNS(
                "http://www.w3.org/1999/xhtml",
                "wpt-defined-scoped"
              );
              const before = [
                uncustomized.namespaceURI,
                uncustomized.matches(":defined"),
                candidate.namespaceURI,
                candidate.matches(":defined")
              ].join("|");
              registry.initialize(candidate);
              return [
                before,
                candidate.customElementRegistry === registry,
                candidate.matches(":defined")
              ].join("||");
            })()
            "#,
        )
        .expect("defined pseudo scoped registry detached probe should evaluate");

    assert_eq!(
        result,
        "|true|http://www.w3.org/1999/xhtml|false||true||true"
    );
}

#[test]
fn detached_defined_pseudo_matches_ascii_case_insensitively() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const registry = new CustomElementRegistry();
              registry.define("wpt-defined-case", class extends HTMLElement {});
              const candidate = doc.createElement("wpt-defined-case");
              const missing = doc.createElement("wpt-defined-missing");
              doc.body.append(candidate, missing);
              registry.initialize(candidate);
              return [
                candidate.matches(":Defined"),
                candidate.matches("wpt-defined-case:DEFINED"),
                missing.matches(":DEFINED"),
                doc.body.querySelector("wpt-defined-case:Defined") === candidate,
                doc.body.querySelector("wpt-defined-missing:DEFINED") === null
              ].join("|");
            })()
            "#,
        )
        .expect("detached :defined case-insensitive probe should evaluate");

    assert_eq!(result, "true|true|false|true|true");
}

#[test]
fn custom_element_connected_callback_runs_when_shadow_root_children_move_to_new_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class ShadowMoveElement extends HTMLElement {
                connectedCallback() { calls.push("connected"); }
                disconnectedCallback() { calls.push("disconnected"); }
                adoptedCallback(oldDocument, newDocument) {
                  calls.push("adopted");
                  calls.push(oldDocument === document);
                  calls.push(newDocument === targetDoc);
                }
              }
              customElements.define("wpt-shadow-move", ShadowMoveElement);

              const targetDoc = document.implementation.createHTMLDocument("");
              const instance = document.createElement("wpt-shadow-move");
              const host = document.createElement("div");
              const shadowRoot = host.attachShadow({ mode: "closed" });
              shadowRoot.appendChild(instance);
              (document.body || document.documentElement || document).appendChild(host);

              calls.length = 0;
              targetDoc.documentElement.appendChild(shadowRoot);
              return calls.join("|");
            })()
            "#,
        )
        .expect("shadow root move custom element lifecycle probe should evaluate");

    assert_eq!(result, "disconnected|adopted|true|true|connected");
}

#[test]
fn custom_elements_get_name_and_constructor_create_registered_element() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              class DirectElement extends HTMLElement {}
              class DirectButton extends HTMLButtonElement {}
              class OtherElement extends HTMLElement {}
              customElements.define("wpt-direct-element", DirectElement);
              customElements.define("wpt-direct-button", DirectButton, { extends: "button" });
              const autonomous = new DirectElement();
              const button = new DirectButton();
              const invalid = (() => {
                try {
                  customElements.getName({});
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              })();
              return [
                autonomous instanceof DirectElement,
                autonomous.localName,
                button instanceof DirectButton,
                button.localName,
                button.getAttribute("is"),
                customElements.getName(DirectElement),
                customElements.getName(DirectButton),
                customElements.getName(OtherElement) === null,
                invalid
              ].join("|");
            })()
            "#,
        )
        .expect("customElements.getName and direct constructor probe should evaluate");

    assert_eq!(
        result,
        "true|wpt-direct-element|true|button||wpt-direct-element|wpt-direct-button|true|TypeError"
    );
}

#[test]
fn document_create_element_reports_failed_custom_element_construction() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const events = [];
              const thrown = { name: "thrown-object" };
              window.onerror = function(message, url, line, column, error) {
                events.push(error === thrown ? "same" : error && error.name);
                return true;
              };

              class ThrowsElement extends HTMLElement {
                constructor() {
                  super();
                  throw thrown;
                }
              }
              customElements.define("wpt-fails-throw", ThrowsElement);
              const thrownFallback = document.createElement("wpt-fails-throw");

              class AttributeElement extends HTMLElement {
                constructor() {
                  super();
                  this.setAttribute("id", "bad");
                }
              }
              customElements.define("wpt-fails-attr", AttributeElement);
              const attrFallback = document.createElement("wpt-fails-attr");

              class ObjectElement extends HTMLElement {
                constructor() {
                  return { foo: "bar" };
                }
              }
              customElements.define("wpt-fails-object", ObjectElement);
              const directObject = new ObjectElement();
              const objectFallback = document.createElement("wpt-fails-object");

              class CleanElement extends HTMLElement {
                constructor() {
                  super();
                  this.setAttribute("data-temp", "1");
                  this.removeAttribute("data-temp");
                  this.appendChild(document.createElement("span"));
                  this.removeChild(this.firstChild);
                }
              }
              customElements.define("wpt-clean-constructor", CleanElement);
              const eventCountBeforeClean = events.length;
              const clean = document.createElement("wpt-clean-constructor");

              customElements.upgrade(attrFallback);

              return [
                events.join(","),
                events.length === eventCountBeforeClean,
                thrownFallback.localName,
                thrownFallback instanceof HTMLUnknownElement,
                thrownFallback instanceof ThrowsElement,
                attrFallback.localName,
                attrFallback instanceof HTMLUnknownElement,
                attrFallback instanceof AttributeElement,
                objectFallback.localName,
                objectFallback instanceof HTMLUnknownElement,
                objectFallback instanceof ObjectElement,
                directObject.foo,
                clean instanceof CleanElement,
                events.length
              ].join("|");
            })()
            "#,
        )
        .expect("custom element failed construction probe should evaluate");

    assert_eq!(
        result,
        "same,NotSupportedError,TypeError|true|wpt-fails-throw|true|false|wpt-fails-attr|true|false|wpt-fails-object|true|false|bar|true|3"
    );
}

#[test]
fn document_create_element_validates_owner_document_after_construction_adoption() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const events = [];
              window.onerror = function(message, url, line, column, error) {
                events.push(error && error.name);
                return true;
              };

              const HTML_NS = "http://www.w3.org/1999/xhtml";

              function probe(label, makeDocument) {
                const other = makeDocument();
                if (!other.documentElement) {
                  other.appendChild(other.createElement("html"));
                }

                const during = [];
                const awayName = `wpt-construction-adopt-away-${label}`;
                class AdoptAwayElement extends HTMLElement {
                  constructor() {
                    super();
                    other.adoptNode(this);
                    during.push(this.ownerDocument === other);
                  }
                }
                customElements.define(awayName, AdoptAwayElement);
                const awayEventCount = events.length;
                const away = document.createElement(awayName);

                const insertName = `wpt-construction-insert-away-${label}`;
                class InsertAwayElement extends HTMLElement {
                  constructor() {
                    super();
                    other.documentElement.appendChild(this);
                    during.push(this.ownerDocument === other);
                    during.push(this.parentNode === other.documentElement);
                  }
                }
                customElements.define(insertName, InsertAwayElement);
                const insertEventCount = events.length;
                const inserted = document.createElement(insertName);

                const backName = `wpt-construction-adopt-back-${label}`;
                class AdoptBackElement extends HTMLElement {
                  constructor() {
                    super();
                    other.adoptNode(this);
                    during.push(this.ownerDocument === other);
                    document.adoptNode(this);
                    during.push(this.ownerDocument === document);
                  }
                }
                customElements.define(backName, AdoptBackElement);
                const backEventCount = events.length;
                const back = document.createElement(backName);

                return {
                  label,
                  during,
                  awayNewEvent: events.length > awayEventCount ? events[awayEventCount] : null,
                  awayFallback: away instanceof HTMLUnknownElement,
                  awayCustom: away instanceof AdoptAwayElement,
                  awayOwner: away.ownerDocument === document,
                  insertNewEvent: events.length > insertEventCount ? events[insertEventCount] : null,
                  insertFallback: inserted instanceof HTMLUnknownElement,
                  insertCustom: inserted instanceof InsertAwayElement,
                  insertOwner: inserted.ownerDocument === document,
                  backNewEvent: events.length > backEventCount ? events[backEventCount] : null,
                  backCustom: back instanceof AdoptBackElement,
                  backFallback: back instanceof HTMLUnknownElement,
                  backOwner: back.ownerDocument === document
                };
              }

              return JSON.stringify({
                events,
                probes: [
                  probe("html", () => document.implementation.createHTMLDocument("other")),
                  probe("plain", () => new Document()),
                  probe("xhtml", () => document.implementation.createDocument(HTML_NS, "html", null))
                ]
              });
            })()
            "#,
        )
        .expect("custom element construction adoption validation probe should evaluate");

    assert_eq!(
        result,
        r#"{"events":["NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError"],"probes":[{"label":"html","during":[true,true,true,true,true],"awayNewEvent":"NotSupportedError","awayFallback":true,"awayCustom":false,"awayOwner":true,"insertNewEvent":"NotSupportedError","insertFallback":true,"insertCustom":false,"insertOwner":true,"backNewEvent":null,"backCustom":true,"backFallback":false,"backOwner":true},{"label":"plain","during":[true,true,true,true,true],"awayNewEvent":"NotSupportedError","awayFallback":true,"awayCustom":false,"awayOwner":true,"insertNewEvent":"NotSupportedError","insertFallback":true,"insertCustom":false,"insertOwner":true,"backNewEvent":null,"backCustom":true,"backFallback":false,"backOwner":true},{"label":"xhtml","during":[true,true,true,true,true],"awayNewEvent":"NotSupportedError","awayFallback":true,"awayCustom":false,"awayOwner":true,"insertNewEvent":"NotSupportedError","insertFallback":true,"insertCustom":false,"insertOwner":true,"backNewEvent":null,"backCustom":true,"backFallback":false,"backOwner":true}]}"#
    );
}

#[test]
fn custom_element_attribute_callback_is_snapshotted_and_receives_namespace() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class AttrElement extends HTMLElement {}
              AttrElement.observedAttributes = ["title"];
              AttrElement.prototype.attributeChangedCallback = function(name, oldValue, newValue, namespace) {
                calls.push(["old", name, oldValue, newValue, namespace].join(":"));
              };
              customElements.define("wpt-attr-element", AttrElement);
              AttrElement.prototype.attributeChangedCallback = function() {
                calls.push("new");
              };
              const element = document.createElement("wpt-attr-element");
              element.setAttributeNS("urn:moli:test", "lm:title", "one");
              element.removeAttributeNS("urn:moli:test", "title");
              element.setAttribute("title", "two");

              class IterableAttrElement extends HTMLElement {}
              IterableAttrElement.observedAttributes = {
                [Symbol.iterator]: function* () {
                  yield "lang";
                }
              };
              IterableAttrElement.prototype.attributeChangedCallback = function(name, oldValue, newValue, namespace) {
                calls.push(["iterable", name, oldValue, newValue, namespace].join(":"));
              };
              const arrayFrom = Array.from;
              Array.from = () => { throw new Error("observedAttributes must not use Array.from"); };
              customElements.define("wpt-iterable-attr-element", IterableAttrElement);
              Array.from = arrayFrom;
              document.createElement("wpt-iterable-attr-element").setAttribute("lang", "en");

              class AriaStringElement extends HTMLElement {}
              AriaStringElement.observedAttributes = ["aria-atomic"];
              AriaStringElement.prototype.attributeChangedCallback = function(name, oldValue, newValue, namespace) {
                calls.push(["aria-string", name, oldValue, newValue, namespace].join(":"));
              };
              customElements.define("wpt-aria-string-element", AriaStringElement);
              const ariaString = document.createElement("wpt-aria-string-element");
              ariaString.ariaAtomic = "true";
              ariaString.ariaAtomic = "false";
              ariaString.ariaAtomic = null;

              class AriaElementRefElement extends HTMLElement {}
              AriaElementRefElement.observedAttributes = ["aria-controls"];
              AriaElementRefElement.prototype.attributeChangedCallback = function(name, oldValue, newValue, namespace) {
                calls.push(["aria-element", name, oldValue, newValue, namespace].join(":"));
              };
              customElements.define("wpt-aria-element-ref-element", AriaElementRefElement);
              const target = document.createElement("div");
              const ariaElement = document.createElement("wpt-aria-element-ref-element");
              ariaElement.ariaControlsElements = [target];
              ariaElement.ariaControlsElements = [target];
              return calls.join("|");
            })()
            "#,
        )
        .expect("custom element attribute callback probe should evaluate");

    assert_eq!(
        result,
        "old:title::one:urn:moli:test|old:title:one::urn:moli:test|old:title::two:|iterable:lang::en:|aria-string:aria-atomic::true:|aria-string:aria-atomic:true:false:|aria-string:aria-atomic:false::|aria-element:aria-controls:::|aria-element:aria-controls:::"
    );
}

#[test]
fn custom_element_attribute_callback_runs_for_same_value_sets() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class SameValueElement extends HTMLElement {
                static observedAttributes = ["title", "label"];
                attributeChangedCallback(name, oldValue, newValue, namespace) {
                  calls.push([name, oldValue, newValue, namespace || ""].join(":"));
                }
              }
              customElements.define("wpt-same-value-element", SameValueElement);
              const element = document.createElement("wpt-same-value-element");
              element.setAttribute("title", "a");
              element.setAttribute("title", "a");
              element.setAttributeNS("urn:moli:test", "lm:label", "b");
              element.setAttributeNS("urn:moli:test", "lm:label", "b");
              return calls.join("|");
            })()
            "#,
        )
        .expect("same-value custom element attribute callback probe should evaluate");

    assert_eq!(
        result,
        "title::a:|title:a:a:|label::b:urn:moli:test|label:b:b:urn:moli:test"
    );
}

#[test]
fn custom_element_style_webkit_filter_alias_enqueues_attribute_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class StyleAliasElement extends HTMLElement {
                static observedAttributes = ["style"];
                attributeChangedCallback(name, oldValue, newValue, namespace) {
                  calls.push([name, oldValue, newValue, namespace || ""].join(":"));
                }
              }
              customElements.define("wpt-style-alias-element", StyleAliasElement);
              const camel = document.createElement("wpt-style-alias-element");
              camel.style.webkitFilter = "grayscale(20%)";
              camel.style.webkitFilter = "grayscale(30%)";

              const dashed = document.createElement("wpt-style-alias-element");
              dashed.style["-webkit-filter"] = "grayscale(40%)";

              return [
                camel.getAttribute("style"),
                dashed.getAttribute("style"),
                calls.join("|")
              ].join("||");
            })()
            "#,
        )
        .expect("webkit filter style alias custom-element reaction probe should evaluate");

    assert_eq!(
        result,
        "filter: grayscale(30%);||filter: grayscale(40%);||style::filter: grayscale(20%);:|style:filter: grayscale(20%);:filter: grayscale(30%);:|style::filter: grayscale(40%);:"
    );
}

#[test]
fn custom_element_style_mutations_use_native_reaction_path() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class StyleElement extends HTMLElement {
                static observedAttributes = ["style"];
                attributeChangedCallback(name, oldValue, newValue) {
                  calls.push({
                    name,
                    oldValue,
                    newValue,
                    current: this.getAttribute(name),
                    present: this.hasAttribute(name)
                  });
                }
              }
              customElements.define("wpt-style-native-element", StyleElement);
              const element = document.createElement("wpt-style-native-element");
              element.setAttribute = () => { throw new Error("public setAttribute called"); };
              element.removeAttribute = () => { throw new Error("public removeAttribute called"); };

              element.style.color = "red";
              element.style.setProperty("background-color", "blue");
              element.style.removeProperty("color");
              element.style.cssText = "margin-left: 1px";
              element.style = "padding-top: 2px";

              return JSON.stringify({
                calls,
                style: element.getAttribute("style")
              });
            })()
            "#,
        )
        .expect("style custom-element reaction path probe should evaluate");

    assert_eq!(
        result,
        r#"{"calls":[{"name":"style","oldValue":null,"newValue":"color: red;","current":"color: red;","present":true},{"name":"style","oldValue":"color: red;","newValue":"color: red; background-color: blue;","current":"color: red; background-color: blue;","present":true},{"name":"style","oldValue":"color: red; background-color: blue;","newValue":"background-color: blue;","current":"background-color: blue;","present":true},{"name":"style","oldValue":"background-color: blue;","newValue":"margin-left: 1px;","current":"margin-left: 1px;","present":true},{"name":"style","oldValue":"margin-left: 1px;","newValue":"padding-top: 2px;","current":"padding-top: 2px;","present":true}],"style":"padding-top: 2px;"}"#
    );
}

#[test]
fn custom_element_html_element_keyword_boolean_reflections_enqueue_attribute_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class KeywordBooleanElement extends HTMLElement {
                static observedAttributes = ["translate", "draggable", "spellcheck"];
                attributeChangedCallback(name, oldValue, newValue, namespace) {
                  calls.push([name, oldValue, newValue, namespace || ""].join(":"));
                }
              }
              customElements.define("wpt-keyword-boolean-element", KeywordBooleanElement);
              const element = document.createElement("wpt-keyword-boolean-element");
              element.translate = true;
              element.translate = false;
              element.draggable = true;
              element.draggable = false;
              element.spellcheck = true;
              element.spellcheck = false;
              return [
                element.getAttribute("translate"),
                element.getAttribute("draggable"),
                element.getAttribute("spellcheck"),
                calls.join("|")
              ].join("||");
            })()
            "#,
        )
        .expect("HTMLElement keyword boolean reflection reaction probe should evaluate");

    assert_eq!(
        result,
        "no||false||false||translate::yes:|translate:yes:no:|draggable::true:|draggable:true:false:|spellcheck::true:|spellcheck:true:false:"
    );
}

#[test]
fn custom_element_reflected_attribute_setters_use_native_reaction_path() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const elementCalls = [];
              class ReflectedElement extends HTMLElement {
                static observedAttributes = ["id", "hidden", "popover"];
                attributeChangedCallback(name, oldValue, newValue) {
                  elementCalls.push({
                    name,
                    oldValue,
                    newValue,
                    current: this.getAttribute(name),
                    present: this.hasAttribute(name)
                  });
                }
              }
              customElements.define("wpt-reflected-native-element", ReflectedElement);
              const element = document.createElement("wpt-reflected-native-element");
              element.setAttribute = () => { throw new Error("public setAttribute called"); };
              element.removeAttribute = () => { throw new Error("public removeAttribute called"); };
              element.id = "alpha";
              element.hidden = true;
              element.hidden = false;
              element.popover = "manual";
              element.popover = null;

              const imageCalls = [];
              class ReflectedImage extends HTMLImageElement {
                static observedAttributes = ["src", "loading"];
                attributeChangedCallback(name, oldValue, newValue) {
                  imageCalls.push({
                    name,
                    oldValue,
                    newValue,
                    current: this.getAttribute(name),
                    present: this.hasAttribute(name)
                  });
                }
              }
              customElements.define("wpt-reflected-native-img", ReflectedImage, { extends: "img" });
              const image = document.createElement("img", { is: "wpt-reflected-native-img" });
              image.setAttribute = () => { throw new Error("public setAttribute called"); };
              image.removeAttribute = () => { throw new Error("public removeAttribute called"); };
              image.src = "/assets/pixel.png";
              image.loading = "lazy";

              return JSON.stringify({
                elementCalls,
                imageCalls,
                element: {
                  id: element.getAttribute("id"),
                  hidden: element.hasAttribute("hidden"),
                  popover: element.hasAttribute("popover")
                },
                image: {
                  src: image.getAttribute("src"),
                  loading: image.getAttribute("loading")
                }
              });
            })()
            "#,
        )
        .expect("reflected attribute setter custom-element reaction probe should evaluate");

    assert_eq!(
        result,
        r#"{"elementCalls":[{"name":"id","oldValue":null,"newValue":"alpha","current":"alpha","present":true},{"name":"hidden","oldValue":null,"newValue":"","current":"","present":true},{"name":"hidden","oldValue":"","newValue":null,"current":null,"present":false},{"name":"popover","oldValue":null,"newValue":"manual","current":"manual","present":true},{"name":"popover","oldValue":"manual","newValue":null,"current":null,"present":false}],"imageCalls":[{"name":"src","oldValue":null,"newValue":"/assets/pixel.png","current":"/assets/pixel.png","present":true},{"name":"loading","oldValue":null,"newValue":"lazy","current":"lazy","present":true}],"element":{"id":"alpha","hidden":false,"popover":false},"image":{"src":"/assets/pixel.png","loading":"lazy"}}"#
    );
}

#[test]
fn custom_element_dataset_and_dom_token_list_use_native_reaction_path() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const elementCalls = [];
              class TokenDatasetElement extends HTMLElement {
                static observedAttributes = ["data-foo", "class", "part"];
                attributeChangedCallback(name, oldValue, newValue) {
                  elementCalls.push({
                    name,
                    oldValue,
                    newValue,
                    current: this.getAttribute(name),
                    present: this.hasAttribute(name)
                  });
                }
              }
              customElements.define("wpt-token-dataset-element", TokenDatasetElement);
              const element = document.createElement("wpt-token-dataset-element");
              element.setAttribute = () => { throw new Error("public setAttribute called"); };
              element.removeAttribute = () => { throw new Error("public removeAttribute called"); };

              element.dataset.foo = "one";
              delete element.dataset.foo;
              element.classList.add("alpha", "beta");
              element.classList.remove("alpha");
              element.classList.toggle("gamma", true);
              element.classList.replace("gamma", "delta");
              element.classList.value = "omega";
              element.classList = "setter";
              element.part.add("piece");
              element.part.value = "final-piece";
              element.part = "setter-piece";

              const linkCalls = [];
              class TokenLinkElement extends HTMLLinkElement {
                static observedAttributes = ["rel"];
                attributeChangedCallback(name, oldValue, newValue) {
                  linkCalls.push({
                    name,
                    oldValue,
                    newValue,
                    current: this.getAttribute(name),
                    present: this.hasAttribute(name)
                  });
                }
              }
              customElements.define("wpt-token-link-element", TokenLinkElement, { extends: "link" });
              const link = document.createElement("link", { is: "wpt-token-link-element" });
              link.setAttribute = () => { throw new Error("public setAttribute called"); };
              link.removeAttribute = () => { throw new Error("public removeAttribute called"); };
              link.relList.add("preload");
              link.relList.value = "stylesheet";
              link.relList = "modulepreload";

              return JSON.stringify({
                elementCalls,
                linkCalls,
                element: {
                  dataFoo: element.getAttribute("data-foo"),
                  className: element.getAttribute("class"),
                  part: element.getAttribute("part")
                },
                link: {
                  rel: link.getAttribute("rel")
                }
              });
            })()
            "#,
        )
        .expect("dataset and DOMTokenList custom-element reaction probe should evaluate");

    assert_eq!(
        result,
        r#"{"elementCalls":[{"name":"data-foo","oldValue":null,"newValue":"one","current":"one","present":true},{"name":"data-foo","oldValue":"one","newValue":null,"current":null,"present":false},{"name":"class","oldValue":null,"newValue":"alpha beta","current":"alpha beta","present":true},{"name":"class","oldValue":"alpha beta","newValue":"beta","current":"beta","present":true},{"name":"class","oldValue":"beta","newValue":"beta gamma","current":"beta gamma","present":true},{"name":"class","oldValue":"beta gamma","newValue":"beta delta","current":"beta delta","present":true},{"name":"class","oldValue":"beta delta","newValue":"omega","current":"omega","present":true},{"name":"class","oldValue":"omega","newValue":"setter","current":"setter","present":true},{"name":"part","oldValue":null,"newValue":"piece","current":"piece","present":true},{"name":"part","oldValue":"piece","newValue":"final-piece","current":"final-piece","present":true},{"name":"part","oldValue":"final-piece","newValue":"setter-piece","current":"setter-piece","present":true}],"linkCalls":[{"name":"rel","oldValue":null,"newValue":"preload","current":"preload","present":true},{"name":"rel","oldValue":"preload","newValue":"stylesheet","current":"stylesheet","present":true},{"name":"rel","oldValue":"stylesheet","newValue":"modulepreload","current":"modulepreload","present":true}],"element":{"dataFoo":null,"className":"setter","part":"setter-piece"},"link":{"rel":"modulepreload"}}"#
    );
}

#[test]
fn custom_element_inner_and_outer_text_replacements_enqueue_disconnected_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class TextReplacementElement extends HTMLElement {
                connectedCallback() { calls.push(`${this.id}:connected`); }
                disconnectedCallback() { calls.push(`${this.id}:disconnected`); }
              }
              customElements.define("wpt-text-replacement-element", TextReplacementElement);
              const target = document.body || document.documentElement || document;

              const innerParent = document.createElement("div");
              const inner = document.createElement("wpt-text-replacement-element");
              inner.id = "inner";
              innerParent.appendChild(inner);
              target.appendChild(innerParent);
              innerParent.innerText = "";

              const outerParent = document.createElement("div");
              const outer = document.createElement("wpt-text-replacement-element");
              outer.id = "outer";
              outerParent.appendChild(outer);
              target.appendChild(outerParent);
              outer.outerText = "";

              return [
                innerParent.childNodes.length,
                outerParent.childNodes.length,
                calls.join("|")
              ].join("||");
            })()
            "#,
        )
        .expect("HTMLElement text replacement lifecycle probe should evaluate");

    assert_eq!(
        result,
        "0||1||inner:connected|inner:disconnected|outer:connected|outer:disconnected"
    );
}

#[test]
fn custom_element_popover_reflection_is_visible_on_html_element_prototype() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class PopoverElement extends HTMLElement {
                static observedAttributes = ["popover"];
                attributeChangedCallback(name, oldValue, newValue, namespace) {
                  calls.push([name, oldValue, newValue, namespace || ""].join(":"));
                }
              }
              customElements.define("wpt-popover-reflection-element", PopoverElement);
              const element = document.createElement("wpt-popover-reflection-element");
              const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "popover");
              descriptor.set.call(element, "auto");
              element.popover = "manual";
              return [
                "popover" in HTMLElement.prototype,
                descriptor && typeof descriptor.get,
                descriptor && typeof descriptor.set,
                element.getAttribute("popover"),
                calls.join("|")
              ].join("||");
            })()
            "#,
        )
        .expect("HTMLElement.prototype popover reflection probe should evaluate");

    assert_eq!(
        result,
        "true||function||function||manual||popover::auto:|popover:auto:manual:"
    );
}

#[test]
fn element_aria_element_reference_reflection_tracks_dom_owned_associations() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const html = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                html.appendChild(document.createElement("body"));
              const container = document.createElement("div");
              container.innerHTML = `
                <input aria-activedescendant="first" aria-describedby="first second">
                <p id="first"></p><p id="second"></p>`;
              body.appendChild(container);
              const element = container.querySelector("input");
              const first = container.querySelector("#first");
              const second = container.querySelector("#second");
              const initialDescriptions = element.ariaDescribedByElements;
              const initialActive = element.ariaActiveDescendantElement;

              container.remove();
              const disconnectedDescriptions = element.ariaDescribedByElements;
              const disconnectedActive = element.ariaActiveDescendantElement;
              body.appendChild(container);

              const assigned = [first, second];
              element.ariaControlsElements = assigned;
              assigned.pop();
              const explicitControlsAttributeBlank =
                element.getAttribute("aria-controls") === "";
              const controls = element.ariaControlsElements;
              const cachedControls = element.ariaControlsElements;
              container.remove();
              const disconnectedControls = element.ariaControlsElements;
              body.appendChild(container);

              element.setAttribute("aria-controls", "second");
              const contentAttributeControls = element.ariaControlsElements;
              element.removeAttribute("aria-controls");

              element.ariaActiveDescendantElement = first;
              const explicitActive = element.ariaActiveDescendantElement;
              element.ariaActiveDescendantElement = null;

              const missing = document.createElement("div");
              const reactions = [];
              class AriaReferenceElement extends HTMLElement {
                static observedAttributes = ["aria-controls"];
                attributeChangedCallback() {
                  reactions.push(this.ariaControlsElements?.[0] === first);
                }
              }
              customElements.define("aria-reference-element", AriaReferenceElement);
              const custom = document.createElement("aria-reference-element");
              body.appendChild(custom);
              custom.ariaControlsElements = [first];
              return [
                missing.ariaDescribedByElements === null,
                initialDescriptions.length === 2 && initialDescriptions[0] === first && initialDescriptions[1] === second,
                initialActive === first,
                disconnectedDescriptions.length === 2 && disconnectedDescriptions[0] === first && disconnectedDescriptions[1] === second,
                disconnectedActive === first,
                explicitControlsAttributeBlank,
                controls.length === 2 && controls[0] === first && controls[1] === second,
                Object.isFrozen(controls),
                controls === cachedControls,
                disconnectedControls === controls,
                contentAttributeControls.length === 1 && contentAttributeControls[0] === second,
                element.ariaControlsElements === null,
                explicitActive === first,
                element.ariaActiveDescendantElement === null,
                !element.hasAttribute("aria-activedescendant"),
                probe(() => { element.ariaActiveDescendantElement = "not an element"; }),
                probe(() => { element.ariaControlsElements = [first, 1]; }),
                probe(() => { element.ariaControlsElements = first; }),
                reactions.length === 1 && reactions[0]
              ].join("|");
            })()
            "##,
        )
        .expect("ARIA element reference reflection probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|TypeError|TypeError|TypeError|true"
    );
}

#[test]
fn custom_element_attach_internals_exposes_element_internals_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

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

              class InternalsElement extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-internals-element", InternalsElement);
              const element = new InternalsElement();
              const states = element.internals.states;
              states.add("--foo");
              states.add("--bar");
              states.delete("--foo");
              element.internals.role = "button";
              element.internals.ariaLabel = "Save";
              const target = document.createElement("div");
              element.internals.ariaControlsElements = [target];
              const stateMethodDescriptors = ["add", "delete", "clear", "has"]
                .map(name => {
                  const descriptor = Object.getOwnPropertyDescriptor(CustomStateSet.prototype, name);
                  return [
                    name,
                    typeof descriptor?.value,
                    descriptor?.value?.name,
                    descriptor?.value?.length,
                    descriptor?.enumerable,
                    descriptor?.writable,
                    descriptor?.configurable
                  ].join(":");
                })
                .join(";");

              class DisabledInternalsElement extends HTMLElement {
                static disabledFeatures = ["internals"];
              }
              customElements.define("wpt-disabled-internals", DisabledInternalsElement);

              const pending = document.createElement("wpt-late-internals");
              customElements.define("wpt-late-internals", class extends HTMLElement {});
              const preUpgrade = probe(() => pending.attachInternals());
              customElements.upgrade(pending);
              const postUpgrade = pending.attachInternals();

              const preShadow = document.createElement("wpt-pre-shadow");
              preShadow.attachShadow({ mode: "closed" });
              customElements.define("wpt-pre-shadow", class extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              });
              customElements.upgrade(preShadow);
              const preAttachedClosedHidden =
                preShadow.internals.shadowRoot === null;

              customElements.define(
                "wpt-defined-shadow-before-internals",
                class extends HTMLElement {}
              );
              const definedShadowHost =
                document.createElement("wpt-defined-shadow-before-internals");
              const definedShadow = definedShadowHost.attachShadow({ mode: "open" });
              const definedInternals = definedShadowHost.attachInternals();
              const definedShadowVisible =
                definedInternals.shadowRoot === definedShadow;

              let postInternalsShadowVisible = false;
              customElements.define("wpt-post-internals-shadow", class extends HTMLElement {
                constructor() {
                  super();
                  const internals = this.attachInternals();
                  const shadow = this.attachShadow({ mode: "closed" });
                  postInternalsShadowVisible =
                    internals.shadowRoot === shadow && this.shadowRoot === null;
                }
              });
              document.createElement("wpt-post-internals-shadow");

              return [
                typeof ElementInternals,
                element.internals instanceof ElementInternals,
                probe(() => element.attachInternals()),
                "role" in element.internals,
                "ariaLabel" in element.internals,
                "ariaControlsElements" in element.internals,
                element.internals.role,
                element.internals.ariaLabel,
                element.internals.ariaControlsElements[0] === target,
                states instanceof CustomStateSet,
                Object.prototype.toString.call(states),
                stateMethodDescriptors,
                states.size,
                [...states].join(","),
                probe(() => new DisabledInternalsElement().attachInternals()),
                preUpgrade,
                postUpgrade instanceof ElementInternals,
                preAttachedClosedHidden,
                definedShadowVisible,
                postInternalsShadowVisible
              ].join("|");
            })()
            "#,
        )
        .expect("ElementInternals custom element probe should evaluate");

    assert_eq!(
        result,
        "function|true|NotSupportedError|true|true|true|button|Save|true|true|[object CustomStateSet]|add:function:add:1:true:true:true;delete:function:delete:1:true:true:true;clear:function:clear:0:true:true:true;has:function:has:1:true:true:true|1|--bar|NotSupportedError|NotSupportedError|true|true|true|true"
    );
}

#[test]
fn element_internals_aria_strings_use_nullable_dom_string_conversion() {
    let mut vm = new_storage_test_vm("https://element-internals-aria-string.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class AriaInternalsElement extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-aria-internals-element", AriaInternalsElement);
              const internals = new AriaInternalsElement().internals;
              if (internals.role !== null || internals.ariaLabel !== null) {
                throw new Error("unset ARIA strings should be null");
              }
              internals.role = 7;
              internals.ariaLabel = { toString() { return "Save"; } };
              if (internals.role !== "7" || internals.ariaLabel !== "Save") {
                throw new Error("ARIA strings should use DOMString conversion");
              }
              const target = document.createElement("div");
              const references = [target];
              internals.ariaControlsElements = references;
              if (internals.ariaControlsElements !== references) {
                throw new Error("element-reference values should retain their own conversion path");
              }
              internals.role = null;
              internals.ariaLabel = undefined;
              if (internals.role !== null || internals.ariaLabel !== null) {
                throw new Error("nullish ARIA strings should reset to null");
              }
              try {
                internals.role = Symbol("role");
                throw new Error("Symbol should not convert to DOMString");
              } catch (error) {
                if (error.name !== "TypeError") throw error;
              }
              return "ok";
            })()
            "#,
        )
        .expect("ElementInternals nullable ARIA string probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn element_internals_keeps_target_element_alive_after_gc() {
    let mut vm = new_storage_test_vm("https://element-internals-gc.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              customElements.define("wpt-internals-gc", class extends HTMLElement {});
              globalThis.__heldInternals = [];
              for (let i = 0; i < 1000; i++) {
                const target = document.createElement("wpt-internals-gc");
                target.attachShadow({ mode: "open" });
                globalThis.__heldInternals.push(target.attachInternals());
              }
              return globalThis.__heldInternals.every(
                internals => internals.shadowRoot instanceof ShadowRoot
              );
            })()
            "#,
        )
        .expect("ElementInternals GC setup should evaluate");

    vm.renderer_document_isolate_ops()
        .collect_renderer_document_isolate_garbage()
        .expect("renderer isolate GC should complete");

    let after_gc = vm
        .eval(
            r#"
            globalThis.__heldInternals.every(
              internals => internals.shadowRoot instanceof ShadowRoot
            )
            "#,
        )
        .expect("ElementInternals GC probe should evaluate");

    assert_eq!(initial, "true");
    assert_eq!(after_gc, "true");
}

#[test]
fn custom_state_set_fallback_uses_declared_surface_when_set_is_missing() {
    let mut vm = new_storage_test_vm("https://custom-state-set-fallback.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class InternalsElement extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("fallback-state-set", InternalsElement);
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.head) {
                document.documentElement.appendChild(document.createElement("head"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const style = document.createElement("style");
              style.textContent = `
                fallback-state-set { color: rgb(255, 0, 0); }
                fallback-state-set:state(--green) { color: rgb(0, 0, 255); }
              `;
              document.head.appendChild(style);
              const element = new InternalsElement();
              document.body.appendChild(element);
              const originalSet = globalThis.Set;
              globalThis.Set = undefined;
              const states = element.internals.states;
              const again = element.internals.states;
              const addReturn = states.add("--green") === states;
              const afterAdd = [
                element.matches(":state(--green)"),
                getComputedStyle(element).color,
                states.has("--green")
              ].join(",");
              const deleteReturn = states.delete("--green");
              const afterDelete = [
                element.matches(":state(--green)"),
                getComputedStyle(element).color,
                states.has("--green")
              ].join(",");
              globalThis.Set = originalSet;
              return [
                states === again,
                states instanceof CustomStateSet,
                Object.prototype.toString.call(states),
                Object.keys(states).join(","),
                Object.getOwnPropertyNames(states).join(","),
                addReturn,
                afterAdd,
                deleteReturn,
                afterDelete
              ].join("|");
            })()
            "#,
        )
        .expect("CustomStateSet fallback probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object CustomStateSet]|||true|true,rgb(0, 0, 255),true|true|false,rgb(255, 0, 0),false"
    );
}

#[test]
fn custom_state_set_updates_state_selectors_and_invalidates_has_ancestors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.head) {
                document.documentElement.appendChild(document.createElement("head"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }

              const style = document.createElement("style");
              style.textContent = `
                #subject { background-color: rgb(255, 0, 0); }
                #subject:has(:state(--green)) { background-color: rgb(0, 128, 0); }
                wpt-state-target { color: rgb(255, 0, 0); }
                wpt-state-target:state(--green) { color: rgb(0, 0, 255); }
              `;
              document.head.appendChild(style);

              class StateTarget extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-state-target", StateTarget);

              const subject = document.createElement("section");
              subject.id = "subject";
              const target = new StateTarget();
              subject.appendChild(target);
              document.body.appendChild(subject);

              const states = target.internals.states;
              const snapshot = () => [
                target.matches(":state(--green)"),
                subject.matches("#subject:has(:state(--green))"),
                getComputedStyle(target).color,
                getComputedStyle(subject).backgroundColor,
                states.has("--green")
              ].join(",");

              const before = snapshot();
              const addReturn = states.add("--green") === states;
              const afterAdd = snapshot();
              const numericState = [states.add(1) === states, states.has("1"), states.delete(1), states.has("1")].join(",");
              const deleteReturn = states.delete("--green");
              const afterDelete = snapshot();
              states.add("--green");
              states.clear();
              const afterClear = snapshot();
              return [before, addReturn, afterAdd, numericState, deleteReturn, afterDelete, afterClear].join("|");
            })()
            "##,
        )
        .expect("CustomStateSet style invalidation probe should evaluate");

    assert_eq!(
        result,
        "false,false,rgb(255, 0, 0),rgb(255, 0, 0),false|true|true,true,rgb(0, 0, 255),rgb(0, 128, 0),true|true,true,true,false|true|false,false,rgb(255, 0, 0),rgb(255, 0, 0),false|false,false,rgb(255, 0, 0),rgb(255, 0, 0),false"
    );
}

#[test]
fn custom_state_set_clear_invalidates_nth_child_of_state_siblings() {
    let mut vm = new_storage_test_vm("https://custom-state-nth-of.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.head) {
                document.documentElement.appendChild(document.createElement("head"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }

              const style = document.createElement("style");
              style.textContent = `
                :nth-child(1), :nth-child(2) {
                  color: rgb(255, 0, 0);
                }
                :nth-child(2 of :state(--green)) {
                  color: rgb(0, 255, 0);
                }
                :nth-child(2 of :state(--green)) + p {
                  color: rgb(0, 0, 255);
                }
              `;
              document.head.appendChild(style);

              class StateTarget extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-state-target", StateTarget);

              const first = new StateTarget();
              const firstSibling = document.createElement("p");
              const second = new StateTarget();
              const secondSibling = document.createElement("p");
              document.body.append(first, firstSibling, second, secondSibling);

              const colors = () => [
                getComputedStyle(first).color,
                getComputedStyle(firstSibling).color,
                getComputedStyle(second).color,
                getComputedStyle(secondSibling).color
              ].join(",");

              const before = colors();
              first.internals.states.add("--green");
              const afterFirstGreen = colors();
              second.internals.states.add("--green");
              const afterSecondGreen = colors();
              first.internals.states.add("--foo");
              second.internals.states.add("--foo");
              first.internals.states.clear();
              const afterClear = colors();

              return [
                before,
                afterFirstGreen,
                afterSecondGreen,
                afterClear,
                first.internals.states.has("--green"),
                first.internals.states.has("--foo"),
                second.internals.states.has("--green"),
                second.internals.states.has("--foo")
              ].join("|");
            })()
            "##,
        )
        .expect("CustomStateSet nth-of invalidation probe should evaluate");

    assert_eq!(
        result,
        "rgb(255, 0, 0),rgb(255, 0, 0),rgb(255, 0, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(255, 0, 0),rgb(255, 0, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(255, 0, 0),rgb(0, 255, 0),rgb(0, 0, 255)|rgb(255, 0, 0),rgb(255, 0, 0),rgb(255, 0, 0),rgb(255, 0, 0)|false|false|true|true"
    );
}

#[test]
fn custom_state_set_invalidates_shadow_source_scopes() {
    let mut vm = new_storage_test_vm("https://custom-state-shadow-scope.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }

              class ShadowStateTarget extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-shadow-state-target", ShadowStateTarget);
              customElements.define("wpt-shadow-state-host", class extends HTMLElement {
                constructor() {
                  super();
                  const shadow = this.attachShadow({ mode: "open" });
                  const style = document.createElement("style");
                  style.textContent = `
                    :host { color: rgb(255, 0, 0); }
                    :host(:has(wpt-shadow-state-target:state(--green))) { color: rgb(0, 0, 255); }
                    wpt-shadow-state-target { color: rgb(255, 0, 0); }
                    wpt-shadow-state-target:state(--green) { color: rgb(0, 0, 255); }
                    ::slotted(wpt-shadow-state-target:state(--green)) { background-color: rgb(0, 128, 0); }
                  `;
                  shadow.appendChild(style);
                  shadow.appendChild(document.createElement("slot"));
                  this.inner = new ShadowStateTarget();
                  shadow.appendChild(this.inner);
                }
              });

              const host = document.createElement("wpt-shadow-state-host");
              const slotted = new ShadowStateTarget();
              host.appendChild(slotted);
              document.body.appendChild(host);
              const inner = host.shadowRoot.querySelector("wpt-shadow-state-target");
              const snapshot = () => [
                getComputedStyle(host).color,
                getComputedStyle(inner).color,
                getComputedStyle(slotted).backgroundColor,
                inner.matches(":state(--green)"),
                slotted.matches(":state(--green)")
              ].join(",");

              const before = snapshot();
              inner.internals.states.add("--green");
              const afterInner = snapshot();
              slotted.internals.states.add("--green");
              const afterSlotted = snapshot();
              return [before, afterInner, afterSlotted].join("|");
            })()
            "##,
        )
        .expect("shadow custom-state scope probe should evaluate");

    assert_eq!(
        result,
        "rgb(255, 0, 0),rgb(255, 0, 0),rgba(0, 0, 0, 0),false,false|rgb(255, 0, 0),rgb(0, 0, 255),rgba(0, 0, 0, 0),true,false|rgb(0, 0, 255),rgb(0, 0, 255),rgb(0, 128, 0),true,true"
    );
}

#[test]
fn form_associated_custom_element_resolves_form_owner_and_listed_collections() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceOwnerElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.history = [];
                }
                formAssociatedCallback(form) {
                  this.history.push(form ? form.id : null);
                }
                get form() {
                  return this.internals.form;
                }
              }
              customElements.define("wpt-face-owner", FaceOwnerElement);

              const container = document.createElement("div");
              const target = document.body || document.documentElement || document;
              target.appendChild(container);
              container.innerHTML = `
                <fieldset id="fs">
                  <form id="f">
                    <input>
                    <wpt-face-owner id="inside"></wpt-face-owner>
                    <select></select>
                  </form>
                </fieldset>
                <wpt-face-owner id="external" form="f"></wpt-face-owner>
              `;

              const form = container.querySelector("#f");
              const fieldset = container.querySelector("#fs");
              const inside = container.querySelector("#inside");
              const external = container.querySelector("#external");
              const controls = form.elements;
              const initialLength = controls.length;
              const initialInside = controls[1] === inside;
              const initialExternal = controls[3] === external;
              external.setAttribute("form", "missing");

              return [
                inside.form === form,
                external.form === null,
                initialLength,
                initialInside,
                initialExternal,
                fieldset.elements[1] === inside,
                inside.history.join(","),
                external.history.join(",")
              ].join("|");
            })()
            "##,
        )
        .expect("form-associated custom element owner probe should evaluate");

    assert_eq!(result, "true|true|4|true|true|true|f|f,");
}

#[test]
fn form_associated_custom_element_participates_in_label_association() {
    let mut vm = new_storage_test_vm("https://face-labels.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceLabelElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.i = this.attachInternals();
                  this.clicks = 0;
                  this.addEventListener("click", () => this.clicks++);
                }
              }
              customElements.define("face-label-element", FaceLabelElement);
              const root = document.body || document.documentElement || document;
              const container = root.appendChild(document.createElement("div"));
              container.innerHTML = '<label for="face"></label><form><face-label-element id="face"></face-label-element></form>';
              const label = container.querySelector("label");
              const control = container.querySelector("face-label-element");
              const labels = control.i.labels;
              label.click();
              return JSON.stringify({
                control: label.control === control,
                labelForm: label.form === control.i.form,
                labelsBrand: labels instanceof NodeList,
                labelsLength: labels.length,
                labelsItem: labels[0] === label,
                clicks: control.clicks
              });
            })()
            "#,
        )
        .expect("form-associated label association probe should evaluate");

    assert_eq!(
        result,
        r#"{"control":true,"labelForm":true,"labelsBrand":true,"labelsLength":1,"labelsItem":true,"clicks":1}"#
    );
}

#[test]
fn form_reset_invokes_form_associated_custom_element_callback() {
    let mut vm = new_storage_test_vm("https://face-reset.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceResetElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.i = this.attachInternals();
                  this.resetCount = 0;
                }
                formResetCallback() {
                  this.resetCount++;
                  this.outputValue = this.output && this.output.value;
                }
              }
              customElements.define("face-reset-element", FaceResetElement);
              const root = document.body || document.documentElement || document;
              const container = root.appendChild(document.createElement("div"));
              container.insertAdjacentHTML("beforeend",
                "<form><face-reset-element></face-reset-element><output>default</output></form>");
              const form = container.lastChild;
              const custom = form.firstChild;
              const output = form.lastChild;
              output.value = "updated";
              custom.output = output;
              form.reset();
              return JSON.stringify({
                form: custom.i.form === form,
                inElements: form.elements[0] === custom,
                resetCount: custom.resetCount,
                outputValue: custom.outputValue
              });
            })()
            "#,
        )
        .expect("form-associated reset callback probe should evaluate");

    assert_eq!(
        result,
        r#"{"form":true,"inElements":true,"resetCount":1,"outputValue":"default"}"#
    );
}

#[test]
fn element_internals_declared_form_methods_preserve_descriptors_and_validity() {
    let mut vm = new_storage_test_vm("https://element-internals-declared-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceValidationElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-face-validation", FaceValidationElement);
              const element = new FaceValidationElement();
              const internals = element.internals;
              const methods = ["setFormValue", "setValidity", "checkValidity", "reportValidity"];
              const descriptors = methods.map(name => {
                const descriptor = Object.getOwnPropertyDescriptor(ElementInternals.prototype, name);
                return [
                  name,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              });
              const setFormUndefined = internals.setFormValue("value") === undefined;
              const setValidityUndefined = internals.setValidity({ customError: true }, "bad value") === undefined;
              const invalid = [
                internals.validity.valid,
                internals.validity.customError,
                internals.validationMessage,
                internals.checkValidity(),
                internals.reportValidity()
              ].join("|");
              internals.setValidity({});
              const valid = [
                internals.validity.valid,
                internals.validity.customError,
                internals.validationMessage,
                internals.checkValidity(),
                internals.reportValidity()
              ].join("|");
              return JSON.stringify({
                descriptors,
                setFormUndefined,
                setValidityUndefined,
                invalid,
                valid
              });
            })()
            "#,
        )
        .expect("ElementInternals declared method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["setFormValue:function:setFormValue:1:true:true:true","setValidity:function:setValidity:1:true:true:true","checkValidity:function:checkValidity:0:true:true:true","reportValidity:function:reportValidity:0:true:true:true"],"setFormUndefined":true,"setValidityUndefined":true,"invalid":"false|true|bad value|false|false","valid":"true|false||true|true"}"#
    );
}

#[test]
fn form_associated_custom_element_refreshes_later_form_owner() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceLaterFormElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.history = [];
                }
                formAssociatedCallback(form) {
                  this.history.push(form ? form.id : null);
                }
                get form() {
                  return this.internals.form;
                }
              }
              customElements.define("wpt-face-later-form", FaceLaterFormElement);

              const container = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(container);
              container.innerHTML = `
                <fieldset id="fs">
                  <wpt-face-later-form id="inside" form="target"></wpt-face-later-form>
                  <form id="target"><input></form>
                </fieldset>
                <wpt-face-later-form id="outside" form="target"></wpt-face-later-form>
              `;

              const form = container.querySelector("#target");
              const inside = container.querySelector("#inside");
              const outside = container.querySelector("#outside");
              const fieldset = container.querySelector("#fs");
              const initial = [
                inside.form === form,
                outside.form === form,
                form.elements[0] === inside,
                form.elements[2] === outside,
                fieldset.elements[0] === inside,
                inside.history.join(","),
                outside.history.join(",")
              ].join("|");

              form.remove();
              const removed = [
                inside.form === null,
                outside.form === null,
                inside.history.join(","),
                outside.history.join(",")
              ].join("|");

              container.appendChild(form);
              const reinserted = [
                inside.form === form,
                outside.form === form,
                inside.history.join(","),
                outside.history.join(",")
              ].join("|");

              return `${initial}#${removed}#${reinserted}`;
            })()
            "##,
        )
        .expect("later form-associated custom element owner probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|target|target#true|true|target,|target,#true|true|target,,target|target,,target"
    );
}

#[test]
fn parser_form_pointer_associates_builtin_controls_not_face() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        r##"
        <!doctype html>
        <body>
        <table>
          <fieldset id="fs">
            <form id="f">
              <tr><td><select id="select"></select></tr>
              <tr><td><wpt-face-parser-form id="face"></wpt-face-parser-form></tr>
              <tr><td><input id="input"></tr>
            </form>
          </fieldset>
        </table>
        </body>
        "##,
    );

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceParserFormElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.history = [];
                }
                formAssociatedCallback(form) {
                  this.history.push(form ? form.id : null);
                }
                get form() {
                  return this.internals.form;
                }
              }
              customElements.define("wpt-face-parser-form", FaceParserFormElement);

              customElements.upgrade(document.body);
              const form = document.querySelector("#f");
              const face = document.querySelector("#face");
              const controls = form.elements;
              return [
                controls.length,
                controls[0] && controls[0].id,
                controls[1] && controls[1].id,
                face.form === null,
                face.history.join(","),
                document.querySelector("#fs").elements.length
              ].join("|");
            })()
            "##,
        )
        .expect("parser form-pointer association probe should evaluate");

    assert_eq!(result, "2|select|input|true||0");
}

#[test]
fn parser_created_form_associated_custom_element_runs_initial_form_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class ParserDefinedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals_ = this.attachInternals();
                  this.formHistory_ = [];
                }
                formAssociatedCallback(form) {
                  this.formHistory_.push(form ? form.id : null);
                }
                get form() { return this.internals_.form; }
                formHistory() { return this.formHistory_; }
              }
              customElements.define('parser-defined-face', ParserDefinedFace);
              const container = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(container);
              container.innerHTML = `
                <fieldset id="fs1">
                  <form id="form1">
                    <input>
                    <parser-defined-face id="pd1"></parser-defined-face>
                    <select></select>
                  </form>
                </fieldset>
                <fieldset id="fs2">
                  <parser-defined-face id="pd2" form="form2"></parser-defined-face>
                  <form id="form2">
                    <input>
                    <select></select>
                  </form>
                </fieldset>
                <parser-defined-face id="pd3" form="form2"></parser-defined-face>
              `;
              const ids = ["pd1", "pd2", "pd3"];
              return ids.map(id => {
                const element = document.getElementById(id);
                return `${id}:${element.form && element.form.id}:${element.formHistory().map(form => form && form.id || form).join(",")}`;
              }).join("|");
            })()
            "##,
        )
        .expect("parser-created FACE initial form callback probe should evaluate");

    assert_eq!(result, "pd1:form1:form1|pd2:form2:form2|pd3:form2:form2");
}

#[test]
fn custom_element_constructor_error_reports_to_definition_window() {
    let mut vm = new_storage_test_vm("https://ce-error-realm.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frameA = document.createElement("iframe");
              const frameB = document.createElement("iframe");
              const root = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || root.appendChild(document.createElement("body"));
              body.append(frameA, frameB);
              const w = frameA.contentWindow;
              const w2 = frameB.contentWindow;
              w.eval("self.__repeatedEvalProbe = 1");
              w.eval("self.MyElement = class extends HTMLElement { constructor() { throw new Error('boom'); } }");
              const events = [];
              const listener = event => {
                events.push({
                  targetA: event.target === w,
                  targetB: event.target === w2,
                  hasError: !!event.error,
                  errorIsDefinitionRealm: !!event.error && event.error.constructor === w.Error,
                  errorCtorType: typeof w.Error
                });
                event.preventDefault();
              };
              for (const current of [window, w, w2]) current.addEventListener("error", listener);
              w2.customElements.define("realm-error-element", w.MyElement);
              const element = w2.document.createElement("realm-error-element");
              return JSON.stringify({
                unknown: element instanceof w2.HTMLUnknownElement,
                events
              });
            })()
            "#,
        )
        .expect("custom element constructor error realm probe should evaluate");

    assert_eq!(
        result,
        r#"{"unknown":true,"events":[{"targetA":true,"targetB":false,"hasError":true,"errorIsDefinitionRealm":true,"errorCtorType":"function"}]}"#
    );
}

#[test]
fn failed_existing_upgrade_preserves_definition_prototype_and_clears_reactions() {
    let mut vm = new_storage_test_vm("https://ce-failed-upgrade.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              window.addEventListener("error", event => event.preventDefault());
              const root = document.body || document.documentElement || document;
              const frame = root.appendChild(document.createElement("iframe"));
              const childWindow = frame.contentWindow;
              const childDocument = childWindow.document;
              childDocument.write('<failed-upgrade-probe id="some" class="foo"></failed-upgrade-probe>');
              const element = childDocument.querySelector("failed-upgrade-probe");
              const before = Object.getPrototypeOf(element) === childWindow.HTMLElement.prototype;
              const log = [];
              class FailedUpgradeProbe extends childWindow.HTMLElement {
                constructor() {
                  super();
                  log.push("constructor");
                  throw new Error("boom");
                }
                connectedCallback() {
                  log.push("connected");
                }
                attributeChangedCallback() {
                  log.push("attribute");
                }
                static get observedAttributes() { return ["id", "class"]; }
              }
              childWindow.customElements.define("failed-upgrade-probe", FailedUpgradeProbe);
              return JSON.stringify({
                before,
                after: Object.getPrototypeOf(element) === FailedUpgradeProbe.prototype,
                log
              });
            })()
            "#,
        )
        .expect("failed existing custom element upgrade probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":true,"after":true,"log":["constructor"]}"#
    );
}

#[test]
fn form_associated_custom_element_disabled_state_and_form_value() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceDisabledElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.log = [];
                  this.internals.setFormValue("face-value");
                }
                formDisabledCallback(disabled) {
                  this.log.push(disabled);
                }
              }
              customElements.define("wpt-face-disabled", FaceDisabledElement);

              const form = document.createElement("form");
              form.innerHTML = `
                <fieldset id="fs">
                  <legend><wpt-face-disabled id="legend" name="legend" tabindex="0"></wpt-face-disabled></legend>
                  <wpt-face-disabled id="inside" name="inside" tabindex="0"></wpt-face-disabled>
                </fieldset>
                <wpt-face-disabled id="outside" name="outside" tabindex="0"></wpt-face-disabled>
              `;
              (document.body || document.documentElement || document).appendChild(form);

              const fs = form.querySelector("#fs");
              const legend = form.querySelector("#legend");
              const inside = form.querySelector("#inside");
              const outside = form.querySelector("#outside");
              const formEntries = () => Array.from(new FormData(form))
                .map(([name, value]) => `${name}=${value}`);

              const initial = {
                outsideEnabled: outside.matches(":enabled"),
                outsideDisabled: outside.matches(":disabled"),
                insideEnabled: inside.matches(":enabled"),
                entries: formEntries()
              };

              outside.setAttribute("disabled", "");
              outside.focus();
              const ownDisabled = {
                log: outside.log.slice(),
                disabled: outside.matches(":disabled"),
                omitted: new FormData(form).get("outside") === null,
                focused: document.activeElement === outside
              };

              outside.removeAttribute("disabled");
              outside.focus();
              const ownEnabled = {
                log: outside.log.slice(),
                enabled: outside.matches(":enabled"),
                value: new FormData(form).get("outside"),
                focused: document.activeElement === outside
              };

              fs.setAttribute("disabled", "");
              const fieldsetDisabled = {
                insideLog: inside.log.slice(),
                legendLog: legend.log.slice(),
                insideDisabled: inside.matches(":disabled"),
                legendEnabled: legend.matches(":enabled"),
                insideOmitted: new FormData(form).get("inside") === null,
                legendValue: new FormData(form).get("legend")
              };

              inside.setAttribute("disabled", "");
              inside.removeAttribute("disabled");
              const dedupedInsideLog = inside.log.slice();

              fs.removeAttribute("disabled");
              const fieldsetEnabled = {
                insideLog: inside.log.slice(),
                insideEnabled: inside.matches(":enabled"),
                entries: formEntries()
              };

              const detachedContainer = document.createElement("fieldset");
              detachedContainer.innerHTML = "<fieldset><fieldset><wpt-face-disabled></wpt-face-disabled></fieldset></fieldset>";
              const detachedMiddleFieldset = detachedContainer.firstChild;
              const detachedControl = detachedContainer.querySelector("wpt-face-disabled");
              detachedMiddleFieldset.disabled = true;
              detachedMiddleFieldset.disabled = false;
              detachedContainer.disabled = true;
              detachedControl.remove();
              detachedMiddleFieldset.appendChild(detachedControl);
              const detachedRelationship = {
                log: detachedControl.log.slice(),
                disabled: detachedControl.matches(":disabled")
              };

              return JSON.stringify({
                initial,
                ownDisabled,
                ownEnabled,
                fieldsetDisabled,
                dedupedInsideLog,
                fieldsetEnabled,
                detachedRelationship
              });
            })()
            "##,
        )
        .expect("form-associated custom element disabled-state probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":{"outsideEnabled":true,"outsideDisabled":false,"insideEnabled":true,"entries":["legend=face-value","inside=face-value","outside=face-value"]},"ownDisabled":{"log":[true],"disabled":true,"omitted":true,"focused":false},"ownEnabled":{"log":[true,false],"enabled":true,"value":"face-value","focused":true},"fieldsetDisabled":{"insideLog":[true],"legendLog":[],"insideDisabled":true,"legendEnabled":true,"insideOmitted":true,"legendValue":"face-value"},"dedupedInsideLog":[true],"fieldsetEnabled":{"insideLog":[true,false],"insideEnabled":true,"entries":["legend=face-value","inside=face-value","outside=face-value"]},"detachedRelationship":{"log":[true,false,true,false,true],"disabled":true}}"#
    );
}

#[test]
fn form_associated_custom_element_form_data_value_preserves_entry_list_snapshot() {
    let mut vm = new_storage_test_vm("https://face-form-data.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceFormDataElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("face-form-data-element", FaceFormDataElement);

              const form = document.createElement("form");
              const control = document.createElement("face-form-data-element");
              control.name = "ignored-owner-name";
              const supplied = new FormData();
              supplied.append("line", "alpha");
              supplied.append("line", "beta");
              supplied.append("meta", "first");
              control.internals.setFormValue(supplied);
              supplied.append("line", "late-mutation");
              supplied.delete("meta");
              form.append(control);
              (document.body || document.documentElement || document).append(form);

              return JSON.stringify(Array.from(new FormData(form)));
            })()
            "#,
        )
        .expect("form-associated FormData entry-list snapshot should evaluate");

    assert_eq!(
        result,
        r#"[["line","alpha"],["line","beta"],["meta","first"]]"#
    );
}

#[test]
fn form_associated_callbacks_distinguish_detached_insertion_from_connected_upgrade() {
    let mut vm = new_storage_test_vm("https://face-reaction-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const insertionHistory = [];
              class InsertedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.attachInternals();
                }
                connectedCallback() { insertionHistory.push("connected"); }
                formAssociatedCallback(form) {
                  insertionHistory.push(`form:${form?.id ?? "null"}`);
                }
              }
              customElements.define("inserted-reaction-face", InsertedFace);
              const insertedForm = document.createElement("form");
              insertedForm.id = "inserted-form";
              insertedForm.append(document.createElement("inserted-reaction-face"));
              const afterDetachedInsertion = insertionHistory.slice();
              (document.body || document.documentElement || document).append(insertedForm);

              const upgradeHistory = [];
              const upgradeForm = document.createElement("form");
              upgradeForm.id = "upgrade-form";
              upgradeForm.innerHTML = "<upgraded-reaction-face></upgraded-reaction-face>";
              (document.body || document.documentElement || document).append(upgradeForm);
              class UpgradedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.attachInternals();
                }
                connectedCallback() { upgradeHistory.push("connected"); }
                formAssociatedCallback(form) {
                  upgradeHistory.push(`form:${form?.id ?? "null"}`);
                }
              }
              customElements.define("upgraded-reaction-face", UpgradedFace);

              return JSON.stringify({
                afterDetachedInsertion,
                insertionHistory,
                upgradeHistory
              });
            })()
            "#,
        )
        .expect("FACE connection reaction order probe should evaluate");

    assert_eq!(
        result,
        r#"{"afterDetachedInsertion":["form:inserted-form"],"insertionHistory":["form:inserted-form","connected"],"upgradeHistory":["connected","form:upgrade-form"]}"#
    );
}

#[test]
fn document_adopt_node_defers_disconnected_reaction_until_owner_retarget() {
    let mut vm = new_storage_test_vm("https://face-adoption-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class AdoptedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.attachInternals();
                  this.history = [];
                }
                connectedCallback() {
                  this.history.push(`connected:${this.ownerDocument === document ? "main" : "other"}`);
                }
                disconnectedCallback() {
                  this.history.push(`disconnected:${this.ownerDocument === document ? "main" : "other"}`);
                }
                adoptedCallback(oldDocument, newDocument) {
                  this.history.push(`adopted:${oldDocument === document ? "main" : "other"}->${newDocument === document ? "main" : "other"}`);
                }
                formAssociatedCallback(form) {
                  this.history.push(`form:${form?.id ?? "null"}`);
                }
              }
              customElements.define("adopted-reaction-face", AdoptedFace);

              const first = document.createElement("form");
              first.id = "first";
              const face = document.createElement("adopted-reaction-face");
              first.append(face);
              (document.body || document.documentElement || document).append(first);
              const other = document.implementation.createHTMLDocument("other");
              other.adoptNode(face);
              other.body.append(face);
              document.adoptNode(face);
              const second = document.createElement("form");
              second.id = "second";
              second.append(face);
              (document.body || document.documentElement || document).append(second);
              return face.history.join("|");
            })()
            "#,
        )
        .expect("adoptNode reaction ordering probe should evaluate");

    assert_eq!(
        result,
        "form:first|connected:main|disconnected:other|form:null|adopted:main->other|connected:other|disconnected:main|adopted:other->main|form:second|connected:main"
    );
}

#[test]
fn form_associated_custom_element_validation_participates_in_forms() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceValidityElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.i = this.attachInternals();
                }
              }
              customElements.define("wpt-face-validity", FaceValidityElement);

              const probe = fn => {
                try {
                  fn();
                  return "ok";
                } catch (error) {
                  return `${error.name}:${error.code || 0}`;
                }
              };

              const control = document.createElement("wpt-face-validity");
              const root = document.body || document.documentElement || document;
              root.appendChild(control);
              const missingMessage = probe(() => control.i.setValidity({ valueMissing: true }));
              control.i.setValidity({ valueMissing: true }, "missing");
              const invalidEvents = [];
              control.addEventListener("invalid", event => {
                invalidEvents.push(`${event.cancelable}:${event.target === control}`);
              });
              const directCheck = control.i.checkValidity();
              const directReport = control.i.reportValidity();

              const outside = document.createElement("div");
              root.appendChild(outside);
              const outsideAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", outside);
              });
              const foreign = document.createElementNS("urn:not-html", "foo");
              control.appendChild(foreign);
              const foreignAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", foreign);
              });
              foreign.remove();
              const lightChild = document.createElement("span");
              control.appendChild(lightChild);
              const lightAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", lightChild);
              });
              const shadow = control.attachShadow({ mode: "open" });
              const shadowChild = document.createElement("span");
              shadow.appendChild(shadowChild);
              const shadowAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", shadowChild);
              });
              control.remove();

              const container = document.createElement("div");
              container.innerHTML = `
                <form>
                  <fieldset>
                    <wpt-face-validity></wpt-face-validity>
                    <input type="submit">
                  </fieldset>
                </form>
              `;
              root.appendChild(container);
              const form = container.querySelector("form");
              const fieldset = container.querySelector("fieldset");
              const face = container.querySelector("wpt-face-validity");
              let aggregateInvalids = 0;
              face.addEventListener("invalid", () => ++aggregateInvalids);

              const initialCss = `${face.matches(":valid")}/${form.matches(":valid")}/${fieldset.matches(":valid")}`;
              face.i.setValidity({ customError: true }, "bad");
              const invalidCss = `${face.matches(":invalid")}/${form.matches(":invalid")}/${fieldset.matches(":invalid")}`;
              const formCheck = form.checkValidity();
              const formReport = form.reportValidity();
              container.querySelector("input").click();
              const aggregateCount = aggregateInvalids;
              face.remove();
              const detachedCss = `${face.matches(":invalid")}/${form.matches(":valid")}/${fieldset.matches(":valid")}`;
              fieldset.appendChild(face);
              const reattachedCss = `${form.matches(":invalid")}/${fieldset.matches(":invalid")}`;
              face.i.setValidity({});
              const clearedCss = `${face.matches(":valid")}/${form.matches(":valid")}/${fieldset.matches(":valid")}`;

              return JSON.stringify({
                missingMessage,
                directCheck,
                directReport,
                invalidEvents,
                outsideAnchor,
                foreignAnchor,
                lightAnchor,
                shadowAnchor,
                initialCss,
                invalidCss,
                formCheck,
                formReport,
                aggregateCount,
                detachedCss,
                reattachedCss,
                clearedCss
              });
            })()
            "##,
        )
        .expect("form-associated custom element validation probe should evaluate");

    assert_eq!(
        result,
        r#"{"missingMessage":"TypeError:0","directCheck":false,"directReport":false,"invalidEvents":["true:true","true:true"],"outsideAnchor":"NotFoundError:8","foreignAnchor":"TypeError:0","lightAnchor":"ok","shadowAnchor":"ok","initialCss":"true/true/true","invalidCss":"true/true/true","formCheck":false,"formReport":false,"aggregateCount":3,"detachedCss":"true/true/true","reattachedCss":"true/true","clearedCss":"true/true/true"}"#
    );
}

#[test]
fn custom_element_lifecycle_runs_in_detached_document_trees() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class DetachedLifecycleElement extends HTMLElement {
                connectedCallback() { calls.push("connected:" + this.localName); }
                disconnectedCallback() { calls.push("disconnected:" + this.localName); }
              }
              customElements.define("wpt-detached-lifecycle", DetachedLifecycleElement);

              const doc = document.implementation.createHTMLDocument("");
              const direct = document.createElement("wpt-detached-lifecycle");
              doc.documentElement.appendChild(direct);
              doc.documentElement.removeChild(direct);

              const host = doc.createElement("div");
              const shadowRoot = host.attachShadow({ mode: "closed" });
              const shadowChild = document.createElement("wpt-detached-lifecycle");
              shadowRoot.appendChild(shadowChild);
              calls.push("detached:" + calls.length);
              doc.documentElement.appendChild(host);
              doc.documentElement.removeChild(host);

              const template = document.createElement("template");
              const templateDoc = template.content.ownerDocument;
              templateDoc.appendChild(templateDoc.createElement("html"));
              const templateChild = document.createElement("wpt-detached-lifecycle");
              templateDoc.documentElement.appendChild(templateChild);
              templateDoc.documentElement.removeChild(templateChild);

              const clonedDoc = document.cloneNode(false);
              clonedDoc.appendChild(clonedDoc.createElement("html"));
              const clonedChild = document.createElement("wpt-detached-lifecycle");
              clonedDoc.documentElement.appendChild(clonedChild);
              clonedDoc.documentElement.removeChild(clonedChild);

              return calls.join("|");
            })()
            "#,
        )
        .expect("detached document custom element lifecycle probe should evaluate");

    assert_eq!(
        result,
        "connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle|detached:2|connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle|connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle|connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle"
    );
}

#[test]
fn detached_native_attribute_node_mutation_flushes_reactions_after_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              let target;
              let attribute;
              class DetachedAttributeElement extends HTMLElement {
                static get observedAttributes() { return ["data-probe", "class", "id"]; }
                attributeChangedCallback(name, oldValue, newValue) {
                  calls.push([
                    name,
                    oldValue,
                    newValue,
                    attribute.ownerElement === target,
                    this.getAttribute(name)
                  ].join(":"));
                }
              }
              const registry = new CustomElementRegistry();
              registry.define("wpt-detached-attr-reaction", DetachedAttributeElement);

              const doc = document.implementation.createHTMLDocument("");
              target = doc.createElement("wpt-detached-attr-reaction");
              registry.initialize(target);
              attribute = doc.createAttribute("data-probe");
              attribute.value = "one";
              target.setAttributeNode(attribute);
              calls.push("after:" + (attribute.ownerElement === target));
              target.dataset.probe = "two";
              calls.push("after-dataset");
              target.classList.add("ready");
              calls.push("after-class");
              attribute.value = "three";
              calls.push("after-attr-value");
              target.id = "native-id";
              calls.push("after-id");
              return calls.join("|");
            })()
            "#,
        )
        .expect("detached native attribute-node reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "data-probe::one:true:one|after:true|data-probe:one:two:true:two|after-dataset|class::ready:true:ready|after-class|data-probe:two:three:true:three|after-attr-value|id::native-id:true:native-id|after-id"
    );
}

#[test]
fn detached_native_tree_mutation_flushes_reactions_after_operation_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const calls = [];
              const childSummary = (parent) => Array.from(parent.childNodes).map((node) => {
                return node.nodeType === Node.TEXT_NODE ? "#text" : node.id;
              }).join(",");
              class DetachedTreeReactionElement extends HTMLElement {
                connectedCallback() {
                  calls.push([
                    "connected",
                    this.id,
                    this.parentNode && this.parentNode.childNodes.length,
                    childSummary(this.parentNode)
                  ].join(":"));
                }
                disconnectedCallback() {
                  calls.push([
                    "disconnected",
                    this.id,
                    doc.body.childNodes.length,
                    doc.body.firstElementChild && doc.body.firstElementChild.id
                  ].join(":"));
                }
              }
              customElements.define("wpt-detached-tree-reaction", DetachedTreeReactionElement);

              const doc = document.implementation.createHTMLDocument("");
              const first = document.createElement("wpt-detached-tree-reaction");
              first.id = "first";
              const second = document.createElement("wpt-detached-tree-reaction");
              second.id = "second";
              doc.body.append(first, "middle", second);
              calls.push("after-append:" + doc.body.childNodes.length);

              const replacement = document.createElement("wpt-detached-tree-reaction");
              replacement.id = "replacement";
              doc.body.replaceChildren(replacement);
              calls.push(
                "after-replace:" +
                doc.body.childNodes.length +
                ":" +
                doc.body.firstElementChild.id
              );
              return calls.join("|");
            })()
            "##,
        )
        .expect("detached native tree mutation reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "connected:first:3:first,#text,second|connected:second:3:first,#text,second|after-append:3|disconnected:first:1:replacement|disconnected:second:1:replacement|connected:replacement:1:replacement|after-replace:1:replacement"
    );
}

#[test]
fn detached_native_text_content_flushes_reactions_after_replacement_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const calls = [];
              class DetachedTextContentReactionElement extends HTMLElement {
                disconnectedCallback() {
                  calls.push([
                    "disconnected",
                    this.id,
                    doc.body.childNodes.length,
                    doc.body.firstChild && doc.body.firstChild.nodeValue
                  ].join(":"));
                }
              }
              customElements.define(
                "wpt-detached-textcontent-reaction",
                DetachedTextContentReactionElement
              );

              const doc = document.implementation.createHTMLDocument("");
              const first = document.createElement("wpt-detached-textcontent-reaction");
              first.id = "first";
              const second = document.createElement("wpt-detached-textcontent-reaction");
              second.id = "second";
              doc.body.append(first, second);

              doc.body.textContent = "fresh";
              calls.push(
                "after-set:" +
                doc.body.childNodes.length +
                ":" +
                doc.body.firstChild.nodeValue
              );
              return calls.join("|");
            })()
            "##,
        )
        .expect("detached native textContent reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "disconnected:first:1:fresh|disconnected:second:1:fresh|after-set:1:fresh"
    );
}

#[test]
fn detached_native_inner_html_flushes_reactions_after_replacement_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const calls = [];
              class DetachedInnerHTMLReactionElement extends HTMLElement {
                disconnectedCallback() {
                  calls.push([
                    "disconnected",
                    this.id,
                    doc.body.childNodes.length,
                    doc.body.firstElementChild && doc.body.firstElementChild.localName,
                    doc.body.textContent
                  ].join(":"));
                }
              }
              customElements.define(
                "wpt-detached-innerhtml-reaction",
                DetachedInnerHTMLReactionElement
              );

              const doc = document.implementation.createHTMLDocument("");
              const first = document.createElement("wpt-detached-innerhtml-reaction");
              first.id = "first";
              const second = document.createElement("wpt-detached-innerhtml-reaction");
              second.id = "second";
              doc.body.append(first, second);

              doc.body.innerHTML = "<p>fresh</p>";
              calls.push(
                "after-set:" +
                doc.body.childNodes.length +
                ":" +
                doc.body.firstElementChild.localName +
                ":" +
                doc.body.textContent
              );
              return calls.join("|");
            })()
            "##,
        )
        .expect("detached native innerHTML reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "disconnected:first:1:p:fresh|disconnected:second:1:p:fresh|after-set:1:p:fresh"
    );
}

#[test]
fn custom_element_adopted_callback_runs_before_connected_in_new_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class AdoptedLifecycleElement extends HTMLElement {
                connectedCallback() {
                  calls.push([
                    "connected",
                    this.ownerDocument === globalThis.__adoptTarget,
                    this.isConnected
                  ].join(":"));
                }
                disconnectedCallback() { calls.push(`disconnected:${this.isConnected}`); }
                adoptedCallback(oldDocument, newDocument) {
                  calls.push([
                    "adopted",
                    oldDocument === document,
                    newDocument === globalThis.__adoptTarget
                  ].join(":"));
                }
              }
              customElements.define("wpt-adopted-lifecycle", AdoptedLifecycleElement);

              const detached = document.implementation.createHTMLDocument("");
              globalThis.__adoptTarget = detached;
              const first = document.createElement("wpt-adopted-lifecycle");
              detached.documentElement.appendChild(first);

              const second = document.createElement("wpt-adopted-lifecycle");
              (document.body || document.documentElement || document).appendChild(second);
              detached.documentElement.appendChild(second);

              const cloned = document.cloneNode(false);
              cloned.appendChild(cloned.createElement("html"));
              globalThis.__adoptTarget = cloned;
              const third = document.createElement("wpt-adopted-lifecycle");
              cloned.documentElement.appendChild(third);

              return calls.join("|");
            })()
            "#,
        )
        .expect("custom element adoptedCallback probe should evaluate");

    assert_eq!(
        result,
        "adopted:true:true|connected:true:true|connected:false:true|disconnected:true|adopted:true:true|connected:true:true|adopted:true:true|connected:true:true"
    );
}

#[test]
fn nested_disconnected_reaction_flushes_pending_connected_before_remove_returns() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.body || document.documentElement || document;
              const logs = [];

              class ParentElement extends HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.remove();
                  logs.push("end");
                }
              }
              customElements.define("wpt-nested-reaction-parent", ParentElement);

              class ChildElement extends HTMLElement {
                connectedCallback() { logs.push("connected"); }
                disconnectedCallback() { logs.push("disconnected"); }
              }
              customElements.define("wpt-nested-reaction-child", ChildElement);

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              target.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("nested disconnected custom element reaction probe should evaluate");

    assert_eq!(result, "begin|connected|disconnected|end");
}

#[test]
fn nested_removal_without_disconnected_callback_leaves_pending_connected_in_outer_queue() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.body || document.documentElement || document;
              const logs = [];

              class ParentElement extends HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.remove();
                  logs.push("end");
                }
              }
              customElements.define("wpt-nested-no-disconnected-parent", ParentElement);

              class ChildElement extends HTMLElement {
                connectedCallback() { logs.push("connected"); }
              }
              customElements.define("wpt-nested-no-disconnected-child", ChildElement);

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              target.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("nested removal without disconnected callback probe should evaluate");

    assert_eq!(result, "begin|end|connected");
}

#[test]
fn nested_observed_attribute_reaction_flushes_pending_connected_before_set_attribute_returns() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.body || document.documentElement || document;
              const logs = [];

              class ParentElement extends HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.setAttribute("title", "foo");
                  logs.push("end");
                }
              }
              customElements.define("wpt-nested-attribute-parent", ParentElement);

              class ChildElement extends HTMLElement {
                static get observedAttributes() { return ["title"]; }
                connectedCallback() { logs.push("connected"); }
                attributeChangedCallback() { logs.push("attributeChanged"); }
              }
              customElements.define("wpt-nested-attribute-child", ChildElement);

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              target.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("nested observed attribute custom element reaction probe should evaluate");

    assert_eq!(result, "begin|connected|attributeChanged|end");
}

#[test]
fn child_window_nested_observed_attribute_reaction_flushes_pending_connected() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const target = document.body || document.documentElement || document;
              target.appendChild(frame);
              const { contentWindow, contentDocument } = frame;
              const logs = [];

              class ParentElement extends contentWindow.HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.setAttribute("title", "foo");
                  logs.push("end");
                }
              }
              contentWindow.customElements.define(
                "wpt-child-nested-attribute-parent",
                ParentElement
              );

              class ChildElement extends contentWindow.HTMLElement {
                static get observedAttributes() { return ["title"]; }
                connectedCallback() { logs.push("connected"); }
                attributeChangedCallback() { logs.push("attributeChanged"); }
              }
              contentWindow.customElements.define(
                "wpt-child-nested-attribute-child",
                ChildElement
              );

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              contentDocument.body.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("child-window nested observed attribute reaction probe should evaluate");

    assert_eq!(result, "begin|connected|attributeChanged|end");
}

#[test]
fn html_fragment_parser_upgrade_reaction_runs_before_descendant_connected_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const logs = [];
              let childConnected = 0;

              class Parenter extends HTMLElement {
                connectedCallback() {
                  logs.push("parent:connected");
                  const child = this.firstChild;
                  this.removeChild(child);
                  logs.push("parent:removed");
                  this.appendChild(child);
                  logs.push("parent:appended");
                }
              }
              customElements.define("wpt-fragment-reaction-parent", Parenter);

              class Child extends HTMLElement {
                connectedCallback() {
                  childConnected++;
                  logs.push(`child:connected:${childConnected}`);
                }
              }
              customElements.define("wpt-fragment-reaction-child", Child);

              const target = document.createElement("section");
              document.appendChild(target);
              target.innerHTML =
                "<wpt-fragment-reaction-parent><wpt-fragment-reaction-child></wpt-fragment-reaction-child></wpt-fragment-reaction-parent>";

              return [
                childConnected,
                logs.join("|"),
                document.querySelector("wpt-fragment-reaction-child") instanceof Child
              ].join(",");
            })()
            "#,
        )
        .expect("HTML fragment custom element upgrade reaction probe should evaluate");

    assert_eq!(
        result,
        "1,parent:connected|parent:removed|parent:appended|child:connected:1,true"
    );
}

#[test]
fn html_fragment_parser_upgrade_reaction_survives_move_to_new_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              const targetDocument = new Document();
              targetDocument.appendChild(targetDocument.createElement("html"));
              const logs = [];
              let childConnected = 0;

              class Parenter extends HTMLElement {
                connectedCallback() {
                  logs.push("parent:connected");
                  const child = this.firstChild;
                  this.removeChild(child);
                  logs.push("parent:removed");
                  this.appendChild(child);
                  logs.push("parent:appended");
                }
              }
              customElements.define("wpt-fragment-move-parent", Parenter);

              class Child extends HTMLElement {
                connectedCallback() {
                  childConnected++;
                  logs.push(`child:connected:${childConnected}`);
                }
              }
              customElements.define("wpt-fragment-move-child", Child);

              document.documentElement.innerHTML =
                "<wpt-fragment-move-parent><wpt-fragment-move-child></wpt-fragment-move-child></wpt-fragment-move-parent>";
              targetDocument.documentElement.appendChild(document.documentElement.firstChild);

              return [childConnected, logs.join("|")].join(",");
            })()
            "#,
        )
        .expect("HTML fragment custom element move probe should evaluate");

    assert_eq!(
        result,
        "1,parent:connected|parent:removed|parent:appended|child:connected:1"
    );
}

#[test]
fn document_element_inner_html_fragment_preserves_html_context_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              document.documentElement.innerHTML =
                "<wpt-html-context-parent><wpt-html-context-child></wpt-html-context-child></wpt-html-context-parent>";
              return Array.from(document.documentElement.childNodes)
                .map(node => node.nodeName)
                .join("|");
            })()
            "#,
        )
        .expect("documentElement innerHTML html-context probe should evaluate");

    assert_eq!(result, "HEAD|BODY");
}

#[test]
fn empty_detached_html_inner_html_creates_head_and_body() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.createElement("html");
              html.innerHTML = "";
              return Array.from(html.childNodes)
                .map(node => node.nodeName)
                .join("|");
            })()
            "#,
        )
        .expect("empty detached HTML innerHTML probe should evaluate");

    assert_eq!(result, "HEAD|BODY");
}

#[test]
fn child_window_text_replacement_setters_dispatch_disconnected_callbacks() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const target = document.body || document.documentElement || document;
              target.appendChild(frame);
              const { contentWindow, contentDocument } = frame;
              const logs = [];

              class ChildElement extends contentWindow.HTMLElement {
                connectedCallback() { logs.push("connected"); }
                disconnectedCallback() { logs.push("disconnected"); }
              }
              contentWindow.customElements.define(
                "wpt-child-text-replacement-child",
                ChildElement
              );

              const probe = (html, selector, setup, apply) => {
                logs.length = 0;
                contentDocument.body.innerHTML = html;
                const element = contentDocument.querySelector(selector);
                setup(element);
                const before = element.innerHTML;
                const initialLog = logs.join("|");
                logs.length = 0;
                apply(element);
                return [
                  before,
                  element.innerHTML,
                  initialLog,
                  logs.join("|")
                ].join(",");
              };

              return [
                probe(
                  "<a><wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child></a>",
                  "a",
                  () => {},
                  element => { element.text = "world"; }
                ),
                probe(
                  "<select><option></option></select>",
                  "option",
                  element => {
                    const child = contentDocument.createElement("wpt-child-text-replacement-child");
                    element.appendChild(child);
                    child.textContent = "hello";
                  },
                  element => { element.text = "world"; }
                ),
                probe(
                  "<output><wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child></output>",
                  "output",
                  () => {},
                  element => { element.value = "world"; }
                ),
                probe(
                  "<output><wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child></output>",
                  "output",
                  () => {},
                  element => { element.defaultValue = "world"; }
                )
              ].join(";");
            })()
            "#,
        )
        .expect("child window text replacement setter probe should evaluate");

    assert_eq!(
        result,
        "<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected;<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected;<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected;<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected"
    );
}

#[test]
fn custom_elements_when_defined_resolves_with_constructor() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              class PendingElement extends HTMLElement {}
              globalThis.__pendingElementCtor = PendingElement;
              globalThis.__whenDefinedResolved = null;
              customElements.whenDefined("wpt-script-vm-pending").then((value) => {
                globalThis.__whenDefinedResolved = value;
              });
              customElements.define("wpt-script-vm-pending", PendingElement);
              return "setup";
            })()
            "#,
    )
    .expect("customElements.whenDefined setup should evaluate");

    let result = vm
        .eval(
            r#"
            (() => {
              return [
                globalThis.__whenDefinedResolved === globalThis.__pendingElementCtor,
                customElements.get("wpt-script-vm-pending") === globalThis.__pendingElementCtor
              ].join("|");
            })()
            "#,
        )
        .expect("customElements.whenDefined should resolve with the registered constructor");

    assert_eq!(result, "true|true");
}

#[test]
fn detached_template_shadow_root_reflection_setters_update_native_attributes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const template = doc.createElement("template");
              const prototypeSetter = (name, value) => {
                Object
                  .getOwnPropertyDescriptor(HTMLTemplateElement.prototype, name)
                  .set.call(template, value);
              };

              template.shadowRootMode = "closed";
              template.shadowRootSlotAssignment = "manual";
              template.shadowRootAdoptedStyleSheets = "sheet-1";
              template.shadowRootDelegatesFocus = true;
              template.shadowRootClonable = true;
              template.shadowRootSerializable = true;
              template.shadowRootCustomElementRegistry = "first";
              const direct = [
                template.shadowRootMode,
                template.getAttribute("shadowrootmode"),
                template.shadowRootSlotAssignment,
                template.getAttribute("shadowrootslotassignment"),
                template.shadowRootAdoptedStyleSheets,
                template.getAttribute("shadowrootadoptedstylesheets"),
                template.shadowRootDelegatesFocus,
                template.hasAttribute("shadowrootdelegatesfocus"),
                template.shadowRootClonable,
                template.hasAttribute("shadowrootclonable"),
                template.shadowRootSerializable,
                template.hasAttribute("shadowrootserializable"),
                template.shadowRootCustomElementRegistry,
                template.getAttribute("shadowrootcustomelementregistry")
              ].join(":");

              prototypeSetter("shadowRootMode", "open");
              prototypeSetter("shadowRootSlotAssignment", "named");
              prototypeSetter("shadowRootAdoptedStyleSheets", "sheet-2");
              prototypeSetter("shadowRootDelegatesFocus", false);
              prototypeSetter("shadowRootClonable", false);
              prototypeSetter("shadowRootSerializable", false);
              prototypeSetter("shadowRootCustomElementRegistry", "second");
              const prototype = [
                template.shadowRootMode,
                template.getAttribute("shadowrootmode"),
                template.shadowRootSlotAssignment,
                template.getAttribute("shadowrootslotassignment"),
                template.shadowRootAdoptedStyleSheets,
                template.getAttribute("shadowrootadoptedstylesheets"),
                template.shadowRootDelegatesFocus,
                template.hasAttribute("shadowrootdelegatesfocus"),
                template.shadowRootClonable,
                template.hasAttribute("shadowrootclonable"),
                template.shadowRootSerializable,
                template.hasAttribute("shadowrootserializable"),
                template.shadowRootCustomElementRegistry,
                template.getAttribute("shadowrootcustomelementregistry")
              ].join(":");

              return direct + "|" + prototype;
            })()
            "#,
        )
        .expect("detached template shadowRoot reflection setter probe should evaluate");

    assert_eq!(
        result,
        "closed:closed:manual:manual:sheet-1:sheet-1:true:true:true:true:true:true:first:first|open:open:named:named:sheet-2:sheet-2:false:false:false:false:false:false:second:second"
    );
}

#[test]
fn parser_created_customelementregistry_sets_null_registry_once() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const documentElement = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                documentElement.appendChild(document.createElement("body"));
              const frame = document.createElement("iframe");
              body.appendChild(frame);
              const doc = frame.contentDocument;
              const win = frame.contentWindow;
              win.customElements.define("wpt-null-attr", class extends win.HTMLElement {});

              doc.documentElement.setHTMLUnsafe(
                "<div id='builtin' customelementregistry></div>" +
                "<wpt-null-attr id='defined' customelementregistry></wpt-null-attr>" +
                "<a-b id='candidate' customelementregistry></a-b>"
              );
              const builtin = doc.getElementById("builtin");
              const defined = doc.getElementById("defined");
              const candidate = doc.getElementById("candidate");

              const dynamic = doc.createElement("div");
              dynamic.setAttribute("customelementregistry", "");

              const explicitNull = doc.createElement("div", {
                customElementRegistry: null
              });
              const registryOptionSetterHits = [];
              Object.defineProperty(Object.prototype, "customElementRegistry", {
                configurable: true,
                get() { return undefined; },
                set(value) {
                  const receiverKind = this instanceof win.Element ? "element" : "plain";
                  registryOptionSetterHits.push(receiverKind);
                  Object.defineProperty(this, "customElementRegistry", {
                    configurable: true,
                    enumerable: true,
                    writable: true,
                    value
                  });
                }
              });
              let explicitNullClone;
              let parserClone;
              try {
                explicitNullClone = explicitNull.cloneNode(false);
                parserClone = builtin.cloneNode(true);
              } finally {
                delete Object.prototype.customElementRegistry;
              }

              const container = doc.createElement("div", {
                customElementRegistry: null
              });
              container.innerHTML =
                "<a-b id='nested'><wpt-null-attr id='nested-defined'></wpt-null-attr></a-b>";
              const nested = container.querySelector("#nested");
              const nestedDefined = container.querySelector("#nested-defined");

              return JSON.stringify({
                builtin: builtin.customElementRegistry,
                defined: defined.customElementRegistry,
                candidate: candidate.customElementRegistry,
                dynamicUsesDefault:
                  dynamic.customElementRegistry === win.customElements,
                explicitNull: explicitNull.customElementRegistry,
                explicitNullClone: explicitNullClone.customElementRegistry,
                parserClone: parserClone.customElementRegistry,
                registryOptionSetterHits: registryOptionSetterHits
                  .filter(hit => hit === "plain"),
                nested: nested.customElementRegistry,
                nestedDefined: nestedDefined.customElementRegistry
              });
            })()
            "##,
        )
        .expect("customelementregistry parser attribute probe should evaluate");

    assert_eq!(
        result,
        r#"{"builtin":null,"defined":null,"candidate":null,"dynamicUsesDefault":true,"explicitNull":null,"explicitNullClone":null,"parserClone":null,"registryOptionSetterHits":[],"nested":null,"nestedDefined":null}"#
    );
}

#[test]
fn custom_elements_when_defined_rejects_invalid_name() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              globalThis.__whenDefinedRejected = "pending";
              customElements.whenDefined("notcustom").then(
                () => {
                  globalThis.__whenDefinedRejected = "fulfilled";
                },
                (error) => {
                  globalThis.__whenDefinedRejected = [
                    error && error.name,
                    error instanceof DOMException
                  ].join("|");
                }
              );
              return "setup";
            })()
            "#,
    )
    .expect("customElements.whenDefined invalid-name setup should evaluate");

    let result = vm
        .eval(
            r#"
            (() => globalThis.__whenDefinedRejected)()
            "#,
        )
        .expect("customElements.whenDefined invalid-name rejection should settle");

    assert_eq!(result, "SyntaxError|true");
}

#[test]
fn child_parser_checkpoints_once_before_adoption_agency_custom_element_construction() {
    let mut vm = new_storage_test_vm("https://parser-checkpoint.test/");

    vm.eval(
        r##"
            (() => {
              globalThis.__childParserCheckpoint = "pending";
              const frame = document.createElement("iframe");
              frame.srcdoc = `<!doctype html><body><script>
                class ParserCheckpointElement extends HTMLElement {
                  constructor() {
                    super();
                    const nodeLabel = node => node.nodeType === Node.TEXT_NODE
                      ? "#text:" + node.data
                      : node.localName;
                    top.__childParserCheckpoint = JSON.stringify(
                      recordsList.map(records => records.map(record => [
                        nodeLabel(record.target),
                        Array.prototype.map.call(record.addedNodes, nodeLabel).join(",")
                      ]))
                    );
                  }
                }
                customElements.define(
                  "parser-checkpoint-element",
                  ParserCheckpointElement
                );
                const recordsList = [];
                new MutationObserver(records => recordsList.push(records)).observe(
                  document.body,
                  { childList: true, subtree: true }
                );
              </script><b><i>hello</b><parser-checkpoint-element>`;
              (document.body || document.documentElement || document).appendChild(frame);
              return "scheduled";
            })()
            "##,
    )
    .expect("child parser checkpoint setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval("globalThis.__childParserCheckpoint")
        .expect("child parser checkpoint result should evaluate");

    assert_eq!(
        result,
        r##"[[["body","b"],["b","i"],["i","#text:hello"],["body","i"]]]"##
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_write_custom_element_reaction_queue_wpt_shape() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://custom-element-reaction-queue.test/",
        &loader,
    );

    vm.eval(
        r#"
            (() => {
              globalThis.__reactionQueueResult = "pending";
              new Promise((resolve) => {
                const frame = document.createElement("iframe");
                frame.srcdoc = "";
                frame.onload = () => resolve(frame.contentWindow);
                (document.body || document.documentElement || document).appendChild(frame);
              }).then((contentWindow) => {
                try {
                  const contentDocument = contentWindow.document;
                  contentDocument.write('<test-element id="first-element">');
                  contentDocument.write('<test-element id="second-element">');

                  const element1 = contentDocument.getElementById("first-element");
                  const element2 = contentDocument.getElementById("second-element");
                  const log = [];

                  class TestElement extends contentWindow.HTMLElement {
                    constructor() {
                      super();
                      log.push(`constructed:${this.id}`);
                    }
                    connectedCallback() {
                      log.push(`connected:${this.id}`);
                    }
                    attributeChangedCallback(name, oldValue, newValue, namespace) {
                      log.push([
                        "attribute",
                        this.id,
                        name,
                        oldValue,
                        newValue,
                        this.getAttributeNS(namespace, name)
                      ].join(":"));
                    }
                    static get observedAttributes() { return ["id"]; }
                  }

                  contentWindow.customElements.define("test-element", TestElement);
                  globalThis.__reactionQueueResult = JSON.stringify({
                    element1: element1 && element1.localName,
                    element2: element2 && element2.localName,
                    element1ProtoAfterUpgrade:
                      element1 && Object.getPrototypeOf(element1) === TestElement.prototype,
                    element2ProtoAfterUpgrade:
                      element2 && Object.getPrototypeOf(element2) === TestElement.prototype,
                    log
                  });
                } catch (error) {
                  globalThis.__reactionQueueResult =
                    "throw:" + error.name + ":" + error.message;
                }
              }, (error) => {
                globalThis.__reactionQueueResult =
                  "reject:" + error.name + ":" + error.message;
              });
              return "scheduled";
            })()
            "#,
    )
    .expect("child document.write custom element WPT shape should evaluate");

    let expected = r#"{"element1":"test-element","element2":"test-element","element1ProtoAfterUpgrade":true,"element2ProtoAfterUpgrade":true,"log":["constructed:first-element","attribute:first-element:id::first-element:first-element","connected:first-element","constructed:second-element","attribute:second-element:id::second-element:second-element","connected:second-element"]}"#;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__reactionQueueResult",
        expected,
        "child document.write custom-element reaction queue",
    )
    .await;

    let result = vm
        .eval("globalThis.__reactionQueueResult")
        .expect("child document.write custom element WPT shape result should evaluate");

    assert_eq!(result, expected);
}

#[test]
fn child_document_write_constructs_and_connects_predefined_custom_elements_without_a_body() {
    let mut vm = new_storage_test_vm("https://document-write-custom-elements.test/");

    vm.eval(
        r#"
            (() => {
              const target = document.body || document.documentElement || document;
              window.__writeCustomElementFrames = ["write", "writeln"].map(() => {
                const frame = document.createElement("iframe");
                target.appendChild(frame);
                frame.srcdoc = "";
                return frame;
              });
              return "scheduled";
            })()
            "#,
    )
    .expect("child document.write predefined custom element setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const exercise = (frame, method, name) => {
                const childWindow = frame.contentWindow;
                const childDocument = frame.contentDocument;
                const registry = childWindow.customElements;
                let constructorCount = 0;
                let connectedCount = 0;
                let errorName = null;
                childWindow.addEventListener("error", event => {
                  errorName = event.error && event.error.name;
                  event.preventDefault();
                }, { once: true });
                class DefinedElement extends childWindow.HTMLElement {
                  constructor() {
                    super();
                    constructorCount++;
                  }
                  connectedCallback() {
                    connectedCount++;
                  }
                }
                registry.define(name, DefinedElement);
                const definitionBeforeWrite = registry.get(name) === DefinedElement;

                childDocument[method](`<${name}></${name}>`);
                const element = childDocument.querySelector(name);
                return {
                  registryPreserved: childWindow.customElements === registry,
                  definitionBeforeWrite,
                  definitionAfterWrite: registry.get(name) === DefinedElement,
                  constructorCount,
                  connectedCount,
                  errorName,
                  htmlElement: element instanceof childWindow.HTMLElement,
                  customElement: element instanceof DefinedElement,
                  ownerDocument: element.ownerDocument === childDocument
                };
              };

              const [writeFrame, writelnFrame] = window.__writeCustomElementFrames;
              return JSON.stringify({
                write: exercise(writeFrame, "write", "write-defined-element"),
                writeln: exercise(writelnFrame, "writeln", "writeln-defined-element")
              });
            })()
            "#,
        )
        .expect("child document.write predefined custom element result should evaluate");

    assert_eq!(
        result,
        r#"{"write":{"registryPreserved":true,"definitionBeforeWrite":true,"definitionAfterWrite":true,"constructorCount":1,"connectedCount":1,"errorName":null,"htmlElement":true,"customElement":true,"ownerDocument":true},"writeln":{"registryPreserved":true,"definitionBeforeWrite":true,"definitionAfterWrite":true,"constructorCount":1,"connectedCount":1,"errorName":null,"htmlElement":true,"customElement":true,"ownerDocument":true}}"#
    );
}
