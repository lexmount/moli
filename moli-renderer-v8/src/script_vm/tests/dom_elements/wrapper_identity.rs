use super::*;

#[test]
fn cached_dom_wrapper_lookup_preserves_javascript_prototype() {
    let mut vm = new_storage_test_vm("https://wrapper-lookup.test/");
    let result = vm.eval(r##"
        (() => {
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const frame = body.appendChild(document.createElement("iframe"));
          const failures = [];
          let cases = 0;
          for (const [realm, w] of [["main", window], ["child", frame.contentWindow]]) {
            const doc = w.document;
            const root = doc.body || doc.documentElement;
            for (const kind of ["null", "object", "inherited"]) {
              const element = doc.createElement("div");
              const initialRealm = Object.getPrototypeOf(element) === w.HTMLDivElement.prototype;
              element.id = `wrapper-${kind}`;
              root.appendChild(element);
              const selected = kind === "null" ? null :
                  kind === "object" ? {} : Object.create(w.HTMLDivElement.prototype);
              Object.setPrototypeOf(element, selected);
              const lookups = [doc.querySelector(`#wrapper-${kind}`),
                  doc.getElementById(`wrapper-${kind}`), root.lastChild,
                  doc.querySelectorAll(`#wrapper-${kind}`)[0]];
              const same = lookups.every(value => value === element);
              const preserved = lookups.every(value => Object.getPrototypeOf(value) === selected);
              if (!initialRealm || !same || !preserved) {
                failures.push({realm, kind, initialRealm, same, preserved});
              }
              root.removeChild(element);
              cases++;
            }
          }
          return JSON.stringify({cases, failures});
        })()
    "##).expect("DOM lookups should preserve the existing JavaScript object and its prototype");
    assert_eq!(result, r#"{"cases":6,"failures":[]}"#);
}

#[test]
fn cached_dom_wrapper_adoption_preserves_identity_and_prototype() {
    let mut vm = new_storage_test_vm("https://wrapper-adoption.test/");
    let result = vm.eval(r##"
        (() => {
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const a = body.appendChild(document.createElement("iframe")).contentWindow;
          const b = body.appendChild(document.createElement("iframe")).contentWindow;
          const ownerDocument = Object.getOwnPropertyDescriptor(Node.prototype, "ownerDocument").get;
          const failures = [];
          let cases = 0;
          for (const [direction, source, target] of [
              ["main-child", window, a], ["child-main", a, window],
              ["child-sibling", a, b], ["sibling-child", b, a]]) {
            for (const kind of ["native", "null", "inherited"]) {
              const element = source.document.createElement("div");
              element.id = `adopted-${cases}`;
              (source.document.body || source.document.documentElement).appendChild(element);
              const selected = kind === "native" ? source.HTMLDivElement.prototype :
                  kind === "null" ? null : Object.create(source.HTMLDivElement.prototype);
              if (kind !== "native") Object.setPrototypeOf(element, selected);
              const adopted = target.document.adoptNode(element);
              const afterAdopt = Object.getPrototypeOf(adopted) === selected;
              const root = target.document.body || target.document.documentElement;
              root.appendChild(adopted);
              const lookedUp = target.document.querySelector(`#adopted-${cases}`);
              const same = adopted === element && lookedUp === element;
              const ownerChanged = ownerDocument.call(element) === target.document;
              const afterLookup = Object.getPrototypeOf(lookedUp) === selected;
              if (!same || !ownerChanged || !afterAdopt || !afterLookup) {
                failures.push({direction, kind, same, ownerChanged, afterAdopt, afterLookup});
              }
              root.removeChild(element);
              cases++;
            }
          }
          return JSON.stringify({cases, failures});
        })()
    "##).expect("adoption should change the owner document while preserving the JavaScript wrapper");
    assert_eq!(result, r#"{"cases":12,"failures":[]}"#);
}

#[test]
fn new_dom_wrappers_use_child_realm_prototypes() {
    let mut vm = new_storage_test_vm("https://wrapper-initialization.test/");
    let result = vm.eval(r##"
        (() => {
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const w = body.appendChild(document.createElement("iframe")).contentWindow;
          const doc = w.document;
          const created = [
            [doc, w.HTMLDocument.prototype],
            [doc.createRange(), w.Range.prototype],
            [doc.createElement("div"), w.HTMLDivElement.prototype],
            [doc.createTextNode("created"), w.Text.prototype],
            [doc.createComment("created"), w.Comment.prototype]
          ];
          doc.body.innerHTML = "<div id='parsed'>text<!--comment--></div>";
          const parsed = doc.querySelector("#parsed");
          const nodes = [...created, [parsed, w.HTMLDivElement.prototype],
              [parsed.firstChild, w.Text.prototype], [parsed.lastChild, w.Comment.prototype]];
          return nodes.map(([node, prototype]) => Object.getPrototypeOf(node) === prototype).join(":");
        })()
    "##).expect("new DOM wrappers should receive their child-realm native prototypes on first access");
    assert_eq!(result, "true:true:true:true:true:true:true:true");
}

#[test]
fn child_dom_wrappers_do_not_read_public_constructor_properties() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-intrinsics.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const w = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const doc = w.document;
          const names = ["HTMLDivElement", "Text", "Comment"];
          const prototypes = names.map(name => w[name].prototype);
          let reads = 0;
          for (const name of names) {
            Object.defineProperty(w, name, {
              configurable: true,
              get() { reads++; return {prototype: {}}; }
            });
          }
          const created = [doc.createElement("div"), doc.createTextNode("created"),
              doc.createComment("created")];
          doc.body.innerHTML = "<div>parsed<!--comment--></div>";
          const div = doc.body.firstChild;
          const parsed = [div, div.firstChild, div.lastChild];
          return [reads, ...[created, parsed].flatMap(nodes =>
              nodes.map((node, index) => Object.getPrototypeOf(node) === prototypes[index]))].join(":");
        })()
    "#).expect("wrapper creation should use intrinsic prototypes without invoking author getters");
    assert_eq!(result, "0:true:true:true:true:true:true");
}

#[test]
fn child_dom_collections_wrap_nodes_in_the_collection_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-collections.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const w = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const doc = w.document;
          doc.body.innerHTML = Array.from({length: 1100}, (_, i) => `<span id='node-${i}'></span>`).join("");
          const list = doc.querySelectorAll("span");
          const live = doc.body.children;
          const nodes = [list[0], NodeList.prototype.item.call(list, 1),
              Object.getOwnPropertyDescriptor(list, "2").value,
              live[3], HTMLCollection.prototype.item.call(live, 4),
              live["node-5"], HTMLCollection.prototype.namedItem.call(live, "node-6"),
              Object.getOwnPropertyDescriptor(live, "7").value,
              Object.getOwnPropertyDescriptor(live, "node-8").value];
          return nodes.map(node => Object.getPrototypeOf(node) === w.HTMLSpanElement.prototype).join(":");
        })()
    "#).expect("collection access should create node wrappers in the collection's realm");
    assert_eq!(result, "true:true:true:true:true:true:true:true:true");
}

