use super::*;

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
