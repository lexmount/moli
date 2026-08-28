use super::*;

#[test]
fn top_level_history_back_delegates_to_the_browser_history_controller() {
    let url = "https://browser-owned-history.test/current";
    let mut vm = new_storage_test_vm(url);

    vm.eval("history.back(); 'queued'")
        .expect("hidden renderer bookkeeping entry should delegate to the browser");
    assert!(!vm.has_pending_location_navigation());
    assert_eq!(
        vm.take_pending_top_level_history_traversal()
            .expect("browser-owned back traversal request")
            .delta(),
        -1
    );

    vm.install_navigation_bootstrap_entry(Some(moli_page_types::initial_navigation_history_seed(
        false, url,
    )));
    assert_eq!(
        vm.eval("String(history.length)")
            .expect("single renderer-visible history entry"),
        "1"
    );
    vm.eval("history.back(); 'queued'")
        .expect("unknown browser-side back entry should queue");

    assert!(!vm.has_pending_location_navigation());
    assert_eq!(
        vm.take_pending_top_level_history_traversal()
            .expect("browser-owned back traversal request")
            .delta(),
        -1
    );

    vm.eval("history.forward(); 'queued'")
        .expect("unknown browser-side forward entry should queue");
    assert_eq!(
        vm.take_pending_top_level_history_traversal()
            .expect("browser-owned forward traversal request")
            .delta(),
        1
    );
}

#[test]
fn top_level_cross_document_history_traversal_carries_the_exact_destination_seed() {
    let first_url =
        url::Url::parse("https://browser-owned-history.test/first").expect("first history URL");
    let second_url =
        url::Url::parse("https://browser-owned-history.test/second").expect("second history URL");
    let initial_seed = moli_page_types::initial_navigation_history_seed(false, first_url.as_str());
    let current_seed = moli_page_types::cross_document_navigation_seed(
        initial_seed.entries,
        initial_seed.current_index,
        0,
        &second_url,
        moli_page_types::NavigationHistoryMutation::Push,
    );
    let mut vm = new_storage_test_vm(second_url.as_str());
    vm.install_navigation_bootstrap_entry(Some(current_seed));

    vm.eval("history.back(); 'queued'")
        .expect("cross-Document history traversal should queue");
    let traversal = vm
        .take_pending_top_level_history_traversal()
        .expect("cross-Document history traversal owner action");

    assert_eq!(traversal.delta(), -1);
    let (destination_url, destination_seed) = traversal
        .cross_document_destination()
        .expect("known cross-Document target should carry its exact seed");
    assert_eq!(destination_url, first_url.as_str());
    assert_eq!(destination_seed.current_index, 0);
    assert_eq!(destination_seed.entries.len(), 2);
    assert_eq!(
        destination_seed
            .activation
            .as_ref()
            .and_then(|activation| activation.navigation_type.as_deref()),
        Some("traverse")
    );
}

#[test]
fn top_level_navigation_intents_keep_one_last_writer_state() {
    let url = "https://browser-owned-history.test/current";
    let mut vm = new_storage_test_vm(url);
    vm.install_navigation_bootstrap_entry(Some(moli_page_types::initial_navigation_history_seed(
        false, url,
    )));

    vm.eval("history.back(); location.assign('/location-wins'); 'queued'")
        .expect("location navigation should replace the history traversal intent");
    assert!(vm.take_pending_top_level_history_traversal().is_none());
    assert_eq!(
        vm.take_pending_location_navigation_with_seed()
            .expect("location navigation intent")
            .url
            .as_str(),
        "https://browser-owned-history.test/location-wins"
    );

    vm.eval("location.assign('/history-loses-location'); history.forward(); 'queued'")
        .expect("history traversal should replace the location navigation intent");
    assert!(!vm.has_pending_location_navigation());
    assert_eq!(
        vm.take_pending_top_level_history_traversal()
            .expect("history traversal intent")
            .delta(),
        1
    );
}

