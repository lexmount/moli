use super::*;

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
        "0||0||inner:connected|inner:disconnected|outer:connected|outer:disconnected"
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
fn element_aria_element_reference_reflection_preserves_assigned_value() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const element = document.createElement("div");
              const defaultReferences = element.ariaDescribedByElements;
              element.ariaControlsElements = [target];
              element.ariaActiveDescendantElement = target;
              return [
                Array.isArray(defaultReferences),
                defaultReferences.length,
                element.getAttribute("aria-controls") === "",
                element.ariaControlsElements[0] === target,
                element.getAttribute("aria-activedescendant") === "",
                element.ariaActiveDescendantElement === target
              ].join("|");
            })()
            "#,
        )
        .expect("ARIA element reference reflection probe should evaluate");

    assert_eq!(result, "true|0|true|true|true|true");
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
