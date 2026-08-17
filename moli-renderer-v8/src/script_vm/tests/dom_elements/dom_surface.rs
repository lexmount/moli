use super::*;

async fn expect_one_child_frame_task_source(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    let source = vm.run_next_child_frame_semantic_turn_for_test().await;
    assert_eq!(source, Some(expected), "{context}");
}

/// Settle at most one realm-materialization prerequisite before observing the
/// requested child-family turn. This is test setup, not a one-turn executor.
async fn expect_child_frame_task_source_after_realm_prerequisite(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::RealmMaterialization,
        )
    {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{context}: exact child realm prerequisite"
        );
    }
    expect_one_child_frame_task_source(vm, expected, context).await;
}

/// Observe one exact child semantic family through the production Page
/// selected-task dispatcher, allowing the single realm-materialization
/// prerequisite that can precede it.
async fn expect_page_child_frame_task_source_after_realm_prerequisite(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && page
            .run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                loader,
            )
            .await
            .expect("exact child realm prerequisite should run")
    {
        // Realm materialization is the only prerequisite this helper may
        // consume before the requested family.
    }
    assert!(
        page.run_one_child_frame_task_executor_turn(expected, loader)
            .await
            .expect("exact child semantic task should run"),
        "{context}"
    );
}

fn drain_image_load_event_bodies_for_test(vm: &mut ScriptVm) -> usize {
    let mut count = 0;
    while vm
        .apply_next_image_load_event_body_for_test()
        .expect("DOM-manipulation task should run")
    {
        count += 1;
    }
    count
}

#[test]
fn document_wrapper_does_not_expose_element_only_accessors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const props = ["dataset", "classList", "id", "className", "name"];
              return JSON.stringify({
                own: Object.fromEntries(props.map((key) => [key, Object.prototype.hasOwnProperty.call(document, key)])),
                enumerableKeys: Object.keys(document).filter((key) => props.includes(key)),
                querySelectorType: typeof document.querySelector,
                getElementsByTagNameType: typeof document.getElementsByTagName
              });
            })()
            "#,
        )
        .expect("document element-only accessor probe should evaluate");

    assert_eq!(
        result,
        r#"{"own":{"dataset":false,"classList":false,"id":false,"className":false,"name":false},"enumerableKeys":[],"querySelectorType":"function","getElementsByTagNameType":"function"}"#
    );
}

#[test]
fn dom_api_known_pseudo_element_selectors_with_after_part_pseudo_classes_return_empty() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = (target, selector) => {
                try {
                  return String(target.querySelector(selector));
                } catch (error) {
                  return `throw:${error && error.name}:${error && error.message}`;
                }
              };
              const parsed = new DOMParser().parseFromString("<main></main>", "text/html");
              return JSON.stringify({
                documentHover: probe(document, "::part(label):hover"),
                documentLang: probe(document, "::part(label):lang(en)"),
                detachedHover: probe(parsed, "::part(label):hover"),
                invalidStructural: probe(document, "::part(label):first-child")
              });
            })()
            "#,
        )
        .expect("DOM API pseudo-element selector probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["documentHover"], "null");
    assert_eq!(result["documentLang"], "null");
    assert_eq!(result["detachedHover"], "null");
    assert!(
        result["invalidStructural"]
            .as_str()
            .is_some_and(|value| value.starts_with("throw:SyntaxError:"))
    );
}

#[test]
fn element_scroll_into_view_surface_is_available() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              return JSON.stringify({
                instanceType: typeof div.scrollIntoView,
                protoType: typeof Element.prototype.scrollIntoView,
                inElement: "scrollIntoView" in div,
                ownProperty: Object.prototype.hasOwnProperty.call(div, "scrollIntoView"),
                callResult: String(div.scrollIntoView({ block: "center" })),
                protoEnumerable: Object.prototype.propertyIsEnumerable.call(Element.prototype, "scrollIntoView")
              });
            })()
            "#,
        )
        .expect("element scrollIntoView probe should evaluate");

    assert_eq!(
        result,
        r#"{"instanceType":"function","protoType":"function","inElement":true,"ownProperty":false,"callResult":"undefined","protoEnumerable":false}"#
    );
}

#[test]
fn element_scroll_into_view_if_needed_updates_observable_window_scroll() {
    let mut vm = new_storage_test_vm("https://example.com/");
    vm.force_fresh_layout_reads_for_test();

    vm.eval(
        r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              const visible = document.createElement("div");
              visible.id = "visible";
              document.body.appendChild(visible);

              const spacer = document.createElement("div");
              spacer.style.height = "3000px";
              document.body.appendChild(spacer);
              const target = document.createElement("div");
              target.id = "target";
              target.style.height = "20px";
              document.body.appendChild(target);
              return "installed";
            })()
            "#,
    )
    .expect("scrollIntoViewIfNeeded fixture should initialize");
    refresh_layout_for_test(&mut vm);

    let result = vm
        .eval(
            r#"
            (() => {
              const visible = document.getElementById("visible");
              const target = document.getElementById("target");
              visible.scrollIntoViewIfNeeded();
              const visibleScroll = window.scrollY;
              target.scrollIntoViewIfNeeded();
              const hiddenScroll = window.scrollY;
              target.scrollIntoViewIfNeeded();
              const repeatedScroll = window.scrollY;
              window.scrollTo(0, 0);
              target.scrollIntoView({ block: "center", inline: "nearest" });

              return JSON.stringify({
                visibleScroll,
                hiddenScroll,
                repeatedScroll,
                standardScroll: window.scrollY,
                scrollingElementScroll: document.scrollingElement.scrollTop
              });
            })()
            "#,
        )
        .expect("scrollIntoViewIfNeeded probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("scroll result should be JSON");
    assert_eq!(result["visibleScroll"], 0);
    assert!(
        result["hiddenScroll"]
            .as_f64()
            .is_some_and(|value| value > 0.0)
    );
    assert_eq!(result["repeatedScroll"], result["hiddenScroll"]);
    assert!(
        result["standardScroll"]
            .as_f64()
            .is_some_and(|value| value > 0.0)
    );
    assert_eq!(result["scrollingElementScroll"], result["standardScroll"]);
}

#[test]
fn wheel_default_action_scrolls_and_refreshes_geometry_unless_canceled() {
    let mut vm = new_storage_test_vm("https://wheel-default-scroll.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML =
            '<div style="height: 500px"></div>' +
            '<div id="marker" style="height: 20px"></div>' +
            '<div style="height: 2500px"></div>';
          window.__wheelDeltas = [];
          window.addEventListener("wheel", event => {
            window.__wheelDeltas.push(event.deltaY);
          }, { capture: true });
        })()
        "#,
    )
    .expect("wheel fixture should initialize");
    refresh_layout_for_test(&mut vm);

    let before = vm
        .eval("document.getElementById('marker').getBoundingClientRect().top")
        .expect("initial marker geometry should evaluate")
        .parse::<f64>()
        .expect("initial marker top should be numeric");
    let outcome = vm
        .dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 120.0)
        .expect("wheel input should dispatch");
    assert!(outcome.handled);

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              scrollY,
              scrollingElementScrollTop: document.scrollingElement.scrollTop,
              markerTop: document.getElementById("marker").getBoundingClientRect().top,
              wheelDeltas: window.__wheelDeltas
            })
            "#,
        )
        .expect("post-wheel geometry should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&result).expect("post-wheel result should be JSON");
    assert_eq!(result["scrollY"], 120.0);
    assert_eq!(result["scrollingElementScrollTop"], 120.0);
    assert_eq!(result["markerTop"], before - 120.0);
    assert_eq!(result["wheelDeltas"], serde_json::json!([120]));

    vm.eval(
        r#"
        window.addEventListener("wheel", event => event.preventDefault(), {
          capture: true,
          passive: false
        })
        "#,
    )
    .expect("wheel cancellation listener should install");
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 0.0, 80.0)
        .expect("canceled wheel input should dispatch");
    assert_eq!(
        vm.eval("String(scrollY)")
            .expect("canceled wheel scroll position should evaluate"),
        "120"
    );
}

#[test]
fn window_scroll_refreshes_intersection_observer_geometry() {
    let mut vm = new_storage_test_vm("https://scroll-intersection-observer.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML =
            '<div style="height: 800px"></div>' +
            '<div id="lazy-target" style="height: 20px"></div>' +
            '<div style="height: 1000px"></div>';
        })()
        "#,
    )
    .expect("intersection scroll fixture should initialize");
    refresh_layout_for_test(&mut vm);

    vm.eval(
        r#"
        (() => {
          window.__intersectionStates = [];
          window.__intersectionObserver = new IntersectionObserver(entries => {
            window.__intersectionStates.push(entries[0].isIntersecting);
          });
          window.__intersectionObserver.observe(
            document.getElementById("lazy-target")
          );
        })()
        "#,
    )
    .expect("intersection observer should register");
    assert_eq!(
        vm.eval("JSON.stringify(window.__intersectionStates)")
            .expect("initial intersection state should flush"),
        "[false]"
    );

    vm.eval("window.scrollTo(0, 400)")
        .expect("window scroll should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(window.__intersectionStates)")
            .expect("scrolled intersection state should flush"),
        "[false,true]"
    );
}

#[test]
fn wheel_default_action_scrolls_the_innermost_container_then_chains_to_the_root() {
    let mut vm = new_storage_test_vm("https://wheel-scroll-chain.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML =
            '<div id="scroller" style="width: 200px; height: 100px; overflow: auto">' +
              '<div style="width: 500px; height: 500px"></div>' +
            '</div>' +
            '<div style="width: 2000px; height: 2000px"></div>';
        })()
        "#,
    )
    .expect("nested wheel fixture should initialize");
    refresh_layout_for_test(&mut vm);

    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 35.0, 60.0)
        .expect("nested wheel input should dispatch");
    assert_eq!(
        vm.eval(
            "JSON.stringify([document.getElementById('scroller').scrollLeft, document.getElementById('scroller').scrollTop, window.scrollX, window.scrollY])"
        )
            .expect("nested wheel positions should evaluate"),
        "[35,60,0,0]"
    );

    vm.eval(
        "document.getElementById('scroller').scrollTo(document.getElementById('scroller').scrollWidth, document.getElementById('scroller').scrollHeight)"
    )
    .expect("nested scroller should move to its boundary");
    vm.dispatch_mouse_event_at_point(10.0, 10.0, "wheel", -1, Some(0), 25.0, 40.0)
        .expect("chained wheel input should dispatch");
    assert_eq!(
        vm.eval("JSON.stringify([window.scrollX, window.scrollY])")
            .expect("root chained scroll position should evaluate"),
        "[25,40]"
    );
}

#[test]
fn dom_core_prototype_accessors_brand_check_live_and_detached_receivers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const outcome = callback => {
                try {
                  const value = callback();
                  return `ok:${value === null ? "null" : String(value)}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const getter = (prototype, name, receiver) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.get.call(receiver));
              };
              const setter = (prototype, name, receiver, value) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.set.call(receiver, value));
              };

              const parsed = new DOMParser().parseFromString(
                '<!doctype html PUBLIC "pub" "sys"><html><body><div id="det"><span>b</span></div></body></html>',
                "text/html"
              );
              const detached = parsed.getElementById("det");
              const live = document.createElement("section");
              const piDoc = document.implementation.createDocument("", "root");
              const pi = piDoc.createProcessingInstruction("xml-stylesheet", 'href="x"');
              piDoc.appendChild(pi);

              return JSON.stringify({
                liveTagName: getter(Element.prototype, "tagName", live),
                detachedTagName: getter(Element.prototype, "tagName", detached),
                detachedLocalName: getter(Element.prototype, "localName", detached),
                detachedNamespaceURI: getter(Element.prototype, "namespaceURI", detached),
                detachedPrefix: getter(Element.prototype, "prefix", detached),
                detachedInnerHTML: getter(Element.prototype, "innerHTML", detached),
                detachedOuterHTML: getter(Element.prototype, "outerHTML", detached),
                setDetachedInnerHTML: setter(Element.prototype, "innerHTML", detached, "<em>c</em>"),
                detachedInnerHTMLAfterSet: detached.innerHTML,
                fakeTagName: getter(Element.prototype, "tagName", {}),
                documentTagName: getter(Element.prototype, "tagName", document),
                fakeLocalName: getter(Element.prototype, "localName", {}),
                fakeNamespaceURI: getter(Element.prototype, "namespaceURI", {}),
                fakePrefix: getter(Element.prototype, "prefix", {}),
                fakeInnerHTML: getter(Element.prototype, "innerHTML", {}),
                documentInnerHTML: getter(Element.prototype, "innerHTML", document),
                fakeOuterHTML: getter(Element.prototype, "outerHTML", {}),
                documentOuterHTML: getter(Element.prototype, "outerHTML", document),
                fakeInnerHTMLSet: setter(Element.prototype, "innerHTML", {}, "x"),
                documentOuterHTMLSet: setter(Element.prototype, "outerHTML", document, "x"),
                detachedPublicId: getter(DocumentType.prototype, "publicId", parsed.doctype),
                detachedSystemId: getter(DocumentType.prototype, "systemId", parsed.doctype),
                fakePublicId: getter(DocumentType.prototype, "publicId", {}),
                documentPublicId: getter(DocumentType.prototype, "publicId", document),
                piTarget: getter(ProcessingInstruction.prototype, "target", pi),
                fakePiTarget: getter(ProcessingInstruction.prototype, "target", {}),
                textPiTarget: getter(ProcessingInstruction.prototype, "target", document.createTextNode("x"))
              });
            })()
            "#,
        )
        .expect("DOM core accessor brand-check probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["liveTagName"], "ok:SECTION");
    assert_eq!(result["detachedTagName"], "ok:DIV");
    assert_eq!(result["detachedLocalName"], "ok:div");
    assert_eq!(
        result["detachedNamespaceURI"],
        "ok:http://www.w3.org/1999/xhtml"
    );
    assert_eq!(result["detachedPrefix"], "ok:null");
    assert_eq!(result["detachedInnerHTML"], "ok:<span>b</span>");
    assert_eq!(
        result["detachedOuterHTML"],
        "ok:<div id=\"det\"><span>b</span></div>"
    );
    assert_eq!(result["setDetachedInnerHTML"], "ok:undefined");
    assert_eq!(result["detachedInnerHTMLAfterSet"], "<em>c</em>");
    assert_eq!(result["detachedPublicId"], "ok:pub");
    assert_eq!(result["detachedSystemId"], "ok:sys");
    assert_eq!(result["piTarget"], "ok:xml-stylesheet");
    for key in [
        "fakeTagName",
        "documentTagName",
        "fakeLocalName",
        "fakeNamespaceURI",
        "fakePrefix",
        "fakeInnerHTML",
        "documentInnerHTML",
        "fakeOuterHTML",
        "documentOuterHTML",
        "fakeInnerHTMLSet",
        "documentOuterHTMLSet",
        "fakePublicId",
        "documentPublicId",
        "fakePiTarget",
        "textPiTarget",
    ] {
        assert_eq!(result[key], "throw:TypeError", "{key}");
    }
}

#[test]
fn node_and_mixin_prototype_members_brand_check_live_and_detached_receivers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const outcome = callback => {
                try {
                  const value = callback();
                  return `ok:${value === null ? "null" : String(value)}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const getter = (prototype, name, receiver) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.get.call(receiver));
              };
              const setter = (prototype, name, receiver, value) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.set.call(receiver, value));
              };
              const method = (prototype, name, receiver, ...args) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.value.call(receiver, ...args));
              };

              const parsed = new DOMParser().parseFromString(
                '<!doctype html><html><body><div id="parent"><em id="prev"></em><span id="child">t</span></div></body></html>',
                "text/html"
              );
              const detachedParent = parsed.getElementById("parent");
              const detachedChild = parsed.getElementById("child");
              const liveParent = document.createElement("div");
              liveParent.append(document.createElement("b"), document.createTextNode("x"));
              const liveText = document.createTextNode("x");
              const liveChild = document.createElement("span");
              liveParent.append(liveChild);
              const fragment = document.createDocumentFragment();
              fragment.append(document.createElement("i"));
              const docWithDoctype = document.implementation.createDocument(null, "root", null);
              const doctype = document.implementation.createDocumentType("root", "", "");
              docWithDoctype.insertBefore(doctype, docWithDoctype.documentElement);

              const nodeGetterNames = [
                "nodeType",
                "nodeName",
                "nodeValue",
                "isConnected",
                "ownerDocument",
                "baseURI",
                "parentNode",
                "parentElement",
                "childNodes",
                "firstChild",
                "lastChild",
                "previousSibling",
                "nextSibling",
                "textContent"
              ];
              const nodeMethodArgs = {
                appendChild: [liveText],
                insertBefore: [liveText, null],
                removeChild: [liveText],
                replaceChild: [liveText, liveText],
                cloneNode: [],
                contains: [liveText],
                hasChildNodes: [],
                isSameNode: [liveText],
                isEqualNode: [liveText],
                compareDocumentPosition: [liveText],
                getRootNode: [],
                lookupPrefix: ["urn:x"],
                lookupNamespaceURI: [null],
                isDefaultNamespace: [null],
                normalize: []
              };
              const ownNames = [
                "children",
                "firstElementChild",
                "lastElementChild",
                "childElementCount",
                "append",
                "prepend",
                "replaceChildren",
                "querySelector",
                "querySelectorAll",
                "before",
                "after",
                "replaceWith",
                "remove",
                "previousElementSibling",
                "nextElementSibling"
              ];
              return JSON.stringify({
                nodeTypeLive: getter(Node.prototype, "nodeType", liveParent),
                nodeTypeDetached: getter(Node.prototype, "nodeType", detachedChild),
                nodeTextDetached: getter(Node.prototype, "textContent", detachedChild),
                setDetachedText: setter(Node.prototype, "textContent", detachedChild, "det"),
                detachedTextAfterSet: detachedChild.textContent,
                containsDetached: method(Node.prototype, "contains", detachedParent, detachedChild),
                hasDetachedChildren: method(Node.prototype, "hasChildNodes", detachedParent),
                cloneDetachedType: outcome(() => Object.getOwnPropertyDescriptor(Node.prototype, "cloneNode").value.call(detachedChild, true).nodeType),
                childrenLiveLength: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "children").get.call(liveParent).length),
                childrenDetachedLength: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "children").get.call(detachedParent).length),
                fragmentChildElementCount: getter(DocumentFragment.prototype, "childElementCount", fragment),
                queryDetached: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "querySelector").value.call(detachedParent, "#child").id),
                appendLive: method(Element.prototype, "append", liveParent, "tail"),
                beforeDetached: method(Element.prototype, "before", detachedChild, "lead"),
                prevElementDetached: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "previousElementSibling").get.call(detachedChild).id),
                fakeNodeGetters: Object.fromEntries(nodeGetterNames.map(name => [name, getter(Node.prototype, name, {})])),
                fakeNodeValueSet: setter(Node.prototype, "nodeValue", {}, "x"),
                fakeTextContentSet: setter(Node.prototype, "textContent", {}, "x"),
                fakeNodeMethods: Object.fromEntries(Object.entries(nodeMethodArgs).map(([name, args]) => [name, method(Node.prototype, name, {}, ...args)])),
                parentGetterOnText: getter(Element.prototype, "children", liveText),
                parentCountOnText: getter(Element.prototype, "childElementCount", liveText),
                parentMethodOnText: method(Element.prototype, "append", liveText, "x"),
                queryOnText: method(Element.prototype, "querySelector", liveText, "*"),
                childMethodOnDocument: method(Element.prototype, "remove", document),
                nonDocumentTypePrevOnDoctype: getter(Element.prototype, "previousElementSibling", doctype),
                nonDocumentTypeNextOnDoctype: getter(Element.prototype, "nextElementSibling", doctype),
                fakePreviousElementSibling: getter(Element.prototype, "previousElementSibling", {}),
                ownSurfaceClean: [liveParent, detachedParent, liveText, detachedChild].every(target =>
                  ownNames.every(name => !Object.prototype.hasOwnProperty.call(target, name))
                ),
                prototypeOwners: [
                  Object.prototype.hasOwnProperty.call(Element.prototype, "children"),
                  Object.prototype.hasOwnProperty.call(Document.prototype, "children"),
                  Object.prototype.hasOwnProperty.call(DocumentFragment.prototype, "children"),
                  Object.prototype.hasOwnProperty.call(Element.prototype, "append"),
                  Object.prototype.hasOwnProperty.call(Element.prototype, "before"),
                  Object.prototype.hasOwnProperty.call(CharacterData.prototype, "before"),
                  Object.prototype.hasOwnProperty.call(Element.prototype, "previousElementSibling"),
                  Object.prototype.hasOwnProperty.call(CharacterData.prototype, "previousElementSibling")
                ].every(Boolean)
              });
            })()
            "##,
        )
        .expect("Node and mixin prototype brand-check probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["nodeTypeLive"], "ok:1");
    assert_eq!(result["nodeTypeDetached"], "ok:1");
    assert_eq!(result["nodeTextDetached"], "ok:t");
    assert_eq!(result["setDetachedText"], "ok:undefined");
    assert_eq!(result["detachedTextAfterSet"], "det");
    assert_eq!(result["containsDetached"], "ok:true");
    assert_eq!(result["hasDetachedChildren"], "ok:true");
    assert_eq!(result["cloneDetachedType"], "ok:1");
    assert_eq!(result["childrenLiveLength"], "ok:2");
    assert_eq!(result["childrenDetachedLength"], "ok:2");
    assert_eq!(result["fragmentChildElementCount"], "ok:1");
    assert_eq!(result["queryDetached"], "ok:child");
    assert_eq!(result["appendLive"], "ok:undefined");
    assert_eq!(result["beforeDetached"], "ok:undefined");
    assert_eq!(result["prevElementDetached"], "ok:prev");
    assert_eq!(result["fakeNodeValueSet"], "throw:TypeError");
    assert_eq!(result["fakeTextContentSet"], "throw:TypeError");
    for name in [
        "nodeType",
        "nodeName",
        "nodeValue",
        "isConnected",
        "ownerDocument",
        "baseURI",
        "parentNode",
        "parentElement",
        "childNodes",
        "firstChild",
        "lastChild",
        "previousSibling",
        "nextSibling",
        "textContent",
    ] {
        assert_eq!(result["fakeNodeGetters"][name], "throw:TypeError", "{name}");
    }
    for name in [
        "appendChild",
        "insertBefore",
        "removeChild",
        "replaceChild",
        "cloneNode",
        "contains",
        "hasChildNodes",
        "isSameNode",
        "isEqualNode",
        "compareDocumentPosition",
        "getRootNode",
        "lookupPrefix",
        "lookupNamespaceURI",
        "isDefaultNamespace",
        "normalize",
    ] {
        assert_eq!(result["fakeNodeMethods"][name], "throw:TypeError", "{name}");
    }
    for key in [
        "parentGetterOnText",
        "parentCountOnText",
        "parentMethodOnText",
        "queryOnText",
        "childMethodOnDocument",
        "nonDocumentTypePrevOnDoctype",
        "nonDocumentTypeNextOnDoctype",
        "fakePreviousElementSibling",
    ] {
        assert_eq!(result[key], "throw:TypeError", "{key}");
    }
    assert_eq!(result["ownSurfaceClean"], true);
    assert_eq!(result["prototypeOwners"], true);
}