#[test]
fn traversal_factories_apply_webidl_boundaries() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-webidl.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.createElement("div");
              const child = document.createElement("span");
              root.appendChild(child);
              document.body.appendChild(root);
              const out = [];
              const filter = { acceptNode() { return NodeFilter.FILTER_ACCEPT; } };
              const iterator = document.createNodeIterator(
                root,
                { valueOf() { out.push("iterator-show"); return NodeFilter.SHOW_ELEMENT; } },
                filter
              );
              out.push(iterator.whatToShow === NodeFilter.SHOW_ELEMENT);
              out.push(iterator.filter === filter);
              const walker = document.createTreeWalker(root, undefined, null);
              out.push(walker.whatToShow === NodeFilter.SHOW_ALL);
              out.push(walker.filter === null);

              const checks = [];
              function record(label, callback) {
                try {
                  callback();
                  checks.push(`${label}:no`);
                } catch (error) {
                  checks.push(`${label}:${error.name}`);
                }
              }
              record("iterator-missing-root", () => document.createNodeIterator());
              record("iterator-symbol-show", () => document.createNodeIterator(root, Symbol()));
              record("iterator-throw-show", () => document.createNodeIterator(root, { valueOf() { throw new RangeError("show"); } }));
              record("iterator-primitive-filter", () => document.createNodeIterator(root, NodeFilter.SHOW_ALL, 7));
              record("walker-missing-root", () => document.createTreeWalker());
              record("walker-symbol-show", () => document.createTreeWalker(root, Symbol()));
              record("walker-bad-root", () => document.createTreeWalker(undefined));
              return `${out.join(",")}|${checks.join(",")}`;
            })()
            "#,
        )
        .expect("traversal factory WebIDL boundary probe should evaluate");

    assert_eq!(
        result,
        "iterator-show,true,true,true,true|iterator-missing-root:TypeError,iterator-symbol-show:TypeError,iterator-throw-show:RangeError,iterator-primitive-filter:TypeError,walker-missing-root:TypeError,walker-symbol-show:TypeError,walker-bad-root:TypeError"
    );
}
#[test]
fn traversal_node_filter_constants_and_current_node_setter_match_webidl() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-node-filter-current-node.test/",
        "<!doctype html><html><body><div id='root'><span></span></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.createElement("div");
              root.appendChild(document.createElement("p"));
              document.body.appendChild(root);
              const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
              const constants = [
                NodeFilter.FILTER_ACCEPT,
                NodeFilter.FILTER_REJECT,
                NodeFilter.FILTER_SKIP,
                NodeFilter.SHOW_ALL,
                NodeFilter.SHOW_ELEMENT,
                NodeFilter.SHOW_TEXT,
                typeof NodeFilter === "function"
              ].join(",");
              const setterErrors = [];
              for (const value of [null, {}, window]) {
                try {
                  walker.currentNode = value;
                  setterErrors.push("no");
                } catch (error) {
                  setterErrors.push(error.name);
                }
              }
              walker.currentNode = root.firstChild;
              return [
                constants,
                setterErrors.join(","),
                walker.currentNode === root.firstChild
              ].join("|");
            })()
            "#,
        )
        .expect("traversal constants and currentNode setter checks should evaluate");

    assert_eq!(
        result,
        "1,2,3,4294967295,1,4,true|TypeError,TypeError,TypeError|true"
    );
}

#[test]
fn node_filter_global_property_and_constants_are_declared() {
    let mut vm = new_parsed_test_vm(
        "https://node-filter-declared-surface.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const constants = [
                ["FILTER_ACCEPT", 1],
                ["FILTER_REJECT", 2],
                ["FILTER_SKIP", 3],
                ["SHOW_ALL", 0xFFFFFFFF],
                ["SHOW_ELEMENT", 0x1],
                ["SHOW_ATTRIBUTE", 0x2],
                ["SHOW_TEXT", 0x4],
                ["SHOW_CDATA_SECTION", 0x8],
                ["SHOW_ENTITY_REFERENCE", 0x10],
                ["SHOW_ENTITY", 0x20],
                ["SHOW_PROCESSING_INSTRUCTION", 0x40],
                ["SHOW_COMMENT", 0x80],
                ["SHOW_DOCUMENT", 0x100],
                ["SHOW_DOCUMENT_TYPE", 0x200],
                ["SHOW_DOCUMENT_FRAGMENT", 0x400],
                ["SHOW_NOTATION", 0x800]
              ];
              const globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "NodeFilter");
              const constantShape = (name, expected) => {
                const descriptor = Object.getOwnPropertyDescriptor(NodeFilter, name);
                return [
                  name,
                  descriptor && descriptor.value,
                  descriptor && descriptor.value === expected,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.writable,
                  descriptor && descriptor.configurable
                ].join(":");
              };
              return JSON.stringify({
                global: [
                  typeof globalDescriptor.value,
                  globalDescriptor.enumerable,
                  globalDescriptor.writable,
                  globalDescriptor.configurable,
                  globalDescriptor.value === NodeFilter
                ].join(":"),
                constants: constants.map(([name, value]) => constantShape(name, value)),
                keys: Object.keys(NodeFilter),
                windowKeyVisible: Object.keys(window).includes("NodeFilter"),
                prototypeIsFunction: Object.getPrototypeOf(NodeFilter) === Function.prototype,
                hasPrototypeProperty: Object.hasOwn(NodeFilter, "prototype"),
                name: NodeFilter.name,
                length: NodeFilter.length,
                callThrows: (() => { try { NodeFilter(); return false; } catch (error) { return error instanceof TypeError; } })(),
                constructThrows: (() => { try { new NodeFilter(); return false; } catch (error) { return error instanceof TypeError; } })()
              });
            })()
            "#,
        )
        .expect("NodeFilter declaration surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"global":"function:false:true:true:true","constants":["FILTER_ACCEPT:1:true:true:false:false","FILTER_REJECT:2:true:true:false:false","FILTER_SKIP:3:true:true:false:false","SHOW_ALL:4294967295:true:true:false:false","SHOW_ELEMENT:1:true:true:false:false","SHOW_ATTRIBUTE:2:true:true:false:false","SHOW_TEXT:4:true:true:false:false","SHOW_CDATA_SECTION:8:true:true:false:false","SHOW_ENTITY_REFERENCE:16:true:true:false:false","SHOW_ENTITY:32:true:true:false:false","SHOW_PROCESSING_INSTRUCTION:64:true:true:false:false","SHOW_COMMENT:128:true:true:false:false","SHOW_DOCUMENT:256:true:true:false:false","SHOW_DOCUMENT_TYPE:512:true:true:false:false","SHOW_DOCUMENT_FRAGMENT:1024:true:true:false:false","SHOW_NOTATION:2048:true:true:false:false"],"keys":["FILTER_ACCEPT","FILTER_REJECT","FILTER_SKIP","SHOW_ALL","SHOW_ELEMENT","SHOW_ATTRIBUTE","SHOW_TEXT","SHOW_CDATA_SECTION","SHOW_ENTITY_REFERENCE","SHOW_ENTITY","SHOW_PROCESSING_INSTRUCTION","SHOW_COMMENT","SHOW_DOCUMENT","SHOW_DOCUMENT_TYPE","SHOW_DOCUMENT_FRAGMENT","SHOW_NOTATION"],"windowKeyVisible":false,"prototypeIsFunction":true,"hasPrototypeProperty":false,"name":"NodeFilter","length":0,"callThrows":true,"constructThrows":true}"#
    );
}