#[test]
fn borrowed_dom_methods_create_wrappers_in_the_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-receiver.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const child = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const failures = [];
          let cases = 0;
          for (const [source, target] of [[window, child], [child, window]]) {
            const doc = target.document;
            const methods = source.Document.prototype;
            const nodes = [
              [methods.createElement.call(doc, "div"), target.HTMLDivElement.prototype],
              [methods.createElementNS.call(doc, "http://www.w3.org/1999/xhtml", "span"), target.HTMLSpanElement.prototype],
              [methods.createTextNode.call(doc, "text"), target.Text.prototype],
              [methods.createComment.call(doc, "comment"), target.Comment.prototype],
              [methods.createDocumentFragment.call(doc), target.DocumentFragment.prototype]
            ];
            const root = doc.createElement("section");
            root.innerHTML = "<div></div><span></span>";
            const firstChild = Object.getOwnPropertyDescriptor(source.Node.prototype, "firstChild").get;
            nodes.push([firstChild.call(root), target.HTMLDivElement.prototype]);
            nodes.push([source.Element.prototype.querySelector.call(root, "span"), target.HTMLSpanElement.prototype]);
            nodes.push([source.Element.prototype.querySelectorAll.call(root, "span"), target.NodeList.prototype]);
            const childNodes = Object.getOwnPropertyDescriptor(source.Node.prototype, "childNodes").get;
            nodes.push([childNodes.call(root), target.NodeList.prototype]);
            nodes.push([methods.getElementsByTagName.call(doc, "span"), target.HTMLCollection.prototype]);
            const table = doc.createElement("table");
            nodes.push([source.HTMLTableElement.prototype.createTBody.call(table), target.HTMLTableSectionElement.prototype]);
            for (const [node, prototype] of nodes) {
              if (Object.getPrototypeOf(node) !== prototype) failures.push(cases);
              cases++;
            }
          }
          return JSON.stringify({cases, failures});
        })()
    "#).expect("borrowed DOM methods should wrap returned nodes in their receiver's realm");
    assert_eq!(result, r#"{"cases":22,"failures":[]}"#);
}