#[test]
fn element_prototype_members_brand_check_live_and_detached_receivers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const outcome = callback => {
                try {
                  const value = callback();
                  if (value === null) return "ok:null";
                  if (value === undefined) return "ok:undefined";
                  if (typeof value === "object") return `ok:${Object.prototype.toString.call(value)}`;
                  return `ok:${String(value)}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const getter = (prototype, name, receiver) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.get.call(receiver));
              };
              const setter = (prototype, name, receiver, value) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.set.call(receiver, value));
              };
              const method = (prototype, name, receiver, ...args) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.value.call(receiver, ...args));
              };

              const parsed = new DOMParser().parseFromString(
                '<!doctype html><html><body><div id="detached" class="box"><span class="child" name="n"></span></div></body></html>',
                "text/html"
              );
              const detached = parsed.getElementById("detached");
              const live = document.createElement("div");
              live.id = "live";
              live.className = "box";
              live.innerHTML = '<span class="child" name="n"></span>';
              const fragment = document.createDocumentFragment();
              fragment.appendChild(document.createElement("span"));
              const shadowHost = document.createElement("section");
              const shadowRoot = shadowHost.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = "<span></span>";
              const text = document.createTextNode("x");
              const attr = document.createAttribute("data-new");
              attr.value = "new";
              const fake = {};

              const getterNames = [
                "id",
                "className",
                "tagName",
                "localName",
                "namespaceURI",
                "prefix",
                "innerHTML",
                "outerHTML",
                "classList",
                "part",
                "attributes",
                "customElementRegistry"
              ];
              const setterArgs = {
                id: "x",
                className: "x",
                innerHTML: "<b>x</b>",
                outerHTML: "<section></section>",
                classList: "x",
                part: "x"
              };
              const methodArgs = {
                getBoundingClientRect: [],
                getClientRects: [],
                hasAttribute: ["id"],
                hasAttributeNS: [null, "id"],
                hasAttributes: [],
                getAttributeNames: [],
                getAttribute: ["id"],
                getAttributeNS: [null, "id"],
                setAttribute: ["data-x", "1"],
                setAttributeNS: [null, "data-y", "2"],
                removeAttribute: ["data-x"],
                removeAttributeNS: [null, "data-y"],
                toggleAttribute: ["hidden"],
                getAttributeNode: ["id"],
                getAttributeNodeNS: [null, "id"],
                setAttributeNode: [attr],
                removeAttributeNode: [attr],
                matches: ["div.box"],
                closest: ["div"],
                getElementsByTagName: ["span"],
                getElementsByTagNameNS: ["*", "*"],
                getElementsByClassName: ["child"],
                getElementsByName: ["n"],
                getHTML: [],
                setHTMLUnsafe: ["<i></i>"]
              };

              return JSON.stringify({
                idLive: getter(Element.prototype, "id", live),
                idDetached: getter(Element.prototype, "id", detached),
                tagDetached: getter(Element.prototype, "tagName", detached),
                innerDetached: getter(Element.prototype, "innerHTML", detached),
                getAttributeLive: method(Element.prototype, "getAttribute", live, "id"),
                getAttributeDetached: method(Element.prototype, "getAttribute", detached, "id"),
                rectLive: method(Element.prototype, "getBoundingClientRect", live),
                matchesDetached: method(Element.prototype, "matches", detached, "div.box"),
                closestDetached: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "closest").value.call(detached.querySelector("span"), "div").id),
                tagCollectionLive: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "getElementsByTagName").value.call(live, "span").length),
                tagCollectionDocument: outcome(() => Object.getOwnPropertyDescriptor(Document.prototype, "getElementsByTagName").value.call(document, "body").length),
                tagCollectionFragment: outcome(() => Object.getOwnPropertyDescriptor(DocumentFragment.prototype, "getElementsByTagName").value.call(fragment, "span").length),
                tagCollectionShadowRoot: outcome(() => Object.getOwnPropertyDescriptor(ShadowRoot.prototype, "getElementsByTagName").value.call(shadowRoot, "span").length),
                setHTMLUnsafeLive: method(Element.prototype, "setHTMLUnsafe", live, "<em></em>"),
                getHTMLLive: method(Element.prototype, "getHTML", live),
                documentCustomRegistry: getter(Document.prototype, "customElementRegistry", document),
                shadowRootCustomRegistry: getter(ShadowRoot.prototype, "customElementRegistry", shadowRoot),
                fakeGetters: Object.fromEntries(getterNames.map(name => [name, getter(Element.prototype, name, fake)])),
                textGetters: Object.fromEntries(getterNames.map(name => [name, getter(Element.prototype, name, text)])),
                fakeSetters: Object.fromEntries(Object.entries(setterArgs).map(([name, value]) => [name, setter(Element.prototype, name, fake, value)])),
                textSetters: Object.fromEntries(Object.entries(setterArgs).map(([name, value]) => [name, setter(Element.prototype, name, text, value)])),
                fakeMethods: Object.fromEntries(Object.entries(methodArgs).map(([name, args]) => [name, method(Element.prototype, name, fake, ...args)])),
                textMethods: Object.fromEntries(Object.entries(methodArgs).map(([name, args]) => [name, method(Element.prototype, name, text, ...args)])),
                ownSurfaceClean: [live, detached].every(target =>
                  getterNames.every(name => !Object.prototype.hasOwnProperty.call(target, name)) &&
                  Object.keys(methodArgs).every(name => !Object.prototype.hasOwnProperty.call(target, name))
                )
              });
            })()
            "##,
        )
        .expect("Element prototype brand-check probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["idLive"], "ok:live");
    assert_eq!(result["idDetached"], "ok:detached");
    assert_eq!(result["tagDetached"], "ok:DIV");
    assert_eq!(
        result["innerDetached"],
        r#"ok:<span class="child" name="n"></span>"#
    );
    assert_eq!(result["getAttributeLive"], "ok:live");
    assert_eq!(result["getAttributeDetached"], "ok:detached");
    assert_eq!(result["rectLive"], "ok:[object DOMRect]");
    assert_eq!(result["matchesDetached"], "ok:true");
    assert_eq!(result["closestDetached"], "ok:detached");
    assert_eq!(result["tagCollectionLive"], "ok:1");
    assert_eq!(result["tagCollectionDocument"], "ok:0");
    assert_eq!(result["tagCollectionFragment"], "ok:1");
    assert_eq!(result["tagCollectionShadowRoot"], "ok:1");
    assert_eq!(result["setHTMLUnsafeLive"], "ok:undefined");
    assert_eq!(result["getHTMLLive"], "ok:<em></em>");
    assert_eq!(
        result["documentCustomRegistry"],
        "ok:[object CustomElementRegistry]"
    );
    assert_eq!(
        result["shadowRootCustomRegistry"],
        "ok:[object CustomElementRegistry]"
    );
    assert_eq!(result["ownSurfaceClean"], true);
    for name in [
        "id",
        "className",
        "tagName",
        "localName",
        "namespaceURI",
        "prefix",
        "innerHTML",
        "outerHTML",
        "classList",
        "part",
        "attributes",
        "customElementRegistry",
    ] {
        assert_eq!(result["fakeGetters"][name], "throw:TypeError", "{name}");
        assert_eq!(result["textGetters"][name], "throw:TypeError", "{name}");
    }
    for name in [
        "id",
        "className",
        "innerHTML",
        "outerHTML",
        "classList",
        "part",
    ] {
        assert_eq!(result["fakeSetters"][name], "throw:TypeError", "{name}");
        assert_eq!(result["textSetters"][name], "throw:TypeError", "{name}");
    }
    for name in [
        "getBoundingClientRect",
        "getClientRects",
        "hasAttribute",
        "hasAttributeNS",
        "hasAttributes",
        "getAttributeNames",
        "getAttribute",
        "getAttributeNS",
        "setAttribute",
        "setAttributeNS",
        "removeAttribute",
        "removeAttributeNS",
        "toggleAttribute",
        "getAttributeNode",
        "getAttributeNodeNS",
        "setAttributeNode",
        "removeAttributeNode",
        "matches",
        "closest",
        "getElementsByTagName",
        "getElementsByTagNameNS",
        "getElementsByClassName",
        "getElementsByName",
        "getHTML",
        "setHTMLUnsafe",
    ] {
        assert_eq!(result["fakeMethods"][name], "throw:TypeError", "{name}");
        assert_eq!(result["textMethods"][name], "throw:TypeError", "{name}");
    }
}

#[test]
fn element_scroll_methods_leave_a_detached_element_at_zero() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              div.scroll(50, 60);
              const first = [div.scrollLeft, div.scrollTop];
              div.scrollTo({ left: 10 });
              const second = [div.scrollLeft, div.scrollTop];
              div.scrollBy({ left: 5, top: 7 });
              const third = [div.scrollLeft, div.scrollTop];
              div.scroll({});
              const fourth = [div.scrollLeft, div.scrollTop];
              div.scroll();
              const fifth = [div.scrollLeft, div.scrollTop];
              return JSON.stringify({
                protoScroll: typeof Element.prototype.scroll,
                protoScrollTo: typeof Element.prototype.scrollTo,
                protoScrollBy: typeof Element.prototype.scrollBy,
                first,
                second,
                third,
                fourth,
                fifth,
                enumerable: Object.prototype.propertyIsEnumerable.call(Element.prototype, "scroll")
              });
            })()
            "#,
        )
        .expect("element scroll method probe should evaluate");

    assert_eq!(
        result,
        r#"{"protoScroll":"function","protoScrollTo":"function","protoScrollBy":"function","first":[0,0],"second":[0,0],"third":[0,0],"fourth":[0,0],"fifth":[0,0],"enumerable":false}"#
    );
}
#[test]
fn document_own_enumerable_surface_matches_browser_location_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const d = Object.getOwnPropertyDescriptor(document, "location");
              return JSON.stringify({
                keys: Object.keys(document),
                internalOwnNames: Object.getOwnPropertyNames(document)
                  .filter(name => name.startsWith("__moliWindowLocation")),
                ownLocation: Object.prototype.hasOwnProperty.call(document, "location"),
                locationEnumerable: !!d?.enumerable,
                locationConfigurable: !!d?.configurable,
                locationIdentity: document.location === window.location
              });
            })()
            "#,
        )
        .expect("document own enumerable surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"keys":["location"],"internalOwnNames":[],"ownLocation":true,"locationEnumerable":true,"locationConfigurable":false,"locationIdentity":true}"#
    );
}
#[test]
fn customized_built_in_constructors_can_extend_specialized_html_elements() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              let directError = null;
              try {
                new HTMLButtonElement();
              } catch (error) {
                directError = error && error.name;
              }

              let constructorCalls = 0;
              class BuiltInButton extends HTMLButtonElement {
                constructor() {
                  super();
                  constructorCalls += 1;
                  this.customized = true;
                }
              }

              customElements.define("wpt-specialized-button", BuiltInButton, { extends: "button" });
              const button = document.createElement("button", { is: "wpt-specialized-button" });
              return [
                directError,
                constructorCalls,
                button instanceof BuiltInButton,
                button instanceof HTMLButtonElement,
                button instanceof HTMLElement,
                button.tagName,
                button.customized === true
              ].join("|");
            })()
            "#,
        )
        .expect("customized built-in specialized constructor probe should evaluate");

    assert_eq!(result, "TypeError|1|true|true|true|BUTTON|true");
}
#[test]
fn custom_element_disabled_features_shadow_blocks_attach_shadow() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = (callback) => {
                try {
                  const value = callback();
                  return value instanceof ShadowRoot ? "shadow" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };

              class MyCustom extends HTMLElement {}
              customElements.define("my-custom", MyCustom);
              const autonomous = probe(() =>
                document.createElement("my-custom").attachShadow({ mode: "open" })
              );
              const undefinedAutonomous = probe(() =>
                document.createElement("undefined-custom").attachShadow({ mode: "open" })
              );

              class ShadowDisabledElement extends HTMLElement {
                static get disabledFeatures() { return ["shadow"]; }
              }
              const beforeDefinitionHost = document.createElement("shadow-disabled-element");
              const beforeDefinition = probe(() =>
                beforeDefinitionHost.attachShadow({ mode: "closed" })
              );
              const beforeDefinitionDuplicate = probe(() =>
                beforeDefinitionHost.attachShadow({ mode: "closed" })
              );
              customElements.define("shadow-disabled-element", ShadowDisabledElement);
              const afterDefinitionDuplicate = probe(() =>
                beforeDefinitionHost.attachShadow({ mode: "closed" })
              );
              const afterDefinitionNew = probe(() =>
                document.createElement("shadow-disabled-element").attachShadow({ mode: "closed" })
              );

              class ShadowDisabledHeadingElement extends HTMLHeadingElement {
                static get disabledFeatures() { return ["shadow"]; }
              }
              const builtInHost = document.createElement("h2", {
                is: "shadow-disabled-heading-element"
              });
              const builtInBeforeDefinition = probe(() =>
                builtInHost.attachShadow({ mode: "closed" })
              );
              const builtInBeforeDefinitionDuplicate = probe(() =>
                builtInHost.attachShadow({ mode: "closed" })
              );
              const builtInCreatedBeforeDefinition = document.createElement("h2", {
                is: "shadow-disabled-heading-element"
              });
              customElements.define(
                "shadow-disabled-heading-element",
                ShadowDisabledHeadingElement,
                { extends: "h2" }
              );
              const builtInAfterDefinitionDuplicate = probe(() =>
                builtInHost.attachShadow({ mode: "closed" })
              );
              const builtInAfterDefinitionNew = probe(() =>
                document.createElement("h2", {
                  is: "shadow-disabled-heading-element"
                }).attachShadow({ mode: "closed" })
              );
              const builtInCreatedBeforeDefinitionLaterAttach = probe(() =>
                builtInCreatedBeforeDefinition.attachShadow({ mode: "closed" })
              );

              class CapitalShadowDisabledElement extends HTMLElement {
                static get disabledFeatures() { return ["SHADOW"]; }
              }
              customElements.define(
                "capital-shadow-disabled-element",
                CapitalShadowDisabledElement
              );
              const capitalShadow = probe(() =>
                document.createElement("capital-shadow-disabled-element")
                  .attachShadow({ mode: "open" })
              );

              class MyInput extends HTMLInputElement {}
              customElements.define("my-input", MyInput, { extends: "input" });
              const inputBuiltin = probe(() =>
                document.createElement("input", { is: "my-input" })
                  .attachShadow({ mode: "open" })
              );

              return [
                autonomous,
                undefinedAutonomous,
                beforeDefinition,
                beforeDefinitionDuplicate,
                afterDefinitionDuplicate,
                afterDefinitionNew,
                builtInBeforeDefinition,
                builtInBeforeDefinitionDuplicate,
                builtInAfterDefinitionDuplicate,
                builtInAfterDefinitionNew,
                builtInCreatedBeforeDefinitionLaterAttach,
                capitalShadow,
                inputBuiltin
              ].join("|");
            })()
            "#,
        )
        .expect("custom element disabledFeatures shadow probe should evaluate");

    assert_eq!(
        result,
        "shadow|shadow|shadow|NotSupportedError|NotSupportedError|NotSupportedError|shadow|NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|shadow|NotSupportedError"
    );
}
#[test]
fn domexception_surface_matches_upstream_wpt_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const exception = new DOMException("message", "InvalidCharacterError");
              const messageDesc = Object.getOwnPropertyDescriptor(DOMException.prototype, "message");
              const nameDesc = Object.getOwnPropertyDescriptor(DOMException.prototype, "name");
              const codeDesc = Object.getOwnPropertyDescriptor(DOMException.prototype, "code");
              exception.message = "spoof";
              let getterBrandCheck = false;
              let prototypeToStringBrandCheck = false;
              try {
                messageDesc.get.call({});
              } catch (error) {
                getterBrandCheck = error instanceof TypeError;
              }
              try {
                DOMException.prototype.toString();
              } catch (error) {
                prototypeToStringBrandCheck = error instanceof TypeError;
              }
              return JSON.stringify({
                protoExtendsError: Object.getPrototypeOf(DOMException.prototype) === Error.prototype,
                ownMessage: Object.prototype.hasOwnProperty.call(exception, "message"),
                ownName: Object.prototype.hasOwnProperty.call(exception, "name"),
                ownCode: Object.prototype.hasOwnProperty.call(exception, "code"),
                ownMessageDescriptor: Object.getOwnPropertyDescriptor(exception, "message") ?? null,
                ownNameDescriptor: Object.getOwnPropertyDescriptor(exception, "name") ?? null,
                ownCodeDescriptor: Object.getOwnPropertyDescriptor(exception, "code") ?? null,
                messageGetterType: typeof messageDesc?.get,
                messageGetterName: messageDesc?.get?.name,
                messageGetterLength: messageDesc?.get?.length,
                messageSetterType: typeof messageDesc?.set,
                messageEnumerable: !!messageDesc?.enumerable,
                messageConfigurable: !!messageDesc?.configurable,
                nameGetterType: typeof nameDesc?.get,
                nameGetterName: nameDesc?.get?.name,
                nameGetterLength: nameDesc?.get?.length,
                nameSetterType: typeof nameDesc?.set,
                nameEnumerable: !!nameDesc?.enumerable,
                nameConfigurable: !!nameDesc?.configurable,
                codeGetterType: typeof codeDesc?.get,
                codeGetterName: codeDesc?.get?.name,
                codeGetterLength: codeDesc?.get?.length,
                codeSetterType: typeof codeDesc?.set,
                codeEnumerable: !!codeDesc?.enumerable,
                codeConfigurable: !!codeDesc?.configurable,
                messageAssignOwn: Object.prototype.hasOwnProperty.call(exception, "message"),
                messageAfterAssign: exception.message,
                ownToString: Object.prototype.hasOwnProperty.call(exception, "toString"),
                protoOwnToString: Object.prototype.hasOwnProperty.call(DOMException.prototype, "toString"),
                stringified: exception.toString(),
                getterBrandCheck,
                prototypeToStringBrandCheck
              });
            })()
            "#,
        )
        .expect("DOMException surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"protoExtendsError":true,"ownMessage":false,"ownName":false,"ownCode":false,"ownMessageDescriptor":null,"ownNameDescriptor":null,"ownCodeDescriptor":null,"messageGetterType":"function","messageGetterName":"get message","messageGetterLength":0,"messageSetterType":"undefined","messageEnumerable":true,"messageConfigurable":true,"nameGetterType":"function","nameGetterName":"get name","nameGetterLength":0,"nameSetterType":"undefined","nameEnumerable":true,"nameConfigurable":true,"codeGetterType":"function","codeGetterName":"get code","codeGetterLength":0,"codeSetterType":"undefined","codeEnumerable":true,"codeConfigurable":true,"messageAssignOwn":false,"messageAfterAssign":"message","ownToString":false,"protoOwnToString":false,"stringified":"InvalidCharacterError: message","getterBrandCheck":true,"prototypeToStringBrandCheck":true}"#
    );
}
#[test]
fn domexception_names_share_central_legacy_code_table_without_constructors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const cases = [
                ["NotFoundError", 8],
                ["InvalidStateError", 11],
                ["SecurityError", 18],
                ["NetworkError", 19],
                ["AbortError", 20],
                ["TimeoutError", 23],
                ["DataError", 0],
                ["OperationError", 0],
                ["ConstraintError", 0],
                ["TransactionInactiveError", 0],
                ["VersionError", 0],
                ["UnknownError", 0],
                ["NotAllowedError", 0],
                ["EncodingError", 0],
                ["ReadOnlyError", 0],
                ["NotReadableError", 0],
                ["WebSocketError", 0]
              ];
              const failures = [];
              for (const [name, code] of cases) {
                const error = new DOMException("message", name);
                if (error.name !== name || error.message !== "message" || error.code !== code) {
                  failures.push(`${name}:shape:${error.name}:${error.message}:${error.code}`);
                }
              }
              for (const name of [
                "AbortError",
                "DataError",
                "OperationError",
                "InvalidStateError",
                "EncodingError",
                "UnknownError"
              ]) {
                if (typeof globalThis[name] !== "undefined") {
                  failures.push(`${name}:constructor`);
                }
              }
              const socketNamed = new DOMException("message", "WebSocketError");
              if (socketNamed instanceof WebSocketError) {
                failures.push("WebSocketError:ordinary-constructor");
              }
              if (DOMException.NOT_FOUND_ERR !== 8 || DOMException.DATA_CLONE_ERR !== 25) {
                failures.push("constants");
              }
              return JSON.stringify(failures);
            })()
            "#,
        )
        .expect("DOMException code table probe should evaluate");

    assert_eq!(result, "[]");
}
#[test]
fn detached_native_xml_text_content_queues_child_list_record() {
    let mut vm = new_storage_test_vm("https://detached-xml-text-content-mutation.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const xml = new DOMParser().parseFromString('<root></root>', 'text/xml');
              const element = xml.createElement('sample');
              element.appendChild(xml.createCDATASection('foo'));
              const observer = new MutationObserver(() => {});
              observer.observe(element, { childList: true });
              element.textContent = 'foo';
              const record = observer.takeRecords()[0];
              return [
                record && record.removedNodes[0].nodeType,
                record && record.removedNodes[0].data,
                record && record.addedNodes[0].nodeType,
                record && record.addedNodes[0].data
              ].join('|');
            })()
            "#,
        )
        .expect("detached native XML textContent should queue childList mutation");

    assert_eq!(result, "4|foo|3|foo");
}
#[test]
fn set_range_text_preserve_mode_adjusts_selection_by_replacement_delta() {
    let mut vm = new_storage_test_vm("https://forms-selection-set-range-text.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';

              input.value = 'hello world';
              input.setSelectionRange(6, 11);
              input.setRangeText('xy', 0, 5, 'preserve');
              const afterPrefixReplacement = [
                input.value,
                input.selectionStart,
                input.selectionEnd
              ].join(':');

              input.value = 'abcdef';
              input.setSelectionRange(1, 3);
              input.setRangeText('Z', 2, 4, 'preserve');
              const afterOverlapReplacement = [
                input.value,
                input.selectionStart,
                input.selectionEnd
              ].join(':');

              return [afterPrefixReplacement, afterOverlapReplacement].join('|');
            })()
            "#,
        )
        .expect("setRangeText preserve mode should adjust selection positions");

    assert_eq!(result, "xy world:3:8|abZef:1:3");
}

#[test]
fn selection_collapses_to_removed_editing_host_position() {
    let mut vm = new_storage_test_vm("https://selection-editing-host-removal.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const container = document.createElement("div");
  (document.body || document.documentElement || document).appendChild(container);

  function probe(action) {
    container.textContent = "";
    const editingHost = document.createElement("div");
    editingHost.contentEditable = true;
    editingHost.innerHTML = "ABC<br>";
    const wrapper = action === "ancestor-remove" ? document.createElement("div") : null;
    if (wrapper) {
      wrapper.appendChild(editingHost);
      container.appendChild(wrapper);
    } else {
      container.appendChild(editingHost);
    }
    editingHost.focus();
    const selection = getSelection();
    selection.collapse(editingHost, 0);
    if (action === "remove") {
      editingHost.remove();
    } else if (action === "ancestor-remove") {
      wrapper.remove();
    } else if (action === "replace-with-self") {
      editingHost.replaceWith(editingHost);
    } else {
      container.replaceChild(editingHost, editingHost);
    }
    const range = selection.getRangeAt(0);
    return [
      selection.anchorNode === container,
      selection.anchorOffset,
      selection.focusNode === container,
      selection.focusOffset,
      range.startContainer === container,
      range.startOffset,
      range.endContainer === container,
      range.endOffset
    ].join(":");
  }

  return [
    probe("remove"),
    probe("ancestor-remove"),
    probe("replace-with-self"),
    probe("replace-child-self")
  ].join("|");
})()
"##,
        )
        .expect("editing-host removal selection probe should evaluate");

    assert_eq!(
        result,
        "true:0:true:0:true:0:true:0|true:0:true:0:true:0:true:0|true:0:true:0:true:0:true:0|true:0:true:0:true:0:true:0"
    );
}

#[test]
fn selection_collapse_focuses_contenteditable_editing_host() {
    let mut vm = new_storage_test_vm("https://selection-contenteditable-collapse.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const button = document.createElement("button");
  const host1 = document.createElement("div");
  const host2 = document.createElement("div");
  const host3 = document.createElement("div");
  const nonEditable = document.createElement("div");
  const host4 = document.createElement("div");
  const container = document.createElement("div");
  host1.contentEditable = "true";
  host2.contentEditable = "true";
  host3.contentEditable = "true";
  nonEditable.contentEditable = "false";
  host4.contentEditable = "true";
  nonEditable.appendChild(host4);
  host3.appendChild(nonEditable);
  container.append(button, host1, host2, host3);
  (document.body || document.documentElement || document).appendChild(container);

  function clearFocus() {
    button.focus();
  }

  function probe(first, second) {
    clearFocus();
    const selection = getSelection();
    selection.collapse(first, 0);
    const firstActive = document.activeElement;
    selection.collapse(second, 0);
    return [
      firstActive === first,
      document.activeElement === second,
      selection.anchorNode === second,
      selection.anchorOffset,
      selection.focusNode === second,
      selection.focusOffset
    ].join(":");
  }

  return [
    probe(host1, host2),
    probe(host4, host3),
    probe(host3, host4)
  ].join("|");
})()
"##,
        )
        .expect("contenteditable collapse focus probe should evaluate");

    assert_eq!(
        result,
        "true:true:true:0:true:0|true:true:true:0:true:0|true:true:true:0:true:0"
    );
}

#[test]
fn selection_modify_extend_word_updates_focus_and_associated_range() {
    let mut vm = new_storage_test_vm("https://selection-modify-extend-word.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const middleLine = document.createElement("p");
  middleLine.textContent = "middle line";
  (document.body || document.documentElement || document).appendChild(middleLine);
  const text = middleLine.firstChild;
  const selection = getSelection();

  selection.collapse(text, "mid".length);
  selection.modify("extend", "backward", "word");
  const backward = selection.getRangeAt(0);
  const backwardResult = [
    selection.anchorNode === text,
    selection.anchorOffset,
    selection.focusNode === text,
    selection.focusOffset,
    backward.startContainer === text,
    backward.startOffset,
    backward.endContainer === text,
    backward.endOffset
  ].join(":");

  selection.collapse(text, "middle li".length);
  selection.modify("extend", "forward", "word");
  const forward = selection.getRangeAt(0);
  const forwardResult = [
    selection.anchorNode === text,
    selection.anchorOffset,
    selection.focusNode === text,
    selection.focusOffset,
    forward.startContainer === text,
    forward.startOffset,
    forward.endContainer === text,
    forward.endOffset
  ].join(":");

  return `${backwardResult}|${forwardResult}`;
})()
"##,
        )
        .expect("Selection.modify extend word probe should evaluate");

    assert_eq!(
        result,
        "true:3:true:0:true:0:true:3|true:9:true:11:true:9:true:11"
    );
}

#[test]
fn selection_modify_move_line_and_paragraph_collapses_selection_to_adjacent_text() {
    let mut vm = new_storage_test_vm("https://selection-modify-line-paragraph.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  host.innerHTML = "text1<br>text2";
  (document.body || document.documentElement || document).appendChild(host);
  host.focus();

  const first = host.childNodes[0];
  const second = host.childNodes[2];
  const selection = getSelection();

  function label(node) {
    if (node === first) return "first";
    if (node === second) return "second";
    return "other";
  }

  function probe(start, anchorOffset, focusOffset, direction, granularity) {
    selection.setBaseAndExtent(start, anchorOffset, start, focusOffset);
    selection.modify("move", direction, granularity);
    return [
      direction,
      granularity,
      selection.isCollapsed,
      label(selection.focusNode),
      selection.focusOffset,
      label(selection.anchorNode),
      selection.anchorOffset
    ].join(":");
  }

  return [
    probe(second, 0, 5, "backward", "line"),
    probe(second, 5, 0, "backward", "paragraph"),
    probe(first, 0, 5, "forward", "line"),
    probe(first, 5, 0, "forward", "paragraph")
  ].join("|");
})()
"#,
        )
        .expect("Selection.modify line/paragraph move probe should evaluate");

    assert_eq!(
        result,
        "backward:line:true:first:5:first:5|backward:paragraph:true:first:0:first:0|forward:line:true:second:5:second:5|forward:paragraph:true:second:0:second:0"
    );
}

#[test]
fn selection_modify_skips_contenteditable_false_islands() {
    let mut vm = new_storage_test_vm("https://selection-modify-non-editable.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  host.innerHTML =
    " <span contenteditable=false>non-editable</span>editable<span contenteditable=false>non-editable</span> ";
  (document.body || document.documentElement || document).appendChild(host);
  host.focus();

  const preceding = host.firstChild;
  const middle = host.querySelector("span").nextSibling;
  const trailing = host.lastChild;
  const selection = getSelection();

  function label(node) {
    if (node === preceding) return "preceding";
    if (node === middle) return "middle";
    if (node === trailing) return "trailing";
    if (node === host) return "host";
    return "other";
  }

  function probe(node, offset, direction, selectAllFirst) {
    if (selectAllFirst) {
      selection.selectAllChildren(host);
    }
    selection.collapse(node, offset);
    selection.modify("move", direction, "character");
    const range = selection.getRangeAt(0);
    return [
      direction,
      selectAllFirst ? "after-selectAll" : "direct",
      label(range.startContainer),
      range.startOffset,
      label(range.endContainer),
      range.endOffset
    ].join(":");
  }

  const out = [];
  for (const selectAllFirst of [false, true]) {
    for (const direction of ["forward", "right"]) {
      out.push(probe(preceding, preceding.length, direction, selectAllFirst));
      out.push(probe(middle, middle.length, direction, selectAllFirst));
    }
    for (const direction of ["backward", "left"]) {
      out.push(probe(middle, 0, direction, selectAllFirst));
      out.push(probe(trailing, 0, direction, selectAllFirst));
    }
  }
  return out.join("|");
})()
"#,
        )
        .expect("Selection.modify non-editable island probe should evaluate");

    assert_eq!(
        result,
        "forward:direct:middle:0:middle:0|forward:direct:trailing:0:trailing:0|right:direct:middle:0:middle:0|right:direct:trailing:0:trailing:0|backward:direct:preceding:1:preceding:1|backward:direct:middle:8:middle:8|left:direct:preceding:1:preceding:1|left:direct:middle:8:middle:8|forward:after-selectAll:middle:0:middle:0|forward:after-selectAll:trailing:0:trailing:0|right:after-selectAll:middle:0:middle:0|right:after-selectAll:trailing:0:trailing:0|backward:after-selectAll:preceding:1:preceding:1|backward:after-selectAll:middle:8:middle:8|left:after-selectAll:preceding:1:preceding:1|left:after-selectAll:middle:8:middle:8"
    );
}

#[test]
fn document_query_command_support_and_enabled_follow_design_mode() {
    let mut vm = new_storage_test_vm("https://query-command-design-mode.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methods = [
    "queryCommandEnabled",
    "queryCommandIndeterm",
    "queryCommandState",
    "queryCommandSupported",
    "queryCommandValue"
  ].map(name => {
    const method = Document.prototype[name];
    return `${name}:${typeof method}:${method.length}`;
  });
  const supported = ["delete", "DeLeTe", "forwardDelete", "selectAll", "copy"]
    .map(command => document.queryCommandSupported(command));
  const initial = [document.designMode, document.queryCommandEnabled("delete")];
  document.designMode = "ON";
  const enabled = [document.designMode, document.queryCommandEnabled("DeLeTe")];
  document.designMode = "invalid";
  const invalid = [document.designMode, document.queryCommandEnabled("delete")];
  document.designMode = "off";
  const disabled = [document.designMode, document.queryCommandEnabled("delete")];
  const neutral = [
    document.queryCommandIndeterm("delete"),
    document.queryCommandState("delete"),
    document.queryCommandValue("delete")
  ];
  const xml = document.implementation.createDocument(null, "root");
  const xmlErrors = [
    () => xml.execCommand("delete"),
    () => xml.queryCommandEnabled("delete"),
    () => xml.queryCommandIndeterm("delete"),
    () => xml.queryCommandState("delete"),
    () => xml.queryCommandSupported("delete"),
    () => xml.queryCommandValue("delete")
  ].map(invoke => {
    try {
      invoke();
      return null;
    } catch (error) {
      return error.name;
    }
  });
  const xhtml = new DOMParser().parseFromString(
    '<html xmlns="http://www.w3.org/1999/xhtml"><body/></html>',
    "application/xhtml+xml"
  );
  const xhtmlResults = [
    xhtml.queryCommandSupported("delete"),
    xhtml.queryCommandEnabled("delete"),
    xhtml.queryCommandIndeterm("delete"),
    xhtml.queryCommandState("delete"),
    xhtml.queryCommandValue("delete")
  ];
  return JSON.stringify({
    methods,
    supported,
    unknown: document.queryCommandSupported("not-a-command"),
    initial,
    enabled,
    invalid,
    disabled,
    neutral,
    xmlErrors,
    xhtmlResults
  });
})()
"#,
        )
        .expect("Document editing-command query probe should evaluate");

    assert_eq!(
        result,
        r#"{"methods":["queryCommandEnabled:function:1","queryCommandIndeterm:function:1","queryCommandState:function:1","queryCommandSupported:function:1","queryCommandValue:function:1"],"supported":[true,true,true,true,true],"unknown":false,"initial":["off",false],"enabled":["on",true],"invalid":["on",true],"disabled":["off",false],"neutral":[false,false,""],"xmlErrors":["InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError"],"xhtmlResults":[true,false,false,false,""]}"#
    );
}

#[test]
fn exec_command_select_all_targets_editing_host_and_skips_inert_text() {
    let mut vm = new_storage_test_vm("https://exec-command-select-all-selection.test/");

    vm.eval(
        r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const root = document.createElement("section");
  root.id = "selection-root";
  body.appendChild(root);
  root.innerHTML = '<div inert>hidden inert</div><div>visible text</div>';
  return 'installed';
})()
"##,
    )
    .expect("document selectAll fixture should initialize");
    refresh_layout_for_test(&mut vm);
    let document_result = vm
        .eval(
            r##"
(() => {
  const body = document.body;
  const selection = getSelection();
  body.focus();
  const documentReturned = document.execCommand("selectAll");
  const documentText = selection.toString().trim();
  const documentRange = selection.getRangeAt(0);
  const documentRangeSpansBody =
    documentRange.startContainer === body &&
    documentRange.startOffset === 0 &&
    documentRange.endContainer === body &&
    documentRange.endOffset === body.childNodes.length;
  selection.removeAllRanges();
  return JSON.stringify({ documentReturned, documentText, documentRangeSpansBody });
})()
"##,
        )
        .expect("document selectAll selection probe should evaluate");
    assert_eq!(
        document_result,
        r#"{"documentReturned":true,"documentText":"visible text","documentRangeSpansBody":true}"#
    );

    vm.eval(
        r##"
(() => {
  const root = document.getElementById("selection-root");
  root.innerHTML =
    '<p>preceding text</p><div id="host" contenteditable>editable text</div><p>following text</p>';
  return 'installed';
})()
"##,
    )
    .expect("editing-host selectAll fixture should initialize");
    refresh_layout_for_test(&mut vm);
    let host_result = vm
        .eval(
            r##"
(() => {
  const root = document.getElementById("selection-root");
  const selection = getSelection();
  const host = root.querySelector("#host");
  host.focus();
  const hostReturned = document.execCommand("selectAll");
  const hostText = selection.toString();
  const hostRange = selection.getRangeAt(0);

  return JSON.stringify({
    hostReturned,
    hostHasEditableText: hostText.includes("editable text"),
    hostHasPrecedingText: hostText.includes("preceding text"),
    hostHasFollowingText: hostText.includes("following text"),
    hostRangeSpansHost:
      hostRange.startContainer === host &&
      hostRange.startOffset === 0 &&
      hostRange.endContainer === host &&
      hostRange.endOffset === host.childNodes.length
  });
})()
"##,
        )
        .expect("execCommand selectAll selection probe should evaluate");

    assert_eq!(
        host_result,
        r#"{"hostReturned":true,"hostHasEditableText":true,"hostHasPrecedingText":false,"hostHasFollowingText":false,"hostRangeSpansHost":true}"#
    );
}

#[test]
fn exec_command_select_all_respects_modal_dialog_inertness() {
    let mut vm = new_storage_test_vm("https://exec-command-select-all-modal-dialog.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  body.textContent = "";
  body.append(
    document.createTextNode("Here is a text node you can't select while the dialog is open."),
    document.createElement("dialog"),
    document.createTextNode("Trailing text.")
  );
  const dialog = body.querySelector("dialog");
  dialog.textContent = "I'm selectable.";
  const selection = getSelection();

  dialog.showModal();
  selection.selectAllChildren(body);
  const manualBodyText = selection.toString();
  selection.removeAllRanges();

  const commandReturned = document.execCommand("selectAll");
  const commandText = selection.toString();
  const commandRange = selection.getRangeAt(0);
  const commandRangeSpansDialog =
    commandRange.startContainer === dialog &&
    commandRange.startOffset === 0 &&
    commandRange.endContainer === dialog &&
    commandRange.endOffset === dialog.childNodes.length;

  dialog.close();
  selection.selectAllChildren(body);
  const afterCloseText = selection.toString();

  return JSON.stringify({
    manualBodyText,
    commandReturned,
    commandText,
    commandRangeSpansDialog,
    afterCloseHasOutside: afterCloseText.includes("text node you can't select"),
    afterCloseHasDialog: afterCloseText.includes("I'm selectable."),
    afterCloseHasTrailing: afterCloseText.includes("Trailing text.")
  });
})()
"##,
        )
        .expect("execCommand selectAll modal dialog inertness probe should evaluate");

    assert_eq!(
        result,
        r#"{"manualBodyText":"I'm selectable.","commandReturned":true,"commandText":"I'm selectable.","commandRangeSpansDialog":true,"afterCloseHasOutside":true,"afterCloseHasDialog":false,"afterCloseHasTrailing":true}"#
    );
}

#[test]
fn exec_command_delete_preserves_inert_selection_endpoint_contents() {
    let mut vm = new_storage_test_vm("https://exec-command-delete-inert-selection.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  (document.body || document.documentElement || document).appendChild(host);
  const selection = getSelection();

  function run(html, anchorSelector, anchorOffset, focusSelector, focusOffset) {
    host.innerHTML = html;
    host.focus();
    const initial = host.innerHTML;
    const anchor = host.querySelector(anchorSelector).firstChild;
    const focus = host.querySelector(focusSelector).firstChild;
    selection.setBaseAndExtent(anchor, anchorOffset, focus, focusOffset);
    document.execCommand("delete");
    return `${initial}=>${host.innerHTML}`;
  }

  const noDelete = run(
    '<span id="a" inert>abc</span><span id="b">def</span>',
    "#a",
    1,
    "#b",
    2
  );
  const keepFocusInert = run(
    '<span id="a">abc</span><span id="b" inert>def</span>',
    "#a",
    1,
    "#b",
    2
  );
  const keepReverseFocusInert = run(
    '<span id="a" inert>def</span><span id="b">abc</span>',
    "#b",
    2,
    "#a",
    1
  );

  host.innerHTML = 'a<span id="inert" inert>XYZ</span>f';
  const before = host.firstChild;
  const after = host.lastChild;
  selection.setBaseAndExtent(before, 1, after, 0);
  document.execCommand("delete");
  const deleteContained = host.innerHTML;

  return [
    noDelete,
    host.querySelector("#inert") === null && deleteContained === "af",
    /<span id="b" inert="">def<\/span>/.test(keepFocusInert),
    /<span id="a" inert="">def<\/span>/.test(keepReverseFocusInert)
  ].join("|");
})()
"##,
        )
        .expect("execCommand delete inert selection probe should evaluate");

    assert_eq!(
        result,
        r#"<span id="a" inert="">abc</span><span id="b">def</span>=><span id="a" inert="">abc</span><span id="b">def</span>|true|true|true"#
    );
}

#[test]
fn exec_command_insert_text_edits_focused_text_controls_and_contenteditable() {
    let mut vm = new_storage_test_vm("https://exec-command-insert-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const elements = [
    document.createElement("input"),
    document.createElement("textarea"),
    document.createElement("div")
  ];
  elements[2].contentEditable = "true";
  body.append(...elements);

  return elements.map(element => {
    const events = [];
    element.addEventListener("textInput", () => events.push("textInput"));
    element.addEventListener("input", () => events.push("input"));
    element.focus();
    const returned = document.execCommand("insertText", false, "a");
    const value = "value" in element ? element.value : element.textContent;
    return [element.localName, returned, value, events.join(",")].join(":");
  }).join("|");
})()
"#,
        )
        .expect("execCommand insertText probe should evaluate");

    assert_eq!(
        result,
        "input:true:a:input|textarea:true:a:input|div:true:a:input"
    );
}

#[test]
fn exec_command_insert_text_reuses_adjacent_text_at_element_boundaries() {
    let mut vm = new_storage_test_vm("https://exec-command-insert-text-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  (document.body || document.documentElement || document).appendChild(host);
  host.focus();
  const selection = getSelection();

  host.innerHTML = "<img>foo<img>";
  selection.collapse(host, 1);
  const insertedBefore = document.execCommand("insertText", false, "x");
  const beforeText = host.childNodes[1];
  const before = [
    insertedBefore,
    host.childNodes.length,
    beforeText.data,
    selection.anchorNode === beforeText,
    selection.anchorOffset
  ].join(":");

  host.innerHTML = "<img>foo<img>";
  selection.collapse(host, 2);
  const insertedAfter = document.execCommand("insertText", false, "x");
  const afterText = host.childNodes[1];
  const after = [
    insertedAfter,
    host.childNodes.length,
    afterText.data,
    selection.anchorNode === afterText,
    selection.anchorOffset
  ].join(":");

  return `${before}|${after}`;
})()
"#,
        )
        .expect("execCommand boundary insertText probe should evaluate");

    assert_eq!(result, "true:3:xfoo:true:1|true:3:foox:true:4");
}

#[test]
fn selection_to_string_uses_rendered_native_range_projection() {
    let mut vm = new_storage_test_vm("https://selection-rendered-string.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const root = document.createElement("div");
  (document.body || document.documentElement || document).appendChild(root);
  const selection = getSelection();

  function selectedStringFor(range) {
    selection.removeAllRanges();
    selection.addRange(range);
    return selection.toString();
  }

  const p = document.createElement("div");
  p.append("\n");
  const hiddenStyle = document.createElement("style");
  hiddenStyle.style.display = "none";
  hiddenStyle.textContent = "hidden";
  const visibleStyle = document.createElement("style");
  visibleStyle.style.display = "block";
  visibleStyle.textContent = "style   text\nline";
  const hiddenScript = document.createElement("script");
  hiddenScript.textContent = "hiddenScript()";
  const visibleScript = document.createElement("script");
  visibleScript.style.display = "block";
  visibleScript.textContent = "function x() {\n  return 1;\n}";
  const pre = document.createElement("pre");
  pre.textContent = "PASS";
  p.append(hiddenStyle, visibleStyle, "\n", hiddenScript, "\n", visibleScript, "\n", pre);
  root.appendChild(p);

  const table = document.createElement("table");
  table.innerHTML = "<tr><td id=left>Foo</td><td id=right>Hello</td></tr>";
  root.appendChild(table);
  const rangeFromElementEnd = document.createRange();
  rangeFromElementEnd.setStart(table.querySelector("#left"), 1);
  rangeFromElementEnd.setEnd(table.querySelector("#right").firstChild, 4);

  const basic = document.createElement("div");
  basic.innerHTML = "a<span style='user-select: none;'>b</span>c";
  const nested = document.createElement("div");
  nested.innerHTML = "start <span style='user-select: none;'>unselectable <strong>nested</strong> text</span> end";
  const container = document.createElement("div");
  container.style.userSelect = "none";
  container.innerHTML = "<span style='user-select: text;'>selectable</span> unselectable <span style='user-select: text;'>text</span>";
  const contentHidden = document.createElement("div");
  contentHidden.setAttribute("style", "content-visibility: hidden");
  contentHidden.textContent = "hidden content";
  root.append(basic, nested, container, contentHidden);
  const scriptStyleRange = document.createRange();
  scriptStyleRange.selectNode(p);
  const scriptStyle = selectedStringFor(scriptStyleRange).replace(/\r\n/g, "\n");
  const fromElementEnd = selectedStringFor(rangeFromElementEnd).trim();
  function selectContents(node) {
    const range = document.createRange();
    range.selectNodeContents(node);
    return selectedStringFor(range);
  }

  return [
    scriptStyle,
    fromElementEnd,
    selectContents(basic),
    selectContents(nested),
    selectContents(container),
    selectContents(contentHidden)
  ].join("|");
})()
"##,
        )
        .expect("selection rendered string probe should evaluate");

    assert_eq!(
        result,
        "\nstyle text line\nfunction x() { return 1; }\n\nPASS|Hell|ac|start  end|selectabletext|"
    );
}

#[test]
fn month_and_week_inputs_do_not_support_variable_length_selection() {
    let mut vm = new_storage_test_vm("https://forms-selection-temporal.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              function summarize(type) {
                const input = document.createElement('input');
                input.type = type;
                input.value = type === 'month' ? '2026-05' : '2026-W20';
                const select = probe(() => input.select());
                const reads = [
                  input.selectionStart === null,
                  input.selectionEnd === null,
                  input.selectionDirection === null
                ].join(',');
                const writes = [
                  probe(() => { input.selectionStart = 0; }),
                  probe(() => { input.selectionEnd = 0; }),
                  probe(() => { input.selectionDirection = 'forward'; }),
                  probe(() => input.setSelectionRange(0, 0)),
                  probe(() => input.setRangeText('', 0, 0))
                ].join(',');
                return `${type}:${select}:${reads}:${writes}`;
              }
              return [summarize('month'), summarize('week')].join('|');
            })()
            "#,
        )
        .expect("temporal input selection APIs should match HTML selection applicability");

    assert_eq!(
        result,
        "month:undefined:true,true,true:throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError|week:undefined:true,true,true:throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError"
    );
}
#[test]
fn input_button_offset_width_reflects_label_value() {
    let mut vm = new_storage_test_vm("https://forms-input-button-width.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const empty = document.createElement('input');
              empty.type = 'button';
              const labelled = document.createElement('input');
              labelled.type = 'button';
              labelled.value = 'BUTTON';
              const html = document.documentElement || document.appendChild(document.createElement('html'));
              const body = document.body || html.appendChild(document.createElement('body'));
              body.append(empty, labelled);
              return [
                empty.value,
                labelled.value,
                empty.offsetWidth,
                labelled.offsetWidth,
                labelled.offsetWidth > empty.offsetWidth
              ].join('|');
            })()
            "#,
        )
        .expect("button input intrinsic width should reflect its label");

    let fields = result.split('|').collect::<Vec<_>>();
    assert_eq!(fields.len(), 5);
    assert_eq!(fields[0], "");
    assert_eq!(fields[1], "BUTTON");
    let empty_width = fields[2]
        .parse::<u32>()
        .expect("empty button width should be numeric");
    let labelled_width = fields[3]
        .parse::<u32>()
        .expect("labelled button width should be numeric");
    assert!(empty_width > 0);
    assert!(labelled_width > empty_width);
    assert_eq!(fields[4], "true");
}
#[test]
fn meter_progress_numeric_setters_parse_webidl_values() {
    let mut vm = new_storage_test_vm("https://forms-meter-progress-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const meter = document.createElement('meter');
              const progress = document.createElement('progress');
              let meterValueCalls = 0;
              let progressValueCalls = 0;
              let progressMaxCalls = 0;
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              meter.value = {
                valueOf() {
                  meterValueCalls += 1;
                  return '0.75';
                }
              };
              const meterValueObject = `${meter.value}:${meterValueCalls}`;
              const meterInfinity = probe(() => { meter.value = Infinity; });
              const meterSymbol = probe(() => { meter.min = Symbol('min'); });
              const meterThrowing = probe(() => {
                meter.max = {
                  valueOf() {
                    throw new RangeError('max');
                  }
                };
              });
              meter.low = null;
              const meterNull = meter.low;
              progress.value = {
                valueOf() {
                  progressValueCalls += 1;
                  return '0.4';
                }
              };
              const progressValueObject = `${progress.value}:${progressValueCalls}`;
              progress.max = {
                valueOf() {
                  progressMaxCalls += 1;
                  return '4.5';
                }
              };
              const progressMaxObject = `${progress.max}:${progressMaxCalls}`;
              const progressZero = probe(() => { progress.max = 0; return progress.max; });
              const progressBadString = probe(() => { progress.value = 'bad'; });
              const progressSymbol = probe(() => { progress.max = Symbol('max'); });
              const progressThrowing = probe(() => {
                progress.value = {
                  valueOf() {
                    throw new RangeError('value');
                  }
                };
              });
              return [
                meterValueObject,
                meterInfinity,
                meterSymbol,
                meterThrowing,
                meterNull,
                progressValueObject,
                progressMaxObject,
                progressZero,
                progressBadString,
                progressSymbol,
                progressThrowing
              ].join('|');
            })()
            "#,
        )
        .expect("meter/progress numeric setters should parse WebIDL values");

    assert_eq!(
        result,
        "0.75:1|throw:TypeError|throw:TypeError|throw:RangeError|0|0.4:1|4.5:1|4.5|throw:TypeError|throw:TypeError|throw:RangeError"
    );
}
#[test]
fn non_document_wrappers_do_not_leak_document_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              const text = document.createTextNode("x");
              return [
                typeof div.createElement,
                "createElement" in div,
                typeof div.all,
                "all" in div,
                typeof text.createElement,
                "all" in text
              ].join("|");
            })()
            "#,
        )
        .expect("non-document wrappers should not leak document surface");

    assert_eq!(result, "undefined|false|undefined|false|undefined|false");
}
#[test]
fn live_script_elements_preserve_htmlscriptelement_brand_across_access_paths() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head><script id=\"probe\"></script></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const byId = document.getElementById("probe");
              const byScripts = document.scripts[0];
              const byAll = document.all("probe");
              return [
                byId.toString(),
                Object.prototype.toString.call(byId),
                byScripts.toString(),
                Object.prototype.toString.call(byScripts),
                byAll.toString(),
                Object.prototype.toString.call(byAll),
                byId instanceof HTMLScriptElement,
                byScripts instanceof HTMLScriptElement,
                byAll instanceof HTMLScriptElement
              ].join("|");
            })()
            "#,
        )
        .expect("live script brand probe should evaluate");

    assert_eq!(
        result,
        "[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|true|true|true"
    );
}
#[test]
fn html_script_element_supports_static_method_matches_supported_type_tokens() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(HTMLScriptElement, "supports");
              return JSON.stringify({
              type: typeof HTMLScriptElement.supports,
              name: HTMLScriptElement.supports.name,
              length: HTMLScriptElement.supports.length,
              descriptor: [
                !!descriptor,
                descriptor && descriptor.enumerable,
                descriptor && descriptor.writable,
                descriptor && descriptor.configurable
              ],
              keysContainSupports: Object.keys(HTMLScriptElement).includes("supports"),
              classic: HTMLScriptElement.supports("classic"),
              module: HTMLScriptElement.supports("module"),
              importmap: HTMLScriptElement.supports("importmap"),
              jsMime: HTMLScriptElement.supports("text/javascript"),
              padded: HTMLScriptElement.supports(" module "),
              upper: HTMLScriptElement.supports("Module"),
              unsupported: HTMLScriptElement.supports("unsupported"),
              missingThrows: (() => {
                try {
                  HTMLScriptElement.supports();
                  return false;
                } catch (error) {
                  return error instanceof TypeError;
                }
              })()
            });
            })()
            "#,
        )
        .expect("HTMLScriptElement.supports probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"function","name":"supports","length":1,"descriptor":[true,true,true,true],"keysContainSupports":true,"classic":true,"module":true,"importmap":true,"jsMime":false,"padded":false,"upper":false,"unsupported":false,"missingThrows":true}"#
    );
}
#[test]
fn live_dom_collections_report_browser_to_string_tags() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div></div><span></span></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              htmlCollectionTag: Object.prototype.toString.call(document.getElementsByTagName("div")),
              htmlCollectionCtor: document.getElementsByTagName("div").constructor?.name ?? null,
              queryNodeListTag: Object.prototype.toString.call(document.querySelectorAll("div,span")),
              queryNodeListCtor: document.querySelectorAll("div,span").constructor?.name ?? null,
              childNodeListTag: Object.prototype.toString.call(document.body.childNodes),
              childNodeListCtor: document.body.childNodes.constructor?.name ?? null
            }))()
            "#,
        )
        .expect("live collection brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"htmlCollectionTag":"[object HTMLCollection]","htmlCollectionCtor":"HTMLCollection","queryNodeListTag":"[object NodeList]","queryNodeListCtor":"NodeList","childNodeListTag":"[object NodeList]","childNodeListCtor":"NodeList"}"#
    );
}
#[test]
fn live_element_matches_methods_live_on_element_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div id=\"probe\" class=\"hit\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const el = document.getElementById("probe");
              let invalid = null;
              try {
                el.matches(":not(");
              } catch (error) {
                invalid = error.name;
              }
              return [
                el.matches("div.hit"),
                el.webkitMatchesSelector("body > div"),
                Object.prototype.hasOwnProperty.call(el, "matches"),
                Object.prototype.hasOwnProperty.call(el, "webkitMatchesSelector"),
                typeof Element.prototype.matches,
                typeof Element.prototype.webkitMatchesSelector,
                Element.prototype.matches.length,
                Element.prototype.webkitMatchesSelector.length,
                invalid
              ].join("|");
            })()
            "#,
        )
        .expect("Element.matches prototype probe should evaluate");

    assert_eq!(
        result,
        "true|true|false|false|function|function|1|1|SyntaxError"
    );
}
#[test]
fn live_elements_expose_named_node_map_attributes_surface() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body data-x=\"1\" hidden dir=\"rtl\"></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const attrs = document.body.attributes;
              const parts = [];
              for (let i = 0; i < attrs.length; i++) {
                parts.push(`${attrs[i].name}=${attrs[i].value}`);
              }
              return JSON.stringify({
                hasAttributesSurface: "attributes" in document.body,
                tag: Object.prototype.toString.call(attrs),
                ctor: attrs.constructor?.name ?? null,
                length: attrs.length,
                parts: parts.join("|"),
                protoEnumerable: Object.getOwnPropertyDescriptor(Element.prototype, "attributes")?.enumerable ?? null,
                protoConfigurable: Object.getOwnPropertyDescriptor(Element.prototype, "attributes")?.configurable ?? null,
                protoGetterType: typeof Object.getOwnPropertyDescriptor(Element.prototype, "attributes")?.get
              });
            })()
            "#,
        )
        .expect("element attributes surface should evaluate");

    assert_eq!(
        result,
        r#"{"hasAttributesSurface":true,"tag":"[object NamedNodeMap]","ctor":"NamedNodeMap","length":3,"parts":"data-x=1|hidden=|dir=rtl","protoEnumerable":true,"protoConfigurable":true,"protoGetterType":"function"}"#
    );
}

#[test]
fn legacy_platform_objects_use_webidl_property_descriptors_and_key_order() {
    let mut vm = new_storage_test_vm("https://legacy-platform-object.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = (object, key) => {
    const value = Object.getOwnPropertyDescriptor(object, key);
    return value && [value.writable, value.enumerable, value.configurable].join(',');
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const element = document.createElement('div');
  element.className = 'foo';
  const classList = element.classList;
  const classListDefine = throwsTypeError(() => {
    Object.defineProperty(classList, '1', { value: 'bar' });
  });

  const dataList = document.createElement('datalist');
  const namedOption = document.createElement('option');
  namedOption.id = 'named';
  dataList.append(namedOption);
  const collection = dataList.options;
  collection.expando = 1;

  const select = document.createElement('select');
  const option = document.createElement('option');
  Object.defineProperty(select, '0', {
    value: option,
    writable: false,
    enumerable: false,
    configurable: false
  });

  const dataset = element.dataset;
  Object.defineProperty(dataset, 'entry', {
    value: 'value',
    writable: false,
    enumerable: false,
    configurable: false
  });
  const datasetAccessor = throwsTypeError(() => {
    Object.defineProperty(dataset, 'other', { get() { return 'wrong'; } });
  });

  const attributeElement = document.createElement('div');
  const attributes = attributeElement.attributes;
  attributes.first = 1;
  const symbol = Symbol('marker');
  attributes[symbol] = 2;
  attributes.second = 3;
  attributeElement.setAttribute('id', 'target');
  attributeElement.setAttribute('title', 'title');

  return JSON.stringify({
    classList: descriptor(classList, '0'),
    classListDefine,
    collectionNamed: descriptor(collection, 'named'),
    collectionExpando: descriptor(collection, 'expando'),
    select: descriptor(select, '0'),
    selectKeys: Object.keys(select),
    dataset: descriptor(dataset, 'entry'),
    datasetAccessor,
    attributeKeys: Reflect.ownKeys(attributes).map(key =>
      typeof key === 'symbol' ? key.toString() : key)
  });
})()
"#,
        )
        .expect("legacy platform object property probe should evaluate");

    assert_eq!(
        result,
        r#"{"classList":"false,true,true","classListDefine":true,"collectionNamed":"false,false,true","collectionExpando":"true,true,true","select":"true,true,true","selectKeys":["0"],"dataset":"true,true,true","datasetAccessor":true,"attributeKeys":["0","1","id","title","first","second","Symbol(marker)"]}"#
    );
}

#[test]
fn draggable_uses_html_element_defaults_for_auto_state() {
    let mut vm = new_storage_test_vm("https://draggable-defaults.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const value = (name, draggable, href = false) => {
    const element = document.createElement(name);
    if (draggable !== null) element.setAttribute('draggable', draggable);
    if (href) element.setAttribute('href', 'target');
    return element.draggable;
  };

  const namespacedDraggable = document.createElement('img');
  namespacedDraggable.setAttributeNS('urn:test', 'draggable', 'false');
  const namespacedHref = document.createElement('a');
  namespacedHref.setAttributeNS('urn:test', 'href', 'target');

  return JSON.stringify({
    div: [value('div', null), value('div', 'true'), value('div', 'auto')],
    anchor: [
      value('a', null),
      value('a', null, true),
      value('a', 'false', true),
      value('a', 'AUTO', true),
      value('a', 'invalid', true)
    ],
    image: [
      value('img', null),
      value('img', 'false'),
      value('img', 'FaLsE'),
      value('img', 'falſe'),
      value('img', 'invalid')
    ],
    namespaced: [namespacedDraggable.draggable, namespacedHref.draggable]
  });
})()
"#,
        )
        .expect("draggable default-state probe should evaluate");

    assert_eq!(
        result,
        r#"{"div":[false,true,false],"anchor":[false,true,false,true,true],"image":[true,false,false,true,true],"namespaced":[true,false]}"#
    );
}

#[test]
fn dataset_only_exposes_supported_property_names() {
    let mut vm = new_storage_test_vm("https://dataset-supported-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttribute('data--foo', 'upper');
  element.setAttribute('data---bar', 'dash-upper');
  const dataset = element.dataset;

  if (dataset.Foo !== 'upper' || dataset['-Bar'] !== 'dash-upper') {
    throw new Error('supported dataset names should reflect their attributes');
  }
  if (dataset['-foo'] !== undefined || '-foo' in dataset ||
      Object.getOwnPropertyDescriptor(dataset, '-foo') !== undefined) {
    throw new Error('invalid alias should not be exposed');
  }
  if (!Object.keys(dataset).includes('-Bar') ||
      Object.getOwnPropertyDescriptor(dataset, '-Bar')?.value !== 'dash-upper') {
    throw new Error('consecutive dashes should produce a supported property');
  }

  if (!delete dataset['-foo'] || element.getAttribute('data--foo') !== 'upper') {
    throw new Error('deleting an invalid alias should not remove the attribute');
  }
  if (!delete dataset['-Bar'] || element.hasAttribute('data---bar')) {
    throw new Error('deleting a supported property should remove the attribute');
  }
  return 'ok';
})()
"#,
        )
        .expect("dataset supported-property-name probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn content_editable_accessors_validate_keywords_and_receivers() {
    let mut vm = new_storage_test_vm("https://content-editable-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.contentEditable = 'TRUE';
  const truthy = [element.contentEditable, element.getAttribute('contenteditable')];

  element.contentEditable = { toString() { return 'PLAINTEXT-ONLY'; } };
  const plaintext = [element.contentEditable, element.getAttribute('contenteditable')];

  element.contentEditable = 'INHERIT';
  const inherited = [element.contentEditable, element.getAttribute('contenteditable')];

  element.setAttribute('contenteditable', 'false');
  let invalid;
  try {
    element.contentEditable = 'falſe';
    invalid = 'none';
  } catch (error) {
    invalid = error.name;
  }

  const setter = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'contentEditable'
  ).set;
  const getter = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'contentEditable'
  ).get;
  let incompatible;
  try {
    setter.call(document.createElementNS('urn:test', 'div'), 'true');
    incompatible = 'none';
  } catch (error) {
    incompatible = error.name;
  }

  let incompatibleGetter;
  try {
    getter.call(document.createElementNS('urn:test', 'div'));
    incompatibleGetter = 'none';
  } catch (error) {
    incompatibleGetter = error.name;
  }

  const namespaced = document.createElement('div');
  namespaced.setAttributeNS('urn:test', 'contenteditable', 'true');

  return JSON.stringify({
    truthy,
    plaintext,
    inherited,
    invalid: [invalid, element.getAttribute('contenteditable')],
    incompatible: [incompatibleGetter, incompatible],
    namespaced: [namespaced.contentEditable, namespaced.isContentEditable]
  });
})()
"#,
        )
        .expect("contentEditable setter probe should evaluate");

    assert_eq!(
        result,
        r#"{"truthy":["true","true"],"plaintext":["plaintext-only","plaintext-only"],"inherited":["inherit",null],"invalid":["SyntaxError","false"],"incompatible":["TypeError","TypeError"],"namespaced":["inherit",false]}"#
    );
}

#[test]
fn dataset_writes_and_deletes_only_null_namespace_attributes() {
    let mut vm = new_storage_test_vm("https://dataset-namespace.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttributeNS('urn:first', 'data-my-custom-attr', 'first');
  element.setAttributeNS('urn:second', 'data-my-custom-attr', 'second');

  element.dataset.myCustomAttr = 'third';
  const afterSet = Array.from(element.attributes, attribute => [
    attribute.namespaceURI,
    attribute.name,
    attribute.value
  ]);
  const nullNamespaceValue = element.getAttributeNS(null, 'data-my-custom-attr');

  delete element.dataset.myCustomAttr;
  const afterDelete = Array.from(element.attributes, attribute => [
    attribute.namespaceURI,
    attribute.name,
    attribute.value
  ]);

  return JSON.stringify({ afterSet, nullNamespaceValue, afterDelete });
})()
"#,
        )
        .expect("dataset namespace-isolation probe should evaluate");

    assert_eq!(
        result,
        r#"{"afterSet":[["urn:first","data-my-custom-attr","first"],["urn:second","data-my-custom-attr","second"],[null,"data-my-custom-attr","third"]],"nullNamespaceValue":"third","afterDelete":[["urn:first","data-my-custom-attr","first"],["urn:second","data-my-custom-attr","second"]]}"#
    );
}

#[test]
fn dataset_numeric_names_use_named_property_semantics() {
    let mut vm = new_storage_test_vm("https://dataset-numeric-name.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  const dataset = element.dataset;
  element.setAttribute('data-9', 'initial');
  const initialDescriptor = Object.getOwnPropertyDescriptor(dataset, 9);

  const prototype = Object.getPrototypeOf(dataset);
  let setterCalls = 0;
  Object.defineProperty(prototype, 10, {
    get() { return 'prototype'; },
    set() { setterCalls++; },
    configurable: true
  });
  const inheritedBeforeSet = dataset[10];
  dataset[10] = 'written';
  delete prototype[10];

  return JSON.stringify({
    initialDescriptor,
    keys: Object.keys(dataset),
    inheritedBeforeSet,
    setterCalls,
    written: dataset[10],
    attribute: element.getAttribute('data-10')
  });
})()
"#,
        )
        .expect("dataset numeric-name binding probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialDescriptor":{"value":"initial","writable":true,"enumerable":true,"configurable":true},"keys":["9","10"],"inheritedBeforeSet":"prototype","setterCalls":0,"written":"written","attribute":"written"}"#
    );
}

#[test]
fn access_key_label_matches_chromium_single_key_surface() {
    let mut vm = new_storage_test_vm("https://access-key-label.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'accessKeyLabel'
  );
  const label = value => {
    const button = document.createElement('button');
    if (value !== null) button.setAttribute('accesskey', value);
    return button.accessKeyLabel;
  };
  const incompatible = (() => {
    try {
      descriptor.get.call(document.createElementNS('urn:test', 'button'));
      return 'none';
    } catch (error) {
      return error.name;
    }
  })();

  return JSON.stringify({
    descriptor: [descriptor.set, descriptor.enumerable, descriptor.configurable],
    valid: label('b'),
    missing: label(null),
    empty: label(''),
    multiple: label('s 0'),
    nonBmp: label('\u{1F600}'),
    incompatible
  });
})()
"#,
        )
        .expect("accessKeyLabel surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":[null,true,true],"valid":"Alt+b","missing":"","empty":"","multiple":"","nonBmp":"","incompatible":"TypeError"}"#
    );
}

#[test]
fn autocapitalize_canonicalizes_and_inherits_from_form_owner() {
    let mut vm = new_parsed_test_vm(
        "https://autocapitalize.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const div = document.createElement('div');
  const canonical = [null, '', 'NoNe', 'OFF', 'characters', 'WORDS', 'on', 'invalid']
    .map(value => {
      if (value === null) div.removeAttribute('autocapitalize');
      else div.setAttribute('autocapitalize', value);
      return div.autocapitalize;
    });

  div.autocapitalize = { toString() { return 'ON'; } };
  const setter = [div.autocapitalize, div.getAttribute('autocapitalize')];

  const form = document.createElement('form');
  form.id = 'owner';
  form.setAttribute('autocapitalize', 'WORDS');
  document.body.appendChild(form);
  const inheritedNames = ['button', 'fieldset', 'input', 'output', 'select', 'textarea'];
  const inherited = inheritedNames.map(name => {
    const child = document.createElement(name);
    form.appendChild(child);
    const external = document.createElement(name);
    external.setAttribute('form', 'owner');
    document.body.appendChild(external);
    child.setAttribute('autocapitalize', '');
    external.setAttributeNS('urn:test', 'autocapitalize', 'off');
    const values = [child.autocapitalize, external.autocapitalize];
    child.setAttribute('autocapitalize', 'off');
    values.push(child.autocapitalize);
    return values.join(':');
  });

  const nonInherited = ['img', 'object'].map(name => {
    const element = document.createElement(name);
    form.appendChild(element);
    return element.autocapitalize;
  });

  const descriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'autocapitalize'
  );
  const incompatible = operation => {
    try {
      operation(document.createElementNS('urn:test', 'div'));
      return 'none';
    } catch (error) {
      return error.name;
    }
  };

  return JSON.stringify({
    descriptor: [descriptor.enumerable, descriptor.configurable],
    canonical,
    setter,
    inherited,
    nonInherited,
    incompatible: [
      incompatible(receiver => descriptor.get.call(receiver)),
      incompatible(receiver => descriptor.set.call(receiver, 'words'))
    ]
  });
})()
"#,
        )
        .expect("autocapitalize semantics should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":[true,true],"canonical":["","","none","none","characters","words","sentences","sentences"],"setter":["sentences","ON"],"inherited":["words:words:none","words:words:none","words:words:none","words:words:none","words:words:none","words:words:none"],"nonInherited":["",""],"incompatible":["TypeError","TypeError"]}"#
    );
}

#[test]
fn named_node_map_accessor_expandos_remain_writable_through_setters() {
    let mut vm = new_storage_test_vm("https://named-node-map-accessor-expando.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const attributes = document.createElement('div').attributes;
  const symbol = Symbol('accessor');
  const writes = [];
  let stringValue = 'string-initial';
  let symbolValue = 'symbol-initial';

  Object.defineProperty(attributes, 'accessor', {
    get() { return stringValue; },
    set(value) {
      stringValue = value;
      writes.push(`string:${value}`);
    },
    enumerable: true,
    configurable: true
  });
  Object.defineProperty(attributes, symbol, {
    get() { return symbolValue; },
    set(value) {
      symbolValue = value;
      writes.push(`symbol:${value}`);
    },
    enumerable: false,
    configurable: true
  });

  attributes.accessor = 'sloppy';
  attributes[symbol] = 'sloppy';
  let stringStrictError = null;
  let symbolStrictError = null;
  try {
    (() => {
      'use strict';
      attributes.accessor = 'strict';
    })();
  } catch (error) {
    stringStrictError = error.name;
  }
  try {
    (() => {
      'use strict';
      attributes[symbol] = 'strict';
    })();
  } catch (error) {
    symbolStrictError = error.name;
  }

  const stringDescriptor = Object.getOwnPropertyDescriptor(attributes, 'accessor');
  const symbolDescriptor = Object.getOwnPropertyDescriptor(attributes, symbol);
  return JSON.stringify({
    stringValue: attributes.accessor,
    symbolValue: attributes[symbol],
    writes,
    stringStrictError,
    symbolStrictError,
    stringDescriptor: [
      typeof stringDescriptor.get,
      typeof stringDescriptor.set,
      stringDescriptor.enumerable,
      stringDescriptor.configurable,
      'writable' in stringDescriptor
    ],
    symbolDescriptor: [
      typeof symbolDescriptor.get,
      typeof symbolDescriptor.set,
      symbolDescriptor.enumerable,
      symbolDescriptor.configurable,
      'writable' in symbolDescriptor
    ]
  });
})()
"#,
        )
        .expect("NamedNodeMap accessor expando probe should evaluate");

    assert_eq!(
        result,
        r#"{"stringValue":"strict","symbolValue":"strict","writes":["string:sloppy","symbol:sloppy","string:strict","symbol:strict"],"stringStrictError":null,"symbolStrictError":null,"stringDescriptor":["function","function",true,true,false],"symbolDescriptor":["function","function",false,true,false]}"#
    );
}

#[test]
fn live_named_node_map_cache_uses_private_slot_and_ignores_public_spoofing() {
    let mut vm = new_storage_test_vm("https://named-node-map-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const element = document.createElement('div');
  body.appendChild(element);
  element.setAttribute('data-real', 'one');
  element.setAttribute('title', 'initial');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliNamedNodeMap'))
    .sort()
    .join(',');
  const attrs = element.attributes;
  const afterCacheNames = internalNames(element);
  const mapOwnNames = internalNames(attrs);
  const fakeMap = {
    length: 99,
    getNamedItem() {
      return { name: 'data-real', value: 'fake' };
    }
  };
  Element.prototype.__moliNamedNodeMapCache = fakeMap;
  element.__moliNamedNodeMapCache = fakeMap;
  const spoofedOwnNames = internalNames(element);
  const afterSpoof = element.attributes;
  element.setAttribute('data-real', 'two');
  return JSON.stringify({
    afterCacheNames,
    mapOwnNames,
    spoofedOwnNames,
    sameWrapper: afterSpoof === attrs,
    length: afterSpoof.length,
    namedValue: afterSpoof.getNamedItem('data-real').value,
    indexedName: afterSpoof[0].name,
    indexedValue: afterSpoof[0].value
  });
})()
"#,
        )
        .expect("live NamedNodeMap cache should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"afterCacheNames":"","mapOwnNames":"","spoofedOwnNames":"__moliNamedNodeMapCache","sameWrapper":true,"length":2,"namedValue":"two","indexedName":"data-real","indexedValue":"two"}"#
    );
}

#[test]
fn detached_dom_parser_elements_expose_named_node_map_attributes_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = new DOMParser().parseFromString(
                '<!doctype html><html><body data-x="1" hidden dir="rtl"></body></html>',
                'text/html'
              );
              const attrs = doc.body.attributes;
              const parts = [];
              for (let i = 0; i < attrs.length; i++) {
                parts.push(`${attrs[i].name}=${attrs[i].value}`);
              }
              return JSON.stringify({
                hasAttributesSurface: "attributes" in doc.body,
                tag: Object.prototype.toString.call(attrs),
                ctor: attrs.constructor?.name ?? null,
                length: attrs.length,
                parts: parts.join("|")
              });
            })()
            "#,
        )
        .expect("detached element attributes surface should evaluate");

    assert_eq!(
        result,
        r#"{"hasAttributesSurface":true,"tag":"[object NamedNodeMap]","ctor":"NamedNodeMap","length":3,"parts":"data-x=1|hidden=|dir=rtl"}"#
    );
}
#[test]
fn live_element_attribute_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://element-attribute-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const el = document.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  el.setAttribute(null, undefined);
  el.setAttributeNS(null, "data-local", "local");
  el.setAttributeNS(undefined, "data-undefined-ns", "undefined-ns");
  el.setAttributeNS("urn:moli:test", "lm:flag", "on");
  const attrNode = el.getAttributeNode({ toString() { return "null"; } });
  const attrNodeNs = el.getAttributeNodeNS(
    { toString() { return "urn:moli:test"; } },
    { toString() { return "flag"; } }
  );
  const toggledNull = el.toggleAttribute("data-toggle", null);
  const toggledUndefined = el.toggleAttribute("data-toggle", undefined);
  const beforeRemove = [
    el.getAttribute("null"),
    el.getAttributeNS(null, "data-local"),
    el.getAttributeNS(undefined, "data-local"),
    el.getAttributeNS(null, "data-undefined-ns"),
    el.hasAttributeNS("urn:moli:test", "flag"),
    attrNode && attrNode.value,
    attrNodeNs && attrNodeNs.value,
    toggledNull,
    toggledUndefined,
    el.hasAttribute("data-toggle")
  ].join(",");
  el.removeAttributeNS("urn:moli:test", "flag");
  return [
    beforeRemove,
    el.hasAttributeNS("urn:moli:test", "flag"),
    probe(() => el.getAttribute()),
    probe(() => el.getAttributeNode()),
    probe(() => el.getAttributeNodeNS(undefined)),
    probe(() => el.getAttributeNS(undefined)),
    probe(() => el.getAttributeNode(Symbol())),
    probe(() => el.setAttribute("x", Symbol())),
    probe(() => el.getAttribute(Symbol()))
  ].join("|");
})()
"#,
        )
        .expect("live Element attribute WebIDL args should evaluate");

    assert_eq!(
        result,
        "undefined,local,local,undefined-ns,true,undefined,on,false,true,true|false|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn live_element_attribute_name_validation_matches_chromium() {
    let mut vm = new_storage_test_vm("https://element-attribute-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const allowed = [
    "@slotchange$lit$",
    ".ariahidden$lit$",
    "?inert$lit$",
    "1name",
    "invalid^Name",
    "\\",
    "'",
    "\"",
    "~",
    "<",
    "\u0001"
  ];
  const invalid = ["", "name\u0000", "has space", "name>", "name/name", "name="];
  function probe(callback) {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return "throw:" + error.name;
    }
  }
  const setAllowed = allowed.every(name => {
    const el = document.createElement("div");
    return probe(() => el.setAttribute(name, "v")) === "undefined" &&
      el.hasAttribute(name) &&
      el.getAttribute(name) === "v";
  });
  const toggleAllowed = allowed.every(name => {
    const el = document.createElement("div");
    return probe(() => el.toggleAttribute(name)) === "true" &&
      el.hasAttribute(name);
  });
  const createAllowed = allowed.every(name =>
    probe(() => document.createAttribute(name).name.length === name.length) === "true"
  );
  const nsAllowed = [
    "@slotchange$lit$",
    "1name",
    "a:0",
    "0:a",
    "a:b:c"
  ].every(name => {
    const el = document.createElement("div");
    return probe(() => el.setAttributeNS("urn:test", name, "v")) === "undefined";
  });
  const invalidSet = invalid.map(name =>
    probe(() => document.createElement("div").setAttribute(name, "v"))
  ).join(",");
  const invalidToggle = invalid.map(name =>
    probe(() => document.createElement("div").toggleAttribute(name))
  ).join(",");
  const invalidCreate = invalid.map(name =>
    probe(() => document.createAttribute(name))
  ).join(",");
  const invalidRemove = invalid.map(name => {
    const el = document.createElement("div");
    el.setAttribute("data-ok", "1");
    return probe(() => el.removeAttribute(name)) + ":" + el.getAttribute("data-ok");
  }).join(",");
  return [
    setAllowed,
    toggleAllowed,
    createAllowed,
    nsAllowed,
    invalidSet,
    invalidToggle,
    invalidCreate,
    invalidRemove
  ].join("|");
})()
"#,
        )
        .expect("live attribute name validation should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|undefined:1,undefined:1,undefined:1,undefined:1,undefined:1,undefined:1"
    );
}

#[test]
fn element_attribute_names_and_attr_nodes_match_dom_edge_cases() {
    let mut vm = new_storage_test_vm("https://element-attribute-edge-cases.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = document.implementation.createDocument(null, "");
  const weirdNames = ["xml:lang", "_name.1", "俄语"];
  const weird = weirdNames.map(name => {
    const attr = xml.createAttribute(name);
    return `${attr.name}:${attr.specified}`;
  }).join(",");

  const foreign = document.createElementNS("http://www.example.com", "foo");
  foreign.setAttribute("A", "test");
  const foreignParts = [
    foreign.hasAttribute("A"),
    foreign.hasAttributeNS("", "A"),
    foreign.getAttributeNS(null, "A")
  ].join(",");

  const div = document.createElement("div");
  const attr1 = document.createAttributeNS("ns1", "p1:name");
  attr1.value = "value1";
  const attr2 = document.createAttributeNS("ns2", "p2:name");
  attr2.value = "value2";
  div.setAttributeNode(attr1);
  div.setAttributeNodeNS(attr2);

  const caseEl = document.createElement("div");
  const lower = document.createAttributeNS("ns1", "foobar");
  const upper = document.createAttributeNS("ns1", "FOOBAR");
  caseEl.setAttributeNode(lower);
  const old = caseEl.setAttributeNode(upper);

  const htmlInXml = xml.createElementNS("http://www.w3.org/1999/xhtml", "div");
  htmlInXml.setAttributeNS("foo", "A:B", "");
  htmlInXml.setAttributeNS("", "I", "");

  return [
    weird,
    foreignParts,
    div.getAttributeNodeNS("ns1", "name").value,
    div.getAttributeNodeNS("ns2", "name").value,
    old === null,
    lower.ownerElement === caseEl,
    upper.ownerElement === caseEl,
    Object.getOwnPropertyNames(htmlInXml.attributes).join(",")
  ].join("|");
})()
"#,
        )
        .expect("attribute edge case probe should evaluate");

    assert_eq!(
        result,
        "xml:lang:true,_name.1:true,俄语:true|true,true,test|value1|value2|true|true|true|0,1,A:B,I"
    );
}
#[test]
fn document_element_name_validation_matches_dom_edge_cases() {
    let mut vm = new_storage_test_vm("https://document-element-name-edge-cases.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const XHTML_NS = "http://www.w3.org/1999/xhtml";
  const xml = document.implementation.createDocument(null, "");
  const xhtml = document.implementation.createDocument(XHTML_NS, "html");
  const htmlUpper = document.createElementNS(XHTML_NS, "FOO");
  const xhtmlPlain = xhtml.createElement("f:oo");
  const xhtmlNsNull = xhtml.createElementNS(null, "foo");
  const xmlControl = xml.createElement("A\u0001");
  const attrControl = probe(() => document.createAttribute("\u0001"));
  function probe(callback) {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  }
  return [
    xhtmlPlain.localName,
    xhtmlPlain.namespaceURI,
    xhtmlNsNull.localName,
    xhtmlNsNull.namespaceURI,
    xhtmlNsNull.nodeName,
    xmlControl.localName.charCodeAt(1),
    attrControl,
    htmlUpper instanceof HTMLUnknownElement,
    probe(() => document.createElement("foo/bar")),
    probe(() => document.createElementNS("urn:test", "/:div")),
    probe(() => document.createAttribute("="))
  ].join("|");
})()
"#,
        )
        .expect("document element name edge case probe should evaluate");

    assert_eq!(
        result,
        "f:oo|http://www.w3.org/1999/xhtml|foo||foo|1|ok|true|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError"
    );
}
#[test]
fn html_unknown_element_brand_survives_clone_node_custom_prototype() {
    let mut vm = new_storage_test_vm("https://html-unknown-clone-node.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const unknown = document.createElement("unknown");
  const clone = unknown.cloneNode();
  const proto = Object.create(HTMLElement.prototype);
  const customProtoNode = document.createElement("hi");
  Object.setPrototypeOf(customProtoNode, proto);
  const customProtoClone = customProtoNode.cloneNode(true);
  const autonomous = document.createElement("x-foo");
  return [
    unknown instanceof HTMLUnknownElement,
    Object.prototype.toString.call(unknown),
    clone instanceof HTMLUnknownElement,
    Object.prototype.toString.call(clone),
    proto.isPrototypeOf(customProtoNode),
    proto.isPrototypeOf(customProtoClone),
    customProtoClone instanceof HTMLUnknownElement,
    Object.prototype.toString.call(customProtoClone),
    autonomous instanceof HTMLElement,
    autonomous instanceof HTMLUnknownElement
  ].join("|");
})()
"#,
        )
        .expect("HTMLUnknownElement cloneNode probe should evaluate");

    assert_eq!(
        result,
        "true|[object HTMLUnknownElement]|true|[object HTMLUnknownElement]|true|false|true|[object HTMLUnknownElement]|true|false"
    );
}

#[test]
fn basefont_uses_html_element_interface_across_creation_paths() {
    let mut vm = new_storage_test_vm("https://basefont-interface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const namespace = "http://www.w3.org/1999/xhtml";
  const elements = [
    document.createElementNS(namespace, "basefont"),
    new DOMParser().parseFromString("<basefont>", "text/html").querySelector("basefont"),
    document.createElement("BASEFONT")
  ];
  return elements.map(element => [
    Object.prototype.toString.call(element),
    element instanceof HTMLElement,
    element instanceof HTMLUnknownElement
  ].join(":")).join("|");
})()
"#,
        )
        .expect("basefont interface probe should evaluate");

    assert_eq!(
        result,
        "[object HTMLElement]:true:false|[object HTMLElement]:true:false|[object HTMLElement]:true:false"
    );
}
#[test]
fn processing_instruction_target_validation_matches_xml_names() {
    let mut vm = new_storage_test_vm("https://processing-instruction-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const detached = document.implementation.createHTMLDocument("");
  function probe(doc, target, data = "x") {
    try {
      const node = doc.createProcessingInstruction(target, data);
      return `${node.target}:${node.data}`;
    } catch (error) {
      return error && error.name;
    }
  }
  return [
    probe(document, "xml:fail"),
    probe(document, "A\u00b7A"),
    probe(document, "\u00b7A"),
    probe(document, "\u00d7A"),
    probe(document, "A\u00d7"),
    probe(document, "\\A"),
    probe(document, "\f"),
    probe(document, "A", "?>"),
    probe(detached, "a0"),
    probe(detached, "\u00d7A")
  ].join("|");
})()
"#,
        )
        .expect("processing instruction target validation probe should evaluate");

    assert_eq!(
        result,
        "xml:fail:x|A·A:x|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|a0:x|InvalidCharacterError"
    );
}
#[test]
fn detached_insert_adjacent_methods_match_element_surface() {
    let mut vm = new_storage_test_vm("https://detached-insert-adjacent.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("div");
  const child = doc.createElement("span");
  const loose = doc.createElement("p");
  doc.body.appendChild(host);
  function probe(callback) {
    try {
      const value = callback();
      return value === null ? "null" : value === undefined ? "undefined" : String(value);
    } catch (error) {
      return error && error.name;
    }
  }
  const returned = host.insertAdjacentElement("beforeend", child);
  host.insertAdjacentText("afterbegin", "text");
  return [
    returned === child,
    host.firstChild.textContent,
    host.lastChild.localName,
    probe(() => loose.insertAdjacentElement("beforebegin", doc.createElement("b"))),
    probe(() => doc.documentElement.insertAdjacentElement("beforebegin", doc.createElement("b"))),
    probe(() => doc.documentElement.insertAdjacentText("beforebegin", "x")),
    probe(() => host.insertAdjacentElement("sideways", doc.createElement("i"))),
    probe(() => host.insertAdjacentElement("beforeend", doc.doctype))
  ].join("|");
})()
"#,
        )
        .expect("detached insertAdjacent methods should evaluate");

    assert_eq!(
        result,
        "true|text|span|null|HierarchyRequestError|HierarchyRequestError|SyntaxError|TypeError"
    );
}
#[test]
fn tag_name_live_collection_freezes_document_htmlness() {
    let mut vm = new_storage_test_vm("https://tag-name-htmlness.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const XHTML_NS = "http://www.w3.org/1999/xhtml";
  const parent = document.createElement("div");
  const child1 = document.createElementNS(XHTML_NS, "a");
  child1.textContent = "xhtml:a";
  const child2 = document.createElementNS(XHTML_NS, "A");
  child2.textContent = "xhtml:A";
  const child3 = document.createElementNS("", "a");
  child3.textContent = "a";
  const child4 = document.createElementNS("", "A");
  child4.textContent = "A";
  parent.append(child1, child2, child3, child4);

  const before = parent.getElementsByTagName("A");
  const xml = document.implementation.createDocument(null, "root");
  xml.documentElement.appendChild(parent);
  const after = parent.getElementsByTagName("A");
  parent.append(child1, child2, child3, child4);

  const text = list => Array.from(list).map(node => node.textContent).join(",");
  return [
    text(before),
    text(after),
    before === after,
    text(parent.getElementsByTagName("A"))
  ].join("|");
})()
"#,
        )
        .expect("tag name HTMLness probe should evaluate");

    assert_eq!(result, "xhtml:a,A|xhtml:A,A|false|xhtml:A,A");
}

#[test]
fn tag_name_live_collections_track_create_clone_import_and_adopt_identities() {
    let mut vm = new_storage_test_vm("https://tag-name-query-identities.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const XHTML_NS = "http://www.w3.org/1999/xhtml";
  const byQualifiedName = document.getElementsByTagName("P:QUERY-NODE");
  const byNamespaceLocalName = document.getElementsByTagNameNS(XHTML_NS, "query-node");
  const container = document.createElement("main");
  document.appendChild(container);

  const created = document.createElementNS(XHTML_NS, "p:query-node");
  created.id = "created";
  container.appendChild(created);

  const cloned = created.cloneNode(false);
  cloned.id = "cloned";
  container.appendChild(cloned);

  const xml = document.implementation.createDocument(XHTML_NS, "x:query-node", null);
  const imported = document.importNode(xml.documentElement, false);
  imported.id = "imported";
  container.appendChild(imported);

  const other = document.implementation.createHTMLDocument("");
  const adopted = other.createElementNS(XHTML_NS, "p:query-node");
  adopted.id = "adopted";
  other.body.appendChild(adopted);
  document.adoptNode(adopted);
  container.appendChild(adopted);

  const ids = collection => Array.from(collection).map(node => node.id).join(",");
  return [
    ids(byQualifiedName),
    ids(byNamespaceLocalName),
    document.getElementsByTagName("x:query-node")[0] === imported,
    document.getElementsByTagNameNS(XHTML_NS, "QUERY-NODE").length,
    container.getElementsByTagNameNS(XHTML_NS, "query-node").length
  ].join("|");
})()
"#,
        )
        .expect("tag-name identity lifecycle probe should evaluate");

    assert_eq!(
        result,
        "created,cloned,adopted|created,cloned,imported,adopted|true|0|4"
    );
}

#[test]
fn lookup_namespace_uri_handles_non_element_roots_and_attrs() {
    let mut vm = new_storage_test_vm("https://lookup-namespace-uri.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const fragment = document.createDocumentFragment();
  const doctype = document.implementation.createDocumentType("html", "", "");
  const detachedDocument = new Document();
  const disconnectedAttr = document.createAttribute("foo");
  const connectedAttr = document.createAttribute("bar");
  root.setAttributeNode(connectedAttr);
  const value = item => item === null ? "null" : String(item);
  return [
    fragment.lookupNamespaceURI("xml"),
    fragment.lookupNamespaceURI("xmlns"),
    doctype.lookupNamespaceURI("xml"),
    doctype.lookupNamespaceURI("xmlns"),
    typeof detachedDocument.lookupNamespaceURI,
    detachedDocument.lookupNamespaceURI("xml"),
    disconnectedAttr.lookupNamespaceURI("xml"),
    connectedAttr.lookupNamespaceURI("xml"),
    connectedAttr.lookupNamespaceURI("xmlns")
  ].map(value).join("|");
})()
"#,
        )
        .expect("lookupNamespaceURI namespace edge cases should evaluate");

    assert_eq!(
        result,
        "null|null|null|null|function|null|null|http://www.w3.org/XML/1998/namespace|http://www.w3.org/2000/xmlns/"
    );
}

#[test]
fn attr_nodes_inherit_node_but_tree_mutation_rejects_them() {
    let mut vm = new_storage_test_vm("https://attr-node-tree-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement("div");
  const child = document.createElement("span");
  const attr = document.createAttribute("data-value");
  parent.appendChild(child);
  const outcome = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  };

  return [
    attr instanceof Attr,
    attr instanceof Node,
    Attr.prototype instanceof Node,
    Object.getPrototypeOf(Attr.prototype) === Node.prototype,
    outcome(() => parent.appendChild(attr)),
    outcome(() => parent.insertBefore(attr, child)),
    outcome(() => parent.replaceChild(attr, child)),
    outcome(() => parent.append(attr)),
    outcome(() => child.before(attr)),
    outcome(() => parent.removeChild(attr)),
    outcome(() => parent.insertBefore(document.createTextNode("x"), attr)),
    parent.firstChild === child,
    parent.childNodes.length
  ].join("|");
})()
"#,
        )
        .expect("Attr Node inheritance and tree mutation probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|HierarchyRequestError:3|HierarchyRequestError:3|HierarchyRequestError:3|HierarchyRequestError:3|HierarchyRequestError:3|NotFoundError:8|NotFoundError:8|true|1"
    );
}

#[test]
fn attr_clone_node_copies_attribute_metadata_without_owner() {
    let mut vm = new_storage_test_vm("https://attr-clone-node.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodShape = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      Object.prototype.hasOwnProperty.call(owner, name),
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable,
      descriptor.writable
    ].join(':');
  };
  const attr = document.createAttribute("data-value");
  attr.value = "one";
  const attrClone = attr.cloneNode(true);

  const namespaced = document.createAttributeNS("urn:moli:test", "lm:flag");
  namespaced.value = "two";
  const namespacedClone = namespaced.cloneNode(false);

  const element = document.createElement("div");
  const live = document.createAttributeNS("urn:moli:live", "lm:item");
  live.value = "before";
  element.setAttributeNodeNS(live);
  element.setAttributeNS("urn:moli:live", "lm:item", "after");
  const liveClone = live.cloneNode();

  return JSON.stringify({
    attr: [
      attrClone !== attr,
      attrClone instanceof Attr,
      Object.prototype.toString.call(attrClone),
      attrClone.nodeType,
      attrClone.name,
      attrClone.localName,
      attrClone.value,
      attrClone.ownerElement === null,
      typeof attrClone.cloneNode,
      attrClone.cloneNode().value
    ],
    namespaced: [
      namespacedClone !== namespaced,
      namespacedClone.name,
      namespacedClone.localName,
      namespacedClone.prefix,
      namespacedClone.namespaceURI,
      namespacedClone.value,
      namespacedClone.ownerElement === null
    ],
    live: [
      live.ownerElement === element,
      live.value,
      liveClone !== live,
      liveClone.ownerElement === null,
      liveClone.name,
      liveClone.localName,
      liveClone.prefix,
      liveClone.namespaceURI,
      liveClone.value
    ],
    methods: [
      methodShape(attr, "isSameNode"),
      methodShape(attr, "cloneNode"),
      methodShape(attr, "lookupNamespaceURI")
    ],
    methodBehavior: [
      attr.isSameNode(attr),
      attr.isSameNode(attrClone),
      live.lookupNamespaceURI("xml")
    ]
  });
})()
"#,
        )
        .expect("Attr cloneNode probe should evaluate");

    assert_eq!(
        result,
        r#"{"attr":[true,true,"[object Attr]",2,"data-value","data-value","one",true,"function","one"],"namespaced":[true,"lm:flag","flag","lm","urn:moli:test","two",true],"live":[true,"after",true,true,"lm:item","item","lm","urn:moli:live","after"],"methods":["true:function:isSameNode:0:false:true:true","true:function:cloneNode:0:false:true:true","true:function:lookupNamespaceURI:0:false:true:true"],"methodBehavior":[true,false,"http://www.w3.org/XML/1998/namespace"]}"#
    );
}

#[test]
fn attr_to_string_method_matches_declared_surface() {
    let mut vm = new_storage_test_vm("https://attr-to-string-declared-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const attr = document.createAttribute("data-value");
  attr.value = "one";
  const descriptor = Object.getOwnPropertyDescriptor(Attr.prototype, "toString");
  return JSON.stringify({
    descriptor: [
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable,
      descriptor.writable
    ],
    enumerableKeys: Object.keys(Attr.prototype).includes("toString"),
    ownNamesInclude: Object.getOwnPropertyNames(Attr.prototype).includes("toString"),
    attrString: attr.toString(),
    objectString: Object.prototype.toString.call(attr),
    fakeReceiverString: Attr.prototype.toString.call({})
  });
})()
"#,
        )
        .expect("Attr.prototype.toString descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":["function","toString",0,false,true,true],"enumerableKeys":false,"ownNamesInclude":true,"attrString":"[object Attr]","objectString":"[object Attr]","fakeReceiverString":"[object Attr]"}"#
    );
}

#[test]
fn attr_state_slot_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://attr-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const internalNames = attr => Object.getOwnPropertyNames(attr)
    .filter(name => name.startsWith("__moliAttr"))
    .sort();
  const element = document.createElement("div");
  const other = document.createElement("section");
  element.setAttribute("data-real", "before");
  const live = element.getAttributeNode("data-real");
  const detached = document.createAttributeNS("urn:moli:test", "lm:flag");
  detached.value = "real";
  const internalNamesBefore = {
    live: internalNames(live),
    detached: internalNames(detached)
  };
  live.__moliAttrState = {
    name: "data-spoof",
    value: "spoofed",
    ownerElement: other,
    ownerDocument: document,
    namespaceURI: null,
    prefix: null,
    localName: "data-spoof"
  };
  detached.__moliAttrState = {
    name: "fake:flag",
    value: "spoofed",
    ownerElement: other,
    ownerDocument: document,
    namespaceURI: "urn:fake",
    prefix: "fake",
    localName: "flag"
  };
  live.value = "after";
  const fake = {
    __moliAttrState: {
      name: "fake",
      value: "fake",
      ownerElement: other,
      ownerDocument: document,
      namespaceURI: "urn:fake",
      prefix: "fake",
      localName: "fake"
    }
  };
  return JSON.stringify({
    internalNamesBefore,
    liveName: live.name,
    liveValue: live.value,
    elementValue: element.getAttribute("data-real"),
    spoofedOwnerValue: other.getAttribute("data-spoof"),
    detachedName: detached.name,
    detachedLocalName: detached.localName,
    detachedPrefix: detached.prefix,
    detachedNamespace: detached.namespaceURI,
    detachedValue: detached.value,
    fakeClone: live.cloneNode.call(fake),
    fakeLookupNamespace: live.lookupNamespaceURI.call(fake, "xml"),
    fakeStateValue: fake.__moliAttrState.value
  });
})()
"#,
        )
        .expect("Attr private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"internalNamesBefore":{"live":[],"detached":[]},"liveName":"data-real","liveValue":"after","elementValue":"after","spoofedOwnerValue":null,"detachedName":"lm:flag","detachedLocalName":"flag","detachedPrefix":"lm","detachedNamespace":"urn:moli:test","detachedValue":"real","fakeClone":null,"fakeLookupNamespace":null,"fakeStateValue":"fake"}"#
    );
}

#[test]
fn attr_owner_document_tracks_element_adoption() {
    let mut vm = new_storage_test_vm("https://attr-owner-document.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const div = document.createElement("div");
  div.id = "target";
  const attr = div.attributes[0];
  const other = document.implementation.createHTMLDocument("");
  other.body.appendChild(div);
  const created = other.createAttribute("data-created");
  return [
    attr.ownerDocument === other,
    div.attributes[0].ownerDocument === other,
    created.ownerDocument === other
  ].join("|");
})()
"#,
        )
        .expect("Attr ownerDocument adoption probe should evaluate");

    assert_eq!(result, "true|true|true");
}
#[test]
fn dom_mixin_members_have_webidl_unscopables() {
    let mut vm = new_storage_test_vm("https://dom-unscopables.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const expected = new Map([
    [CharacterData, ["after", "before", "remove", "replaceWith"]],
    [Document, ["append", "fullscreen", "prepend", "replaceChildren"]],
    [DocumentFragment, ["append", "prepend", "replaceChildren"]],
    [DocumentType, ["after", "before", "remove", "replaceWith"]],
    [Element, ["after", "append", "before", "prepend", "remove", "replaceChildren", "replaceWith", "slot"]]
  ]);

  for (const [constructor, names] of expected) {
    const prototype = constructor.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, Symbol.unscopables);
    assert(!!descriptor, `${constructor.name} @@unscopables descriptor`);
    assert(descriptor.value === prototype[Symbol.unscopables], `${constructor.name} value`);
    assert(descriptor.writable === false, `${constructor.name} writable`);
    assert(descriptor.enumerable === false, `${constructor.name} enumerable`);
    assert(descriptor.configurable === true, `${constructor.name} configurable`);
    const unscopables = descriptor.value;
    assert(Object.getPrototypeOf(unscopables) === null, `${constructor.name} null prototype`);
    assert(Object.getOwnPropertySymbols(unscopables).length === 0, `${constructor.name} symbol keys`);
    assert(
      Object.getOwnPropertyNames(unscopables).sort().join("|") === [...names].sort().join("|"),
      `${constructor.name} exact names`
    );
    for (const name of names) {
      const property = Object.getOwnPropertyDescriptor(unscopables, name);
      assert(property.value === true, `${constructor.name}.${name} value`);
      assert(property.writable === true, `${constructor.name}.${name} writable`);
      assert(property.enumerable === true, `${constructor.name}.${name} enumerable`);
      assert(property.configurable === true, `${constructor.name}.${name} configurable`);
    }
  }

  window.prepend = "global-prepend";
  window.append = "global-append";
  const element = document.createElement("div");
  element.setAttribute(
    "onclick",
    "globalThis.__domUnscopableResolution = [prepend, append];"
  );
  element.dispatchEvent(new Event("click"));
  assert(
    globalThis.__domUnscopableResolution.join("|") === "global-prepend|global-append",
    "Document ParentNode methods must not shadow globals after Element scope"
  );
  delete globalThis.__domUnscopableResolution;
  delete window.prepend;
  delete window.append;
  return "ok";
})()
"#,
        )
        .expect("DOM unscopables probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn node_mixin_methods_install_unscopables_on_each_interface() {
    let mut vm = new_storage_test_vm("https://node-mixin-unscopables.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parentNodeNames = ["prepend", "append", "replaceChildren"];
  const childNodeNames = ["before", "after", "replaceWith", "remove"];
  const hasUnscopables = (prototype, names) => {
    const unscopables = prototype[Symbol.unscopables];
    return Object.getPrototypeOf(unscopables) === null &&
      names.every(name => unscopables[name] === true);
  };
  return [
    hasUnscopables(Document.prototype, parentNodeNames),
    hasUnscopables(DocumentFragment.prototype, parentNodeNames),
    hasUnscopables(Element.prototype, [...parentNodeNames, ...childNodeNames]),
    hasUnscopables(DocumentType.prototype, childNodeNames),
    hasUnscopables(CharacterData.prototype, childNodeNames)
  ].join("|");
})()
"#,
        )
        .expect("node mixin unscopables probe should evaluate");

    assert_eq!(result, "true|true|true|true|true");
}

#[test]
fn element_event_handler_honors_document_parent_node_unscopables() {
    let mut vm = new_storage_test_vm("https://event-handler-unscopables.test/");

    let result = vm
        .eval(
            r#"
(() => {
  window.prepend = "global prepend";
  window.append = "global append";
  const element = document.createElement("div");
  element.setAttribute("onclick", `
    window.__unscopablesResult = [
      typeof prepend,
      typeof append,
      typeof this.prepend,
      typeof this.append
    ].join("|");
  `);
  element.dispatchEvent(new Event("click"));
  return window.__unscopablesResult;
})()
"#,
        )
        .expect("event handler unscopables probe should evaluate");

    assert_eq!(result, "string|string|function|function");
}
#[test]
fn closest_invalid_uses_effective_option_value() {
    let mut vm = new_storage_test_vm("https://closest-invalid-select.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const fieldset = document.createElement("fieldset");
  fieldset.id = "fieldset";
  const select = document.createElement("select");
  select.id = "select";
  select.required = true;
  const option = document.createElement("option");
  option.id = "option";
  option.selected = true;
  option.textContent = "non-empty fallback";
  const input = document.createElement("input");
  input.required = true;
  select.append(option);
  fieldset.append(select, input);
  root.append(fieldset);
  return [
    select.matches(":invalid"),
    fieldset.matches(":invalid"),
    option.closest(":invalid").id
  ].join("|");
})()
"#,
        )
        .expect("closest :invalid probe should evaluate");

    assert_eq!(result, "false|true|fieldset");
}
#[test]
fn parent_node_append_prepend_validate_before_fragment_conversion() {
    let mut vm = new_storage_test_vm("https://parent-node-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return error && error.name;
    }
  };
  const appendDoc = document.implementation.createHTMLDocument("title");
  const appendAncestor = probe(() => appendDoc.body.append(appendDoc.documentElement));
  const appendBodyPreserved = appendDoc.body && appendDoc.body.parentNode === appendDoc.documentElement;

  const prependDoc = document.implementation.createHTMLDocument("title");
  const prependAncestor = probe(() => prependDoc.body.prepend(prependDoc.documentElement));
  const prependBodyPreserved = prependDoc.body && prependDoc.body.parentNode === prependDoc.documentElement;

  const doc = document.implementation.createHTMLDocument("title");
  const otherDoc = document.implementation.createHTMLDocument("other");
  const documentNode = probe(() => doc.append(otherDoc));
  const doctypeIntoElement = probe(() => doc.createElement("a").append(doc.doctype));
  const duplicateDoctype = probe(() => doc.append(doc.doctype.cloneNode()));

  return [
    appendAncestor,
    appendBodyPreserved,
    prependAncestor,
    prependBodyPreserved,
    documentNode,
    doctypeIntoElement,
    duplicateDoctype
  ].join("|");
})()
"#,
        )
        .expect("ParentNode pre-insertion validation probe should evaluate");

    assert_eq!(
        result,
        "HierarchyRequestError|true|HierarchyRequestError|true|HierarchyRequestError|HierarchyRequestError|HierarchyRequestError"
    );
}
#[test]
fn element_matches_delegates_cross_realm_elements() {
    let mut vm = new_storage_test_vm("https://matches-cross-realm.test/#target");
    let result = vm
        .eval(
            r##"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = body.appendChild(document.createElement("iframe"));
  const doc = frame.contentDocument;
  const frameRoot = doc.documentElement || doc.appendChild(doc.createElement("html"));
  const frameBody = doc.body || frameRoot.appendChild(doc.createElement("body"));
  frameBody.innerHTML = `
    <div id="universal"><address id="address"><code id="code"></code></address></div>
    <p id="nth"><em id="em1"></em><strong></strong><em id="em2"></em><strong></strong><em id="em3"></em></p>
    <fieldset disabled><input id="disabledInput"></fieldset>
    <div id="target"></div>
  `;
  const code = doc.getElementById("code");
  return [
    code.matches("*"),
    code.matches("#universal > * > *"),
    doc.getElementById("em3").matches("#nth em:nth-of-type(3)"),
    doc.getElementById("disabledInput").matches(":disabled"),
    doc.getElementById("target").matches("#target")
  ].join("|");
})()
"##,
        )
        .expect("cross-realm matches probe should evaluate");

    assert_eq!(result, "true|true|true|true|true");
}
#[tokio::test]
async fn element_matches_delegates_loaded_child_document_elements() {
    let mut vm = new_storage_test_vm("https://matches-loaded-child.test/#parent");
    vm.eval(
        r##"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <body>
      <div id="universal"><address id="address"><code id="code"></code></address></div>
      <p id="nth"><em id="em1"></em><strong></strong><em id="em2"></em><strong></strong><em id="em3"></em></p>
      <fieldset disabled><input id="disabledInput"></fieldset>
      <div id="target"></div>
    </body>
  `;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"##,
    )
    .expect("loaded child matches setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "loaded child matches document")
        .await;

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.querySelector("iframe").contentDocument;
  doc.defaultView.history.replaceState(null, "", "#target");
  const code = doc.getElementById("code");
  return [
    code.ownerDocument === doc,
    doc.querySelectorAll("*").length,
    doc.querySelector("#code") === code,
    code.matches("code"),
    code.matches("#code"),
    code.matches("address code"),
    code.matches("*"),
    code.matches("#universal > * > *"),
    doc.getElementById("em3").matches("#nth em:nth-of-type(3)"),
    doc.getElementById("disabledInput").matches(":disabled"),
    doc.getElementById("target").matches("#target"),
    document.querySelector("iframe").contentDocument.URL.endsWith("#target"),
    document.querySelector(":target") === null,
    doc.querySelector(":target") === doc.getElementById("target"),
    doc.getElementById("target").matches(":target")
  ].join("|");
})()
"##,
        )
        .expect("loaded child matches probe should evaluate");

    assert_eq!(
        result,
        "true|15|true|true|true|true|true|true|true|true|true|true|true|true|true"
    );
}