#[test]
fn traversal_filter_object_requires_callable_accept_node_during_traversal() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-filter-object-webidl.test/",
        "<!doctype html><html><body><div id='root'><span></span></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.getElementById("root");
              const missing = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {});
              const nonCallable = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                { acceptNode: "no" }
              );
              const getterError = { name: "getter" };
              const throwingGetter = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                { get acceptNode() { throw getterError; } }
              );
              const thrown = [];
              for (const walker of [missing, nonCallable]) {
                try {
                  walker.firstChild();
                  thrown.push("no");
                } catch (error) {
                  thrown.push(error.name);
                }
                thrown.push(walker.currentNode === root);
              }
              try {
                throwingGetter.firstChild();
                thrown.push("no");
              } catch (error) {
                thrown.push(error === getterError);
              }
              thrown.push(throwingGetter.currentNode === root);
              return thrown.join("|");
            })()
            "#,
        )
        .expect("traversal filter object WebIDL checks should evaluate");

    assert_eq!(result, "TypeError|true|TypeError|true|true|true");
}

#[test]
fn traversal_filter_uses_webidl_callback_interface_invocation_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-filter-callback-interface.test/",
        "<!doctype html><html><body><div id='root'><span id='a'></span><span id='b'></span></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.getElementById("root");
              let callableThis = "unset";
              let forbiddenOperationCalls = 0;
              function callableFilter() {
                "use strict";
                callableThis = this;
                // Web IDL converts the callback result to unsigned short.
                return 0x10001;
              }
              callableFilter.acceptNode = () => {
                forbiddenOperationCalls++;
                throw new Error("callable branch must ignore acceptNode");
              };
              const iterator = document.createNodeIterator(
                root,
                NodeFilter.SHOW_ELEMENT,
                callableFilter
              );
              const callableAccepted = iterator.nextNode() === root;

              let getterCalls = 0;
              let objectReceiver = false;
              const objectFilter = {
                get acceptNode() {
                  getterCalls++;
                  return function() {
                    objectReceiver = this === objectFilter;
                    return NodeFilter.FILTER_ACCEPT;
                  };
                }
              };
              const walker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                objectFilter
              );
              const first = walker.nextNode();
              const second = walker.nextNode();

              const replaceable = {
                acceptNode() {
                  throw new Error("registration-time operation snapshot");
                }
              };
              const replacementWalker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                replaceable
              );
              replaceable.acceptNode = () => NodeFilter.FILTER_ACCEPT;
              const replacementAccepted =
                replacementWalker.firstChild() === root.firstChild;

              const revocable = Proxy.revocable(
                () => NodeFilter.FILTER_ACCEPT,
                {}
              );
              const proxyWalker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                revocable.proxy
              );
              const proxyAccepted = proxyWalker.firstChild() === root.firstChild;
              revocable.revoke();
              let revokedProxyThrew = false;
              try {
                proxyWalker.nextSibling();
              } catch (error) {
                revokedProxyThrew = error instanceof TypeError;
              }

              const conversionError = { source: "return conversion" };
              const abrupt = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                () => ({
                  valueOf() {
                    throw conversionError;
                  }
                })
              );
              let preservedConversionError = false;
              try {
                abrupt.firstChild();
              } catch (error) {
                preservedConversionError = error === conversionError;
              }

              return [
                callableAccepted,
                callableThis === undefined,
                forbiddenOperationCalls,
                first && first.id,
                second && second.id,
                getterCalls,
                objectReceiver,
                replacementAccepted,
                proxyAccepted,
                revokedProxyThrew,
                preservedConversionError,
                abrupt.currentNode === root
              ].join("|");
            })()
            "#,
        )
        .expect("NodeFilter callback-interface semantics should evaluate");

    assert_eq!(result, "true|true|0|a|b|2|true|true|true|true|true|true");
}