#[test]
fn borrowed_split_text_preserves_receiver_realm_after_adoption() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-split-text.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
        (() => {
          const child = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const failures = [];
          for (const [source, receiverRealm] of [[window, child], [child, window]]) {
            for (const adopt of [false, true]) {
              const text = receiverRealm.document.createTextNode("ab");
              const owner = adopt ? source.document : receiverRealm.document;
              if (adopt) owner.adoptNode(text);
              const parent = owner.createElement("div");
              parent.appendChild(text);
              const result = source.Text.prototype.splitText.call(text, 1);
              if (Object.getPrototypeOf(result) !== receiverRealm.Text.prototype ||
                  result.ownerDocument !== owner || text.data !== "a" || result.data !== "b" ||
                  text.nextSibling !== result || parent.lastChild !== result) {
                failures.push({childReceiver: receiverRealm === child, adopt});
              }
            }
          }
          return JSON.stringify(failures);
        })()
    "#,
        )
        .expect("splitText should create its result in the receiver's realm even after adoption");
    assert_eq!(result, "[]");
}

#[test]
fn borrowed_shadow_and_template_accessors_use_the_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-shadow-template.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const child = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const failures = [];
          for (const [source, target] of [[window, child], [child, window]]) {
            const host = target.document.createElement("div");
            const root = source.Element.prototype.attachShadow.call(host, {mode: "open"});
            const slotRoot = target.document.createElement("div").attachShadow({mode: "open"});
            slotRoot.innerHTML = "<slot><span></span></slot>";
            const slot = slotRoot.firstChild;
            const assigned = source.HTMLSlotElement.prototype.assignedNodes.call(slot, {flatten: true});
            const template = target.document.createElement("template");
            const content = Object.getOwnPropertyDescriptor(source.HTMLTemplateElement.prototype, "content").get.call(template);
            for (const [name, object, prototype] of [
                ["attachShadow", root, target.ShadowRoot.prototype],
                ["assignedNodes", assigned[0], target.HTMLSpanElement.prototype],
                ["template.content", content, target.DocumentFragment.prototype]]) {
              if (!object || Object.getPrototypeOf(object) !== prototype) failures.push(name);
            }
          }
          return JSON.stringify(failures);
        })()
    "#).expect("shadow and template results should use the receiver's realm on first access");
    assert_eq!(result, "[]");
}