#[tokio::test]
async fn child_content_document_getter_does_not_enumerate_script_wrappers_after_load() {
    let mut vm = new_storage_test_vm("https://child-content-document-getter-script-state.test/");
    vm.eval(
        r#"
(() => {
  globalThis.__childGetterCalledPageMethod = 0;
  const frame = document.createElement("iframe");
  frame.srcdoc = `<body><script>
    document.getElementsByTagName = function() {
      top.__childGetterCalledPageMethod += 1;
      throw new Error("contentDocument getter should not call page method");
    };
    top.__childScriptRan = true;
  </` + `script></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child contentDocument getter setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child srcdoc should commit before its parser script",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child srcdoc parser script should run on DocumentScriptReady",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(
        &mut vm,
        "child contentDocument getter srcdoc",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector("iframe");
  const doc = frame.contentDocument;
  return [
    String(globalThis.__childScriptRan),
    String(doc === frame.contentWindow.document),
    String(globalThis.__childGetterCalledPageMethod)
  ].join("|");
})()
"#,
        )
        .expect("child contentDocument getter should not call child document methods");

    assert_eq!(result, "true|true|0");
}

#[tokio::test]
async fn loaded_child_document_top_level_children_remain_removable_from_parent_realm() {
    let mut vm = new_storage_test_vm("https://child-document-top-level-remove.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "restore-frame";
  frame.srcdoc = `<!doctype html><html><head></head><body><p>child</p></body></html>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child top-level removal setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "loaded child top-level removal document",
    )
    .await;

    let frame_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("restore-frame")
        .expect("iframe should have a native handle");
    let child_document = vm
        ._context_host
        .borrow()
        .child_browsing_context_document_handle(frame_handle)
        .expect("iframe should have a child document handle");
    let child_handles = vm
        .document_runtime
        .dom_host()
        .child_handles(child_document)
        .collect::<Vec<_>>();
    assert!(
        child_handles.iter().all(|child| {
            vm.document_runtime
                .dom_host()
                .node(*child)
                .and_then(Node::parent_node)
                == Some(child_document)
        }),
        "native child document top-level children should belong to the child document"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("restore-frame");
  const referenceDoc = document.implementation.createHTMLDocument("reference");
  referenceDoc.removeChild(referenceDoc.documentElement);
  referenceDoc.appendChild(frame.contentDocument.documentElement.cloneNode(true));

  const doc = frame.contentDocument;
  const before = [
    doc === frame.contentWindow.document,
    doc.firstChild.parentNode === doc,
    doc.lastChild.parentNode === doc,
    doc.documentElement && doc.lastChild.isSameNode(doc.documentElement)
  ];

  while (frame.contentDocument.firstChild &&
         frame.contentDocument.firstChild.nodeType != Node.DOCUMENT_TYPE_NODE) {
    frame.contentDocument.removeChild(frame.contentDocument.firstChild);
  }
  while (frame.contentDocument.lastChild &&
         frame.contentDocument.lastChild.nodeType != Node.DOCUMENT_TYPE_NODE) {
    frame.contentDocument.removeChild(frame.contentDocument.lastChild);
  }
  if (!frame.contentDocument.firstChild) {
    frame.contentDocument.appendChild(
      frame.contentDocument.implementation.createDocumentType("html", "", "")
    );
  }
  const appended = frame.contentDocument.appendChild(
    referenceDoc.documentElement.cloneNode(true)
  );

  return JSON.stringify({
    before,
    first: doc.firstChild && doc.firstChild.nodeName,
    last: doc.lastChild && doc.lastChild.nodeName,
    appendedParent: appended.parentNode === doc,
    documentElementIsAppended: doc.documentElement === appended,
    bodyText: doc.body && doc.body.textContent
  });
})()
"#,
        )
        .expect("child document WPT-style restore should evaluate");

    assert_eq!(
        result,
        r#"{"before":[true,true,true,true],"first":"html","last":"HTML","appendedParent":true,"documentElementIsAppended":true,"bodyText":"child"}"#
    );
}