#[test]
fn traversal_filter_uses_callback_relevant_realm_and_exact_window_lifetime() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-filter-realm.test/",
        "<!doctype html><html><body><div id='root'><span></span></div></body></html>",
    );

    vm.eval(
        r#"
        (() => {
          const iframe = document.createElement("iframe");
          iframe.srcdoc = "<!doctype html><html><body></body></html>";
          document.body.appendChild(iframe);
          globalThis.__nodeFilterRealmFrame = iframe;
          return "ready";
        })()
        "#,
    )
    .expect("cross-Realm NodeFilter setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const other = globalThis.__nodeFilterRealmFrame.contentWindow;
              const root = document.getElementById("root");
              const missingOperation = new other.Object();
              const missingWalker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                missingOperation
              );
              let relevantTypeError = false;
              try {
                missingWalker.firstChild();
              } catch (error) {
                relevantTypeError =
                  error instanceof other.TypeError && !(error instanceof TypeError);
              }

              globalThis.__nodeFilterExpectedRealm = other;
              globalThis.__nodeFilterCallFacts = [];
              const crossRealmCallable = other.Function(
                "node",
                `"use strict";
                 parent.__nodeFilterCallFacts = [
                   this === undefined,
                   globalThis === parent.__nodeFilterExpectedRealm,
                   node.localName
                 ];
                 return 1;`
              );
              const callableWalker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                crossRealmCallable
              );
              const accepted = callableWalker.firstChild();

              return [
                relevantTypeError,
                missingWalker.currentNode === root,
                accepted && accepted.localName,
                ...globalThis.__nodeFilterCallFacts
              ].join("|");
            })()
            "#,
        )
        .expect("cross-Realm NodeFilter invocation should evaluate");

    assert_eq!(result, "true|true|span|true|true|span");
}

