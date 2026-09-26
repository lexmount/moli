use super::*;

#[test]
fn existing_upgrade_preserves_prototype_selected_by_wrapping_constructor() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              const element = document.createElement("wrapped-upgrade-element");
              body.appendChild(element);
              const log = [];

              class RealElement extends HTMLElement {
                polymerMethod() { return "available"; }
              }
              class WrappingElement extends HTMLElement {
                constructor() {
                  super();
                  Object.setPrototypeOf(this, RealElement.prototype);
                }
                connectedCallback() {
                  log.push(this.polymerMethod());
                }
              }

              customElements.define("wrapped-upgrade-element", WrappingElement);
              return JSON.stringify({
                realPrototype: Object.getPrototypeOf(element) === RealElement.prototype,
                wrapperPrototype: Object.getPrototypeOf(element) === WrappingElement.prototype,
                realInstance: element instanceof RealElement,
                method: element.polymerMethod(),
                log
              });
            })()
            "#,
        )
        .expect("wrapping constructor prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"realPrototype":true,"wrapperPrototype":false,"realInstance":true,"method":"available","log":["available"]}"#
    );
}

#[test]
fn synchronous_creation_preserves_prototype_selected_by_constructor() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              class RealElement extends HTMLElement {
                polymerMethod() { return "available"; }
              }
              class WrappingElement extends HTMLElement {
                constructor() {
                  super();
                  Object.setPrototypeOf(this, RealElement.prototype);
                }
              }

              customElements.define("wrapped-created-element", WrappingElement);
              const element = document.createElement("wrapped-created-element");
              return JSON.stringify({
                realPrototype: Object.getPrototypeOf(element) === RealElement.prototype,
                wrapperPrototype: Object.getPrototypeOf(element) === WrappingElement.prototype,
                realInstance: element instanceof RealElement,
                method: element.polymerMethod()
              });
            })()
            "#,
        )
        .expect("synchronous wrapping constructor prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"realPrototype":true,"wrapperPrototype":false,"realInstance":true,"method":"available"}"#
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