#[test]
fn small_static_nodelist_array_indexof_remains_fast_after_length_tamper() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.createElement("div");
  for (let i = 0; i < 100; i++) {
    const span = document.createElement("span");
    span.className = "hit";
    root.appendChild(span);
  }
  document.body.appendChild(root);
  const nodes = root.querySelectorAll(".hit");
  const target = nodes[50];
  let before = -1;
  let after = -1;
  const { indexOf } = Array.prototype;
  for (let i = 0; i < 1000; i++) {
    before = indexOf.call(nodes, target);
  }
  Object.defineProperty(nodes, "length", { get() { return 10; } });
  for (let i = 0; i < 1000; i++) {
    after = indexOf.call(nodes, target);
  }
  return before + "|" + after;
})()
"#,
        )
        .expect("static NodeList Array#indexOf probe should evaluate");

    assert_eq!(result, "50|-1");
}

#[test]
fn main_document_static_nodelist_reads_skip_child_realm_prototype_sync() {
    let markup = format!(
        "<!doctype html><html><body>{}</body></html>",
        "<span class=hit></span>".repeat(1_100)
    );
    let mut vm = new_parsed_test_vm("https://example.test/", &markup);
    crate::native_bridge::bindings::reset_wrapper_owner_realm_custom_element_checks_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const nodes = document.querySelectorAll(".hit");
  let nodeTypeSum = 0;
  for (let round = 0; round < 2; round++) {
    for (let index = 0; index < nodes.length; index++) {
      nodeTypeSum += nodes[index].nodeType;
    }
  }
  return `${nodes.length}|${nodeTypeSum}|${nodes[0] === nodes[0]}`;
})()
"#,
        )
        .expect("large static NodeList iteration should evaluate");

    assert_eq!(result, "1100|2200|true");
    assert_eq!(
        crate::native_bridge::bindings::wrapper_owner_realm_custom_element_checks_for_test(),
        0,
        "main-document wrappers cannot require a child-realm prototype"
    );
}

#[test]
fn main_document_large_static_nodelist_indices_survive_isolate_gc() {
    let markup = format!(
        "<!doctype html><html><body>{}</body></html>",
        "<span class=hit></span>".repeat(1_100)
    );
    let mut vm = new_parsed_test_vm("https://example.test/", &markup);

    vm.eval("globalThis.__largeStaticNodes = document.querySelectorAll('.hit')")
        .expect("large static NodeList should be retained across turns");
    vm.collect_renderer_document_isolate_garbage()
        .expect("document isolate garbage collection should run");

    let result = vm
        .eval(
            r#"
(() => {
  let nodeTypeSum = 0;
  for (let index = 0; index < __largeStaticNodes.length; index++) {
    const node = __largeStaticNodes[index];
    if (node === undefined) return `missing:${index}`;
    nodeTypeSum += node.nodeType;
  }
  return `${__largeStaticNodes.length}|${nodeTypeSum}|${__largeStaticNodes[0] === __largeStaticNodes[0]}`;
})()
"#,
        )
        .expect("large static NodeList iteration after GC should evaluate");

    assert_eq!(result, "1100|1100|true");
}

#[test]
fn live_node_wrappers_use_intrinsic_prototypes_after_public_constructor_replacement() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/",
        "<!doctype html><html><body><span data-cp=41></span></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const intrinsicPrototype = HTMLSpanElement.prototype;
  Object.defineProperty(globalThis, "HTMLSpanElement", {
    configurable: true,
    value: undefined,
  });
  const span = document.querySelector("span");
  return `${span.dataset.cp}|${Object.getPrototypeOf(span) === intrinsicPrototype}`;
})()
"#,
        )
        .expect("live node wrapper should use the realm's intrinsic prototype");

    assert_eq!(result, "41|true");
}

#[test]
fn live_html_get_attribute_node_normalizes_cached_attr_name() {
    let mut vm = new_storage_test_vm("https://live-attribute-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const el = document.createElement("div");
  el.setAttribute("DATA-CASE", "v");
  const first = el.getAttributeNode("DATA-CASE");
  const second = el.getAttributeNode("data-case");
  return [
    el.getAttribute("data-case"),
    el.getAttribute("DATA-CASE"),
    first && first.name,
    first === second,
    el.getAttributeNames().join(",")
  ].join("|");
})()
"#,
        )
        .expect("live HTML getAttributeNode cache should evaluate");

    assert_eq!(result, "v|v|data-case|true|data-case");
}
#[test]
fn range_detach_is_a_no_op() {
    let mut vm = new_storage_test_vm("https://range-detach.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const r = document.createRange();
  if (typeof r.detach !== 'function') return 'no-detach';
  r.detach();
  return [
    r.startContainer === document,
    r.endContainer === document,
    r.startOffset,
    r.endOffset,
    r.collapsed,
  ].join(',');
})()
"#,
        )
        .expect("range detach should evaluate");
    assert_eq!(result, "true,true,0,0,true");
}

#[test]
fn range_point_comparison_uses_tree_root_not_owner_document() {
    let mut vm = new_storage_test_vm("https://range-native-root-comparison.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}:${error && error.code}`;
    }
  };

  const left = document.createElement("div");
  const leftText = document.createTextNode("left");
  left.appendChild(leftText);
  const right = document.createElement("div");
  const rightText = document.createTextNode("right");
  right.appendChild(rightText);

  const leftRange = document.createRange();
  leftRange.setStart(leftText, 0);
  leftRange.setEnd(leftText, 2);
  const rightRange = document.createRange();
  rightRange.setStart(rightText, 0);
  rightRange.setEnd(rightText, 2);

  const sameRootRange = document.createRange();
  sameRootRange.setStart(left, 0);
  sameRootRange.setEnd(left, 1);

  const foreignDoc = document.implementation.createHTMLDocument("");
  const foreignText = foreignDoc.createTextNode("foreign");
  foreignDoc.body.appendChild(foreignText);
  const foreignRange = foreignDoc.createRange();
  foreignRange.setStart(foreignText, 1);
  foreignRange.setEnd(foreignText, 4);

  const doctype = document.implementation.createDocumentType("root", "", "");
  const doctypeDoc = document.implementation.createDocument(null, "root", doctype);
  const doctypeRange = doctypeDoc.createRange();
  doctypeRange.setStart(doctypeDoc, 0);
  doctypeRange.setEnd(doctypeDoc, 1);

  return [
    probe(() => leftRange.compareBoundaryPoints(Range.START_TO_START, rightRange)),
    leftRange.isPointInRange(rightText, 0),
    probe(() => leftRange.comparePoint(rightText, 0)),
    probe(() => sameRootRange.compareBoundaryPoints(Range.START_TO_END, leftRange)),
    sameRootRange.isPointInRange(leftText, 1),
    probe(() => sameRootRange.comparePoint(leftText, 1)),
    foreignRange.isPointInRange(foreignText, 2),
    probe(() => foreignRange.comparePoint(foreignText, 0)),
    probe(() => foreignRange.comparePoint(foreignText, 5)),
    probe(() => doctypeRange.isPointInRange(doctype, 0))
  ].join("|");
})()
"#,
        )
        .expect("Range native root comparison probe should evaluate");

    assert_eq!(
        result,
        "throw:WrongDocumentError:4|false|throw:WrongDocumentError:4|1|true|0|true|-1|1|throw:InvalidNodeTypeError:24"
    );
}
#[test]
fn document_title_getter_walks_full_tree_and_setter_respects_head() {
    let mut vm = new_parsed_test_vm(
        "https://document-title.test/",
        "<!doctype html><html><head><title>ORIG</title></head><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  out.push('initial=' + document.title);
  document.title = 'UPDATED';
  out.push('updated=' + document.title);
  // Remove <head>; per spec the setter must be a no-op when no <title> in tree
  // and no <head> exists.
  const head = document.getElementsByTagName('head')[0];
  if (head) head.parentNode.removeChild(head);
  out.push('headRemoved=' + (document.getElementsByTagName('head').length === 0));
  document.title = 'SHOULD_NOT_APPLY';
  out.push('afterHeadGone=' + document.title);
  // Append a <title> under <body>; the getter must find it (first title in tree
  // order, regardless of whether it is under <head>).
  const t = document.createElement('title');
  t.appendChild(document.createTextNode('FROM_BODY'));
  document.body.appendChild(t);
  out.push('bodyTitle=' + document.title);
  // Now the setter has an existing title element to replace; head still absent.
  document.title = 'REPLACED_BODY';
  out.push('replaced=' + document.title);
  return out.join('|');
})()
"#,
        )
        .expect("document.title spec behavior should evaluate");
    assert_eq!(
        result,
        "initial=ORIG|updated=UPDATED|headRemoved=true|afterHeadGone=|bodyTitle=FROM_BODY|replaced=REPLACED_BODY"
    );
}

#[test]
fn document_title_normalizes_html_whitespace_and_uses_svg_namespace_rules() {
    let mut vm = new_parsed_test_vm(
        "https://document-title-semantics.test/",
        "<!doctype html><html><head><title> initial  title </title></head><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const SVG = 'http://www.w3.org/2000/svg';
  const HTML = 'http://www.w3.org/1999/xhtml';
  const out = [];

  out.push('initial=' + document.title);
  document.title = ' one\t\n  two\f\r three\u000bfour ';
  out.push('set=' + document.title);

  const htmlDoc = document.implementation.createHTMLDocument(' detached\t\n title ');
  out.push('detached=' + htmlDoc.title);
  const emptyHtmlDoc = document.implementation.createHTMLDocument('');
  out.push('emptyDetached=' + [
    emptyHtmlDoc.title,
    emptyHtmlDoc.head.firstChild.childNodes.length,
    emptyHtmlDoc.head.firstChild.firstChild.data
  ].join(','));

  const svgDoc = document.implementation.createDocument(SVG, 'svg', null);
  const oldChild = svgDoc.createElementNS(SVG, 'x-child');
  svgDoc.documentElement.appendChild(oldChild);
  svgDoc.title = ' svg\n title ';
  const svgTitle = svgDoc.documentElement.firstChild;
  out.push('svg=' + [
    svgDoc.title,
    svgTitle.namespaceURI,
    svgTitle.localName,
    svgTitle.textContent,
    svgTitle.nextSibling === oldChild
  ].join(','));

  const nestedSvgDoc = document.implementation.createDocument(SVG, 'svg', null);
  const group = nestedSvgDoc.createElementNS(SVG, 'g');
  const nestedTitle = nestedSvgDoc.createElementNS(SVG, 'title');
  nestedTitle.textContent = 'nested';
  group.appendChild(nestedTitle);
  nestedSvgDoc.documentElement.appendChild(group);
  nestedSvgDoc.title = 'direct';
  out.push('nested=' + [
    nestedSvgDoc.title,
    nestedTitle.textContent,
    nestedSvgDoc.documentElement.firstChild.localName
  ].join(','));

  const xmlDoc = document.implementation.createDocument(null, 'root', null);
  const foreignTitle = xmlDoc.createElementNS(HTML, 'title');
  foreignTitle.textContent = 'keep';
  xmlDoc.documentElement.appendChild(foreignTitle);
  xmlDoc.title = 'blocked';
  out.push('xml=' + [xmlDoc.title, foreignTitle.textContent].join(','));

  return out.join('|');
})()
"#,
        )
        .expect("Document.title namespace and normalization behavior should evaluate");
    assert_eq!(
        result,
        "initial=initial title|set=one two three\u{000b}four|detached=detached title|emptyDetached=,1,|svg=svg title,http://www.w3.org/2000/svg,title, svg\n title ,true|nested=direct,nested,title|xml=keep,keep"
    );
}
#[test]
fn named_node_map_lookup_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://namednodemap-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.value ? value.value : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  };
  const el = document.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  el.setAttribute("data-x", "x");
  el.setAttributeNS("urn:moli:test", "lm:flag", "on");
  const attrs = el.attributes;
  const detached = new DOMParser().parseFromString(
    '<html><body><div data-y="y" lm:flag="detached"></div></body></html>',
    'text/html'
  ).querySelector('div').attributes;
  return [
    attrs.getNamedItem({ toString() { return "data-x"; } }).value,
    attrs.getNamedItemNS(
      { toString() { return "urn:moli:test"; } },
      { toString() { return "flag"; } }
    ).value,
    attrs.removeNamedItem({ toString() { return "data-x"; } }).value,
    attrs.getNamedItem("data-x") === null,
    probe(() => attrs.getNamedItem()),
    probe(() => attrs.getNamedItem(Symbol("name"))),
    probe(() => attrs.getNamedItemNS("urn:moli:test")),
    probe(() => attrs.getNamedItemNS(Symbol("namespace"), "flag")),
    probe(() => attrs.removeNamedItem()),
    probe(() => attrs.removeNamedItemNS("urn:moli:test")),
    detached.getNamedItem({ toString() { return "data-y"; } }).value,
    detached.removeNamedItem({ toString() { return "data-y"; } }).value,
    probe(() => detached.getNamedItem()),
    probe(() => detached.getNamedItemNS(null)),
    probe(() => detached.removeNamedItem(Symbol("name")))
  ].join("|");
})()
"#,
        )
        .expect("NamedNodeMap WebIDL lookup argument probe should evaluate");

    assert_eq!(
        result,
        "x|on|x|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|y|y|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn dom_implementation_factory_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://domimplementation-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const impl = document.implementation;
              const probe = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              const htmlNull = impl.createHTMLDocument(null);
              const xmlUndefinedNamespace = impl.createDocument(undefined, "root", undefined);
              const xmlUndefinedQualifiedName = impl.createDocument(undefined, undefined, undefined);
              const xmlNullQualifiedName = impl.createDocument(null, null, null);
              const xmlDoc = impl.createDocument(null, "root", null);
              const htmlDoc = impl.createHTMLDocument("");
              const relaxedNameDoc = impl.createDocument(null, "f}oo", null);
              const multiColonDoc = impl.createDocument("http://example.com/", "f:o:o", null);
              const numericPrefixDoc = impl.createDocument("http://example.com/", "0:a", null);
              const originalDefineProperty = Object.defineProperty;
              Object.defineProperty = () => {
                throw new Error("page-tampered defineProperty should not run");
              };
              const definePropertyTamperDoc = impl.createDocument(null, "tamper", null);
              Object.defineProperty = originalDefineProperty;
              return [
                probe(() => impl.createDocumentType()),
                probe(() => impl.createDocumentType("html", Symbol("public"), "")),
                probe(() => impl.createDocumentType("html", {
                  toString() {
                    throw new RangeError("public");
                  }
                }, "")),
                impl.createDocumentType("test:root", "", "").name,
                impl.createDocumentType("", "", "").name,
                impl.createDocumentType("1bad", "", "").name,
                impl.createDocumentType("a:b:c", "", "").name,
                probe(() => impl.createDocumentType("bad name", "", "")),
                probe(() => impl.createDocumentType("bad>", "", "")),
                probe(() => impl.createHTMLDocument(Symbol("title"))),
                probe(() => impl.createHTMLDocument({
                  toString() {
                    throw new RangeError("title");
                  }
                })),
                htmlNull.title,
                probe(() => impl.createDocument()),
                probe(() => impl.createDocument(Symbol("namespace"), "root", null)),
                probe(() => impl.createDocument(null, Symbol("qualifiedName"), null)),
                probe(() => impl.createDocument(null, {
                  toString() {
                    throw new RangeError("qualifiedName");
                  }
                }, null)),
                probe(() => impl.createDocument(null, "root", Symbol("doctype"))),
                xmlUndefinedNamespace.documentElement.localName,
                String(xmlUndefinedNamespace.documentElement.namespaceURI),
                xmlUndefinedQualifiedName.documentElement.localName,
                String(xmlNullQualifiedName.documentElement),
                Object.getPrototypeOf(xmlDoc) === XMLDocument.prototype,
                xmlDoc instanceof XMLDocument,
                typeof xmlDoc.createElement,
                typeof xmlDoc.appendChild,
                xmlDoc.createElement("child").ownerDocument === xmlDoc,
                Object.getPrototypeOf(htmlDoc) === HTMLDocument.prototype,
                htmlDoc instanceof HTMLDocument,
                relaxedNameDoc.documentElement.localName,
                multiColonDoc.documentElement.prefix,
                multiColonDoc.documentElement.localName,
                numericPrefixDoc.documentElement.prefix,
                numericPrefixDoc.documentElement.localName,
                typeof definePropertyTamperDoc.createElement,
                typeof definePropertyTamperDoc.appendChild,
                definePropertyTamperDoc.createElement("child").ownerDocument === definePropertyTamperDoc,
                probe(() => impl.createDocument(null, ":foo", null)),
                probe(() => impl.createDocument(null, "f:o:o", null)),
                probe(() => impl.createDocument("http://example.com/", "a:0", null))
              ].join("|");
            })()
            "#,
        )
        .expect("DOMImplementation WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|RangeError|test:root||1bad|a:b:c|InvalidCharacterError|InvalidCharacterError|TypeError|RangeError|null|TypeError|TypeError|TypeError|RangeError|TypeError|root|null|undefined|null|true|true|function|function|true|true|true|f}oo|f|o:o|0|a|function|function|true|InvalidCharacterError|NamespaceError|InvalidCharacterError"
    );
}

#[test]
fn dom_implementation_prototype_methods_are_declared_with_expected_descriptors() {
    let mut vm = new_storage_test_vm("https://domimplementation-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const names = [
                "hasFeature",
                "createDocumentType",
                "createHTMLDocument",
                "createDocument"
              ];
              const shape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  DOMImplementation.prototype,
                  name
                );
                return [
                  typeof descriptor.value,
                  descriptor.value.name,
                  descriptor.value.length,
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable
                ].join(",");
              };
              const implementation = document.implementation;
              return [
                Object.getPrototypeOf(implementation) === DOMImplementation.prototype,
                Object.prototype.hasOwnProperty.call(implementation, "hasFeature"),
                Object.keys(DOMImplementation.prototype)
                  .filter(name => names.includes(name))
                  .join(","),
                names.map(shape).join("|"),
                implementation.hasFeature("unused", "unused"),
                implementation.createDocumentType("html", "", "").name,
                implementation.createHTMLDocument("Title").title,
                implementation.createDocument(null, "root", null).documentElement.localName
              ].join(";");
            })()
            "#,
        )
        .expect("DOMImplementation prototype method descriptor probe should evaluate");

    assert_eq!(
        result,
        "true;false;hasFeature,createDocumentType,createHTMLDocument,createDocument;function,hasFeature,0,true,true,true|function,createDocumentType,3,true,true,true|function,createHTMLDocument,0,true,true,true|function,createDocument,2,true,true,true;true;html;Title;root"
    );
}

#[test]
fn document_implementation_accessor_is_declared_without_public_spoofing() {
    let mut vm = new_storage_test_vm("https://document-implementation-accessor.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(
                Document.prototype,
                "implementation"
              );
              const descriptorOwner = (object, name) => {
                for (let current = object; current; current = Object.getPrototypeOf(current)) {
                  if (Object.prototype.hasOwnProperty.call(current, name)) {
                    return current;
                  }
                }
                return null;
              };
              const descriptorOnChain = (object, name) =>
                Object.getOwnPropertyDescriptor(descriptorOwner(object, name), name);
              const liveImplementation = document.implementation;
              const detached = liveImplementation.createHTMLDocument("");
              const detachedImplementation = detached.implementation;
              const detachedDescriptor = Object.getOwnPropertyDescriptor(
                detached,
                "implementation"
              );
              const detachedPrototypeDescriptor = descriptorOnChain(
                Object.getPrototypeOf(detached),
                "implementation"
              );

              document.implementation = { marker: "document" };
              detached.implementation = { marker: "detached" };

              return [
                typeof descriptor.get,
                descriptor.get.name,
                descriptor.set === undefined,
                descriptor.enumerable,
                descriptor.configurable,
                Object.keys(Document.prototype).includes("implementation"),
                Object.prototype.hasOwnProperty.call(document, "implementation"),
                Object.prototype.hasOwnProperty.call(detached, "implementation"),
                detachedDescriptor === undefined,
                descriptorOwner(Object.getPrototypeOf(detached), "implementation") === Document.prototype,
                typeof detachedPrototypeDescriptor.get,
                detachedPrototypeDescriptor.get.name,
                detachedPrototypeDescriptor.set === undefined,
                detachedPrototypeDescriptor.enumerable,
                detachedPrototypeDescriptor.configurable,
                document.implementation === liveImplementation,
                detached.implementation === detachedImplementation,
                detachedImplementation !== liveImplementation,
                liveImplementation.createDocumentType("html", "", "").ownerDocument === document,
                detachedImplementation.createDocumentType("html", "", "").ownerDocument === detached
              ].join("|");
            })()
            "#,
        )
        .expect("Document implementation accessor descriptor probe should evaluate");

    assert_eq!(
        result,
        "function|get implementation|true|true|true|true|false|false|true|true|function|get implementation|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn dom_implementation_create_document_preserves_xml_metadata_and_doctype_owner() {
    let mut vm = new_parsed_test_vm(
        "https://domimplementation-create-document.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const impl = document.implementation;
              const xhtml = impl.createDocument("http://www.w3.org/1999/xhtml", "", null);
              const svg = impl.createDocument("http://www.w3.org/2000/svg", "", null);
              const sourceDoctype = document.doctype;
              const doc = impl.createDocument(null, null, sourceDoctype);
              return [
                xhtml.contentType,
                svg.contentType,
                doc.documentElement === null,
                doc.childNodes.length,
                doc.doctype === sourceDoctype,
                doc.doctype.ownerDocument === doc
              ].join("|");
            })()
            "#,
        )
        .expect("createDocument should preserve metadata and adopt doctype owner");

    assert_eq!(
        result,
        "application/xhtml+xml|image/svg+xml|true|1|true|true"
    );
}

#[test]
fn dom_implementation_create_document_accepts_detached_doctype_after_prototype_tampering() {
    let mut vm = new_parsed_test_vm(
        "https://domimplementation-detached-doctype-brand.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const doctype = document.implementation.createDocumentType("qorflesnorf", "abcde", "x\"y");
              const ownerDocumentGetter = Object.getOwnPropertyDescriptor(Node.prototype, "ownerDocument").get;
              const wasInstance = doctype instanceof DocumentType;
              Object.setPrototypeOf(doctype, null);
              const isInstanceAfterTamper = doctype instanceof DocumentType;
              const doc = document.implementation.createDocument(null, null, doctype);
              return [
                wasInstance,
                isInstanceAfterTamper,
                doc.doctype === doctype,
                doc.childNodes.length,
                doc.doctype.ownerDocument === doc,
                ownerDocumentGetter.call(doc.doctype) === doc
              ].join("|");
            })()
            "#,
        )
        .expect("createDocument should accept detached DocumentType by internal state");

    assert_eq!(result, "true|false|true|1|false|true");
}