#[test]
fn traversal_filter_false_and_zero_are_not_accept() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-filter-coercion.test/",
        "<!doctype html><html><body><div id='root'><span id='a'></span><span id='b'></span></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.getElementById("root");
              const out = [];

              const falseIterator = document.createNodeIterator(
                root,
                NodeFilter.SHOW_ELEMENT,
                () => false
              );
              out.push(falseIterator.nextNode() === null);
              out.push(falseIterator.referenceNode === root);
              out.push(falseIterator.pointerBeforeReferenceNode === true);

              const zeroWalker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                () => 0
              );
              out.push(zeroWalker.firstChild() === null);
              out.push(zeroWalker.currentNode === root);

              const trueIterator = document.createNodeIterator(
                root,
                NodeFilter.SHOW_ELEMENT,
                () => true
              );
              out.push(trueIterator.nextNode() === root);

              return out.join(",");
            })()
            "#,
        )
        .expect("traversal filter coercion checks should evaluate");

    assert_eq!(result, "true,true,true,true,true,true");
}
#[test]
fn traversal_sibling_walk_descends_through_non_reject_filter_results() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-sibling-filter.test/",
        "<!doctype html><html><body><div id='root'><p><span>skip</span></p><section>target</section></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.getElementById("root");
              const walker = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ALL,
                node => node.nodeType === Node.TEXT_NODE
              );
              walker.currentNode = root.firstChild;
              const next = walker.nextSibling();
              return [
                next && next.nodeType,
                next && next.data,
                walker.currentNode === next
              ].join("|");
            })()
            "#,
        )
        .expect("TreeWalker sibling non-reject descent check should evaluate");

    assert_eq!(result, "3|target|true");
}
#[test]
fn traversal_filter_from_removed_child_window_throws_before_callback() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-detached-child-filter.test/",
        "<!doctype html><html><body><div></div></body></html>",
    );

    vm.eval(
        r#"
        (() => {
          const iframe = document.createElement("iframe");
          iframe.srcdoc = `<script>
            function createIterator() {
              function filter() {
                iterator.dummyFilterCalled = true;
                return true;
              }
              const iterator = parent.document.createNodeIterator(parent.document.body, NodeFilter.SHOW_ELEMENT, filter);
              iterator.dummyFilterCalled = false;
              return iterator;
            }
          <\/script>`;
          document.body.appendChild(iframe);
          globalThis.__traversalChildFrame = iframe;
          return "ready";
        })()
        "#,
    )
    .expect("detached child filter setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const iframe = globalThis.__traversalChildFrame;
              const iterator = iframe.contentWindow.createIterator();
              iframe.remove();
              let threw = false;
              try {
                iterator.nextNode();
              } catch (error) {
                threw = true;
              }
              return [
                iframe.contentWindow === null,
                threw,
                iterator.dummyFilterCalled
              ].join("|");
            })()
            "#,
        )
        .expect("detached child filter traversal should evaluate");

    assert_eq!(result, "true|true|false");
}
#[test]
fn traversal_filter_ignores_public_child_context_marker_spoof() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-filter-marker-spoof.test/",
        "<!doctype html><html><body><div id='root'><p></p></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document;
              const root = doc.createElement("div");
              root.appendChild(doc.createElement("p"));
              doc.body.appendChild(root);
              window.__moliChildBrowsingContextHandle = 999999;
              let calls = 0;
              const iterator = doc.createNodeIterator(root, NodeFilter.SHOW_ELEMENT, {
                acceptNode() {
                  calls++;
                  return NodeFilter.FILTER_ACCEPT;
                }
              });
              let threw = false;
              let first = null;
              try {
                first = iterator.nextNode();
              } catch (error) {
                threw = true;
              }
              delete window.__moliChildBrowsingContextHandle;
              return [threw, first === root, calls].join("|");
            })()
            "#,
        )
        .expect("public child context marker spoof check should evaluate");

    assert_eq!(result, "false|true|1");
}
#[test]
fn node_iterator_reference_tracks_removed_subtree() {
    let mut vm = new_parsed_test_vm(
        "https://node-iterator-removal.test/",
        "<!doctype html><html><body><div id='root'><section id='a'><em id='x'></em></section><p id='b'></p></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.getElementById("root");
              const a = document.getElementById("a");
              const x = document.getElementById("x");
              const b = document.getElementById("b");
              const out = [];

              const afterReference = document.createNodeIterator(root);
              afterReference.nextNode(); // root, pointer after
              afterReference.nextNode(); // a, pointer after
              afterReference.nextNode(); // x, pointer after
              out.push(afterReference.referenceNode === x);
              a.remove();
              out.push(afterReference.referenceNode === root);
              out.push(afterReference.pointerBeforeReferenceNode === false);
              out.push(afterReference.nextNode() === b);

              root.insertBefore(a, b);
              const beforeReference = document.createNodeIterator(root);
              beforeReference.nextNode(); // root, pointer after
              beforeReference.nextNode(); // a, pointer after
              beforeReference.previousNode(); // a, pointer before
              out.push(beforeReference.referenceNode === a);
              out.push(beforeReference.pointerBeforeReferenceNode === true);
              a.remove();
              out.push(beforeReference.referenceNode === b);
              out.push(beforeReference.pointerBeforeReferenceNode === true);
              out.push(beforeReference.nextNode() === b);

              return out.join(",");
            })()
            "#,
        )
        .expect("node iterator removal tracking checks should evaluate");

    assert_eq!(result, "true,true,true,true,true,true,true,true,true");
}
#[test]
fn tree_walker_previous_node_handles_regrafted_current_node() {
    let mut vm = new_parsed_test_vm(
        "https://tree-walker-regraft.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.createElement("div");
              const head = document.createElement("head");
              const title = document.createElement("title");
              const body = document.createElement("body");
              const p = document.createElement("p");
              doc.appendChild(head);
              head.appendChild(title);
              doc.appendChild(body);
              body.appendChild(p);

              const walker = document.createTreeWalker(body, NodeFilter.SHOW_ALL, null);
              doc.removeChild(body);
              const out = [];
              out.push(walker.lastChild() === p);
              doc.appendChild(p);
              out.push(walker.previousNode() === title);
              p.appendChild(body);
              out.push(walker.nextNode() === p);
              out.push(walker.nextNode() === body);
              out.push(walker.previousNode() === null);
              return out.join(",");
            })()
            "#,
        )
        .expect("tree walker regraft checks should evaluate");

    assert_eq!(result, "true,true,true,true,true");
}
#[test]
fn traversal_filter_errors_and_node_filter_surface_match_webidl() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-filter-errors.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.createElement("div");
              const child = document.createElement("span");
              root.appendChild(child);
              document.body.appendChild(root);

              const out = [
                "FILTER_ACCEPT" in NodeFilter,
                NodeFilter.FILTER_ACCEPT === 1,
                NodeFilter.SHOW_ALL === 0xFFFFFFFF,
                Object.getPrototypeOf(NodeFilter) === Function.prototype,
                typeof NodeFilter === "function"
              ];

              function record(label, callback) {
                try {
                  callback();
                  out.push(`${label}:no`);
                } catch (error) {
                  out.push(`${label}:${error.name}`);
                }
              }

              const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, null);
              record("current-null", () => { walker.currentNode = null; });
              out.push(walker.currentNode === root);

              const missing = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {});
              record("missing-acceptNode", () => missing.firstChild());
              out.push(missing.currentNode === root);

              const nonCallable = document.createTreeWalker(
                root,
                NodeFilter.SHOW_ELEMENT,
                { acceptNode: "no" }
              );
              record("noncallable-acceptNode", () => nonCallable.firstChild());
              out.push(nonCallable.currentNode === root);

              return out.join(",");
            })()
            "#,
        )
        .expect("traversal filter error probe should evaluate");

    assert_eq!(
        result,
        "true,true,true,true,true,current-null:TypeError,true,missing-acceptNode:TypeError,true,noncallable-acceptNode:TypeError,true"
    );
}
#[test]
fn traversal_from_live_root_preserves_adopted_detached_wrapper_identity() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-adopted-detached.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const source = new Document();
              const first = source.createCDATASection("first");
              const second = source.createCDATASection("second");
              const parent = document.createElement("p");
              parent.appendChild(first);
              parent.appendChild(second);
              document.body.appendChild(parent);

              const filterIdentities = [];
              const iterator = document.createNodeIterator(
                document,
                NodeFilter.SHOW_CDATA_SECTION,
                node => {
                  filterIdentities.push(node === first || node === second);
                  return NodeFilter.FILTER_ACCEPT;
                }
              );
              const firstResult = iterator.nextNode();
              const secondResult = iterator.nextNode();

              const walkerFilterIdentities = [];
              const walker = document.createTreeWalker(
                document,
                NodeFilter.SHOW_CDATA_SECTION,
                node => {
                  walkerFilterIdentities.push(node === first || node === second);
                  return NodeFilter.FILTER_ACCEPT;
                }
              );
              const firstWalked = walker.nextNode();
              const secondWalked = walker.nextNode();
              return [
                parent.firstChild === first,
                parent.lastChild === second,
                firstResult === first,
                secondResult === second,
                iterator.referenceNode === second,
                filterIdentities.length === 2 && filterIdentities.every(Boolean),
                firstWalked === first,
                secondWalked === second,
                walker.currentNode === second,
                walkerFilterIdentities.length === 2 && walkerFilterIdentities.every(Boolean)
              ].join(",");
            })()
            "#,
        )
        .expect("adopted detached traversal wrapper identity probe should evaluate");

    assert_eq!(result, "true,true,true,true,true,true,true,true,true,true");
}
#[test]
fn traversal_results_preserve_foreign_wrapper_identity() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-foreign.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = new DOMParser().parseFromString(
                "<html><body><div id='r'><span id='a'></span><span id='b'></span></div></body></html>",
                "text/html"
              );
              const root = doc.getElementById("r");
              const a = doc.getElementById("a");
              const b = doc.getElementById("b");

              const out = [];
              const iter = document.createNodeIterator(root, NodeFilter.SHOW_ELEMENT, null);
              out.push(iter.root === root);
              out.push(iter.referenceNode === root);
              const n1 = iter.nextNode();
              out.push(n1 === root);
              const n2 = iter.nextNode();
              out.push(n2 === a);
              const n3 = iter.nextNode();
              out.push(n3 === b);
              out.push(iter.referenceNode === b);
              const p = iter.previousNode();
              out.push(p === b);

              const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, null);
              out.push(walker.root === root);
              out.push(walker.currentNode === root);
              const fc = walker.firstChild();
              out.push(fc === a);
              out.push(walker.currentNode === a);
              const ns = walker.nextSibling();
              out.push(ns === b);
              const par = walker.parentNode();
              out.push(par === root);
              return out.join(",");
            })()
            "#,
        )
        .expect("foreign-root traversal identity probe should evaluate");

    assert_eq!(
        result,
        "true,true,true,true,true,true,true,true,true,true,true,true,true"
    );
}
#[test]
fn traversal_foreign_materialization_preserves_source_tree_and_removal_cursors() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-foreign-removal.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const foreignDoc = document.implementation.createHTMLDocument("");
              const p1 = foreignDoc.createElement("p");
              p1.appendChild(foreignDoc.createTextNode("one"));
              const p2 = foreignDoc.createElement("p");
              p2.appendChild(foreignDoc.createTextNode("two"));
              foreignDoc.body.appendChild(p1);
              foreignDoc.body.appendChild(p2);

              const sourceProbe = document.createNodeIterator(p1, NodeFilter.SHOW_ALL, null);
              sourceProbe.nextNode();
              sourceProbe.nextNode();
              const sourceStillLinked =
                p1.parentNode === foreignDoc.body &&
                foreignDoc.body.firstChild === p1 &&
                p1.nextSibling === p2;

              const docIterator = document.createNodeIterator(foreignDoc, NodeFilter.SHOW_ALL, null);
              for (let i = 0; i < 8; i++) docIterator.nextNode();
              foreignDoc.body.removeChild(p1);
              const docCursorAdjusted =
                docIterator.referenceNode === foreignDoc.body &&
                docIterator.pointerBeforeReferenceNode === false;

              const childIterator = document.createNodeIterator(p2, NodeFilter.SHOW_ALL, null);
              childIterator.nextNode();
              childIterator.nextNode();
              const text = p2.firstChild;
              p2.removeChild(text);
              const childCursorAdjusted =
                childIterator.referenceNode === p2 &&
                childIterator.pointerBeforeReferenceNode === false;

              return [sourceStillLinked, docCursorAdjusted, childCursorAdjusted].join(",");
            })()
            "#,
        )
        .expect("foreign traversal removal probe should evaluate");

    assert_eq!(result, "true,true,true");
}
#[test]
fn node_iterator_filter_runs_incrementally_per_step() {
    let mut vm = new_parsed_test_vm(
        "https://node-iterator-incremental.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.createElement("div");
              root.id = "r";
              for (const id of ["a", "b", "c"]) {
                const child = document.createElement("span");
                child.id = id;
                root.appendChild(child);
              }
              document.body.appendChild(root);

              const calls = [];
              const iterator = document.createNodeIterator(
                root,
                NodeFilter.SHOW_ELEMENT,
                {
                  acceptNode(node) {
                    calls.push(node.id);
                    return NodeFilter.FILTER_ACCEPT;
                  }
                }
              );

              const first = iterator.nextNode();
              const firstCalls = calls.slice();
              const second = iterator.nextNode();
              const secondCalls = calls.slice();
              calls.length = 0;
              const previous = iterator.previousNode();

              return [
                first.id,
                firstCalls.join(","),
                second.id,
                secondCalls.join(","),
                previous.id,
                calls.join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("NodeIterator incremental filter probe should evaluate");

    assert_eq!(result, "r|r|a|r,a|a|a");
}
#[test]
fn node_iterator_reference_node_updates_before_removing_subtree() {
    let mut vm = new_parsed_test_vm(
        "https://node-iterator-removal.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const out = [];

              {
                const root = document.createElement("div");
                root.innerHTML = "<section id='a'><span id='b'></span></section><p id='c'></p>";
                document.body.appendChild(root);
                const a = root.querySelector("#a");
                const b = root.querySelector("#b");
                const iter = document.createNodeIterator(root, NodeFilter.SHOW_ELEMENT);
                iter.nextNode();
                iter.nextNode();
                iter.nextNode();
                if (iter.referenceNode !== b || iter.pointerBeforeReferenceNode) {
                  return "setup-after";
                }
                a.remove();
                out.push(`after:${iter.referenceNode === root}:${iter.pointerBeforeReferenceNode}`);
                root.remove();
              }

              {
                const root = document.createElement("div");
                root.innerHTML = "<section id='a'><span id='b'></span></section><p id='c'></p>";
                document.body.appendChild(root);
                const a = root.querySelector("#a");
                const b = root.querySelector("#b");
                const c = root.querySelector("#c");
                const iter = document.createNodeIterator(root, NodeFilter.SHOW_ELEMENT);
                iter.nextNode();
                iter.nextNode();
                iter.nextNode();
                iter.previousNode();
                if (iter.referenceNode !== b || !iter.pointerBeforeReferenceNode) {
                  return "setup-before-next";
                }
                a.remove();
                out.push(`before-next:${iter.referenceNode === c}:${iter.pointerBeforeReferenceNode}`);
                root.remove();
              }

              {
                const root = document.createElement("div");
                root.innerHTML = "<section id='a'><span id='b'></span></section>";
                document.body.appendChild(root);
                const a = root.querySelector("#a");
                const b = root.querySelector("#b");
                const iter = document.createNodeIterator(root, NodeFilter.SHOW_ELEMENT);
                iter.nextNode();
                iter.nextNode();
                iter.nextNode();
                iter.previousNode();
                if (iter.referenceNode !== b || !iter.pointerBeforeReferenceNode) {
                  return "setup-before-none";
                }
                a.remove();
                out.push(`before-none:${iter.referenceNode === root}:${iter.pointerBeforeReferenceNode}`);
                root.remove();
              }

              {
                const outer = document.createElement("div");
                outer.innerHTML = "<section id='a'><span id='b'></span></section>";
                document.body.appendChild(outer);
                const root = outer.querySelector("#a");
                const b = outer.querySelector("#b");
                const iter = document.createNodeIterator(root, NodeFilter.SHOW_ELEMENT);
                iter.nextNode();
                iter.nextNode();
                if (iter.referenceNode !== b || iter.pointerBeforeReferenceNode) {
                  return "setup-ancestor-root";
                }
                outer.remove();
                out.push(`ancestor-root:${iter.referenceNode === b}:${iter.pointerBeforeReferenceNode}`);
              }

              return out.join("|");
            })()
            "##,
        )
        .expect("NodeIterator removal fixup probe should evaluate");

    assert_eq!(
        result,
        "after:true:false|before-next:true:true|before-none:true:false|ancestor-root:true:false"
    );
}
#[test]
fn tree_walker_uses_child_document_wrappers_for_child_document_roots() {
    let mut vm = new_storage_test_vm("https://traversal-child-realm.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const iframe = document.createElement("iframe");
              iframe.srcdoc = "<!DOCTYPE html>";
              (document.body || document.documentElement || document).appendChild(iframe);
              const walker = document.createTreeWalker(
                iframe.contentDocument,
                NodeFilter.SHOW_ELEMENT
              );
              const next = walker.nextNode();
              const iterator = document.createNodeIterator(
                iframe.contentDocument,
                NodeFilter.SHOW_ELEMENT
              );
              const iteratorElement = iterator.nextNode();

              let acceptNodeNode;
              const filtered = document.createTreeWalker(
                iframe.contentDocument,
                NodeFilter.SHOW_ELEMENT,
                {
                  acceptNode(node) {
                    acceptNodeNode = node;
                    return NodeFilter.FILTER_ACCEPT;
                  }
                }
              );
              const filteredNext = filtered.nextNode();

              return [
                next && next.localName,
                walker.currentNode === next,
                walker.currentNode instanceof iframe.contentWindow.Node,
                iteratorElement && iteratorElement.localName,
                iteratorElement instanceof iframe.contentWindow.Node,
                filteredNext && filteredNext.localName,
                acceptNodeNode && acceptNodeNode.localName,
                acceptNodeNode instanceof iframe.contentWindow.Node,
                filtered.currentNode instanceof iframe.contentWindow.Node
              ].join("|");
            })()
            "#,
        )
        .expect("child document TreeWalker realm probe should evaluate");

    assert_eq!(result, "html|true|true|html|true|html|html|true|true");
}
#[tokio::test]
async fn interrupted_intercepted_traversal_rejects_finished_after_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmInterruptedTraversal = { log: [] };
              navigation.navigate("#one");
              navigation.onnavigate = event => event.intercept();
              navigation.oncurrententrychange = event => {
                if (event.navigationType === "traverse") {
                  globalThis.__lmInterruptedTraversal.log.push(`change:${location.hash}`);
                  navigation.navigate("#two");
                }
              };
              const result = navigation.back();
              result.committed.then(
                () => globalThis.__lmInterruptedTraversal.log.push(`committed:${location.hash}`),
                error => globalThis.__lmInterruptedTraversal.log.push(`committed-rejected:${error.name}`)
              );
              result.finished.then(
                () => globalThis.__lmInterruptedTraversal.log.push("finished"),
                error => globalThis.__lmInterruptedTraversal.log.push(`finished-rejected:${error.name}:${location.hash}`)
              );
              return globalThis.__lmInterruptedTraversal.log.join("|");
            })()
            "##,
        )
        .expect("interrupted traversal setup should evaluate");
    assert_eq!(setup, "");

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued interrupted traversal should run")
    );
    let settled = vm
        .eval("globalThis.__lmInterruptedTraversal.log.join('|')")
        .expect("interrupted traversal settlement should evaluate");
    assert_eq!(
        settled,
        "change:|committed:#two|finished-rejected:AbortError:#two"
    );
}