#[test]
fn child_form_indexed_and_named_access_wraps_unseen_controls_in_its_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-form-access.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const child = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const doc = child.document;
          const form = doc.createElement("form");
          form.innerHTML = "<input name='first'><input name='second'><input name='third'><input name='fourth'>";
          const select = doc.createElement("select");
          select.innerHTML = "<option>first</option><option>second</option>";
          const nodes = [
              [form[0], child.HTMLInputElement.prototype],
              [Object.getOwnPropertyDescriptor(form, "1").value, child.HTMLInputElement.prototype],
              [form.third, child.HTMLInputElement.prototype],
              [Object.getOwnPropertyDescriptor(form, "fourth").value, child.HTMLInputElement.prototype],
              [select[0], child.HTMLOptionElement.prototype],
              [Object.getOwnPropertyDescriptor(select, "1").value, child.HTMLOptionElement.prototype]];
          return nodes.map(([node, prototype]) => Object.getPrototypeOf(node) === prototype).join(":");
        })()
    "#).expect("form and select property interceptors should use their holder's realm");
    assert_eq!(result, "true:true:true:true:true:true");
}

#[test]
fn child_window_named_access_wraps_unseen_nodes_in_the_window_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-window-named.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const w = document.body.appendChild(document.createElement("iframe")).contentWindow;
          w.document.body.innerHTML = "<div id='namedNode'></div><img name='namedImages'><img name='namedImages'>";
          const node = w.namedNode;
          const images = w.namedImages;
          return [Object.getPrototypeOf(node) === w.HTMLDivElement.prototype,
              Object.getPrototypeOf(images) === w.HTMLCollection.prototype,
              Object.getPrototypeOf(images[0]) === w.HTMLImageElement.prototype,
              Object.getPrototypeOf(images[1]) === w.HTMLImageElement.prototype].join(":");
        })()
    "#).expect("Window named access should wrap nodes and collections in that Window's realm");
    assert_eq!(result, "true:true:true:true");
}

#[test]
fn borrowed_range_xpath_and_document_all_results_use_the_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-result-containers.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const child = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const failures = [];
          for (const [source, target] of [[window, child], [child, window]]) {
            const doc = target.document;
            const root = doc.createElement("section");
            root.innerHTML = "<p>contents</p>";
            doc.body.appendChild(root);
            const range = doc.createRange();
            range.selectNodeContents(root);
            const clone = source.Range.prototype.cloneContents.call(range);
            const fragment = source.Range.prototype.createContextualFragment.call(range, "<span></span>");
            const extracted = source.Range.prototype.extractContents.call(range);
            const xpathRoot = doc.createElement("section");
            xpathRoot.innerHTML = "<p>found</p>";
            const xpath = source.Document.prototype.evaluate.call(doc, ".//p", xpathRoot, null, 9, null);
            root.innerHTML = "<span id='all-result'></span>";
            const all = Object.getOwnPropertyDescriptor(source.Document.prototype, "all").get.call(doc);
            for (const [name, object, prototype] of [
                ["cloneContents", clone, target.DocumentFragment.prototype],
                ["createContextualFragment", fragment, target.DocumentFragment.prototype],
                ["extractContents", extracted, target.DocumentFragment.prototype],
                ["XPathResult", xpath, target.XPathResult.prototype],
                ["singleNodeValue", xpath.singleNodeValue, target.HTMLParagraphElement.prototype],
                ["document.all", all, target.HTMLAllCollection.prototype],
                ["document.all.item", all.namedItem("all-result"), target.HTMLSpanElement.prototype]]) {
              if (object === null || object === undefined || Object.getPrototypeOf(object) !== prototype) failures.push(name);
            }
            root.remove();
          }
          return JSON.stringify(failures);
        })()
    "#).expect("returned containers and their unseen nodes should use the receiver's realm");
    assert_eq!(result, "[]");
}