#[test]
fn dom_implementation_and_detached_iterator_internal_slots_ignore_public_spoofing() {
    let mut vm = new_storage_test_vm("https://domimplementation-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const singletonSlot = "__moliDOMImplementationSingleton";
              const ownerSlot = "__moliDOMImplementationOwnerDocument";
              const detachedCacheSlot = "__moliDetachedDOMImplementation";
              const iteratorNodesSlot = "__moliDetachedNodeIteratorNodes";
              const iteratorIndexSlot = "__moliDetachedNodeIteratorIndex";
              const descriptorOwner = (object, name) => {
                for (let current = object; current; current = Object.getPrototypeOf(current)) {
                  if (Object.prototype.hasOwnProperty.call(current, name)) {
                    return current;
                  }
                }
                return null;
              };
              const descriptorOnChain = (object, name) =>
                Object.getOwnPropertyDescriptor(descriptorOwner(object, name), name);

              const liveImpl = document.implementation;
              const detached = liveImpl.createHTMLDocument("");
              const detachedImpl = detached.implementation;
              const fakeDoc = liveImpl.createHTMLDocument("fake");

              const singletonOwnBefore =
                Object.getOwnPropertyNames(window).includes(singletonSlot);
              const liveOwnBefore = Object.getOwnPropertyNames(liveImpl).includes(ownerSlot);
              const detachedImplOwnBefore =
                Object.getOwnPropertyNames(detachedImpl).includes(ownerSlot);
              const detachedDocOwnBefore =
                Object.getOwnPropertyNames(detached).includes(detachedCacheSlot);

              window[singletonSlot] = { marker: "public" };
              liveImpl[ownerSlot] = fakeDoc;
              detachedImpl[ownerSlot] = fakeDoc;
              const liveDoctype = liveImpl.createDocumentType("html", "", "");
              const detachedDoctype = detachedImpl.createDocumentType("html", "", "");

              detached[detachedCacheSlot] = { marker: "public" };

              const main = detached.createElement("main");
              const span = detached.createElement("span");
              main.appendChild(span);
              detached.body.appendChild(main);
              const iterator = detached.createNodeIterator(detached.body, 1);
              const nextDescriptor = descriptorOnChain(iterator, "nextNode");
              const tagDescriptor = descriptorOnChain(iterator, Symbol.toStringTag);
              const iteratorOwnBefore = Object.getOwnPropertyNames(iterator)
                .filter(name => name === iteratorNodesSlot || name === iteratorIndexSlot)
                .sort()
                .join(",");
              iterator[iteratorNodesSlot] = [];
              iterator[iteratorIndexSlot] = 99;
              const first = iterator.nextNode();
              const second = iterator.nextNode();

              return [
                singletonOwnBefore,
                liveOwnBefore,
                detachedImplOwnBefore,
                detachedDocOwnBefore,
                window[singletonSlot].marker,
                document.implementation === liveImpl,
                liveImpl[ownerSlot] === fakeDoc,
                detachedImpl[ownerSlot] === fakeDoc,
                detached[detachedCacheSlot].marker,
                liveDoctype.ownerDocument === document,
                detachedDoctype.ownerDocument === detached,
                iteratorOwnBefore,
                Object.prototype.toString.call(iterator),
                descriptorOwner(iterator, "nextNode") === NodeIterator.prototype,
                [
                  nextDescriptor.enumerable,
                  nextDescriptor.writable,
                  nextDescriptor.configurable,
                  nextDescriptor.value.name,
                  nextDescriptor.value.length
                ].join(","),
                [
                  tagDescriptor.enumerable,
                  tagDescriptor.writable,
                  tagDescriptor.configurable,
                  tagDescriptor.value
                ].join(","),
                iterator[iteratorIndexSlot],
                first && first.nodeName,
                second && second.nodeName
              ].join("|");
            })()
            "#,
        )
        .expect("internal slot reflection and spoofing probe should evaluate");

    assert_eq!(
        result,
        "false|false|false|false|public|true|true|true|public|true|true||[object NodeIterator]|true|true,true,true,nextNode,0|false,false,true,NodeIterator|99|BODY|MAIN"
    );
}

#[test]
fn detached_xml_document_clone_preserves_document_interface() {
    let mut vm = new_storage_test_vm("https://xml-document-clone-node.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const empty = document.implementation.createDocument("namespace", "");
  const emptyClone = empty.cloneNode(true);
  const xhtml = document.implementation.createDocument(
    "http://www.w3.org/1999/xhtml",
    "html",
    null
  );
  const xhtmlClone = xhtml.cloneNode(true);
  return [
    empty.constructor === XMLDocument,
    emptyClone.constructor === XMLDocument,
    Object.prototype.toString.call(emptyClone),
    emptyClone.documentElement === null,
    xhtml.constructor === XMLDocument,
    xhtml.documentElement.localName,
    xhtmlClone.constructor === XMLDocument,
    Object.prototype.toString.call(xhtmlClone),
    xhtmlClone.documentElement.localName,
    xhtmlClone.documentElement.namespaceURI
  ].join("|");
})()
"#,
        )
        .expect("XMLDocument cloneNode brand probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object XMLDocument]|true|true|html|true|[object XMLDocument]|html|http://www.w3.org/1999/xhtml"
    );
}
#[test]
fn document_construction_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://document-construction-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const probe = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              const detached = document.implementation.createHTMLDocument("");
              const parsed = new DOMParser().parseFromString(
                "<html><body></body></html>",
                "text/html"
              );
              const parsedXml = new DOMParser().parseFromString(
                "<root/>",
                "application/xml"
              );
              const svgElement = document.createElementNS(
                { toString() { return SVG_NS; } },
                { toString() { return "svg:g"; } }
              );
              const pi = document.createProcessingInstruction(
                "xml-stylesheet",
                "href='style.css'"
              );
              const attr = document.createAttribute("data-value");
              attr.value = null;
              const attrNull = attr.value;
              const attrSymbol = probe(() => { attr.value = Symbol("attr"); });
              attr.nodeValue = { toString() { return "node-value"; } };
              const attrNodeObject = attr.value;
              const attrThrowing = probe(() => {
                attr.textContent = { toString() { throw new Error("attr-text"); } };
              });
              return [
                document.createElement({ toString() { return "ARTICLE"; } }).localName,
                document.createElement(undefined).localName,
                probe(() => document.createElement()),
                probe(() => document.createElement(Symbol("name"))),
                probe(() => document.createElement("1bad")),
                svgElement.namespaceURI + ":" + svgElement.localName,
                probe(() => document.createElementNS()),
                probe(() => document.createElementNS(null)),
                probe(() => document.createElementNS(Symbol("namespace"), "x")),
                probe(() => document.createElementNS(null, "p:root")),
                probe(() => document.createElementNS("urn:not-xml", "xml:root")),
                document.createTextNode({ toString() { return "text"; } }).data,
                probe(() => document.createTextNode()),
                document.createComment(null).data,
                probe(() => document.createComment(Symbol("comment"))),
                pi.target + ":" + pi.data,
                probe(() => document.createProcessingInstruction("xml-stylesheet")),
                probe(() => document.createProcessingInstruction("1bad", Symbol("data"))),
                probe(() => document.createCDATASection()),
                probe(() => document.createCDATASection(Symbol("data"))),
                probe(() => document.createCDATASection("data")),
                document.createAttribute({ toString() { return "DATA-X"; } }).name,
                probe(() => document.createAttribute()),
                probe(() => document.createAttribute(Symbol("attr"))),
                attrNull,
                attrSymbol,
                attrNodeObject,
                attrThrowing,
                document.createAttributeNS(null, { toString() { return "data-z"; } }).localName,
                probe(() => document.createAttributeNS(null)),
                detached.createElement({ toString() { return "section"; } }).localName,
                probe(() => detached.createElement()),
                probe(() => detached.createElement("1bad")),
                detached.createElementNS(null, undefined).localName,
                probe(() => detached.createElementNS(null)),
                probe(() => detached.createElementNS(null, "p:root")),
                detached.createTextNode({ toString() { return "detached"; } }).data,
                probe(() => detached.createTextNode()),
                probe(() => detached.createCDATASection()),
                probe(() => detached.createCDATASection("data")),
                detached.createAttribute({ toString() { return "DATA-X"; } }).name,
                probe(() => detached.createAttribute("bad name")),
                parsed.createElement({ toString() { return "main"; } }).localName,
                probe(() => parsed.createElement()),
                parsed.createAttribute({ toString() { return "DATA-X"; } }).name,
                probe(() => parsed.createAttribute("bad name")),
                parsed.createTextNode({ toString() { return "parsed"; } }).data,
                probe(() => parsed.createTextNode()),
                parsed.createComment(null).data,
                probe(() => parsed.createComment()),
                probe(() => parsed.createElement("1bad")),
                parsed.createElementNS(SVG_NS, "svg:g").namespaceURI + ":" +
                  parsed.createElementNS(SVG_NS, "svg:g").localName,
                probe(() => parsed.createElementNS()),
                probe(() => parsed.createElementNS(null, "p:root")),
                parsed.createProcessingInstruction(
                  "xml-stylesheet",
                  "href='parsed.css'"
                ).target,
                probe(() => parsed.createProcessingInstruction("xml-stylesheet")),
                probe(() => parsed.createProcessingInstruction("1bad", "x")),
                parsedXml.createCDATASection("parsed-data").data,
                probe(() => parsedXml.createCDATASection()),
                probe(() => parsedXml.createCDATASection("bad ]]> data")),
                probe(() => parsed.createCDATASection("data")),
                probe(() => parsed.createCDATASection(Symbol("data")))
              ].join("|");
            })()
            "#,
        )
        .expect("Document construction WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "article|undefined|TypeError|TypeError|InvalidCharacterError|http://www.w3.org/2000/svg:g|TypeError|TypeError|TypeError|NamespaceError|NamespaceError|text|TypeError|null|TypeError|xml-stylesheet:href='style.css'|TypeError|TypeError|TypeError|TypeError|NotSupportedError|data-x|TypeError|TypeError|null|TypeError|node-value|Error|data-z|TypeError|section|TypeError|InvalidCharacterError|undefined|TypeError|NamespaceError|detached|TypeError|TypeError|NotSupportedError|data-x|InvalidCharacterError|main|TypeError|data-x|InvalidCharacterError|parsed|TypeError|null|TypeError|InvalidCharacterError|http://www.w3.org/2000/svg:g|TypeError|NamespaceError|xml-stylesheet|TypeError|InvalidCharacterError|parsed-data|TypeError|InvalidCharacterError|NotSupportedError|TypeError"
    );
}

#[test]
fn cdata_split_text_preserves_cdata_node_type_and_updates_ranges() {
    let mut vm = new_storage_test_vm("https://cdata-split-text.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const xml = document.implementation.createDocument(null, "root");
              const cdata = xml.createCDATASection("abcd");
              xml.documentElement.appendChild(cdata);
              const range = xml.createRange();
              range.setStart(cdata, 3);
              range.setEnd(cdata, 4);
              const right = cdata.splitText(2);
              return [
                cdata.data,
                right.data,
                right.nodeType,
                right instanceof CDATASection,
                right instanceof Text,
                xml.documentElement.childNodes.length,
                range.startContainer === right,
                range.startOffset,
                range.endContainer === right,
                range.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("CDATA splitText regression probe should evaluate");

    assert_eq!(result, "ab|cd|4|true|true|2|true|1|true|2");
}

#[test]
fn detached_xml_create_element_ns_uses_namespace_interface_despite_prefix() {
    let mut vm = new_storage_test_vm("https://detached-prefixed-interface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const XHTML_NS = "http://www.w3.org/1999/xhtml";
              const SVG_NS = "http://www.w3.org/2000/svg";
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const xml = document.implementation.createDocument("foo", null);
              const html = xml.createElementNS(XHTML_NS, "html:span");
              const svg = xml.createElementNS(SVG_NS, "svg:g");
              return [
                html instanceof HTMLElement,
                html instanceof HTMLSpanElement,
                Object.prototype.toString.call(html),
                html.localName,
                html.prefix,
                html.nodeName,
                probe(() => html.attachInternals()),
                svg instanceof SVGElement,
                svg instanceof SVGGElement,
                Object.prototype.toString.call(svg),
                svg.localName,
                svg.prefix,
                svg.nodeName
              ].join("|");
            })()
            "#,
        )
        .expect("detached XML createElementNS prefixed interface probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object HTMLSpanElement]|span|html|html:span|NotSupportedError|true|true|[object SVGGElement]|g|svg|svg:g"
    );
}

#[test]
fn cssom_linkstyle_math_and_svg_element_surfaces_match_idlharness() {
    let mut vm = new_storage_test_vm("https://cssom-idl-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const MATH_NS = "http://www.w3.org/1998/Math/MathML";
              const svg = document.createElementNS(SVG_NS, "svg");
              const svgStyle = document.createElementNS(SVG_NS, "style");
              svgStyle.setAttribute("type", "text/css; charset=utf-8");
              svgStyle.textContent = "svg { color: green; }";
              svg.append(svgStyle);
              const body = document.body || document.appendChild(document.createElement("body"));
              body.append(svg);

              const math = document.createElementNS(MATH_NS, "math");
              const pi = document.createProcessingInstruction(
                "xml-stylesheet",
                "href='data:text/css,' type='text/css; charset=utf-8'"
              );

              return [
                typeof SVGStyleElement,
                svgStyle instanceof SVGStyleElement,
                "sheet" in SVGStyleElement.prototype,
                svgStyle.sheet instanceof CSSStyleSheet,
                typeof MathMLElement,
                math instanceof MathMLElement,
                math.style instanceof CSSStyleProperties,
                "sheet" in ProcessingInstruction.prototype,
                pi.sheet === null
              ].join("|");
            })()
            "#,
        )
        .expect("CSSOM idlharness surface probe should evaluate");

    assert_eq!(
        result,
        "function|true|true|true|function|true|true|true|true"
    );
}

#[test]
fn svg_style_element_declares_and_reflects_its_own_attributes() {
    let mut vm = new_storage_test_vm("https://svg-style-reflection.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const style = document.createElementNS(SVG_NS, "style");
              const prototype = SVGStyleElement.prototype;
              const accessorShape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  !!descriptor,
                  typeof descriptor?.get,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              const throwsTypeError = callback => {
                try {
                  callback();
                  return false;
                } catch (error) {
                  return error instanceof TypeError;
                }
              };

              const missing = [style.media, style.title, style.type];
              style.media = "screen";
              style.title = "theme";
              style.type = "text/example";
              const reflected = [
                style.getAttribute("media"),
                style.getAttribute("title"),
                style.getAttribute("type")
              ];
              style.setAttribute("media", "print");
              style.setAttribute("title", "alternate");
              style.setAttribute("type", "text/css");

              return JSON.stringify({
                accessors: ["media", "title", "type", "disabled"].map(accessorShape),
                missing,
                reflected,
                attributes: [style.media, style.title, style.type],
                disabled: style.disabled,
                incompatible: [
                  throwsTypeError(() => Reflect.get(prototype, "media", {})),
                  throwsTypeError(() => Reflect.set(prototype, "type", "text/css", {})),
                  throwsTypeError(() => Reflect.get(prototype, "disabled", {}))
                ]
              });
            })()
            "#,
        )
        .expect("SVGStyleElement reflection should evaluate");

    assert_eq!(
        result,
        r#"{"accessors":["true:function:function:true:true","true:function:function:true:true","true:function:function:true:true","true:function:function:true:true"],"missing":["","",""],"reflected":["screen","theme","text/example"],"attributes":["print","alternate","text/css"],"disabled":false,"incompatible":[true,true,true]}"#,
    );
}

#[test]
fn svg_script_element_uses_its_declared_surface_and_shared_async_state() {
    let mut vm = new_storage_test_vm("https://svg-script-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const script = document.createElementNS(SVG_NS, "script");
              const typeDescriptor = Object.getOwnPropertyDescriptor(
                SVGScriptElement.prototype,
                "type"
              );
              const asyncDescriptor = Object.getOwnPropertyDescriptor(
                SVGScriptElement.prototype,
                "async"
              );
              const initiallyAsync = script.async;
              script.async = false;
              const explicitlySynchronous = [script.async, script.hasAttribute("async")];
              script.async = true;
              script.type = "module";

              return JSON.stringify({
                constructorType: typeof SVGScriptElement,
                declaredPrototype: Object.getPrototypeOf(script) === SVGScriptElement.prototype,
                svgInheritance: script instanceof SVGElement,
                scriptBrand: script instanceof SVGScriptElement,
                tag: Object.prototype.toString.call(script),
                typeEnumerable: typeDescriptor.enumerable,
                asyncEnumerable: asyncDescriptor.enumerable,
                initiallyAsync,
                explicitlySynchronous,
                asyncAfterSet: script.async,
                asyncAttributeAfterSet: script.hasAttribute("async"),
                typeAfterSet: script.type,
                typeAttributeAfterSet: script.getAttribute("type")
              });
            })()
            "#,
        )
        .expect("SVGScriptElement surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructorType":"function","declaredPrototype":true,"svgInheritance":true,"scriptBrand":true,"tag":"[object SVGScriptElement]","typeEnumerable":true,"asyncEnumerable":true,"initiallyAsync":true,"explicitlySynchronous":[false,false],"asyncAfterSet":true,"asyncAttributeAfterSet":true,"typeAfterSet":"module","typeAttributeAfterSet":"module"}"#
    );
}

#[test]
fn svg_list_objects_keep_declared_brand_and_members() {
    let mut vm = new_storage_test_vm("https://svg-list-objects.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(SVG_NS, "text");
              text.setAttribute("x", "10 20");
              text.setAttribute("rotate", "15 30");
              const group = document.createElementNS(SVG_NS, "g");
              const svg = document.createElementNS(SVG_NS, "svg");
              const lengthList = text.x;
              const numberList = text.rotate;
              const transformList = group.transform;
              const transform = svg.createSVGTransform();
              transformList.baseVal.initialize(transform);

              return [
                lengthList instanceof SVGAnimatedLengthList,
                Object.prototype.toString.call(lengthList),
                lengthList.baseVal instanceof SVGLengthList,
                Object.prototype.toString.call(lengthList.baseVal),
                lengthList.baseVal.numberOfItems,
                lengthList.baseVal.getItem(0) instanceof SVGLength,
                lengthList.baseVal.getItem(0).value,
                lengthList.animVal instanceof SVGLengthList,
                numberList instanceof SVGAnimatedNumberList,
                Object.prototype.toString.call(numberList),
                numberList.baseVal instanceof SVGNumberList,
                Object.prototype.toString.call(numberList.baseVal),
                numberList.baseVal.numberOfItems,
                numberList.baseVal.getItem(0).value,
                transform instanceof SVGTransform,
                Object.prototype.toString.call(transform),
                transform.type,
                transform.angle,
                transform.matrix instanceof SVGMatrix,
                Object.prototype.toString.call(transform.matrix),
                transform.matrix.a,
                transform.matrix.d,
                transform.matrix.e,
                transform.matrix.f,
                transformList instanceof SVGAnimatedTransformList,
                Object.prototype.toString.call(transformList),
                transformList.baseVal instanceof SVGTransformList,
                Object.prototype.toString.call(transformList.baseVal),
                transformList.baseVal.numberOfItems,
                transformList.baseVal.getItem(0) instanceof SVGTransform
              ].join("|");
            })()
            "#,
        )
        .expect("SVG list object probe should evaluate");

    assert_eq!(
        result,
        "true|[object SVGAnimatedLengthList]|true|[object SVGLengthList]|2|true|10|true|true|[object SVGAnimatedNumberList]|true|[object SVGNumberList]|2|15|true|[object SVGTransform]|1|0|true|[object SVGMatrix]|1|1|0|0|true|[object SVGAnimatedTransformList]|true|[object SVGTransformList]|1|true"
    );
}

#[test]
fn svg_list_matrix_and_transform_declared_methods_keep_descriptors() {
    let mut vm = new_storage_test_vm("https://svg-method-descriptors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(SVG_NS, "text");
              text.setAttribute("x", "10 20");
              text.setAttribute("rotate", "15 30");
              const group = document.createElementNS(SVG_NS, "g");
              const svg = document.createElementNS(SVG_NS, "svg");
              const lengthList = text.x.baseVal;
              const numberList = text.rotate.baseVal;
              const transformList = group.transform.baseVal;
              const transform = svg.createSVGTransform();
              const matrix = transform.matrix;
              function methodDescriptor(object, name) {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable,
                  descriptor.value.name,
                  descriptor.value.length
                ].join(",");
              }
              return JSON.stringify({
                lengthGetItem: methodDescriptor(lengthList, "getItem"),
                lengthAppendItem: methodDescriptor(lengthList, "appendItem"),
                numberInsertItemBefore: methodDescriptor(numberList, "insertItemBefore"),
                transformListCreate: methodDescriptor(transformList, "createSVGTransformFromMatrix"),
                transformListConsolidate: methodDescriptor(transformList, "consolidate"),
                transformSetRotate: methodDescriptor(transform, "setRotate"),
                transformSetSkewX: methodDescriptor(transform, "setSkewX"),
                matrixScaleNonUniform: methodDescriptor(matrix, "scaleNonUniform"),
                matrixRotateFromVector: methodDescriptor(matrix, "rotateFromVector"),
                matrixFlipX: methodDescriptor(matrix, "flipX"),
                transformOwnMethods: Object.getOwnPropertyNames(transform)
                  .filter(name => ["setMatrix", "setRotate", "setScale", "setSkewX", "setSkewY", "setTranslate"].includes(name))
                  .sort(),
                matrixOwnMethods: Object.getOwnPropertyNames(matrix)
                  .filter(name => ["flipX", "flipY", "inverse", "multiply", "rotate", "rotateFromVector", "scale", "scaleNonUniform", "skewX", "skewY", "translate"].includes(name))
                  .sort()
              });
            })()
            "#,
        )
        .expect("SVG method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"lengthGetItem":"false,true,true,getItem,1","lengthAppendItem":"false,true,true,appendItem,1","numberInsertItemBefore":"false,true,true,insertItemBefore,2","transformListCreate":"false,true,true,createSVGTransformFromMatrix,1","transformListConsolidate":"false,true,true,consolidate,0","transformSetRotate":"false,true,true,setRotate,3","transformSetSkewX":"false,true,true,setSkewX,1","matrixScaleNonUniform":"false,true,true,scaleNonUniform,2","matrixRotateFromVector":"false,true,true,rotateFromVector,2","matrixFlipX":"false,true,true,flipX,0","transformOwnMethods":["setMatrix","setRotate","setScale","setSkewX","setSkewY","setTranslate"],"matrixOwnMethods":["flipX","flipY","inverse","multiply","rotate","rotateFromVector","scale","scaleNonUniform","skewX","skewY","translate"]}"#
    );
}

#[test]
fn dom_token_list_replace_matches_order_and_validation_edges() {
    let mut vm = new_storage_test_vm("https://dom-token-list-replace.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const element = document.createElement("div");
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              element.className = "c b a";
              const existing = element.classList.replace("c", "a");
              const existingClass = element.className;
              element.className = "a a a  b";
              const observer = new MutationObserver(() => {});
              observer.observe(element, { attributes: true });
              const same = element.classList.replace("a", "a");
              const sameMutations = observer.takeRecords().length;
              observer.disconnect();
              const sameClass = element.className;
              const validation = probe(() => element.classList.replace(" ", ""));
              return JSON.stringify({ existing, existingClass, same, sameMutations, sameClass, validation });
            })()
            "#,
        )
        .expect("DOMTokenList.replace edge cases should evaluate");

    assert_eq!(
        result,
        r#"{"existing":true,"existingClass":"a b","same":true,"sameMutations":1,"sameClass":"a b","validation":"throw:SyntaxError"}"#
    );
}

#[test]
fn dom_token_list_uses_array_iteration_methods() {
    let mut vm = new_storage_test_vm("https://dom-token-list-iterators.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const element = document.createElement("div");
              element.className = "c b a";
              const list = element.classList;
              const seen = [];
              list.forEach((value, index, receiver) => {
                seen.push(`${index}=${value}:${receiver === list}`);
              });
              return JSON.stringify({
                iteratorIdentity: list[Symbol.iterator] === Array.prototype[Symbol.iterator],
                keysIdentity: list.keys === Array.prototype.keys,
                valuesIdentity: list.values === Array.prototype.values,
                entriesIdentity: list.entries === Array.prototype.entries,
                forEachIdentity: list.forEach === Array.prototype.forEach,
                keys: Array.from(list.keys()).join(","),
                values: Array.from(list.values()).join(","),
                entries: Array.from(list.entries()).map(pair => pair.join("=")).join(","),
                seen: seen.join(",")
              });
            })()
            "#,
        )
        .expect("DOMTokenList Array iterator bindings should evaluate");

    assert_eq!(
        result,
        r#"{"iteratorIdentity":true,"keysIdentity":true,"valuesIdentity":true,"entriesIdentity":true,"forEachIdentity":true,"keys":"0,1,2","values":"c,b,a","entries":"0=c,1=b,2=a","seen":"0=c:true,1=b:true,2=a:true"}"#
    );
}

#[test]
fn indexed_webapi_iterators_ignore_public_array_tampering() {
    let mut vm = new_storage_test_vm("https://webapi-intrinsic-iterators.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const ArrayConstructor = Array;
              const prototype = ArrayConstructor.prototype;
              const originals = {
                entries: prototype.entries,
                keys: prototype.keys,
                values: prototype.values,
                forEach: prototype.forEach,
                iterator: prototype[Symbol.iterator]
              };
              const poisoned = function poisonedArrayMethod() {
                throw new Error("public Array.prototype method was observed");
              };
              const failures = [];
              const valueIterableInterfaces = [
                "HTMLCollection",
                "HTMLFormControlsCollection",
                "HTMLOptionsCollection",
                "RadioNodeList",
                "HTMLAllCollection",
                "HTMLFormElement",
                "HTMLSelectElement",
                "FileList",
                "NamedNodeMap",
                "CSSStyleDeclaration",
                "StyleSheetList",
                "CSSRuleList",
                "MediaList",
                "CSSKeyframesRule",
                "DataTransferItemList",
                "Plugin",
                "PluginArray",
                "MimeTypeArray",
                "TextTrackList",
                "TextTrackCueList",
                "TouchList"
              ];
              const checkDescriptor = (interfaceName, member, expected, enumerable) => {
                const constructor = globalThis[interfaceName];
                if (typeof constructor !== "function") {
                  failures.push(`${interfaceName}:constructor`);
                  return;
                }
                const descriptor = Object.getOwnPropertyDescriptor(
                  constructor.prototype,
                  member
                );
                const label = typeof member === "symbol" ? "@@iterator" : member;
                if (!descriptor) {
                  failures.push(`${interfaceName}.${label}:missing`);
                  return;
                }
                if (descriptor.value !== expected) {
                  failures.push(`${interfaceName}.${label}:identity`);
                }
                if (
                  descriptor.enumerable !== enumerable ||
                  descriptor.writable !== true ||
                  descriptor.configurable !== true
                ) {
                  failures.push(`${interfaceName}.${label}:descriptor`);
                }
              };

              prototype.entries = poisoned;
              prototype.keys = poisoned;
              prototype.values = poisoned;
              prototype.forEach = poisoned;
              prototype[Symbol.iterator] = poisoned;
              globalThis.Array = undefined;
              try {
                const iterableInterfaces = ["NodeList", "DOMTokenList"];
                for (let index = 0; index < iterableInterfaces.length; index += 1) {
                  const interfaceName = iterableInterfaces[index];
                  checkDescriptor(interfaceName, "entries", originals.entries, true);
                  checkDescriptor(interfaceName, "keys", originals.keys, true);
                  checkDescriptor(interfaceName, "values", originals.values, true);
                  checkDescriptor(interfaceName, "forEach", originals.forEach, true);
                  checkDescriptor(
                    interfaceName,
                    Symbol.iterator,
                    originals.values,
                    false
                  );
                }
                for (
                  let index = 0;
                  index < valueIterableInterfaces.length;
                  index += 1
                ) {
                  const interfaceName = valueIterableInterfaces[index];
                  checkDescriptor(
                    interfaceName,
                    Symbol.iterator,
                    originals.values,
                    false
                  );
                }
                if (Object.hasOwn(FileList.prototype, "values")) {
                  failures.push("FileList.values:unexpected");
                }
              } finally {
                globalThis.Array = ArrayConstructor;
                prototype.entries = originals.entries;
                prototype.keys = originals.keys;
                prototype.values = originals.values;
                prototype.forEach = originals.forEach;
                prototype[Symbol.iterator] = originals.iterator;
              }
              return failures.join("|");
            })()
            "#,
        )
        .expect("indexed WebAPI templates should use V8 Array primordials");

    assert_eq!(result, "");
}

#[test]
fn html_link_rel_list_exposes_supported_tokens() {
    let mut vm = new_storage_test_vm("https://link-rel-list.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const link = document.createElement("link");
              const list = link.relList;
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              link.rel = "preload stylesheet preload";
              const beforeAdd = {
                tag: Object.prototype.toString.call(list),
                stable: list === link.relList,
                supportsType: typeof list.supports,
                supportsPreload: list.supports("preload"),
                supportsModulepreload: list.supports("modulepreload"),
                supportsUnknown: list.supports("unknown"),
                supportsEmpty: list.supports(""),
                supportsMissing: probe(() => list.supports()),
                length: list.length,
                item0: list.item(0),
                item1: list.item(1),
                containsPreload: list.contains("preload"),
                value: list.value,
                stringValue: String(list)
              };
              list.add("prefetch");
              const afterAddRel = link.rel;
              link.relList = "preconnect";
              return JSON.stringify({
                beforeAdd,
                afterAddRel,
                afterSetterRel: link.rel,
                afterSetterLength: list.length,
                afterSetterContainsPreconnect: list.contains("preconnect")
              });
            })()
            "#,
        )
        .expect("HTMLLinkElement.relList should expose DOMTokenList supported-token behavior");

    assert_eq!(
        result,
        r#"{"beforeAdd":{"tag":"[object DOMTokenList]","stable":true,"supportsType":"function","supportsPreload":true,"supportsModulepreload":true,"supportsUnknown":false,"supportsEmpty":false,"supportsMissing":"throw:TypeError","length":2,"item0":"preload","item1":"stylesheet","containsPreload":true,"value":"preload stylesheet preload","stringValue":"preload stylesheet preload"},"afterAddRel":"preload stylesheet prefetch","afterSetterRel":"preconnect","afterSetterLength":1,"afterSetterContainsPreconnect":true}"#
    );
}

#[test]
fn html_rel_list_supported_tokens_are_ascii_case_insensitive_and_owner_specific() {
    let mut vm = new_storage_test_vm("https://rel-list-supports.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const supports = (tag, token) => document.createElement(tag).relList.supports(token);
              return JSON.stringify({
                linkManifest: supports("link", "manifest"),
                linkUppercaseModulepreload: supports("link", "MODULEPRELOAD"),
                linkNoopener: supports("link", "noopener"),
                anchorNoopener: supports("a", "noopener"),
                anchorUppercaseNoreferrer: supports("a", "NOREFERRER"),
                anchorManifest: supports("a", "manifest"),
                areaOpener: supports("area", "opener"),
                formPreload: supports("form", "preload")
              });
            })()
            "#,
        )
        .expect("HTML relList supported-token matrix should evaluate");

    assert_eq!(
        result,
        r#"{"linkManifest":true,"linkUppercaseModulepreload":true,"linkNoopener":false,"anchorNoopener":true,"anchorUppercaseNoreferrer":true,"anchorManifest":false,"areaOpener":true,"formPreload":false}"#
    );
}

#[test]
fn form_control_autocomplete_uses_the_html_autofill_token_parser() {
    let mut vm = new_storage_test_vm("https://autocomplete.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const value = (tag, autocomplete, type = null) => {
                const element = document.createElement(tag);
                if (type !== null) element.type = type;
                if (autocomplete !== null) element.setAttribute("autocomplete", autocomplete);
                return element.autocomplete;
              };
              const inputDescriptor = Object.getOwnPropertyDescriptor(
                HTMLInputElement.prototype,
                "autocomplete"
              );
              const crossReceiver = callback => {
                try {
                  callback();
                  return "returned";
                } catch (error) {
                  return error && error.name;
                }
              };
              const setterInput = document.createElement("input");
              setterInput.autocomplete = " SECTION-LOGIN  shipping work TEL webauthn ";
              return JSON.stringify({
                missing: value("input", null),
                canonicalField: value("input", " NAME\t"),
                contact: value("textarea", "billing  work  email"),
                credential: value("select", "section-LOGIN shipping work tel webauthn"),
                invalid: value("select", "foo section-foo billing name"),
                hiddenOn: value("input", "on", "hidden"),
                rawSetterAttribute: setterInput.getAttribute("autocomplete"),
                canonicalSetterValue: setterInput.autocomplete,
                getterBrand: crossReceiver(() => inputDescriptor.get.call(document.createElement("textarea"))),
                setterBrand: crossReceiver(() => inputDescriptor.set.call(document.createElement("textarea"), "name"))
              });
            })()
            "#,
        )
        .expect("form-control autocomplete parser probe should evaluate");

    assert_eq!(
        result,
        r#"{"missing":"","canonicalField":"name","contact":"billing work email","credential":"section-login shipping work tel webauthn","invalid":"","hiddenOn":"","rawSetterAttribute":" SECTION-LOGIN  shipping work TEL webauthn ","canonicalSetterValue":"section-login shipping work tel webauthn","getterBrand":"TypeError","setterBrand":"TypeError"}"#
    );
}

#[test]
fn html_link_as_reflects_attribute() {
    let mut vm = new_storage_test_vm("https://link-as.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const link = document.createElement("link");
              const beforeOwn = Object.prototype.hasOwnProperty.call(link, "as");
              link.as = "json";
              const afterSet = {
                as: link.as,
                attr: link.getAttribute("as"),
                own: Object.prototype.hasOwnProperty.call(link, "as")
              };
              link.setAttribute("as", "text");
              const cases = {
                "Image": "image",
                "images": "",
                "scripT": "script",
                "style": "style",
                "": "",
                "foNt": "font",
                "foobar": "",
                "video": "video",
                "audio": "audio",
                "track": "track",
                "fetch": "fetch",
                "json": "json",
                "text": "text"
              };
              const reflected = {};
              for (const key of Object.keys(cases)) {
                link.as = key;
                reflected[key] = link.as;
              }
              return JSON.stringify({
                beforeOwn,
                descriptorOwner: Object.prototype.hasOwnProperty.call(HTMLLinkElement.prototype, "as"),
                afterSet,
                afterAttr: link.as,
                reflected
              });
            })()
            "#,
        )
        .expect("HTMLLinkElement.as should reflect the as content attribute");

    assert_eq!(
        result,
        r#"{"beforeOwn":false,"descriptorOwner":true,"afterSet":{"as":"json","attr":"json","own":false},"afterAttr":"text","reflected":{"Image":"image","images":"","scripT":"script","style":"style","":"","foNt":"font","foobar":"","video":"video","audio":"audio","track":"track","fetch":"fetch","json":"json","text":"text"}}"#
    );
}