#[tokio::test]
async fn traversal_intercept_settlement_promise_ignores_prototype_setter_spoof() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmTraversalPrivatePayload = { log: [] };
              const log = globalThis.__lmTraversalPrivatePayload.log;
              const record = name => log.push(`${name}:${location.hash}`);
              Object.defineProperties(Object.prototype, {
                __lmTraversalInterceptActive: { configurable: true, value: false },
                __lmTraversalInterceptUrl: { configurable: true, value: "spoof-url" },
                __lmTraversalInterceptPromise: {
                  configurable: true,
                  get() {
                    record("prototype-promise-get");
                    return undefined;
                  },
                  set() {
                    record("prototype-promise-set");
                  }
                }
              });
              record(`spoof:${Object.prototype.__lmTraversalInterceptActive}:${Object.prototype.__lmTraversalInterceptUrl}`);
              navigation.navigate("#one");
              navigation.onnavigate = event => {
                record(`navigate:${event.navigationType}`);
                if (event.navigationType === "traverse") {
                  event.intercept({
                    handler() {
                      record("handler");
                      return new Promise(resolve => setTimeout(() => {
                        record("handler-timeout");
                        resolve("done");
                      }, 0));
                    }
                  });
                }
              };
              const result = navigation.back();
              result.committed.then(
                entry => log.push(`committed:${location.hash}:${entry.index}`),
                error => log.push(`committed-rejected:${error.name}`)
              );
              result.finished.then(
                entry => log.push(`finished:${location.hash}:${entry.index}`),
                error => log.push(`finished-rejected:${error.name}:${location.hash}`)
              );
              return log.join("|");
            })()
            "##,
        )
        .expect("traversal intercept private payload setup should evaluate");
    assert_eq!(setup, "spoof:false:spoof-url:");

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued intercepted traversal should run")
    );
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("traversal intercept handler timer should drain");
    let settled = vm
        .eval("globalThis.__lmTraversalPrivatePayload.log.join('|')")
        .expect("traversal intercept private payload log should evaluate");
    assert_eq!(
        settled,
        "spoof:false:spoof-url:|navigate:traverse:#one|handler:|committed::0|handler-timeout:|finished::0"
    );
}