#[test]
fn adopted_element_wraps_unseen_children_in_its_original_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-adopted-children.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
        (() => {
          const child = document.body.appendChild(document.createElement("iframe")).contentWindow;
          const results = [];
          for (const [source, target] of [[window, child], [child, window]]) {
            const root = source.document.createElement("section");
            root.innerHTML = "<div></div><span></span>";
            target.document.adoptNode(root);
            const first = root.firstChild;
            const last = root.querySelector("span");
            results.push(Object.getPrototypeOf(root) === source.HTMLElement.prototype,
                Object.getPrototypeOf(first) === source.HTMLDivElement.prototype,
                Object.getPrototypeOf(last) === source.HTMLSpanElement.prototype,
                first.ownerDocument === target.document, last.ownerDocument === target.document);
          }
          return results.every(Boolean);
        })()
    "#).expect("adoption should not change the realm used by an existing receiver to wrap its children");
    assert_eq!(result, "true");
}

#[test]
fn traversal_filter_wraps_unseen_nodes_in_their_document_realm() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-traversal.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
        (() => {
          const results = [];
          for (const kind of ["TreeWalker", "NodeIterator"]) {
            const w = document.body.appendChild(document.createElement("iframe")).contentWindow;
            const root = w.document.body;
            root.innerHTML = "<div></div>";
            let filtered;
            const traversal = document[`create${kind}`](root, NodeFilter.SHOW_ELEMENT, {
              acceptNode(node) {
                if (node === root) return NodeFilter.FILTER_SKIP;
                filtered = node;
                return NodeFilter.FILTER_ACCEPT;
              }
            });
            const node = traversal.nextNode();
            results.push(Object.getPrototypeOf(traversal) === window[kind].prototype,
                node === filtered, Object.getPrototypeOf(node) === w.HTMLDivElement.prototype);
          }
          return results.join(":");
        })()
    "#,
        )
        .expect(
            "NodeFilter arguments should be wrapped in the node's document realm on first access",
        );
    assert_eq!(result, "true:true:true:true:true:true");
}

#[test]
fn isolated_child_dom_wrappers_use_isolated_intrinsic_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-isolated.test/",
        "<!doctype html><html><body></body></html>",
    );
    vm.eval(
        r#"
        globalThis.frame = document.body.appendChild(document.createElement("iframe"));
        frame.contentDocument.body.innerHTML = "<div id='shared-node'></div>";
    "#,
    )
    .expect("child document should initialize");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "isolated wrapper setup");
    let frame_id = vm
        .child_default_frame_id_for_execution_context_id(child_context_id)
        .expect("child frame id should exist");
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "wrapper-utility", false)
        .expect("isolated world should initialize");
    let result = vm
        .eval_in_isolated_context(
            isolated_context_id,
            r#"
        (() => {
          const element = document.getElementById("shared-node");
          const created = document.createElement("span");
          const traversed = document.createTreeWalker(document, NodeFilter.SHOW_ELEMENT).nextNode();
          const correct = Object.getPrototypeOf(document) === HTMLDocument.prototype &&
              Object.getPrototypeOf(element) === HTMLDivElement.prototype &&
              Object.getPrototypeOf(created) === HTMLSpanElement.prototype &&
              Object.getPrototypeOf(traversed) === HTMLHtmlElement.prototype;
          element.isolatedMarker = true;
          Object.setPrototypeOf(element, null);
          return correct && document.getElementById("shared-node") === element &&
              Object.getPrototypeOf(element) === null;
        })()
    "#,
        )
        .expect("isolated DOM wrappers should use and preserve their own world's prototypes");
    assert_eq!(result, "true");
    assert_eq!(
        vm.eval(
            r#"
        (() => {
          const w = frame.contentWindow;
          const element = w.document.getElementById("shared-node");
          return Object.getPrototypeOf(element) === w.HTMLDivElement.prototype &&
              element.isolatedMarker === undefined;
        })()
    "#
        )
        .expect("default-world wrapper identity should remain separate"),
        "true"
    );
}