#[test]
fn element_part_exposes_dom_token_list() {
    let mut vm = new_storage_test_vm("https://element-part-list.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const element = document.createElement("div");
              const list = element.part;
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const initial = {
                tag: Object.prototype.toString.call(list),
                stable: list === element.part,
                length: list.length,
                value: list.value,
                item0: list.item(0)
              };
              element.setAttribute("part", "alpha beta alpha");
              const afterAttribute = {
                length: list.length,
                item0: list.item(0),
                item1: list.item(1),
                containsAlpha: list.contains("alpha"),
                value: list.value
              };
              list.remove("alpha");
              const afterRemove = element.getAttribute("part");
              list.add("gamma");
              const afterAdd = element.getAttribute("part");
              element.part = "delta";
              const afterSetter = {
                attribute: element.getAttribute("part"),
                length: list.length,
                containsDelta: list.contains("delta")
              };
              const invalid = probe(() => list.add("bad token"));
              const supports = probe(() => list.supports("alpha"));
              return JSON.stringify({ initial, afterAttribute, afterRemove, afterAdd, afterSetter, invalid, supports });
            })()
            "#,
        )
        .expect("Element.part DOMTokenList should evaluate");

    assert_eq!(
        result,
        r#"{"initial":{"tag":"[object DOMTokenList]","stable":true,"length":0,"value":"","item0":null},"afterAttribute":{"length":2,"item0":"alpha","item1":"beta","containsAlpha":true,"value":"alpha beta alpha"},"afterRemove":"beta","afterAdd":"beta gamma","afterSetter":{"attribute":"delta","length":1,"containsDelta":true},"invalid":"throw:InvalidCharacterError","supports":"throw:TypeError"}"#
    );
}
#[test]
fn class_tokens_use_ascii_html_whitespace_only() {
    let mut vm = new_storage_test_vm("https://class-token-whitespace.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const nbsp = "\u00A0";
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const live = document.createElement("div");
              live.className = `alpha${nbsp}beta`;
              const root = document.documentElement || document.appendChild(document.createElement("html"));
              root.append(live);

              const detached = document.implementation.createHTMLDocument("");
              const detachedElement = detached.createElement("div");
              detachedElement.className = nbsp;
              detached.body.append(detachedElement);

              return JSON.stringify({
                liveBeta: document.getElementsByClassName("beta").length,
                liveCombined: document.getElementsByClassName(`alpha${nbsp}beta`).length,
                liveNbsp: document.getElementsByClassName(nbsp).length,
                selectorBeta: document.querySelectorAll(".beta").length,
                classListLength: live.classList.length,
                classListCombined: live.classList.contains(`alpha${nbsp}beta`),
                classListNbspAdd: probe(() => {
                  live.classList.add(nbsp);
                  return live.classList.contains(nbsp);
                }),
                detachedNbsp: detached.getElementsByClassName(nbsp).length
              });
            })()
            "#,
        )
        .expect("class token whitespace probe should evaluate");

    assert_eq!(
        result,
        r#"{"liveBeta":0,"liveCombined":1,"liveNbsp":0,"selectorBeta":0,"classListLength":1,"classListCombined":true,"classListNbspAdd":"true","detachedNbsp":1}"#
    );
}
#[test]
fn dom_token_list_value_setters_apply_webidl_domstring_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const live = document.createElement("div");
              live.classList.value = null;
              const liveNull = live.className;
              live.classList.value = undefined;
              const liveUndefined = live.className;
              live.classList.value = { toString() { return "alpha beta"; } };
              const liveObject = live.className;
              const liveSymbol = probe(() => { live.classList.value = Symbol("class"); });
              const liveAfterSymbol = live.className;
              const liveThrow = probe(() => {
                live.classList.value = { toString() { throw new RangeError("boom"); } };
              });
              const liveAfterThrow = live.className;

              const detached = document.implementation.createHTMLDocument("").body;
              detached.classList.value = null;
              const detachedNull = detached.className;
              detached.classList.value = undefined;
              const detachedUndefined = detached.className;
              detached.classList.value = { toString() { return "detached value"; } };
              const detachedObject = detached.className;
              const detachedSymbol = probe(() => { detached.classList.value = Symbol("class"); });
              const detachedAfterSymbol = detached.className;

              live.classList = null;
              const elementNull = live.className;
              live.classList = undefined;
              const elementUndefined = live.className;
              live.classList = { toString() { return "element setter"; } };
              const elementObject = live.className;
              const elementSymbol = probe(() => { live.classList = Symbol("class"); });
              const elementAfterSymbol = live.className;

              return JSON.stringify({
                liveNull,
                liveUndefined,
                liveObject,
                liveSymbol,
                liveAfterSymbol,
                liveThrow,
                liveAfterThrow,
                detachedNull,
                detachedUndefined,
                detachedObject,
                detachedSymbol,
                detachedAfterSymbol,
                elementNull,
                elementUndefined,
                elementObject,
                elementSymbol,
                elementAfterSymbol
              });
            })()
            "#,
        )
        .expect("DOMTokenList value setters should apply WebIDL DOMString conversion");

    assert_eq!(
        result,
        r#"{"liveNull":"null","liveUndefined":"undefined","liveObject":"alpha beta","liveSymbol":"TypeError","liveAfterSymbol":"alpha beta","liveThrow":"RangeError","liveAfterThrow":"alpha beta","detachedNull":"null","detachedUndefined":"undefined","detachedObject":"detached value","detachedSymbol":"TypeError","detachedAfterSymbol":"detached value","elementNull":"null","elementUndefined":"undefined","elementObject":"element setter","elementSymbol":"TypeError","elementAfterSymbol":"element setter"}"#
    );
}
#[test]
fn detached_node_internal_slots_stay_hidden_before_and_after_live_pairing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement('html'));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement('body'));
              }
              const doc = new DOMParser().parseFromString(
                '<html><body><main><span id="child">x</span></main></body></html>',
                'text/html'
              );
              const node = doc.getElementById('child');
              const host = document.body || document.documentElement || document;
              const snapshot = target => ({
                ownNamesHasState: Object.getOwnPropertyNames(target).includes("__moliDetachedState"),
                ownNamesHasDelegate: Object.getOwnPropertyNames(target).includes("__moliLiveDelegate"),
                ownKeysHasState: Reflect.ownKeys(target).includes("__moliDetachedState"),
                ownKeysHasDelegate: Reflect.ownKeys(target).includes("__moliLiveDelegate"),
                hasStateSlot: "__moliDetachedState" in target,
                hasDelegateSlot: "__moliLiveDelegate" in target,
                stateType: typeof target.__moliDetachedState,
                delegateType: typeof target.__moliLiveDelegate
              });
              const before = snapshot(node);
              host.appendChild(node);
              const after = {
                ...snapshot(node),
                ownerDocumentIsLive: node.ownerDocument === document,
                hostContains: host.contains(node),
                text: node.textContent
              };
              return JSON.stringify({ before, after });
            })()
            "#,
        )
        .expect("detached node private slots should remain hidden across live pairing");

    assert_eq!(
        result,
        r#"{"before":{"ownNamesHasState":false,"ownNamesHasDelegate":false,"ownKeysHasState":false,"ownKeysHasDelegate":false,"hasStateSlot":false,"hasDelegateSlot":false,"stateType":"undefined","delegateType":"undefined"},"after":{"ownNamesHasState":false,"ownNamesHasDelegate":false,"ownKeysHasState":false,"ownKeysHasDelegate":false,"hasStateSlot":false,"hasDelegateSlot":false,"stateType":"undefined","delegateType":"undefined","ownerDocumentIsLive":true,"hostContains":true,"text":"x"}}"#
    );
}
#[test]
fn adopt_node_updates_detached_subtree_owner_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const sourceDocument = document.implementation.createDocument(null, "root", null);
              const root = sourceDocument.documentElement;
              const child = sourceDocument.createElement("child");
              root.appendChild(child);
              const adopted = document.adoptNode(root);
              return JSON.stringify({
                returnedSameNode: adopted === root,
                detachedFromSource: sourceDocument.documentElement === null,
                rootParent: root.parentNode,
                childPreserved: root.firstChild === child,
                rootOwnerIsLive: root.ownerDocument === document,
                childOwnerIsLive: child.ownerDocument === document
              });
            })()
            "#,
        )
        .expect("detached adoptNode ownerDocument probe should evaluate");

    assert_eq!(
        result,
        r#"{"returnedSameNode":true,"detachedFromSource":true,"rootParent":null,"childPreserved":true,"rootOwnerIsLive":true,"childOwnerIsLive":true}"#
    );
}
#[test]
fn document_import_and_adopt_reject_shadow_roots() {
    let mut vm = new_storage_test_vm("https://shadow-root-import-adopt.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const detachedDocument = document.implementation.createHTMLDocument();
              const openHost = document.createElement("div");
              const openRoot = openHost.attachShadow({ mode: "open" });
              const closedHost = document.createElement("div");
              const closedRoot = closedHost.attachShadow({ mode: "closed" });

              function exceptionName(callback) {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              }

              const fragment = document.createDocumentFragment();
              fragment.appendChild(document.createElement("span"));
              const importedFragment = detachedDocument.importNode(fragment, true);

              return [
                exceptionName(() => detachedDocument.importNode(openRoot)),
                exceptionName(() => detachedDocument.importNode(closedRoot)),
                exceptionName(() => detachedDocument.adoptNode(openRoot)),
                exceptionName(() => detachedDocument.adoptNode(closedRoot)),
                exceptionName(() => document.importNode(openRoot)),
                exceptionName(() => document.adoptNode(openRoot)),
                importedFragment.nodeType,
                importedFragment.firstChild.localName
              ].join("|");
            })()
            "#,
        )
        .expect("Document importNode/adoptNode should reject ShadowRoot nodes");

    assert_eq!(
        result,
        "NotSupportedError:9|NotSupportedError:9|HierarchyRequestError:3|HierarchyRequestError:3|NotSupportedError:9|HierarchyRequestError:3|11|span"
    );
}
#[test]
fn detached_plain_document_all_matches_chromium_htmldda_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = new Document();
              const allDescriptor = Object.getOwnPropertyDescriptor(Document.prototype, "all");
              return JSON.stringify({
                ownAll: Object.prototype.hasOwnProperty.call(doc, "all"),
                protoGetter: typeof allDescriptor?.get,
                allType: typeof doc.all,
                loose: doc.all == undefined,
                strict: doc.all === undefined,
                bool: !!doc.all,
                string: String(doc.all),
                tag: Object.prototype.toString.call(doc.all),
                length: doc.all.length,
                noArgNull: doc.all() === null,
                itemZeroNull: doc.all(0) === null,
                itemMissNull: doc.all(999) === null,
                itemMethodNull: doc.all.item(0) === null,
                namedMethodNull: doc.all.namedItem("missing") === null
              });
            })()
            "#,
        )
        .expect("plain detached Document probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownAll":false,"protoGetter":"function","allType":"undefined","loose":true,"strict":false,"bool":false,"string":"[object HTMLAllCollection]","tag":"[object HTMLAllCollection]","length":0,"noArgNull":true,"itemZeroNull":true,"itemMissNull":true,"itemMethodNull":true,"namedMethodNull":true}"#
    );
}
#[test]
fn assigning_top_does_not_replace_legacy_unforgeable_top_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        top = { hacked: true };
        "#,
        None,
    )
    .expect("assigning top should not throw");

    let result = vm
        .eval(
            r#"
            [
                top === globalThis,
                top.hacked === true,
                Object.getOwnPropertyDescriptor(globalThis, "top")?.configurable === false
            ].join("|")
            "#,
        )
        .expect("top alias probe should evaluate");

    assert_eq!(result, "true|false|true");
}
#[test]
fn top_level_lexical_bindings_can_shadow_replaceable_window_alias_names() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        const self = "self-shadow";
        const parent = "parent-shadow";
        const frames = "frames-shadow";
        globalThis.__aliasShadowProbe = [self, parent, frames].join("|");
        "#,
        None,
    )
    .expect("top-level lexical declarations should not conflict with replaceable aliases");

    let result = vm
        .eval("globalThis.__aliasShadowProbe")
        .expect("shadow probe should evaluate");

    assert_eq!(result, "self-shadow|parent-shadow|frames-shadow");
}
#[test]
fn top_level_var_parent_can_replace_global_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        var parent = document.createElement("div");
        parent.id = "shadow-parent";
        globalThis.__varParentProbe = [parent.id, typeof parent.appendChild].join("|");
        "#,
        None,
    )
    .expect("top-level var parent should replace the global alias");

    let result = vm
        .eval("globalThis.__varParentProbe")
        .expect("var parent probe should evaluate");

    assert_eq!(result, "shadow-parent|function");
}
#[test]
fn document_named_item_does_not_shadow_legacy_unforgeable_document_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                if (!document.documentElement) {
                    const html = document.createElement("html");
                    document.appendChild(html);
                }
                if (!document.body) {
                    const body = document.createElement("body");
                    document.documentElement.appendChild(body);
                }
                const node = document.createElement("div");
                node.id = "document";
                document.body.appendChild(node);
                const desc = Object.getOwnPropertyDescriptor(globalThis, "document");
                return [
                    document === globalThis.document,
                    document.body === globalThis.document.body,
                    document.getElementById("document") === node,
                    desc && desc.configurable === false,
                    desc && desc.enumerable === true,
                    typeof (desc && desc.get),
                    typeof (desc && desc.set)
                ].join("|");
            })()
            "#,
        )
        .expect("document alias should not be shadowed by named items");

    assert_eq!(result, "true|true|true|true|true|function|undefined");
}

#[test]
fn window_named_properties_use_the_chromium_prototype_layer() {
    let mut vm = new_storage_test_vm("https://window-named-properties.test/");

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
                const named = document.createElement("div");
                named.id = "namedProbe";
                document.body.appendChild(named);
                const namedProperties = Object.getPrototypeOf(Window.prototype);
                return [
                    Object.getPrototypeOf(globalThis) === Window.prototype,
                    Object.getPrototypeOf(namedProperties) === EventTarget.prototype,
                    namedProperties.constructor === EventTarget,
                    Object.getOwnPropertyNames(namedProperties).length,
                    window.namedProbe === named,
                    Object.prototype.hasOwnProperty.call(window, "namedProbe"),
                    Object.getOwnPropertyDescriptor(window, "namedProbe") === undefined,
                    Object.prototype.hasOwnProperty.call(namedProperties, "namedProbe"),
                    Object.getOwnPropertyDescriptor(namedProperties, "namedProbe").value === named,
                    document === globalThis.document
                ].join("|");
            })()
            "#,
        )
        .expect("Window named properties prototype probe should evaluate");

    assert_eq!(result, "true|true|true|0|true|false|true|true|true|true");
}

#[test]
fn window_named_properties_respect_later_prototype_properties_and_descriptor_flags() {
    let mut vm = new_storage_test_vm("https://window-named-properties.test/");

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
                for (const id of ["visibleNamed", "eventTargetShadow", "objectShadow"]) {
                    const element = document.createElement("span");
                    element.id = id;
                    document.body.appendChild(element);
                }
                EventTarget.prototype.eventTargetShadow = "event-target";
                Object.prototype.objectShadow = "object";

                const namedProperties = Object.getPrototypeOf(Window.prototype);
                const descriptor = Object.getOwnPropertyDescriptor(
                    namedProperties,
                    "visibleNamed"
                );
                return JSON.stringify({
                    visibleValue: namedProperties.visibleNamed.id,
                    visibleOwn: Object.hasOwn(namedProperties, "visibleNamed"),
                    descriptorWritable: descriptor.writable,
                    descriptorEnumerable: descriptor.enumerable,
                    descriptorConfigurable: descriptor.configurable,
                    eventTargetValue: window.eventTargetShadow,
                    eventTargetOwn: Object.hasOwn(namedProperties, "eventTargetShadow"),
                    objectValue: window.objectShadow,
                    objectOwn: Object.hasOwn(namedProperties, "objectShadow")
                });
            })()
            "#,
        )
        .expect("Window named property prototype visibility probe should evaluate");

    assert_eq!(
        result,
        r#"{"visibleValue":"visibleNamed","visibleOwn":true,"descriptorWritable":true,"descriptorEnumerable":false,"descriptorConfigurable":true,"eventTargetValue":"event-target","eventTargetOwn":false,"objectValue":"object","objectOwn":false}"#
    );
}

#[test]
fn window_internal_child_context_identity_is_not_read_from_web_properties() {
    let mut vm = new_storage_test_vm("https://window-private-identity.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const original = document;
                Object.defineProperty(window, "__moliChildBrowsingContextHandle", {
                    configurable: true,
                    value: 123456
                });
                return document === original && window.__moliChildBrowsingContextHandle === 123456;
            })()
            "#,
        )
        .expect("private child context identity probe should evaluate");

    assert_eq!(result, "true");
}

#[test]
fn assigning_document_does_not_replace_legacy_unforgeable_document_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        document = { hacked: true };
        "#,
        None,
    )
    .expect("assigning document should not throw");

    let result = vm
        .eval(
            r#"
            [
                document === globalThis.document,
                typeof document.createElement,
                document.hacked === true
            ].join("|")
            "#,
        )
        .expect("document alias reassignment probe should evaluate");

    assert_eq!(result, "true|function|false");
}
#[test]
fn iframe_id_named_window_property_returns_element_not_child_window() {
    let mut vm = new_storage_test_vm("https://iframe-id-named-property.test/");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.id = 'i';
(document.body || document.documentElement || document).appendChild(frame);
[
  Object.prototype.toString.call(i),
  i instanceof HTMLIFrameElement,
  typeof i.contentWindow,
  typeof i.contentWindow.navigation
].join('|')
"#,
        )
        .expect("iframe id named property should expose the element wrapper");

    assert_eq!(result, "[object HTMLIFrameElement]|true|object|object");
}
#[test]
fn iframe_name_named_window_property_returns_child_window() {
    let mut vm = new_storage_test_vm("https://iframe-name-named-property.test/");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.id = 'frame';
frame.name = 'target';
(document.body || document.documentElement || document).appendChild(frame);
[
  Object.prototype.toString.call(target),
  target === document.getElementById('frame').contentWindow,
  typeof target.navigation
].join('|')
"#,
        )
        .expect("iframe name named property should expose the child window");

    assert_eq!(result, "[object Window]|true|object");
}
#[test]
fn iframe_in_shadow_tree_is_not_a_named_window_property() {
    let mut vm = new_storage_test_vm("https://shadow-iframe-named-property.test/");

    let result = vm
        .eval(
            r#"
const host = document.createElement('div');
(document.body || document.documentElement || document).appendChild(host);
const shadow = host.attachShadow({ mode: 'open' });
const shadowFrame = document.createElement('iframe');
shadowFrame.name = 'shadowTarget';
shadow.appendChild(shadowFrame);
const lightFrame = document.createElement('iframe');
lightFrame.name = 'lightTarget';
(document.body || document.documentElement || document).appendChild(lightFrame);
[
  'shadowTarget' in window,
  window.shadowTarget === undefined,
  shadowFrame.contentWindow !== null,
  'lightTarget' in window,
  window.lightTarget === lightFrame.contentWindow
].join('|')
"#,
        )
        .expect("shadow iframe named property probe should evaluate");

    assert_eq!(result, "false|true|true|true|true");
}
#[test]
fn child_webassembly_constructors_use_newtarget_child_realm_default_prototype() {
    let mut vm = new_storage_test_vm("https://child-wasm-newtarget.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constructorFrame = document.createElement("iframe");
  const newTargetFrame = document.createElement("iframe");
  const otherFrame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(constructorFrame);
  (document.body || document.documentElement || document).appendChild(newTargetFrame);
  (document.body || document.documentElement || document).appendChild(otherFrame);
  const constructorRealm = constructorFrame.contentWindow;
  const newTargetRealm = newTargetFrame.contentWindow;
  const otherRealm = otherFrame.contentWindow;
  const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
  const bindOther = otherRealm.Function.prototype.bind;
  const ProxyOther = otherRealm.Proxy;

  function freshNewTarget(prototype) {
    const newTarget = new newTargetRealm.Function();
    newTarget.prototype = prototype;
    return newTarget;
  }

  function usesNewTargetDefaultPrototype(interfaceName, argument, newTarget) {
    const Constructor = constructorRealm.WebAssembly[interfaceName];
    const object = Reflect.construct(Constructor, [argument], newTarget);
    const ExpectedConstructor = newTargetRealm.WebAssembly[interfaceName];
    return (
      Object.getPrototypeOf(object) === ExpectedConstructor.prototype &&
      object instanceof ExpectedConstructor
    );
  }

  function usesTopRealmDefaultPrototype() {
    function TopNewTarget() {}
    TopNewTarget.prototype = undefined;
    const object = Reflect.construct(
      constructorRealm.WebAssembly.Module,
      [bytes],
      TopNewTarget
    );
    return (
      Object.getPrototypeOf(object) === WebAssembly.Module.prototype &&
      object instanceof WebAssembly.Module
    );
  }

  function ignoresReplacedRealmNamespace() {
    const newTarget = freshNewTarget(undefined);
    const namespace = newTargetRealm.WebAssembly;
    const intrinsicPrototype = namespace.Module.prototype;
    newTargetRealm.WebAssembly = { Module: { prototype: {} } };
    try {
      const object = Reflect.construct(
        constructorRealm.WebAssembly.Module,
        [bytes],
        newTarget
      );
      return Object.getPrototypeOf(object) === intrinsicPrototype;
    } finally {
      newTargetRealm.WebAssembly = namespace;
    }
  }

  return JSON.stringify({
    namespaceIsSeparate:
      constructorRealm.WebAssembly !== WebAssembly &&
      newTargetRealm.WebAssembly !== WebAssembly,
    constructorFunctionPrototype:
      Object.getPrototypeOf(constructorRealm.WebAssembly.Module) ===
      constructorRealm.Function.prototype,
    moduleDirect: usesNewTargetDefaultPrototype("Module", bytes, freshNewTarget(undefined)),
    moduleBound: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      bindOther.call(freshNewTarget(null))
    ),
    moduleProxy: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      new ProxyOther(freshNewTarget(true), {})
    ),
    moduleProxyBound: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      new ProxyOther(bindOther.call(freshNewTarget(NaN)), {})
    ),
    memoryProxy: usesNewTargetDefaultPrototype(
      "Memory",
      { initial: 0 },
      new ProxyOther(freshNewTarget(false), {})
    ),
    compileErrorDirect: usesNewTargetDefaultPrototype(
      "CompileError",
      undefined,
      freshNewTarget(undefined)
    ),
    moduleTopRealm: usesTopRealmDefaultPrototype(),
    moduleIntrinsicSurvivesNamespaceReplacement: ignoresReplacedRealmNamespace()
  });
})()
"#,
        )
        .expect("child WebAssembly NewTarget realm regression should evaluate");

    assert_eq!(
        result,
        r#"{"namespaceIsSeparate":true,"constructorFunctionPrototype":true,"moduleDirect":true,"moduleBound":true,"moduleProxy":true,"moduleProxyBound":true,"memoryProxy":true,"compileErrorDirect":true,"moduleTopRealm":true,"moduleIntrinsicSurvivesNamespaceReplacement":true}"#
    );
}
#[test]
fn detached_iframe_navigation_entry_properties_are_invalidated() {
    let mut vm = new_storage_test_vm("https://detached-navigation-entry.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
const entry = frame.contentWindow.navigation.currentEntry;
const before = [
  entry.sameDocument,
  entry.url !== null,
  entry.key !== '',
  entry.id !== '',
  entry.index
].join(',');
frame.remove();
const after = [
  entry.sameDocument,
  entry.url === null,
  entry.key,
  entry.id,
  entry.index
].join(',');
`${before}|${after}`
"#,
        )
        .expect("detached iframe navigation entry should evaluate");

    assert_eq!(result, "true,true,true,true,0|false,true,,,-1");
}

#[test]
fn navigation_runtime_state_ignores_proto_pollution_slots() {
    let mut vm = new_storage_test_vm("https://navigation-slot-pollution.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    const setter = descriptor?.set;
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof setter,
      setter ? setter.name : "none",
      setter ? setter.length : "none",
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(":");
  };
  const historyDescriptors = [
    accessorDescriptor(History.prototype, "length"),
    accessorDescriptor(History.prototype, "state"),
    accessorDescriptor(History.prototype, "scrollRestoration")
  ];
  const navigationDescriptors = [
    accessorDescriptor(Navigation.prototype, "canGoBack"),
    accessorDescriptor(Navigation.prototype, "canGoForward"),
    accessorDescriptor(Navigation.prototype, "currentEntry"),
    accessorDescriptor(Navigation.prototype, "activation"),
    accessorDescriptor(Navigation.prototype, "transition")
  ];
  History.prototype.__lmHistoryLength = 99;
  History.prototype.__lmHistoryState = { polluted: true };
  History.prototype.__lmHistoryScrollRestoration = "manual";
  Navigation.prototype.__lmNavigationCurrentEntry = null;
  NavigationHistoryEntry.prototype.__lmNavigationEntryInitialIndex = 41;
  NavigationHistoryEntry.prototype.__lmNavigationEntryUrl = "https://proto.invalid/#bad";
  NavigationHistoryEntry.prototype.__lmNavigationEntryStateSnapshot = { value: "proto-state" };
  const internalHistoryNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name === "__moliWindowRuntimeOwner" ||
      name.startsWith("__lmHistory"))
    .sort();
  const internalNavigationNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name === "__moliWindowRuntimeOwner" ||
      name.startsWith("__lmNavigation"))
    .sort();
  const internalEntryNames = entry => Object.getOwnPropertyNames(entry)
    .filter(name => name === "state" ||
      name === "__moliWindowRuntimeOwner" ||
      name.startsWith("__lmNavigationEntry") ||
      name.startsWith("__lmHistoryEntry"))
    .sort();
  const historyOwnSlots = internalHistoryNames(history);
  const navigationOwnSlots = internalNavigationNames(navigation);
  const windowHolderOwnSlots = Object.getOwnPropertyNames(window)
    .filter(name => name === "__moliWindowHistory" ||
      name === "__moliWindowNavigation")
    .sort();
  Object.defineProperties(window, {
    __moliWindowHistory: {
      value: { length: 99 },
      configurable: true
    },
    __moliWindowNavigation: {
      value: { currentEntry: null },
      configurable: true
    }
  });
  Object.defineProperties(history, {
    __lmHistoryLength: { value: 99, configurable: true },
    __lmHistoryState: { value: { polluted: true }, configurable: true },
    __lmHistoryScrollRestoration: { value: "manual", configurable: true },
    __lmHistoryEntries: { value: [], configurable: true },
    __lmHistoryIndex: { value: 41, configurable: true },
    __moliWindowRuntimeOwner: { value: null, configurable: true }
  });
  Object.defineProperties(navigation, {
    __lmNavigationCurrentEntry: { value: null, configurable: true },
    __lmNavigationActivation: { value: { polluted: true }, configurable: true },
    __lmNavigationTransition: { value: { polluted: true }, configurable: true },
    __moliWindowRuntimeOwner: { value: null, configurable: true }
  });
  const first = navigation.currentEntry;
  navigation.updateCurrentEntry({ state: { value: "real-before" } });
  const firstOwnSlots = internalEntryNames(first);
  Object.defineProperties(first, {
    __lmNavigationEntryInitialIndex: { value: 41, configurable: true },
    __lmNavigationEntryUrl: { value: "https://own.invalid/#bad", configurable: true },
    __lmNavigationEntryId: { value: "own-id", configurable: true },
    __lmNavigationEntryKey: { value: "own-key", configurable: true },
    __lmNavigationEntryStateSnapshot: { value: { value: "own-state" }, configurable: true },
    __lmHistoryEntryStateSnapshot: { value: { step: 99 }, configurable: true },
    state: { value: { value: "own-exposed-state" }, configurable: true }
  });
  const before = {
    length: history.length,
    windowHistoryStable: window.history === history,
    windowNavigationStable: window.navigation === navigation,
    windowHolderSpoofed: Object.getOwnPropertyNames(window)
      .filter(name => name === "__moliWindowHistory" ||
        name === "__moliWindowNavigation")
      .sort()
      .join(","),
    historyStateNull: history.state === null,
    scrollRestoration: history.scrollRestoration,
    currentEntryPresent: navigation.currentEntry !== null,
    transitionNull: navigation.transition === null,
    index: first.index,
    hash: new URL(first.url).hash,
    keySpoofed: first.key === "own-key",
    idSpoofed: first.id === "own-id",
    state: first.getState().value
  };
  history.pushState({ step: 1 }, "", "#one");
  const current = navigation.currentEntry;
  navigation.updateCurrentEntry({ state: { value: "real-after" } });
  const currentOwnSlots = internalEntryNames(current);
  Object.defineProperties(current, {
    __lmNavigationEntryInitialIndex: { value: 41, configurable: true },
    __lmNavigationEntryUrl: { value: "https://own.invalid/#bad", configurable: true },
    __lmNavigationEntryId: { value: "own-current-id", configurable: true },
    __lmNavigationEntryKey: { value: "own-current-key", configurable: true },
    __lmNavigationEntryStateSnapshot: { value: { value: "own-current-state" }, configurable: true },
    state: { value: { value: "own-current-exposed-state" }, configurable: true }
  });
  const after = {
    length: history.length,
    historyStateStep: history.state.step,
    scrollRestoration: history.scrollRestoration,
    currentEntryPresent: navigation.currentEntry !== null,
    transitionNull: navigation.transition === null,
    index: current.index,
    indexes: navigation.entries().map(entry => entry.index).join(","),
    hash: new URL(current.url).hash,
    keySpoofed: current.key === "own-current-key",
    idSpoofed: current.id === "own-current-id",
    state: current.getState().value
  };
  return JSON.stringify({
    historyDescriptors,
    navigationDescriptors,
    historyOwnSlots,
    navigationOwnSlots,
    windowHolderOwnSlots,
    firstOwnSlots,
    currentOwnSlots,
    before,
    after
  });
})()
"##,
        )
        .expect("navigation runtime state should ignore prototype slots");

    assert_eq!(
        result,
        r##"{"historyDescriptors":["length:function:get length:0:undefined:none:none:true:true","state:function:get state:0:undefined:none:none:true:true","scrollRestoration:function:get scrollRestoration:0:function:set scrollRestoration:1:true:true"],"navigationDescriptors":["canGoBack:function:get canGoBack:0:undefined:none:none:true:true","canGoForward:function:get canGoForward:0:undefined:none:none:true:true","currentEntry:function:get currentEntry:0:undefined:none:none:true:true","activation:function:get activation:0:undefined:none:none:true:true","transition:function:get transition:0:undefined:none:none:true:true"],"historyOwnSlots":[],"navigationOwnSlots":[],"windowHolderOwnSlots":[],"firstOwnSlots":[],"currentOwnSlots":[],"before":{"length":1,"windowHistoryStable":true,"windowNavigationStable":true,"windowHolderSpoofed":"__moliWindowHistory,__moliWindowNavigation","historyStateNull":true,"scrollRestoration":"auto","currentEntryPresent":true,"transitionNull":true,"index":0,"hash":"","keySpoofed":false,"idSpoofed":false,"state":"real-before"},"after":{"length":2,"historyStateStep":1,"scrollRestoration":"auto","currentEntryPresent":true,"transitionNull":true,"index":1,"indexes":"0,1","hash":"#one","keySpoofed":false,"idSpoofed":false,"state":"real-after"}}"##
    );
}

#[test]
fn targeted_anchor_click_dispatches_navigate_on_named_child_window() {
    let mut vm = new_storage_test_vm("https://targeted-child-navigate.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.name = 'target';
const root = document.body || document.documentElement || document;
root.appendChild(frame);
const link = document.createElement('a');
link.href = '/next.html';
link.target = 'target';
root.appendChild(link);
let seen = [];
frame.contentWindow.navigation.onnavigate = e => {
  seen.push([
    e.navigationType,
    e.cancelable,
    e.canIntercept,
    e.userInitiated,
    e.hashChange,
    e.formData === null,
    e.destination.url,
    e.destination.sameDocument,
    e.destination.key,
    e.destination.id,
    e.destination.index,
    e.sourceElement === link
  ].join(','));
};
link.click();
seen.join('|')
"#,
        )
        .expect("targeted anchor click should dispatch child navigate");

    assert_eq!(
        result,
        "push,true,true,false,false,true,https://targeted-child-navigate.test/next.html,false,,,-1,true"
    );
}
#[test]
fn targeted_anchor_click_reports_same_document_hash_change_for_child_window() {
    let mut vm = new_storage_test_vm("https://targeted-child-hash.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.name = 'target';
const root = document.body || document.documentElement || document;
root.appendChild(frame);
frame.contentWindow.history.pushState(null, '', '/child.html');
const link = document.createElement('a');
link.href = '/child.html#next';
link.target = 'target';
root.appendChild(link);
let seen = [];
frame.contentWindow.navigation.onnavigate = e => {
  seen.push([
    e.hashChange,
    e.destination.sameDocument,
    e.destination.url,
    e.destination.index
  ].join(','));
};
link.click();
seen.join('|')
"#,
        )
        .expect("targeted same-document anchor click should dispatch child navigate");

    assert_eq!(
        result,
        "true,true,https://targeted-child-hash.test/child.html#next,-1"
    );
}
#[test]
fn anchor_click_to_identical_url_dispatches_replace_navigate_event() {
    let mut vm = new_storage_test_vm("https://anchor-same-url.test/page.html");

    let result = vm
        .eval(
            r#"
const root = document.body || document.documentElement || document;
const link = document.createElement('a');
link.href = '/page.html';
root.appendChild(link);
let seen = [];
navigation.onnavigate = e => {
  seen.push([
    e.navigationType,
    e.destination.url,
    e.sourceElement === link
  ].join(','));
  e.intercept({ handler: () => {} });
};
link.click();
seen.join('|')
"#,
        )
        .expect("same-url anchor click should dispatch navigate");

    assert_eq!(
        result,
        "replace,https://anchor-same-url.test/page.html,true"
    );
}
#[test]
fn same_document_anchor_click_updates_navigation_current_entry() {
    let mut vm = new_storage_test_vm("https://same-document-anchor.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const link = document.createElement('a');
  link.href = '#section';
  (document.body || document.documentElement || document).appendChild(link);
  const startIndex = navigation.currentEntry.index;
  let seen = null;
  navigation.oncurrententrychange = e => {
    seen = [
      e.navigationType,
      e.from === navigation.entries()[startIndex],
      e.from.index,
      navigation.currentEntry.index,
      location.hash
    ].join('|');
  };
  link.click();
  return seen;
})()
"##,
        )
        .expect("same-document anchor click should update navigation current entry");

    assert_eq!(result, "push|true|0|1|#section");
}
#[test]
fn replace_state_detaches_previous_navigation_entry_index() {
    let mut vm = new_storage_test_vm("https://replace-entry-index.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const original = navigation.currentEntry;
  let seen = null;
  navigation.oncurrententrychange = e => {
    seen = [
      e.from === original,
      e.from.index,
      navigation.currentEntry.index,
      navigation.entries().includes(original)
    ].join('|');
  };
  history.replaceState(null, '', '#replaced');
  return seen;
})()
"##,
        )
        .expect("replaceState should detach the previous navigation entry index");

    assert_eq!(result, "true|-1|0|false");
}
#[test]
fn document_open_with_three_arguments_uses_associated_window() {
    let mut vm = new_storage_test_vm("https://document-open-window.test/");

    let result = vm
        .eval(
            r#"
(() => {
  window.open = function() { throw new Error('shadowed open should not run'); };
  const live = document.open('/popup', '', '');
  const detached = new DOMParser().parseFromString('', 'text/html');
  let detachedError = '';
  try {
    detached.open('/popup', '', '');
  } catch (error) {
    detachedError = error.name + ':' + error.code;
  }
  return [
    live === window,
    live instanceof live.Window,
    detached.defaultView === null,
    detachedError
  ].join('|');
})()
"#,
        )
        .expect("three-argument document.open should evaluate");

    assert_eq!(result, "true|true|true|InvalidAccessError:15");
}
#[tokio::test]
async fn iframe_javascript_url_string_completion_replaces_child_document() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__javascriptUrlStringCompletionLoadCount = 0;
  const frame = document.createElement('iframe');
  frame.src = "javascript:document.contentType";
  frame.onload = () => {
    globalThis.__javascriptUrlStringCompletionLoadCount++;
  };
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("iframe javascript URL setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not commit the pending javascript URL navigation"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should enqueue javascript URL execution"
    );
    let before_execution_loads = vm
        .eval("String(globalThis.__javascriptUrlStringCompletionLoadCount)")
        .expect("javascript URL pre-execution load count should evaluate");
    assert_eq!(before_execution_loads, "0");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL should execute on DocumentScriptReady",
    )
    .await;
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("javascript URL replacement should run {transition} before HostLoad"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("typeof document.querySelector('iframe').onload")
            .expect("replacement load handler retention should evaluate"),
        "function",
        "navigation must retain the handler registered on the parent-owned frame element"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "javascript URL string-completion replacement should load on HostLoad"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
    return [
    doc !== null,
    doc && doc.contentType,
    doc && doc.documentElement.textContent,
    doc && doc.URL.startsWith('javascript:'),
    globalThis.__javascriptUrlStringCompletionLoadCount
  ].join('|');
})()
"#,
        )
        .expect("iframe javascript URL document should evaluate");

    assert_eq!(result, "true|text/html|text/html|false|1");
}

#[tokio::test]
async fn iframe_javascript_url_non_string_completion_does_not_replace_child_document() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-non-string.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__javascriptUrlNonStringLoadCount = 0;
  const frame = document.createElement('iframe');
  frame.src = "javascript:42";
  frame.onload = () => {
    globalThis.__javascriptUrlNonStringLoadCount++;
  };
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("iframe javascript URL non-string setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not commit the pending javascript URL navigation"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should enqueue javascript URL execution"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL should execute on DocumentScriptReady",
    )
    .await;
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "non-string javascript URL completion should expose the already-complete initial document load"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
  return [
    doc !== null,
    doc && doc.contentType,
    doc && doc.documentElement.textContent,
    doc && doc.URL.startsWith('javascript:'),
    globalThis.__javascriptUrlNonStringLoadCount
  ].join('|');
})()
"#,
        )
        .expect("iframe javascript URL non-string document should evaluate");

    assert_eq!(result, "true|text/html||false|1");
}

#[tokio::test]
async fn iframe_javascript_url_exception_does_not_replace_child_document_or_load() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-exception.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__javascriptUrlExceptionLoadCount = 0;
  const frame = document.createElement('iframe');
  frame.src = "javascript:throw new Error('boom')";
  frame.onload = () => {
    globalThis.__javascriptUrlExceptionLoadCount++;
  };
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("iframe javascript URL exception setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not commit the pending javascript URL navigation"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should enqueue javascript URL execution"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL exception should execute on DocumentScriptReady after realm materialization",
    )
    .await;
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "failed javascript URL execution should not synthesize a load"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
  return [
    doc !== null,
    doc && doc.contentType,
    doc && doc.documentElement.textContent,
    doc && doc.URL.startsWith('javascript:'),
    globalThis.__javascriptUrlExceptionLoadCount
  ].join('|');
})()
"#,
        )
        .expect("iframe javascript URL exception document should evaluate");

    assert_eq!(result, "true|text/html||false|0");
}

#[tokio::test]
async fn stale_iframe_javascript_url_work_cannot_finish_newer_navigation() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-stale.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'stale-javascript-url-frame';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("stale javascript URL setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "stale javascript URL setup should initialize the child document",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "stale javascript URL initial child document",
    );
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;

    vm.eval_in_child_default_context(
        child_context_id,
        "location.href = 'javascript:globalThis.__staleJavascriptUrlRan = true'",
    )
    .expect("first javascript URL navigation should queue");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "first NavigationCommit should enqueue javascript URL work"
    );

    vm.eval_in_child_default_context(
        child_context_id,
        "location.href = 'javascript:globalThis.__freshJavascriptUrlRan = true'",
    )
    .expect("replacement javascript URL navigation should queue");
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "the stale javascript URL work should be consumed as a typed stale task"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "String(globalThis.__staleJavascriptUrlRan)",
        )
        .expect("stale javascript URL execution state should be readable"),
        "undefined",
        "stale javascript URL work must not execute in the still-current realm"
    );
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(child_handle)
        .expect("replacement navigation must remain pending after stale work is dropped");
    assert!(
        matches!(
            pending,
            crate::native_bridge::ChildBrowsingContextBootstrap::Url(ref url)
                if url.as_str().contains("__freshJavascriptUrlRan")
        ),
        "stale work must not clear the replacement navigation"
    );

    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "replacement NavigationCommit should remain runnable"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "replacement javascript URL should execute on its own ready turn"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "JSON.stringify([globalThis.__staleJavascriptUrlRan, globalThis.__freshJavascriptUrlRan])",
        )
        .expect("replacement javascript URL execution state should be readable"),
        "[null,true]"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "a subsequent non-string javascript URL navigation must not synthesize iframe load"
    );
}

#[tokio::test]
async fn no_src_iframe_initial_about_blank_has_no_navigation_activation_after_load() {
    let mut vm = new_storage_test_vm("https://iframe-initial-activation.test/");

    vm.eval(
        r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
"#,
    )
    .expect("no-src iframe setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "no-src iframe initial about:blank should finish synchronously"
    );

    let result = vm
        .eval(
            r#"
(() => {
const frame = document.querySelector('iframe');
return [
  frame.contentWindow.location.href,
  frame.contentWindow.navigation.entries().length,
  frame.contentWindow.navigation.currentEntry.url,
  frame.contentWindow.navigation.activation === null
].join('|')
})()
"#,
        )
        .expect("no-src iframe navigation activation should evaluate");

    assert_eq!(result, "about:blank|1|about:blank|true");
}

#[test]
fn no_src_iframe_initial_about_blank_load_is_synchronous_at_connection() {
    let mut vm = new_storage_test_vm("https://iframe-initial-load-timing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  globalThis.initialBlankLoadEvents = events;
  queueMicrotask(() => events.push("microtask"));
  delete globalThis.Event;
  const frame = document.createElement("iframe");
  document.addEventListener("load", event => {
    if (event.target === frame) {
      events.push("capture");
    }
  }, true);
  frame.onload = () => events.push("handler");
  events.push("before");
  (document.body || document.documentElement || document).appendChild(frame);
  events.push("after");

  const late = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(late);
  late.onload = () => events.push("late");
  return events.join("|");
})()
"#,
        )
        .expect("initial about:blank load timing should evaluate");

    assert_eq!(result, "before|capture|handler|after");
    let settled = vm
        .eval("globalThis.initialBlankLoadEvents.join('|')")
        .expect("initial about:blank microtask order should evaluate");
    assert_eq!(settled, "before|capture|handler|after|microtask");
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "synchronous initial about:blank delivery must not leave HostLoad work"
    );
}