#[tokio::test]
async fn closed_history_traversal_route_rejects_navigation_promises_without_timer_fallback() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    vm.eval(r##"history.pushState(null, "", "#one"); "created""##)
        .expect("history route-retirement fixture should create an entry");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("entry-creation fixture timers should drain");
    drop(
        vm._page_task_residence_for_executor_test
            .take()
            .expect("history route-retirement fixture should own one production consumer"),
    );

    vm.eval(
        r#"
globalThis.__lmClosedHistoryRoute = [];
const result = navigation.back();
result.committed.catch(error => __lmClosedHistoryRoute.push("committed:" + error.name));
result.finished.catch(error => __lmClosedHistoryRoute.push("finished:" + error.name));
"queued"
"#,
    )
    .expect("closed history route should reject instead of falling back");
    assert_eq!(
        vm.eval("__lmClosedHistoryRoute.join('|')")
            .expect("closed history route rejections should settle"),
        "committed:AbortError|finished:AbortError"
    );
    assert!(
        !vm.has_ready_timeout(),
        "a closed history source must not recreate the removed timer transport"
    );
}

#[tokio::test]
async fn history_back_calls_from_default_and_isolated_world_coalesce_per_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    vm.eval(
        r##"
history.pushState(null, "", "#one");
history.pushState(null, "", "#two");
"created"
"##,
    )
    .expect("multi-realm history fixture should create entries");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("multi-realm entry-creation timers should drain");
    let isolated_context_id = vm
        .create_isolated_world("history-traversal-owner", false)
        .expect("history traversal isolated world should be created");

    vm.eval("history.back(); 'default-queued'")
        .expect("default realm should queue traversal");
    vm.eval_in_isolated_context(isolated_context_id, "history.back(); 'isolated-queued'")
        .expect("isolated realm should queue its own exact traversal");
    assert!(
        !vm.has_ready_timeout(),
        "neither realm should route history traversal through PageTimer"
    );

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("default realm traversal should run")
    );
    assert!(
        !vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("history source should be drained after one Window traversal position")
    );
}