#[tokio::test]
async fn top_history_back_routes_to_child_joint_history_entry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://joint-child-back.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child joint-history frame setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  frame.contentWindow.history.pushState({ child: true }, '', '#child');
  return [
    location.href,
    history.length,
    frame.contentWindow.history.length,
    frame.contentWindow.location.href
  ].join('|');
})()
"#,
        )
        .expect("child joint-history setup should evaluate");

    assert_eq!(
        setup,
        "https://joint-child-back.test/page.html|3|3|https://joint-child-back.test/page.html#child"
    );
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await;

    vm.eval("history.back(); 'queued'")
        .expect("top history back should queue traversal");

    let mut result = String::new();
    for _ in 0..4 {
        assert!(
            vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .expect("wait driver should advance joint-history traversal")
        );
        result = vm
            .eval(
                r#"
(() => {
  const frame = document.querySelector('iframe');
  return [
    location.href,
    frame.contentWindow.location.href,
    frame.contentWindow.navigation.currentEntry.url,
    frame.contentWindow.navigation.currentEntry.index
  ].join('|');
})()
"#,
            )
            .expect("joint-history traversal result should evaluate");
        if result == "https://joint-child-back.test/page.html|about:srcdoc|about:srcdoc|0" {
            break;
        }
    }

    assert_eq!(
        result,
        "https://joint-child-back.test/page.html|about:srcdoc|about:srcdoc|0"
    );
}

#[tokio::test]
async fn top_history_back_ignores_removed_child_joint_history_entry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://removed-child-joint-back.test/page.html", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("removed child joint-history frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  frame.contentWindow.history.pushState({ child: true }, '', '#child');
  return [
    location.href,
    navigation.entries().length,
    navigation.currentEntry.index,
    history.length,
    frame.contentWindow.history.length,
    frame.contentWindow.location.href
  ].join('|');
})()
"#,
        )
        .expect("removed child joint-history setup should evaluate");

    assert_eq!(
        setup,
        "https://removed-child-joint-back.test/page.html|1|0|3|3|https://removed-child-joint-back.test/page.html#child"
    );

    vm.eval("document.querySelector('iframe').remove(); history.back(); 'queued'")
        .expect("top history back after child removal should queue no traversal");

    let result = vm
        .eval(
            r#"
(() => [
  location.href,
  navigation.entries().length,
  navigation.currentEntry.index,
  history.length,
  document.querySelector('iframe') === null
].join('|'))()
"#,
        )
        .expect("top history back after child removal should remain a no-op");

    assert_eq!(
        result,
        "https://removed-child-joint-back.test/page.html|1|0|3|true"
    );
}

#[tokio::test]
async fn child_cross_document_pending_navigation_exposes_back_availability() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://child-cross-doc-back.test/page.html", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("frame setup");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval("document.querySelector('iframe').contentWindow.navigation.navigate('?foo'); 'queued'")
        .expect("child navigate");
    vm.drain_pending_child_frame_work_for_test();

    let state = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const n = frame.contentWindow.navigation;
  return [
    frame.contentWindow.location.href,
    frame.contentWindow.history.length,
    n.entries().map(e => `${e.index}:${e.url}`).join(','),
    n.currentEntry && `${n.currentEntry.index}:${n.currentEntry.url}`,
    n.canGoBack,
    n.canGoForward
  ].join('|');
})()
"#,
        )
        .expect("child cross-document navigation state should evaluate");

    assert_eq!(
        state,
        "about:srcdoc|2|0:about:srcdoc|0:about:srcdoc|true|false"
    );
}
#[tokio::test]
async fn detached_child_navigation_error_exposes_committed_entry_during_dispatch() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-detach-navigation-error.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child navigation-error frame setup should evaluate");
    for _ in 0..128 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child navigation-error setup should use the selected-task dispatcher")
        {
            break;
        }
    }

    let initial = vm
        .eval(
            r##"
(() => {
  globalThis.__lmDetachedChildNavigateErrorLog = [];
  const child = document.querySelector('iframe').contentWindow;
  child.history.pushState({ child: true }, "", "#one");
  return [
    child.navigation.entries().length,
    child.navigation.currentEntry.index,
    child.location.href
  ].join("|");
})()
"##,
        )
        .expect("child initial same-document navigation should evaluate");
    assert_eq!(
        initial,
        "2|1|https://child-detach-navigation-error.test/page.html#one"
    );

    let setup = vm
        .eval(
            r##"
(() => {
  const frame = document.querySelector('iframe');
  const child = frame.contentWindow;
  globalThis.__lmDetachedChildNavigateErrorLog.length = 0;
  const target = child.navigation.entries()[0];
  child.navigation.onnavigate = event => {
    event.intercept({
      handler() {
        setTimeout(() => frame.remove(), 0);
        return new Promise(resolve => setTimeout(resolve, 1));
      }
    });
  };
  child.navigation.onnavigatesuccess = () => {
    globalThis.__lmDetachedChildNavigateErrorLog.push("success");
  };
  child.navigation.onnavigateerror = event => {
    const current = child.navigation.currentEntry;
    globalThis.__lmDetachedChildNavigateErrorLog.push([
      "error",
      current === target,
      current instanceof child.NavigationHistoryEntry,
      event.error && event.error.name
    ].join(":"));
  };
  const result = child.navigation.traverseTo(target.key);
  result.committed.then(entry => {
    globalThis.__lmDetachedChildNavigateErrorLog.push(`committed:${entry === target}`);
  }, error => {
    globalThis.__lmDetachedChildNavigateErrorLog.push(`committed-rejected:${error.name}`);
  });
  result.finished.then(() => {
    globalThis.__lmDetachedChildNavigateErrorLog.push("finished");
  }, error => {
    globalThis.__lmDetachedChildNavigateErrorLog.push(`finished-rejected:${error.name}`);
  });
  return "queued";
})()
"##,
        )
        .expect("child detach navigation-error setup should evaluate");
    assert_eq!(setup, "queued");

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("child traversal should run through the production history source")
    );
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child detach navigation-error timeouts should drain");
    let settled = vm
        .eval(
            r#"
(() => {
  return [
    globalThis.__lmDetachedChildNavigateErrorLog.join("|"),
    document.querySelector("iframe") === null
  ].join("||");
})()
"#,
        )
        .expect("child detach navigation-error log should evaluate");

    assert_eq!(
        settled,
        "committed:true|error:true:true:AbortError|finished-rejected:AbortError||true"
    );
}
#[tokio::test]
async fn child_meta_refresh_timer_is_canceled_when_frame_reloads() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__lmChildMetaRefreshLoads = [];
  const frame = document.createElement('iframe');
  frame.onload = () => {
    __lmChildMetaRefreshLoads.push(frame.contentDocument.body.textContent.trim());
  };
  frame.srcdoc = '<meta http-equiv="refresh" content="0"><p>first</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child meta refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let first_loads = vm
        .eval("globalThis.__lmChildMetaRefreshLoads.join('|')")
        .expect("child first meta refresh load log should evaluate");
    assert_eq!(first_loads, "first");

    vm.eval(
        r#"
(() => {
  document.querySelector('iframe').srcdoc = '<p>second</p>';
  return 'queued';
})()
"#,
    )
    .expect("child replacement srcdoc should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let replaced_loads = vm
        .eval("globalThis.__lmChildMetaRefreshLoads.join('|')")
        .expect("child replacement load log should evaluate");
    assert_eq!(replaced_loads, "first|second");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("stale child meta refresh timer should drain without firing");
    let final_loads = vm
        .eval("globalThis.__lmChildMetaRefreshLoads.join('|')")
        .expect("child meta refresh final load log should evaluate");
    assert_eq!(final_loads, "first|second");
}

#[tokio::test]
async fn child_meta_refresh_navigate_event_cancellation_prevents_reload() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__lmChildMetaRefreshLoadCount = 0;
  globalThis.__lmChildMetaRefreshNavigateEvents = [];
  const frame = document.createElement('iframe');
  frame.onload = () => {
    __lmChildMetaRefreshLoadCount += 1;
    frame.contentWindow.navigation.onnavigate = event => {
      __lmChildMetaRefreshNavigateEvents.push([
        event.navigationType,
        event.cancelable,
        event.canIntercept,
        event.destination.sameDocument
      ].join(':'));
      event.preventDefault();
    };
  };
  frame.srcdoc = '<meta http-equiv="refresh" content="0"><p>first</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child meta refresh cancellation setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("canceled child meta refresh timer should settle");
    vm.drain_pending_child_frame_work_for_test();

    let state = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  return [
    __lmChildMetaRefreshLoadCount,
    __lmChildMetaRefreshNavigateEvents.join('|'),
    frame.contentDocument.body.textContent.trim(),
    frame.contentWindow.location.href
  ].join('||');
})()
"#,
        )
        .expect("child meta refresh cancellation state should evaluate");
    assert_eq!(state, "1||reload:true:true:false||first||about:srcdoc");
}

#[tokio::test]
async fn child_meta_refresh_rejects_javascript_urls() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = `<script>globalThis.x = 1<\/script>
    <meta http-equiv="refresh" content="0;url=javascript:globalThis.x=2">`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child javascript refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child timer lane should settle");

    assert_eq!(
        vm.eval("String(document.querySelector('iframe').contentWindow.x)")
            .expect("child script state should remain observable"),
        "1",
        "declarative refresh must never execute a javascript: URL"
    );
}

#[tokio::test]
async fn child_sandbox_blocks_meta_refresh_when_it_is_created() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-same-origin');
  frame.srcdoc = '<meta http-equiv="refresh" content="0;url=#blocked"><p>source</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("sandboxed child refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.eval("document.querySelector('iframe').removeAttribute('sandbox'); 'removed'")
        .expect("sandbox removal should evaluate");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child timer lane should settle");

    assert_eq!(
        vm.eval("document.querySelector('iframe').contentWindow.location.href")
            .expect("child URL should remain observable"),
        "about:srcdoc",
        "removing the sandbox later must not revive a refresh rejected at creation time"
    );
}

#[tokio::test]
async fn child_meta_refresh_remains_scheduled_when_sandbox_is_added_later() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<meta http-equiv="refresh" content="0;url=#allowed"><p>source</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("unsandboxed child refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.eval(
        "document.querySelector('iframe').setAttribute('sandbox', 'allow-same-origin'); 'added'",
    )
    .expect("sandbox addition should evaluate");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("scheduled child refresh should run");

    assert_eq!(
        vm.eval("document.querySelector('iframe').contentWindow.location.href")
            .expect("child URL should remain observable"),
        "https://child-meta-refresh.test/#allowed",
        "the sandbox policy is checked when the refresh is created, not again when it becomes due"
    );
}

#[tokio::test]
async fn child_frame_load_dispatches_document_readiness_before_pageshow() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-lifecycle.test/page.html", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childLifecycleEvents = [];
  globalThis.childStarted = () => {
    const frame = document.getElementById("child");
    frame.contentDocument.addEventListener("DOMContentLoaded", () => {
      __childLifecycleEvents.push("domcontentloaded");
    });
    frame.contentDocument.onreadystatechange = () => {
      __childLifecycleEvents.push(`readystate${frame.contentDocument.readyState}`);
    };
    frame.onload = () => __childLifecycleEvents.push("frameload");
    frame.contentWindow.onpageshow = event => {
      __childLifecycleEvents.push(
        `pageshow:${event.persisted === false}:${'persisted' in event}`
      );
    };
  };
  const frame = document.createElement("iframe");
  frame.id = "child";
  frame.srcdoc = "<head><script>top.childStarted()</" + "script></head><body>child</body>";
  (document.body || document.documentElement || document).appendChild(frame);
  return "ready";
})()
"#,
    )
    .expect("frame setup");
    vm.drain_pending_child_frame_work_for_test();

    let events = vm
        .eval("globalThis.__childLifecycleEvents.join('|')")
        .expect("child lifecycle events should evaluate");

    assert_eq!(
        events,
        "readystateinteractive|domcontentloaded|readystatecomplete|frameload|pageshow:true:true"
    );
}

#[tokio::test]
async fn child_document_open_during_pageshow_does_not_redispatch_pageshow() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader(
        "https://child-document-open-pageshow.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childDocumentOpenPageshowEvents = [];
  globalThis.__replaceChildDuringPageshow = () => {
    __childDocumentOpenPageshowEvents.push("old-pageshow");
    const frame = document.getElementById("child");
    frame.contentDocument.open();
    frame.contentDocument.write(`
      <script>
        onpageshow = () => parent.__childDocumentOpenPageshowEvents.push("new-pageshow");
      <\/script>
      <p>replacement</p>`);
    frame.contentDocument.close();
  };
  const frame = document.createElement("iframe");
  frame.id = "child";
  frame.srcdoc = `
    <script>
      onpageshow = () => parent.__replaceChildDuringPageshow();
    <\/script>
    <p>original</p>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child document.open during pageshow setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("__childDocumentOpenPageshowEvents.join('|')")
            .expect("child document.open pageshow events should evaluate"),
        "old-pageshow"
    );
}

#[tokio::test]
async fn child_window_load_replacement_stops_old_delivery_before_owner_output() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-load-phases.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childLoadPhaseEvents = [];
  globalThis.__childLoadPhaseReplacement = `
    <script>
      addEventListener("load", () => parent.__childLoadPhaseEvents.push("new-window"));
      addEventListener("pageshow", () => parent.__childLoadPhaseEvents.push("new-pageshow"));
      addEventListener("pagehide", () => parent.__childLoadPhaseEvents.push("new-pagehide"));
      addEventListener("unload", () => parent.__childLoadPhaseEvents.push("new-unload"));
    <\/script>
    <body data-version="new">new</body>`;
  const frame = document.createElement("iframe");
  frame.id = "load-phase-frame";
  frame.name = "load-phase-frame-client";
  frame.onload = () => {
    __childLoadPhaseEvents.push(`frame:${frame.contentDocument.body.dataset.version}`);
  };
  frame.srcdoc = `
    <script>
      addEventListener("load", () => {
        window.__oldWindowLoadInvoked = true;
        parent.__childLoadPhaseEvents.push("old-window");
        parent.document.getElementById("load-phase-frame").srcdoc =
          parent.__childLoadPhaseReplacement;
      });
      addEventListener("pagehide", () => parent.__childLoadPhaseEvents.push("old-pagehide"));
      addEventListener("unload", () => parent.__childLoadPhaseEvents.push("old-unload"));
      addEventListener("pageshow", () => parent.__childLoadPhaseEvents.push("old-pageshow"));
    <\/script>
    <body data-version="old">old</body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child load-phase replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "old child load-phase srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "old child load handler should install",
    )
    .await;
    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("load-phase-frame")
        .expect("load-phase iframe should remain in the main document");
    {
        let context_host = vm._context_host.borrow();
        let load_callback_identities =
            context_host.child_window_event_callback_identities_for_test(child_handle, "load");
        assert_eq!(
            load_callback_identities.len(),
            1,
            "old Window load listener must be registered against the reused LocalWindow"
        );
        assert!(
            load_callback_identities
                .iter()
                .filter_map(|(relevant, _)| *relevant)
                .all(|identity| context_host
                    .window_execution_context_identity_is_current(identity)),
            "old Window load listener must retain a current callback relevant realm"
        );
    }
    for context in ["old interactive", "old DOMContentLoaded", "old complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "old Window load should begin exact-owner delivery",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(document.getElementById('load-phase-frame').contentWindow.__oldWindowLoadInvoked === true)"
        )
        .expect("old child Window load invocation marker should evaluate"),
        "true",
        "HostLoad must invoke the old Window listener before it observes replacement"
    );
    assert_eq!(
        vm.eval("__childLoadPhaseEvents.join('|')")
            .expect("old child load-phase trace should evaluate"),
        "old-window",
        "replacement in Window load must suppress old iframe load and pageshow"
    );
    assert_eq!(
        vm.completed_child_frame_navigation_load_count(),
        0,
        "stale old delivery must not publish frame/protocol completion"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "Window-load replacement should commit on its navigation source",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "Window-load replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement child load handlers should install",
    )
    .await;
    for context in [
        "replacement interactive",
        "replacement DOMContentLoaded",
        "replacement complete",
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "replacement load delivery should finish independently",
    )
    .await;

    assert_eq!(
        vm.eval("__childLoadPhaseEvents.join('|')")
            .expect("replacement child load-phase trace should evaluate"),
        "old-window|old-pagehide|old-unload|new-window|frame:new|new-pageshow",
        "started old load must unload once in pagehide-before-unload order before replacement"
    );
    let snapshots = vm.take_completed_child_frame_navigation_loads();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].name.as_deref(),
        Some("load-phase-frame-client")
    );
    assert_eq!(
        snapshots[0].url, "about:srcdoc",
        "srcdoc load snapshots should expose the committed document URL, not its inherited base URL"
    );
    let replacement_loader_id = snapshots[0]
        .loader_id
        .as_deref()
        .expect("completed child navigation should expose its DocumentLoader identity")
        .to_owned();
    let frame_tree = vm.child_browsing_context_frame_tree_snapshot_for_protocol();
    assert_eq!(frame_tree.len(), 1);
    assert_eq!(frame_tree[0].loader_id, replacement_loader_id);
    assert_eq!(
        frame_tree[0].name.as_deref(),
        Some("load-phase-frame-client")
    );
    assert_eq!(
        frame_tree[0].owner_element_id.as_deref(),
        Some("load-phase-frame")
    );

    vm.eval(
        r#"document.getElementById("load-phase-frame").srcdoc = "<body data-version='third'>third</body>""#,
    )
    .expect("second replacement navigation should queue");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "second replacement should retire the second exact document owner",
    )
    .await;
    assert_eq!(
        vm.eval("__childLoadPhaseEvents.join('|')")
            .expect("second replacement unload trace should evaluate"),
        "old-window|old-pagehide|old-unload|new-window|frame:new|new-pageshow|new-pagehide|new-unload",
        "each replacement document must own a fresh exactly-once unload lifecycle"
    );
    let second_replacement_tree = vm.child_browsing_context_frame_tree_snapshot_for_protocol();
    assert_eq!(second_replacement_tree.len(), 1);
    assert_ne!(
        second_replacement_tree[0].loader_id, replacement_loader_id,
        "a cross-document child navigation must replace its DocumentLoader identity"
    );
}

#[tokio::test]
async fn child_pageshow_replacement_stops_old_frame_finish_and_protocol_output() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-pageshow-phases.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childPageshowPhaseEvents = [];
  globalThis.__childPageshowPhaseReplacement = `
    <script>
      addEventListener("load", () => parent.__childPageshowPhaseEvents.push("new-window"));
      addEventListener("pageshow", () => parent.__childPageshowPhaseEvents.push("new-pageshow"));
    <\/script>
    <body data-version="new">new</body>`;
  const frame = document.createElement("iframe");
  frame.id = "pageshow-phase-frame";
  frame.onload = () => {
    __childPageshowPhaseEvents.push(`frame:${frame.contentDocument.body.dataset.version}`);
  };
  frame.srcdoc = `
    <script>
      addEventListener("load", () => {
        window.__oldWindowLoadInvoked = true;
        parent.__childPageshowPhaseEvents.push("old-window");
      });
      addEventListener("pageshow", () => {
        window.__oldWindowPageshowInvoked = true;
        parent.__childPageshowPhaseEvents.push("old-pageshow");
        parent.document.getElementById("pageshow-phase-frame").srcdoc =
          parent.__childPageshowPhaseReplacement;
      });
    <\/script>
    <body data-version="old">old</body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child pageshow-phase replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "old child pageshow-phase srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "old pageshow replacement handler should install",
    )
    .await;
    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("pageshow-phase-frame")
        .expect("pageshow-phase iframe should remain in the main document");
    {
        let context_host = vm._context_host.borrow();
        let load_callback_identities =
            context_host.child_window_event_callback_identities_for_test(child_handle, "load");
        assert_eq!(
            load_callback_identities.len(),
            1,
            "old Window load listener must be registered against the reused LocalWindow"
        );
        let pageshow_callback_identities =
            context_host.child_window_event_callback_identities_for_test(child_handle, "pageshow");
        assert_eq!(
            pageshow_callback_identities.len(),
            1,
            "old Window pageshow listener must be registered against the reused LocalWindow"
        );
        assert!(
            load_callback_identities
                .iter()
                .chain(&pageshow_callback_identities)
                .filter_map(|(relevant, _)| *relevant)
                .all(|identity| context_host
                    .window_execution_context_identity_is_current(identity)),
            "old Window lifecycle listeners must retain current callback relevant realms"
        );
    }
    for context in ["old interactive", "old DOMContentLoaded", "old complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "old pageshow should run before frame finish",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(document.getElementById('pageshow-phase-frame').contentWindow.__oldWindowLoadInvoked === true && document.getElementById('pageshow-phase-frame').contentWindow.__oldWindowPageshowInvoked === true)"
        )
        .expect("old child Window lifecycle invocation markers should evaluate"),
        "true",
        "HostLoad must invoke old Window load and pageshow before replacement"
    );
    assert_eq!(
        vm.eval("__childPageshowPhaseEvents.join('|')")
            .expect("old pageshow-phase trace should evaluate"),
        "old-window|frame:old|old-pageshow"
    );
    assert_eq!(
        vm.completed_child_frame_navigation_load_count(),
        0,
        "replacement in pageshow must suppress old frame/protocol completion"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "pageshow replacement should commit on its navigation source",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "pageshow replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement pageshow handlers should install",
    )
    .await;
    for context in [
        "replacement interactive",
        "replacement DOMContentLoaded",
        "replacement complete",
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "replacement pageshow delivery should finish independently",
    )
    .await;

    assert_eq!(
        vm.eval("__childPageshowPhaseEvents.join('|')")
            .expect("replacement pageshow-phase trace should evaluate"),
        "old-window|frame:old|old-pageshow|new-window|frame:new|new-pageshow"
    );
    assert_eq!(vm.completed_child_frame_navigation_load_count(), 1);
}

#[tokio::test]
async fn child_domcontentloaded_uses_one_document_to_window_event_path() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-dcl-path.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childDclPath = [];
  globalThis.__parentDclLeak = 0;
  addEventListener("DOMContentLoaded", () => {
    globalThis.__parentDclLeak += 1;
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      let firstDclEvent;
      addEventListener("DOMContentLoaded", event => {
        firstDclEvent = event;
        const path = event.composedPath();
        parent.__childDclPath.push([
          "window-capture",
          event.target === document,
          event.currentTarget === window,
          event.eventPhase,
          path[0] === document,
          path[1] === window
        ].join(":"));
      }, true);
      document.addEventListener("DOMContentLoaded", event => {
        parent.__childDclPath.push([
          "document",
          event === firstDclEvent,
          event.target === document,
          event.currentTarget === document,
          event.eventPhase
        ].join(":"));
      });
      addEventListener("DOMContentLoaded", event => {
        parent.__childDclPath.push([
          "window-bubble",
          event === firstDclEvent,
          event.target === document,
          event.currentTarget === window,
          event.eventPhase
        ].join(":"));
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child DOMContentLoaded event-path setup should evaluate");

    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("globalThis.__childDclPath.join('|')")
            .expect("child DOMContentLoaded event path should evaluate"),
        "window-capture:true:true:1:true:true|document:true:true:true:2|window-bubble:true:true:true:3"
    );
    assert_eq!(
        vm.eval("String(globalThis.__parentDclLeak)")
            .expect("parent DOMContentLoaded leak count should evaluate"),
        "0",
        "a child Document event path must not reuse the parent Window target"
    );
}

#[tokio::test]
async fn child_domcontentloaded_document_stop_propagation_blocks_window_bubble() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-dcl-stop.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childDclStop = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      addEventListener("DOMContentLoaded", () => {
        parent.__childDclStop.push("window-capture");
      }, true);
      document.addEventListener("DOMContentLoaded", event => {
        parent.__childDclStop.push("document");
        event.stopPropagation();
      });
      addEventListener("DOMContentLoaded", () => {
        parent.__childDclStop.push("window-bubble");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child DOMContentLoaded stop-propagation setup should evaluate");

    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("globalThis.__childDclStop.join('|')")
            .expect("child DOMContentLoaded stop-propagation result should evaluate"),
        "window-capture|document"
    );
}

#[tokio::test]
async fn child_static_media_delays_complete_and_iframe_load_until_loadeddata() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-media-delay.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childMediaDelayEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "media-frame";
  frame.onload = () => __childMediaDelayEvents.push("frame-load");
  frame.srcdoc = `
    <video id="clip" src="data:video/webm;base64,AA=="></video>
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childMediaDelayEvents.push("dcl");
      });
      const clip = document.getElementById("clip");
      for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
        clip.addEventListener(type, () => {
          parent.__childMediaDelayEvents.push(type);
        });
      }
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child static media setup should evaluate");

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child static media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child media parser script should run before lifecycle",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child media document should enter interactive and accept its media token",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child media document should dispatch DOMContentLoaded",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("blocked child HostLoad probe should succeed"),
        "the media token must keep HostLoad unavailable"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childMediaDelayEvents, readyState: document.getElementById('media-frame').contentDocument.readyState })"
        )
        .expect("blocked child media lifecycle should evaluate"),
        r#"{"events":["dcl"],"readyState":"interactive"}"#
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "child media loadstart turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "child media loadedmetadata turn")
        .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("metadata child HostLoad probe should succeed"),
        "metadata must not release the media delay"
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "child media loadeddata turn")
        .await;
    assert_eq!(
        vm.eval("globalThis.__childMediaDelayEvents.join('|')")
            .expect("child media loadeddata trace should evaluate"),
        "dcl|loadstart|loadedmetadata|loadeddata",
        "loadeddata must dispatch before it exposes complete work"
    );

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "loadeddata should queue a later complete lifecycle turn",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed media document should load on a still later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childMediaDelayEvents, readyState: document.getElementById('media-frame').contentDocument.readyState })"
        )
        .expect("released child media lifecycle should evaluate"),
        r#"{"events":["dcl","loadstart","loadedmetadata","loadeddata","frame-load"],"readyState":"complete"}"#
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "non-blocking child media canplay turn",
    )
    .await;
}

#[tokio::test]
async fn child_media_network_failure_releases_lifecycle_before_later_host_load() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader
        .set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::AUDIO);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &media_url.replace("/media", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childFailedMediaEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "failed-media-frame";
  frame.onload = () => __childFailedMediaEvents.push("frame-load");
  frame.srcdoc = `
    <audio id="failed-media" src={media_url:?}></audio>
    <script>
      const media = document.getElementById("failed-media");
      for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
        media.addEventListener(type, () => parent.__childFailedMediaEvents.push(type));
      }}
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childFailedMediaEvents.push("dcl");
      }});
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
}})()
"#
    ))
    .expect("child failed media setup should evaluate");
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child failed media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child media parser script should install listeners",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child failed media document should become interactive",
    )
    .await;
    let request = request_rx.await.expect("child media request should arrive");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: audio")
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child failed media document should dispatch DOMContentLoaded",
    )
    .await;
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "child failed media loadstart turn",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("pending-media child HostLoad probe should succeed"),
        "the pending media request must keep child HostLoad unavailable"
    );

    release_tx
        .send(())
        .expect("release child failed media response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "child media network failure",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedMediaEvents.join('|')")
            .expect("child media completion trace should evaluate"),
        "dcl|loadstart",
        "resource completion must not inline-dispatch media error or iframe load"
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "child failed media error owner turn",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedMediaEvents.join('|')")
            .expect("child media error trace should evaluate"),
        "dcl|loadstart|error"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "media error should expose complete only on a later lifecycle turn",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed failed-media document should dispatch iframe load later",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedMediaEvents.join('|')")
            .expect("child media HostLoad trace should evaluate"),
        "dcl|loadstart|error|frame-load"
    );
    server.await.expect("child media server should finish");
}

#[tokio::test]
async fn child_image_network_failure_releases_lifecycle_before_later_host_load() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childFailedImageEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "failed-image-frame";
  frame.onload = () => __childFailedImageEvents.push("frame-load");
  frame.srcdoc = `
    <img id="failed-image" src={image_url:?}>
    <script>
      const image = document.getElementById("failed-image");
      image.onload = () => parent.__childFailedImageEvents.push("load");
      image.onerror = () => parent.__childFailedImageEvents.push("error");
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childFailedImageEvents.push("dcl");
      }});
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
}})()
"#
    ))
    .expect("child failed image setup should evaluate");
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child failed image srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child image parser script should install listeners",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child image document should become interactive",
    )
    .await;
    let request = request_rx.await.expect("child image request should arrive");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: image")
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child image document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image pre-terminal trace"),
        "dcl"
    );
    let child_image = (0..vm.document_runtime.dom_host().dom().nodes().len())
        .map(crate::document_runtime::DomHandle::new)
        .find(|handle| {
            vm.document_runtime
                .dom_host()
                .node(*handle)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.attribute("id") == Some("failed-image"))
        })
        .expect("child image handle");
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("pending-image child HostLoad probe should succeed"),
        "the pending image request must keep child HostLoad unavailable"
    );
    let child_pending_before_detach = vm
        ._context_host
        .borrow()
        .pending_image_load_event(child_image)
        .expect("connected child image sequence");
    vm.eval(
        "document.getElementById('failed-image-frame').contentDocument.getElementById('failed-image').remove()",
    )
    .expect("detaching the pending child image should evaluate");
    let detached_pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(child_image)
        .expect("detached child image sequence");
    assert_eq!(detached_pending.id(), child_pending_before_detach.id());
    assert_eq!(
        detached_pending.network_request_id(),
        child_pending_before_detach.network_request_id()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event_is_current(child_image, detached_pending),
        "same-document detach must preserve current image ownership"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("detached-image child HostLoad probe should succeed"),
        "same-document image removal must preserve the child document delay until the event"
    );

    release_tx
        .send(())
        .expect("release child failed image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "child image network failure",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image completion trace"),
        "dcl",
        "resource completion must not inline-dispatch image error or iframe load"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("child image error selected task should run"),
        "child image failure should enqueue one DOM-manipulation turn"
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("child image error source should become idle"),
        "one failed image terminal must produce exactly one selected event task"
    );
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image error trace"),
        "dcl|error"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "image error should expose complete only on a later lifecycle turn",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed failed-image document should dispatch iframe load later",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image HostLoad trace"),
        "dcl|error|frame-load"
    );
    server.await.expect("child image server should finish");
}

#[tokio::test]
async fn child_dynamic_media_accepted_during_dcl_delays_later_load_turns() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-dynamic-media.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childDynamicMediaEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "dynamic-media-frame";
  frame.onload = () => __childDynamicMediaEvents.push("frame-load");
  frame.srcdoc = `<body><script>
    document.addEventListener("DOMContentLoaded", () => {
      parent.__childDynamicMediaEvents.push("dcl");
      const clip = document.createElement("video");
      clip.addEventListener("loadeddata", () => {
        parent.__childDynamicMediaEvents.push("loadeddata");
      });
      document.body.appendChild(clip);
      clip.src = "data:video/webm;base64,AQ==";
    });
  <\/script></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child dynamic media setup should evaluate");

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child dynamic media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "dynamic media parser script should install its DCL producer",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "dynamic media document should enter interactive",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "DCL should accept dynamic media before complete is prepared",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("dynamic-media child HostLoad probe should succeed"),
        "DCL-inserted media must block HostLoad"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "dynamic media loadstart turn")
        .await;
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "dynamic media loadedmetadata turn",
    )
    .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "dynamic media loadeddata turn")
        .await;
    assert_eq!(
        vm.eval("globalThis.__childDynamicMediaEvents.join('|')")
            .expect("dynamic media terminal trace should evaluate"),
        "dcl|loadeddata"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "dynamic media terminal should expose complete later",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "dynamic media completion should expose iframe load later",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childDynamicMediaEvents.join('|')")
            .expect("dynamic media load trace should evaluate"),
        "dcl|loadeddata|frame-load"
    );
}

#[tokio::test]
async fn child_static_text_track_starts_at_interactive_without_own_load_token() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-static-track.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childStaticTrackEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "track-frame";
  frame.onload = () => __childStaticTrackEvents.push("frame-load");
  frame.srcdoc = `
    <video><track id="captions" default src="data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Ahello"></video>
    <script>
      document.getElementById("captions").addEventListener("load", () => {
        parent.__childStaticTrackEvents.push("track:" + document.readyState);
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child static text-track setup should evaluate");

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child static text-track srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child track parser script should install its listener",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "interactive should start the static child track",
    )
    .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("child default text-track mode owner turn"),
        "child default track should use the shared DOM-manipulation source"
    );
    let frame = vm
        .document_runtime
        .get_element_by_id("track-frame")
        .expect("text-track child frame handle");
    {
        let context_host = vm._context_host.borrow();
        let snapshot = context_host
            .frame_owner_current_child_snapshot(frame)
            .expect("text-track child owner snapshot");
        let track = context_host
            .dom_host()
            .element_handle_by_id_in_subtree(snapshot.document_handle, "captions")
            .expect("child text-track handle");
        let pending = context_host
            .pending_text_track_load_sequence(track)
            .expect("child text-track sequence");
        let expected_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        assert_eq!(
            pending.target(),
            crate::native_bridge::WindowDocumentTaskTarget::new(
                crate::native_bridge::WindowDocumentOwner::Frame(expected_owner),
                crate::native_bridge::OwnerDispatchScope::Child(frame),
            )
        );
    }
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("child track load-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("child track terminal networking turn")
    );
    assert_eq!(
        vm.eval("globalThis.__childStaticTrackEvents.join('|')")
            .expect("child track interactive trace should evaluate"),
        "track:interactive",
        "track scheduling must not wait for iframe/window load"
    );

    for (source, transition) in [
        (
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "DOMContentLoaded",
        ),
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "complete"),
        (ChildFrameSemanticTurnKind::HostLoad, "iframe load"),
    ] {
        expect_page_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            &loader,
            source,
            &format!("track document should later advance through {transition}"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("globalThis.__childStaticTrackEvents.join('|')")
            .expect("child track load trace should evaluate"),
        "track:interactive|frame-load"
    );
}

#[tokio::test]
async fn child_document_replacement_retires_media_sequence_and_delay() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-media-replace.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childMediaReplacementEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "media-replacement-frame";
  frame.onload = () => {
    __childMediaReplacementEvents.push(
      "frame-load:" + frame.contentDocument.getElementById("document-marker").textContent
    );
  };
  frame.srcdoc = `
    <video id="old-media" src="data:video/webm;base64,AA=="></video>
    <script>
      document.getElementById("old-media").addEventListener("loadstart", () => {
        parent.__childMediaReplacementEvents.push("stale-media-loadstart");
      });
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childMediaReplacementEvents.push("first-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child media replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "first child media srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first media document parser script should run",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "first media document should enter interactive",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "first media document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childMediaReplacementEvents.join('|')")
            .expect("first media document trace should evaluate"),
        "first-dcl"
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("media-replacement-frame");
  frame.srcdoc = `<body><p id="document-marker">second</p><script>
    document.addEventListener("DOMContentLoaded", () => {
      parent.__childMediaReplacementEvents.push("second-dcl");
    });
  <\/script></body>`;
})()
"#,
    )
    .expect("replacement child document should queue");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "replacement should retire the old media owner before commit",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "media replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement parser script should run",
    )
    .await;
    for (source, transition) in [
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "interactive"),
        (
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "DOMContentLoaded",
        ),
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "complete"),
        (ChildFrameSemanticTurnKind::HostLoad, "iframe load"),
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            source,
            &format!("replacement should advance through {transition}"),
        )
        .await;
    }

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired child media callbacks should remain harmless");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        None,
        "retired media callbacks must not create replacement lifecycle work"
    );
    assert_eq!(
        vm.eval("globalThis.__childMediaReplacementEvents.join('|')")
            .expect("replacement child media result should evaluate"),
        "first-dcl|second-dcl|frame-load:second"
    );
}

#[tokio::test]
async fn moving_pending_child_media_restarts_under_the_new_document_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-media-move.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__movedChildMediaEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "moving-media-frame";
  frame.onload = () => __movedChildMediaEvents.push("frame-load");
  frame.srcdoc = `
    <video id="moving-media" src="data:video/webm;base64,AA=="></video>
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__movedChildMediaEvents.push("child-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("moving child media setup should evaluate");
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "moving child media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "moving media parser script should run",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving media document should enter interactive",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving media document should dispatch DOMContentLoaded",
    )
    .await;
    let frame = vm
        .document_runtime
        .get_element_by_id("moving-media-frame")
        .expect("moving media frame handle");
    let media = {
        let context_host = vm._context_host.borrow();
        let child_document = context_host
            .child_browsing_context_document_handle(frame)
            .expect("moving media child document");
        context_host
            .dom_host()
            .element_handle_by_id_in_subtree(child_document, "moving-media")
            .expect("moving media handle")
    };
    let child_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("child-owned media sequence");
    assert!(matches!(
        child_sequence.owner(),
        crate::native_bridge::PendingMediaLoadOwner::Child { .. }
    ));

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("moving-media-frame");
  const media = document.adoptNode(
    frame.contentDocument.getElementById("moving-media")
  );
  media.addEventListener("loadeddata", () => {
    __movedChildMediaEvents.push("moved-loadeddata");
  });
  (document.body || document.documentElement || document).appendChild(media);
})()
"#,
    )
    .expect("pending child media should move to the parent document");
    let main_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("moved media should restart under the parent owner");
    assert_ne!(child_sequence.id(), main_sequence.id());
    assert!(matches!(
        main_sequence.owner(),
        crate::native_bridge::PendingMediaLoadOwner::Main { .. }
    ));

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving media away should release child complete later",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "the child iframe should load after its media owner moves away",
    )
    .await;
    assert!(
        !vm.has_ready_timeout(),
        "moving a pending media element must not create a synthetic Page timer"
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale child media loadstart turn",
    )
    .await;
    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("moved main media {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("globalThis.__movedChildMediaEvents.join('|')")
            .expect("moved child media trace should evaluate"),
        "child-dcl|frame-load|moved-loadeddata",
        "the stale child callback must not consume the restarted main sequence"
    );
}

#[tokio::test]
async fn child_image_event_delay_blocks_complete_and_host_load_until_terminal() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-image-delay.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childImageDelayEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "image-frame";
  frame.onload = () => __childImageDelayEvents.push("frame-load");
  frame.srcdoc = `
    <img id="hero" src="image.png">
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childImageDelayEvents.push("dcl");
      });
      document.getElementById("hero").addEventListener("load", event => {
        parent.__childImageDelayEvents.push(
          "image-load:" + (event instanceof Event) + ":" + (event instanceof parent.Event)
        );
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child image load-delay setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child image-delay srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child image parser script should run before lifecycle",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child image document should enter interactive",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child image document should dispatch DOMContentLoaded",
    )
    .await;
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "the pending image token must prevent HostLoad delivery"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childImageDelayEvents, readyState: document.getElementById('image-frame').contentDocument.readyState })"
        )
        .expect("blocked child image lifecycle should evaluate"),
        r#"{"events":["dcl"],"readyState":"interactive"}"#,
        "image acceptance must allow DCL but block complete and iframe load"
    );

    assert_eq!(
        drain_image_load_event_bodies_for_test(&mut vm),
        1,
        "child image terminal should enqueue one DOM-manipulation turn"
    );
    assert_eq!(
        vm.eval("globalThis.__childImageDelayEvents.join('|')")
            .expect("child image terminal should evaluate"),
        "dcl|image-load:true:false",
        "the image event task must only enqueue the later lifecycle follow-up"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "image terminal should queue a later complete lifecycle turn",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed image document should load on a still later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childImageDelayEvents, readyState: document.getElementById('image-frame').contentDocument.readyState })"
        )
        .expect("released child image lifecycle should evaluate"),
        r#"{"events":["dcl","image-load:true:false","frame-load"],"readyState":"complete"}"#
    );
}

#[tokio::test]
async fn child_document_replacement_cancels_stale_image_event_and_delay() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-image-replace.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childImageReplacementEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "replacement-frame";
  frame.onload = () => {
    __childImageReplacementEvents.push(
      "frame-load:" + frame.contentDocument.getElementById("document-marker").textContent
    );
  };
  frame.srcdoc = `
    <img id="old-image" src="old.png">
    <script>
      document.getElementById("old-image").addEventListener("load", () => {
        parent.__childImageReplacementEvents.push("stale-image-load");
      });
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childImageReplacementEvents.push("first-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child image replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "first child image srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first image document parser script should run",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "first image document should enter interactive",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "first image document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childImageReplacementEvents.join('|')")
            .expect("first child image document should evaluate"),
        "first-dcl"
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("replacement-frame");
  frame.srcdoc = `<body><p id="document-marker">second</p><script>
    document.addEventListener("DOMContentLoaded", () => {
      parent.__childImageReplacementEvents.push("second-dcl");
    });
  <\/script></body>`;
})()
"#,
    )
    .expect("replacement child document should queue");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "replacement should commit on its navigation turn",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "image replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement parser script should run",
    )
    .await;
    for (source, transition) in [
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "interactive"),
        (
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "DOMContentLoaded",
        ),
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "complete"),
        (ChildFrameSemanticTurnKind::HostLoad, "iframe load"),
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            source,
            &format!("replacement should advance through {transition}"),
        )
        .await;
    }

    let _ = drain_image_load_event_bodies_for_test(&mut vm);
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        None,
        "the canceled image task must not create replacement lifecycle work"
    );
    assert_eq!(
        vm.eval("globalThis.__childImageReplacementEvents.join('|')")
            .expect("replacement child image result should evaluate"),
        "first-dcl|second-dcl|frame-load:second",
        "replacement must retire the old image task/event/token without blocking the new load"
    );
}

#[tokio::test]
async fn moving_pending_child_image_rebinds_event_without_consuming_new_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-image-move.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__movedChildImageEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "moving-image-frame";
  frame.onload = () => __movedChildImageEvents.push("frame-load");
  frame.srcdoc = `
    <img id="moving-image" src="moving.png">
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__movedChildImageEvents.push("child-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("moving child image setup should evaluate");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "moving child image srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "moving image parser script should run",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving image document should enter interactive",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving image document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__movedChildImageEvents.join('|')")
            .expect("blocked moving child image state should evaluate"),
        "child-dcl"
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("moving-image-frame");
  const image = document.adoptNode(
    frame.contentDocument.getElementById("moving-image")
  );
  image.addEventListener("load", () => {
    __movedChildImageEvents.push("moved-image-load");
  });
  (document.body || document.documentElement || document).appendChild(image);
})()
"#,
    )
    .expect("pending child image should move to the parent document");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving the image should release complete on a later lifecycle turn",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "the former image owner should load on a later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__movedChildImageEvents.join('|')")
            .expect("moved image child lifecycle should evaluate"),
        "child-dcl|frame-load",
        "moving the image must release the old child delay through a later lifecycle turn"
    );

    assert!(
        drain_image_load_event_bodies_for_test(&mut vm) > 0,
        "old and rebound image tasks should drain"
    );
    assert_eq!(
        vm.eval("globalThis.__movedChildImageEvents.join('|')")
            .expect("moved image event result should evaluate"),
        "child-dcl|frame-load|moved-image-load",
        "the stale child task must not consume or duplicate the rebound image request"
    );
}

#[test]
fn window_pageshow_uses_original_page_transition_event() {
    let mut vm = new_storage_test_vm("https://window-pageshow-event.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__pageshowShape = "missing";
  window.PageTransitionEvent = function PageTransitionEvent() {
    throw new Error("page replacement should not be invoked");
  };
  addEventListener("pageshow", event => {
    __pageshowShape = `${event.type}:${event.persisted === false}:${'persisted' in event}`;
  });
  return "ready";
})()
"#,
    )
    .expect("pageshow setup should evaluate");
    vm.dispatch_window_load_event()
        .expect("window load should dispatch");

    let shape = vm
        .eval("globalThis.__pageshowShape")
        .expect("pageshow shape should evaluate");
    assert_eq!(shape, "pageshow:true:true");
}

#[test]
fn window_load_uses_original_event_after_global_constructors_are_deleted() {
    let mut vm = new_storage_test_vm("https://window-load-original-event.test/");

    vm.eval(
        r#"
(() => {
  const OriginalEvent = Event;
  const OriginalPageTransitionEvent = PageTransitionEvent;
  globalThis.__windowLifecycleEvents = [];
  addEventListener('load', event => {
    __windowLifecycleEvents.push(
      `load:${event instanceof OriginalEvent}:${event.target === document}:${event.currentTarget === window}`
    );
  });
  addEventListener('pageshow', event => {
    __windowLifecycleEvents.push(
      `pageshow:${event instanceof OriginalPageTransitionEvent}:${event.persisted}`
    );
  });
  delete globalThis.Event;
  delete globalThis.PageTransitionEvent;
})()
"#,
    )
    .expect("window lifecycle constructor deletion setup should evaluate");

    vm.dispatch_window_load_event()
        .expect("window load should use saved original constructors");

    assert_eq!(
        vm.eval("__windowLifecycleEvents.join('|')")
            .expect("window lifecycle results should evaluate"),
        "load:true:true:true|pageshow:true:false"
    );
}

#[test]
fn no_src_iframe_initial_about_blank_fragment_location_does_not_change_current_entry() {
    let mut vm = new_storage_test_vm("https://iframe-initial-currententry.test/");

    let result = vm
        .eval(
            r##"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
const child = frame.contentWindow;
const events = [];
child.navigation.oncurrententrychange = event => {
  events.push(`${event.navigationType}:${child.location.href}:${child.navigation.currentEntry.url}`);
};
child.location.href = "about:blank#1";
child.location.href = "about:blank#2";
[
  child.location.href,
  child.navigation.entries().length,
  child.navigation.currentEntry.url,
  events.join(",")
].join("|")
"##,
        )
        .expect("initial about:blank fragment navigation should evaluate");

    assert_eq!(result, "about:blank#2|1|about:blank|");
}
#[test]
fn contextual_fragment_scripts_run_when_inserted() {
    let mut vm = new_storage_test_vm("https://contextual-fragment-scripts.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__fragmentScriptOrder = [];
  const range = document.createRange();
  const fragment = range.createContextualFragment(
    "<script>__fragmentScriptOrder.push('fragment script')<\/script>"
  );
  __fragmentScriptOrder.push('before append');
  (document.body || document.documentElement || document).appendChild(fragment.firstChild);
  __fragmentScriptOrder.push('after append');
  return __fragmentScriptOrder.join('|');
})()
"#,
        )
        .expect("contextual fragment inline script should evaluate");

    assert_eq!(result, "before append|fragment script|after append");
}

#[test]
fn contextual_fragment_document_element_fragment_append_runs_scripts() {
    let mut vm = new_storage_test_vm("https://contextual-fragment-document-element-scripts.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__fragmentScriptRan = false;
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const range = document.createRange();
  range.selectNodeContents(html);
  const fragment = range.createContextualFragment(
    "<script>__fragmentScriptRan = true<\/script>"
  );
  const beforeShape = Array.from(fragment.childNodes)
    .map(node => `${node.nodeName}:${node.localName}:${node.childNodes.length}`)
    .join(",");
  const beforeAppend = __fragmentScriptRan;
  body.appendChild(fragment);
  return [
    beforeAppend,
    __fragmentScriptRan,
    fragment.childNodes.length,
    beforeShape,
    body.lastChild && body.lastChild.nodeName,
    body.lastChild && body.lastChild.childNodes.length
  ].join("|");
})()
"#,
        )
        .expect("contextual fragment documentElement script should evaluate after append");

    assert_eq!(result, "false|true|0|SCRIPT:script:1|SCRIPT|1");
}

#[test]
fn sandboxed_contextual_fragment_parses_noscript_when_scripting_disabled() {
    let mut vm = new_storage_test_vm("https://sandbox-contextual-fragment-noscript.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.sandbox = "allow-same-origin";
  (document.body || document.documentElement || document).appendChild(iframe);
  const doc = iframe.contentDocument;
  iframe.contentWindow.didRunScript = false;
  const html =
    "<script>window.didRunScript = true<\/script>" +
    "<noscript><div id=nos></div>";
  const fragment = doc.createRange().createContextualFragment(html);
  doc.body.appendChild(fragment);
  return [
    iframe.contentWindow.didRunScript,
    doc.getElementById("nos") !== null
  ].join("|");
})()
"#,
        )
        .expect("sandboxed contextual fragment noscript probe should evaluate");

    assert_eq!(result, "false|true");
}

#[test]
fn document_fragment_insert_runs_nested_scripts_in_tree_order() {
    let mut vm = new_storage_test_vm("https://document-fragment-script-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__documentFragmentScriptOrder = [];
  const fragment = document.createDocumentFragment();
  const first = document.createElement('script');
  first.textContent = "__documentFragmentScriptOrder.push('first')";
  const container = document.createElement('div');
  const nested = document.createElement('script');
  nested.textContent = "__documentFragmentScriptOrder.push('nested')";
  const last = document.createElement('script');
  last.textContent = "__documentFragmentScriptOrder.push('last')";
  fragment.appendChild(first);
  container.appendChild(nested);
  fragment.appendChild(container);
  fragment.appendChild(last);
  __documentFragmentScriptOrder.push('before append');
  (document.body || document.documentElement || document).appendChild(fragment);
  __documentFragmentScriptOrder.push('after append');
  return __documentFragmentScriptOrder.join('|');
})()
"#,
        )
        .expect("document fragment script order should evaluate");

    assert_eq!(result, "before append|first|nested|last|after append");
}

#[test]
fn document_fragment_script_start_revalidates_after_an_earlier_script_mutates_the_batch() {
    let mut vm = new_storage_test_vm("https://document-fragment-script-revalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__documentFragmentRevalidation = [];
  const fragment = document.createDocumentFragment();
  const first = document.createElement('script');
  first.textContent = `
    __documentFragmentRevalidation.push('first');
    document.getElementById('later-script').remove();
  `;
  const later = document.createElement('script');
  later.id = 'later-script';
  later.textContent = "__documentFragmentRevalidation.push('stale-later')";
  fragment.append(first, later);
  (document.body || document.documentElement || document).appendChild(fragment);
  return __documentFragmentRevalidation.join('|');
})()
"#,
        )
        .expect("an earlier fragment script should invalidate a later start candidate");

    assert_eq!(result, "first");
}

#[tokio::test]
async fn child_document_open_in_dom_content_loaded_yields_to_timer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://child-document-open-dcl-timer.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childOpenDclEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `
    <script>
      addEventListener('DOMContentLoaded', function() {
        parent.__childOpenDclEvents.push('dcl');
        document.open();
        setTimeout(function() {
          parent.__childOpenDclEvents.push('timer');
        }, 0);
        document.close();
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child DOMContentLoaded document.open setup should evaluate");

    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("globalThis.__childOpenDclEvents.join('|')")
            .expect("child DCL log should evaluate"),
        "dcl"
    );

    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("timer turn should advance child DCL timer")
    );
    assert_eq!(
        vm.eval("globalThis.__childOpenDclEvents.join('|')")
            .expect("child DCL timer result should evaluate"),
        "dcl|timer"
    );
}

#[tokio::test]
async fn child_document_close_without_defer_queues_replacement_domcontentloaded() {
    let mut vm = new_storage_test_vm("https://child-document-close-dcl.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childCloseLifecycleEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `
    <script>
      document.addEventListener('DOMContentLoaded', function() {
        if (window.__replacementStarted) return;
        window.__replacementStarted = true;
        document.open();
        document.addEventListener('DOMContentLoaded', function() {
          parent.__childCloseLifecycleEvents.push('replacement-dcl');
        });
        document.write('<p>replacement</p>');
        document.close();
        parent.__childCloseLifecycleEvents.push('after-close');
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child document.close lifecycle setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child srcdoc should install its exact document owner before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child parser script should install the original DCL handler",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "original child document should become interactive",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "original child DCL should synchronously finish the replacement parser",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childCloseLifecycleEvents.join('|')")
            .expect("child document.close return trace should evaluate"),
        "after-close",
        "replacement DOMContentLoaded must not dispatch inline from document.close"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "replacement DOMContentLoaded should remain a later lifecycle source turn",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childCloseLifecycleEvents.join('|')")
            .expect("replacement DCL trace should evaluate"),
        "after-close|replacement-dcl"
    );
}

#[tokio::test]
async fn child_script_document_open_after_location_navigation_is_noop() {
    let mut vm = new_storage_test_vm("https://child-script-location-open.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__childTextAfterNavigation = 'unset';
  globalThis.__childLoadHandlerStage = 'unset';
  globalThis.__childLoadListenerStage = 'unset';
  const frame = document.createElement('iframe');
  frame.__childLoadHandlerStage = 'unset';
  frame.__childLoadListenerStage = 'unset';
  frame.srcdoc = `
    <script>
      const blob = new Blob(['PASS'], { type: 'text/html' });
      location.href = URL.createObjectURL(blob);
      frameElement.onload = () => {
        frameElement.__childLoadHandlerStage = 'entered';
        parent.__childTextAfterNavigation = frameElement.contentDocument.body.textContent;
        frameElement.__childLoadHandlerStage = 'completed';
      };
      frameElement.addEventListener('load', () => {
        frameElement.__childLoadListenerStage = 'entered';
        parent.__childLoadListenerStage = 'entered';
      });
      document.open();
      document.write('FAIL');
      document.close();
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return String(frame.contentDocument.body.textContent).includes('FAIL');
})()
"#,
        )
        .expect("child script document.open after location navigation should evaluate");

    assert_eq!(result, "false");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "initial srcdoc navigation must not dispatch load before commit"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "initial srcdoc must commit on its own navigation turn"
    );
    for label in [
        "initial-empty realm retirement before srcdoc script",
        "srcdoc realm materialization before parser script",
    ] {
        expect_one_child_frame_task_source(
            &mut vm,
            ChildFrameSemanticTurnKind::RealmMaterialization,
            label,
        )
        .await;
    }
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "srcdoc parser script must execute on DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childTextAfterNavigation")
            .expect("child load handler precondition should evaluate"),
        "unset",
        "navigation and script turns must not dispatch iframe load inline"
    );
    assert_eq!(
        vm.eval("typeof document.querySelector('iframe').onload")
            .expect("child load handler registration should evaluate"),
        "function",
        "the child callback must be registered on the parent-owned frame element"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "the blob URL assigned by the child script must commit on a later navigation turn"
    );
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "the queued srcdoc lifecycle task must stale-discard before blob lifecycle work",
    )
    .await;
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("blob replacement should run {transition} before HostLoad"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("document.querySelector('iframe').onload === null")
            .expect("retired child callback projection should evaluate"),
        "true",
        "LocalWindow retirement must tombstone handler properties instead of rediscovering the old callback from its wrapper"
    );
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "the committed blob document should settle load from a later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').__childLoadHandlerStage")
            .expect("deferred load delivery stage should evaluate"),
        "unset",
        "the later HostLoad turn must not invoke a callback owned by the retired child LocalWindow"
    );
    assert_eq!(
        vm.eval("document.querySelector('iframe').__childLoadListenerStage")
            .expect("deferred load listener stage should evaluate"),
        "unset",
        "the later HostLoad turn must not invoke a listener owned by the retired child LocalWindow"
    );

    assert_eq!(
        vm.eval(
            r#"
[
  document.querySelector('iframe').__childLoadHandlerStage,
  document.querySelector('iframe').__childLoadListenerStage,
  globalThis.__childTextAfterNavigation,
  globalThis.__childLoadListenerStage,
  document.querySelector('iframe').contentDocument.URL.startsWith('blob:'),
  document.querySelector('iframe').contentDocument.body.textContent,
  frames[0].document.body.textContent,
  String(document.querySelector('iframe').contentDocument.body.textContent).includes('FAIL')
].join('|')
"#
        )
        .expect("child navigation result should evaluate"),
        "unset|unset|unset|unset|true|PASS|PASS|false"
    );
}

#[tokio::test]
async fn main_window_indexed_child_descriptor_matches_window_semantics() {
    let mut vm = new_storage_test_vm("https://window-indexed-descriptor.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'queued';
})()
"#,
    )
    .expect("same-origin child navigation should queue");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "srcdoc child should complete before indexed Window reflection",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("srcdoc child should have a default execution context");
    vm.eval_in_child_default_context(child_context_id, "document.write('replacement')")
        .expect("post-parse child document.write should replace the child Document");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const descriptor = Object.getOwnPropertyDescriptor(window, '0');
  let strictAssignmentThrew = false;
  try {
    (() => {
      'use strict';
      window[0] = null;
    })();
  } catch (error) {
    strictAssignmentThrew = error instanceof TypeError;
  }
  return JSON.stringify({
    valueIsChild: descriptor.value === frame.contentWindow,
    writable: descriptor.writable,
    enumerable: descriptor.enumerable,
    configurable: descriptor.configurable,
    listed: Object.getOwnPropertyNames(window).includes('0'),
    strictAssignmentThrew,
    assignmentPreservedChild: window[0] === frame.contentWindow
  });
})()
"#,
        )
        .expect("main Window indexed child descriptor should evaluate");

    assert_eq!(
        result,
        r#"{"valueIsChild":true,"writable":false,"enumerable":true,"configurable":true,"listed":true,"strictAssignmentThrew":true,"assignmentPreservedChild":true}"#
    );
}

#[test]
fn same_origin_child_window_migration_to_cross_origin_installs_denied_surface() {
    let mut vm = new_storage_test_vm("https://child-cross-origin-migration.test/");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
frame.srcdoc = "<body>same-origin</body>";
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__migratedFrame = frame;
globalThis.__migratedWindow = frame.contentWindow;
globalThis.__migratedWindow.localStorage.setItem("before", "same-origin");
frame.src = "data:text/html,<body>cross-origin</body>";
"#,
        None,
    )
    .expect("same-origin child migration setup should run");

    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const win = globalThis.__migratedWindow;
  const probe = callback => {
    try {
      const value = callback();
      return value === null ? "null" : `${typeof value}:${String(value)}`;
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };
  return [
    __migratedFrame.contentDocument === null,
    win === __migratedFrame.contentWindow,
    Object.getPrototypeOf(win) === null,
    Object.prototype.toString.call(win),
    probe(() => win.document),
    probe(() => win.localStorage),
    probe(() => win.sessionStorage),
    probe(() => win.trustedTypes),
    probe(() => win.location.href)
  ].join("|");
})()
"#,
        )
        .expect("migrated cross-origin window surface should evaluate");

    assert_eq!(
        result,
        "true|true|true|[object Window]|SecurityError:true|SecurityError:true|SecurityError:true|SecurityError:true|SecurityError:true"
    );
}

#[test]
fn child_window_proxy_identity_survives_cross_origin_round_trip() {
    let mut vm = new_storage_test_vm("https://child-window-proxy-round-trip.test/");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
frame.srcdoc = "<body>initial same-origin</body>";
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__roundTripFrame = frame;
globalThis.__roundTripWindow = frame.contentWindow;
"#,
        None,
    )
    .expect("same-origin child WindowProxy setup should run");
    vm.drain_pending_child_frame_work_for_test();
    let initial_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("initial same-origin child realm should materialize");
    vm.eval_in_child_default_context(
        initial_context_id,
        "globalThis.__retiredInnerWindowMarker = 41",
    )
    .expect("initial child inner global marker should evaluate");

    vm.exec(
        r#"
__roundTripFrame.src = "data:text/html,<body>cross-origin</body>";
"#,
        None,
    )
    .expect("cross-origin child navigation should start");
    vm.drain_pending_child_frame_work_for_test();

    let cross_origin_identity = vm
        .eval("__roundTripWindow === __roundTripFrame.contentWindow")
        .expect("cross-origin WindowProxy identity should evaluate");
    assert_eq!(cross_origin_identity, "true");

    vm.exec(
        r#"
__roundTripFrame.src = "about:blank";
"#,
        None,
    )
    .expect("same-origin child navigation should start");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
[
  __roundTripWindow === __roundTripFrame.contentWindow,
  __roundTripFrame.contentDocument !== null,
  __roundTripWindow.document === __roundTripFrame.contentDocument
].join("|")
"#,
        )
        .expect("same-origin WindowProxy restoration should evaluate");
    assert_eq!(result, "true|true|true");

    let replacement_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("replacement same-origin child realm should materialize");
    assert_ne!(replacement_context_id, initial_context_id);
    let replacement_marker = vm
        .eval_in_child_default_context(
            replacement_context_id,
            "typeof globalThis.__retiredInnerWindowMarker",
        )
        .expect("replacement child inner global marker should evaluate");
    assert_eq!(replacement_marker, "undefined");
}

#[test]
fn inherited_child_document_domain_mutation_updates_aliased_window_security_token() {
    let mut vm = new_storage_test_vm("https://www.example.com/page.html");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><body>same-origin child</body>";
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__domainTokenFrame = frame;
"#,
        None,
    )
    .expect("document.domain token setup should run");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("same-origin child realm should materialize");

    assert_eq!(
        vm.eval("__domainTokenFrame.contentWindow.document === __domainTokenFrame.contentDocument")
            .expect("initial child document access should evaluate"),
        "true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "document.domain = document.domain; document.domain",
        )
        .expect("child document.domain mutation should evaluate"),
        "www.example.com"
    );

    assert_eq!(
        vm.eval(
            "__domainTokenFrame.contentWindow.document === __domainTokenFrame.contentDocument",
        )
        .expect("aliased post-domain child access should evaluate"),
        "true"
    );
    assert_eq!(
        vm.eval("document.domain")
            .expect("aliased parent document.domain should evaluate"),
        "www.example.com"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn document_domain_full_access_requires_both_documents_and_resets_on_navigation() {
    const INITIAL_CHILD_HOST: &str = "sub.example.test";
    const REPLACEMENT_CHILD_HOST: &str = "other.example.test";

    let initial_server = StaticHttpServer::spawn(1).await;
    let replacement_server = StaticHttpServer::spawn(1).await;
    let initial_child_url = initial_server.url_for_host(INITIAL_CHILD_HOST, "/child.html");
    let replacement_child_url =
        replacement_server.url_for_host(REPLACEMENT_CHILD_HOST, "/replacement.html");
    let loader = static_http_loader([
        initial_server.resolve_entry(INITIAL_CHILD_HOST),
        replacement_server.resolve_entry(REPLACEMENT_CHILD_HOST),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://www.example.test:8443/page.html",
        &loader,
    );
    vm.eval(&format!(
        "globalThis.__initialDomainChildUrl = {}; globalThis.__replacementDomainChildUrl = {};",
        serde_json::to_string(initial_child_url.as_str())
            .expect("serialize initial document.domain child URL"),
        serde_json::to_string(replacement_child_url.as_str())
            .expect("serialize replacement document.domain child URL")
    ))
    .expect("document.domain child URLs should install");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
globalThis.__domainAccessLoadCount = 0;
frame.onload = () => { globalThis.__domainAccessLoadCount += 1; };
frame.src = globalThis.__initialDomainChildUrl;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__domainAccessFrame = frame;
globalThis.__probeDomainChildDocument = () => {
  try {
    return frame.contentWindow.document.domain;
  } catch (error) {
    return `${error && error.name}:${error instanceof DOMException}`;
  }
};
"#,
        None,
    )
    .expect("cross-origin document.domain frame setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__domainAccessLoadCount)",
        "1",
        "initial document.domain child should load",
    )
    .await;

    let initial_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("cross-origin child realm should materialize");
    assert_eq!(
        vm.eval("__probeDomainChildDocument()")
            .expect("initial cross-origin access probe should evaluate"),
        "SecurityError:true"
    );

    assert_eq!(
        vm.eval("document.domain = 'example.test'; __probeDomainChildDocument()")
            .expect("one-sided parent document.domain probe should evaluate"),
        "SecurityError:true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            initial_context_id,
            r#"
(() => {
  const before = (() => {
    try {
      return parent.document.domain;
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  })();
  document.domain = "example.test";
  return [before, parent.document.domain, top.document.domain].join("|");
})()
"#,
        )
        .expect("child document.domain access probe should evaluate"),
        "SecurityError:true|example.test|example.test"
    );
    assert_eq!(
        vm.eval(
            "[__probeDomainChildDocument(), __domainAccessFrame.contentDocument.domain].join('|')",
        )
        .expect("two-sided document.domain access probe should evaluate"),
        "example.test|example.test"
    );

    vm.exec(
        r#"
__domainAccessFrame.src = globalThis.__replacementDomainChildUrl;
"#,
        None,
    )
    .expect("replacement cross-origin child navigation should start");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__domainAccessLoadCount)",
        "2",
        "replacement document.domain child should load",
    )
    .await;

    let replacement_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("replacement cross-origin child realm should materialize");
    assert_ne!(replacement_context_id, initial_context_id);
    assert_eq!(
        vm.eval("__probeDomainChildDocument()")
            .expect("replacement one-sided document.domain probe should evaluate"),
        "SecurityError:true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            replacement_context_id,
            "document.domain = 'example.test'; parent.document.domain",
        )
        .expect("replacement child document.domain access probe should evaluate"),
        "example.test"
    );
    assert_eq!(initial_server.finish_targets().await, vec!["/child.html"]);
    assert_eq!(
        replacement_server.finish_targets().await,
        vec!["/replacement.html"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn one_sided_document_domain_disables_original_tuple_origin_fast_path() {
    const HOST: &str = "www.example.test";

    let server = StaticHttpServer::spawn(1).await;
    let parent_url = server.url_for_host(HOST, "/page.html");
    let child_url = server.url_for_host(HOST, "/child.html");
    let loader = static_http_loader([server.resolve_entry(HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__oneSidedDomainChildUrl = {};",
        serde_json::to_string(child_url.as_str()).expect("serialize document.domain child URL")
    ))
    .expect("document.domain child URL should install");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
globalThis.__oneSidedDomainChildLoaded = false;
frame.onload = () => { globalThis.__oneSidedDomainChildLoaded = true; };
frame.src = globalThis.__oneSidedDomainChildUrl;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__oneSidedDomainFrame = frame;
globalThis.__probeOneSidedDomainFrame = () => {
  try {
    return frame.contentWindow.document.domain;
  } catch (error) {
    return error && error.name;
  }
};
"#,
        None,
    )
    .expect("same-origin document.domain frame setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__oneSidedDomainChildLoaded)",
        "true",
        "same-origin document.domain child should load",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("same-origin child realm should materialize");
    assert_eq!(
        vm.eval("__probeOneSidedDomainFrame()")
            .expect("initial same-origin access probe should evaluate"),
        "www.example.test"
    );
    assert_eq!(
        vm.eval("document.domain = document.domain; __probeOneSidedDomainFrame()")
            .expect("one-sided exact-domain access probe should evaluate"),
        "SecurityError"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const before = (() => {
    try {
      return parent.document.domain;
    } catch (error) {
      return error && error.name;
    }
  })();
  document.domain = document.domain;
  return [before, parent.document.domain].join("|");
})()
"#,
        )
        .expect("two-sided exact-domain access probe should evaluate"),
        "SecurityError|www.example.test"
    );
    assert_eq!(
        vm.eval("__probeOneSidedDomainFrame()")
            .expect("restored exact-domain access probe should evaluate"),
        "www.example.test"
    );
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}

#[tokio::test(flavor = "current_thread")]
async fn lazy_iframe_location_replace_cancels_pending_attribute_navigation() {
    const PARENT_HOST: &str = "lazy-iframe-replace.test";
    const CROSS_ORIGIN_HOST: &str = "cross-origin.test";

    let server = StaticHttpServer::spawn(2).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let same_origin_navigation_url = server.url_for_host(PARENT_HOST, "/same-nav.html");
    let cross_origin_navigation_url = server.url_for_host(CROSS_ORIGIN_HOST, "/nav.html");
    let loader = static_http_loader([
        server.resolve_entry(PARENT_HOST),
        server.resolve_entry(CROSS_ORIGIN_HOST),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__lazyCrossOriginNavigationUrl = {};",
        serde_json::to_string(cross_origin_navigation_url.as_str())
            .expect("serialize lazy cross-origin navigation URL")
    ))
    .expect("lazy cross-origin navigation URL should install");

    vm.exec(
        r#"
const frame = document.createElement('iframe');
frame.loading = 'lazy';
frame.hidden = true;
frame.src = '/src.html';
(document.body || document.documentElement || document).appendChild(frame);
frame.contentWindow.location.replace('data:text/html,<body>navigated</body>');
frame.hidden = false;
globalThis.__lazyReplaceFrame = frame;

const sameOriginFrame = document.createElement('iframe');
sameOriginFrame.loading = 'lazy';
sameOriginFrame.hidden = true;
sameOriginFrame.src = '/same-src.html';
globalThis.__lazySameOriginLoaded = false;
sameOriginFrame.onload = () => { globalThis.__lazySameOriginLoaded = true; };
(document.body || document.documentElement || document).appendChild(sameOriginFrame);
sameOriginFrame.contentWindow.location.replace('/same-nav.html');
sameOriginFrame.hidden = false;
globalThis.__lazySameOriginReplaceFrame = sameOriginFrame;

const crossOriginFrame = document.createElement('iframe');
crossOriginFrame.loading = 'lazy';
crossOriginFrame.hidden = true;
crossOriginFrame.src = '/cross-src.html';
globalThis.__lazyCrossOriginLoaded = false;
crossOriginFrame.onload = () => { globalThis.__lazyCrossOriginLoaded = true; };
(document.body || document.documentElement || document).appendChild(crossOriginFrame);
crossOriginFrame.contentWindow.location.replace(globalThis.__lazyCrossOriginNavigationUrl);
crossOriginFrame.hidden = false;
globalThis.__lazyCrossOriginReplaceFrame = crossOriginFrame;
"#,
        None,
    )
    .expect("lazy iframe replace setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__lazySameOriginLoaded && globalThis.__lazyCrossOriginLoaded)",
        "true",
        "replacement child navigations should complete",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const probeHref = () => {
    try {
      __lazyReplaceFrame.contentWindow.location.href;
      return "no-throw";
    } catch (error) {
      return error && error.name;
    }
  };
  const probeCrossOriginHref = () => {
    try {
      __lazyCrossOriginReplaceFrame.contentWindow.location.href;
      return "no-throw";
    } catch (error) {
      return error && error.name;
    }
  };
  return [
    __lazyReplaceFrame.contentDocument === null,
    probeHref(),
    __lazySameOriginReplaceFrame.contentWindow.location.href,
    __lazyCrossOriginReplaceFrame.contentDocument === null,
    probeCrossOriginHref()
  ].join('|');
})()
"#,
        )
        .expect("lazy iframe replace result should evaluate");

    assert_eq!(
        result,
        format!("true|SecurityError|{same_origin_navigation_url}|true|SecurityError")
    );
    let mut request_targets = server.finish_targets().await;
    request_targets.sort();
    assert_eq!(request_targets, vec!["/nav.html", "/same-nav.html"]);
}

#[tokio::test(flavor = "current_thread")]
async fn isolated_world_universal_access_is_enforced_by_the_central_window_access_policy() {
    const PARENT_HOST: &str = "web-platform.test";
    const CHILD_HOST: &str = "www1.web-platform.test";

    let server = StaticHttpServer::spawn(1).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let child_url = server.url_for_host(CHILD_HOST, "/child.html");
    let child_origin = child_url.origin().ascii_serialization();
    let loader = static_http_loader([server.resolve_entry(CHILD_HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__universalAccessChildUrl = {};",
        serde_json::to_string(child_url.as_str()).expect("serialize universal-access child URL")
    ))
    .expect("universal-access child URL should install");

    vm.exec(
        r#"
const tupleFrame = document.createElement("iframe");
tupleFrame.id = "tuple-frame";
globalThis.__universalAccessTupleLoaded = false;
tupleFrame.onload = () => { globalThis.__universalAccessTupleLoaded = true; };
tupleFrame.src = globalThis.__universalAccessChildUrl;
(document.body || document.documentElement || document).appendChild(tupleFrame);

const opaqueFrame = document.createElement("iframe");
opaqueFrame.id = "opaque-frame";
opaqueFrame.sandbox = "allow-scripts";
opaqueFrame.srcdoc = "<p id='opaque-secret'>opaque child</p>";
(document.body || document.documentElement || document).appendChild(opaqueFrame);
"#,
        None,
    )
    .expect("universal-access child setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__universalAccessTupleLoaded)",
        "true",
        "tuple-origin child realm should materialize",
    )
    .await;

    let denied_context_id = vm
        .create_isolated_world("origin-enforced-utility", false)
        .expect("origin-enforced isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(
            denied_context_id,
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };
  return [
    probe(() => document.getElementById("tuple-frame").contentWindow.document.URL),
    probe(() => document.getElementById("opaque-frame").contentWindow.document.body.textContent)
  ].join("|");
})()
"#,
        )
        .expect("origin-enforced isolated-world probe should evaluate"),
        "SecurityError:true|SecurityError:true"
    );

    let universal_context_id = vm
        .create_isolated_world("universal-utility", true)
        .expect("universal isolated world should be created");
    let universal_result = vm
        .eval_in_isolated_context(
            universal_context_id,
            r#"
[
  document.getElementById("tuple-frame").contentWindow.document.URL,
  document.getElementById("opaque-frame").contentWindow.document.getElementById("opaque-secret").textContent
].join("|")
"#,
        )
        .expect("universal isolated-world probe should evaluate");
    assert_eq!(universal_result, format!("{child_url}|opaque child"),);

    let tuple_child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.origin == child_origin)
        .map(|realm| realm.context_id)
        .expect("tuple-origin child default realm should exist");
    assert_eq!(
        vm.eval_in_child_default_context(
            tuple_child_context_id,
            r#"
(() => {
  try {
    return parent.document.URL;
  } catch (error) {
    return `${error && error.name}:${error instanceof DOMException}`;
  }
})()
"#,
        )
        .expect("child-to-parent access probe should evaluate"),
        "SecurityError:true",
        "universal access must belong only to the requesting isolated realm"
    );
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}

#[tokio::test(flavor = "current_thread")]
async fn child_cross_origin_window_denials_use_the_child_dom_exception_realm() {
    const PARENT_HOST: &str = "web-platform.test";
    const CHILD_HOST: &str = "www1.web-platform.test";

    let server = StaticHttpServer::spawn(1).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let child_url = server.url_for_host(CHILD_HOST, "/child.html");
    let loader = static_http_loader([server.resolve_entry(CHILD_HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__crossOriginChildUrl = {};",
        serde_json::to_string(child_url.as_str()).expect("serialize cross-origin child URL")
    ))
    .expect("cross-origin child URL should install");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
globalThis.__crossOriginChildLoaded = false;
frame.onload = () => { globalThis.__crossOriginChildLoaded = true; };
frame.src = globalThis.__crossOriginChildUrl;
(document.body || document.documentElement || document).appendChild(frame);
"#,
        None,
    )
    .expect("cross-origin child SecurityError realm setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__crossOriginChildLoaded)",
        "true",
        "cross-origin child realm should materialize",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("cross-origin child realm should materialize");

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  "use strict";
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return [
        error && error.name,
        error instanceof DOMException,
        error && error.constructor === DOMException,
        error && error.code
      ].join(":");
    }
  };
  return [
    probe(() => parent.document),
    probe(() => top.localStorage),
    probe(() => parent.location.href),
    probe(() => { parent.document = null; })
  ].join("|");
})()
"#,
        )
        .expect("child cross-origin SecurityError realm probe should evaluate");

    assert_eq!(
        result,
        "SecurityError:true:true:18|SecurityError:true:true:18|SecurityError:true:true:18|SecurityError:true:true:18"
    );
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}

#[test]
fn blob_child_document_accepts_html_mime_parameters() {
    let mut vm = new_storage_test_vm("https://blob-child-html-mime.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const blob = new Blob(
    ['<!DOCTYPE html><body><span id="value">ok</span></body>'],
    { type: 'text/html;charset=utf-8' }
  );
  const frame = document.createElement('iframe');
  frame.src = URL.createObjectURL(blob);
  (document.body || document.documentElement || document).appendChild(frame);
  return frame.contentDocument !== null;
})()
"#,
        )
        .expect("blob child document should materialize");

    assert_eq!(result, "true");
}
#[test]
fn window_event_target_methods_have_browser_lengths() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            [
                window.addEventListener.length,
                window.removeEventListener.length,
                window.dispatchEvent.length,
                addEventListener.length,
                removeEventListener.length,
                dispatchEvent.length
            ].join("|")
            "#,
        )
        .expect("window EventTarget method lengths should evaluate");

    assert_eq!(result, "2|2|1|2|2|1");
}

#[test]
fn child_browsing_context_lookup_tolerates_document_handle_cycle() {
    let mut vm = new_storage_test_vm("https://child-lookup-cycle.test/");

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.name = "targetFrame";
  (document.body || document.documentElement || document).appendChild(iframe);
  return "ready";
})()
"#,
    )
    .expect("cycle lookup fixture should evaluate");

    let (document, handle) = {
        let host = vm._context_host.borrow();
        (
            host.document_handle(),
            host.child_browsing_context_handle_by_index(0)
                .expect("test iframe should have a child browsing context"),
        )
    };
    vm._context_host
        .borrow_mut()
        .set_child_browsing_context_document_handle_for_test(handle, document);

    let handles = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order();
    assert_eq!(handles, vec![handle]);
    assert_eq!(
        vm._context_host
            .borrow()
            .child_browsing_context_handle_by_name("missingFrame"),
        None
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .child_browsing_context_handle_by_name("targetFrame"),
        Some(handle)
    );
}
