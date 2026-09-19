use super::*;

#[test]
fn detached_document_lifecycle_methods_use_document_prototype_brand_checks() {
    let mut vm = new_storage_test_vm("https://detached-document-lifecycle-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const shape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.value,
      descriptor.value && descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const error = (callback) => {
    try {
      callback();
      return "ok";
    } catch (thrown) {
      return `${thrown.name}:${thrown.code}`;
    }
  };

  const html = document.implementation.createHTMLDocument("");
  const htmlProto = Object.getPrototypeOf(html);
  const openReturn = Document.prototype.open.call(html);
  Document.prototype.write.call(html, "<p id='a'>A</p>");
  Document.prototype.writeln.call(html, "<span id='b'>B</span>");
  const closeReturn = Document.prototype.close.call(html);

  const direct = document.implementation.createHTMLDocument("");
  direct.write("<em>E</em>");
  direct.writeln("<strong>S</strong>");

  const parsed = new DOMParser().parseFromString("<html><body></body></html>", "text/html");
  const parsedOpenReturn = Document.prototype.open.call(parsed);
  parsed.write("<article>parsed</article>");

  const xml = document.implementation.createDocument("urn:test", "root", null);

  return JSON.stringify({
    shapes: ["open", "write", "writeln", "close"].map(shape).join("|"),
    documentOwn: ["open", "write", "writeln", "close"].map((name) => own(document, name)).join(","),
    htmlOwn: ["open", "write", "writeln", "close"].map((name) => own(html, name)).join(","),
    htmlProtoOwn: ["open", "write", "writeln", "close"].map((name) => own(htmlProto, name)).join(","),
    htmlProtoIsStandard: htmlProto === HTMLDocument.prototype,
    openReturn: openReturn === html,
    closeReturn: closeReturn === undefined,
    body: html.body.innerHTML,
    direct: direct.body.innerHTML,
    parsedOpenReturn: parsedOpenReturn === parsed,
    parsed: parsed.body.innerHTML,
    errors: [
      error(() => html.open("/popup", "", "")),
      error(() => Document.prototype.open.call(xml)),
      error(() => Document.prototype.write.call(xml, "x")),
      error(() => Document.prototype.writeln.call(xml, "x")),
      error(() => Document.prototype.close.call(xml))
    ].join("|")
  });
})()
"#,
        )
        .expect("detached Document lifecycle brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"shapes":"true:function:0:true:true|true:function:0:true:true|true:function:0:true:true|true:function:0:true:true","documentOwn":"false,false,false,false","htmlOwn":"false,false,false,false","htmlProtoOwn":"false,false,false,false","htmlProtoIsStandard":true,"openReturn":true,"closeReturn":true,"body":"<p id=\"a\">A</p><span id=\"b\">B</span>\n","direct":"<em>E</em><strong>S</strong>\n","parsedOpenReturn":true,"parsed":"<article>parsed</article>","errors":"InvalidAccessError:15|InvalidStateError:11|InvalidStateError:11|InvalidStateError:11|InvalidStateError:11"}"#
    );
}

#[test]
fn detached_character_data_reads_native_value_after_data_projection_tamper() {
    let mut vm = new_storage_test_vm("https://detached-character-data-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const text = doc.createTextNode("real");
  doc.body.appendChild(text);
  Object.defineProperty(text, "data", {
    value: "fake",
    configurable: true
  });
  return [
    text.data,
    text.nodeValue,
    doc.body.textContent,
    text.isEqualNode(doc.createTextNode("real")),
    text.isEqualNode(doc.createTextNode("fake"))
  ].join("|");
})()
"#,
        )
        .expect("detached character data reads should stay native-backed after data tamper");

    assert_eq!(result, "fake|real|real|true|false");
}

#[test]
fn detached_character_data_clone_reads_native_value_after_data_projection_tamper() {
    let mut vm = new_storage_test_vm("https://detached-character-clone-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const text = doc.createTextNode("real");
  doc.body.appendChild(text);
  Object.defineProperty(text, "data", {
    value: "fake",
    configurable: true
  });
  const shallow = text.cloneNode(false);
  const parent = doc.createElement("div");
  parent.appendChild(text);
  const deep = parent.cloneNode(true);
  return [
    text.data,
    shallow.data,
    shallow.nodeValue,
    deep.firstChild.data,
    deep.firstChild.nodeValue
  ].join("|");
})()
"#,
        )
        .expect("detached character data clone should stay native-backed after data tamper");

    assert_eq!(result, "fake|real|real|real|real");
}

#[test]
fn detached_element_equality_reads_native_attributes_after_method_tamper() {
    let mut vm = new_storage_test_vm("https://detached-attribute-equality-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const left = doc.createElement("div");
  const same = doc.createElement("div");
  const different = doc.createElement("div");
  left.setAttribute("data-real", "one");
  same.setAttribute("data-real", "one");
  different.setAttribute("data-real", "two");
  left.getAttributeNames = () => [];
  left.getAttribute = () => "two";
  return [
    left.getAttributeNames().length,
    left.getAttribute("data-real"),
    left.isEqualNode(same),
    left.isEqualNode(different)
  ].join("|");
})()
"#,
        )
        .expect(
            "detached element equality should stay native-backed after attribute method tamper",
        );

    assert_eq!(result, "0|two|true|false");
}

#[test]
fn detached_html_document_accepts_live_comment_children() {
    let mut vm = new_storage_test_vm("https://detached-document-comment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const left = document.implementation.createHTMLDocument("");
  const right = document.implementation.createHTMLDocument("");
  left.appendChild(document.createComment("data"));
  right.appendChild(document.createComment("data"));
  return [
    left.lastChild.nodeType,
    left.lastChild.data,
    left.isEqualNode(right)
  ].join("|");
})()
"#,
        )
        .expect("detached HTML documents should accept live Comment children");

    assert_eq!(result, "8|data|true");
}

#[test]
fn detached_element_ns_attribute_methods_round_trip() {
    let mut vm = new_storage_test_vm("https://detached-ns-attr.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><div></div></body></html>', 'text/html');
  const el = doc.querySelector('div');
  function probe(callback) {
    try {
      const value = callback();
      return value === null ? "null" : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  el.setAttributeNS("urn:moli:test", "lm:flag", "on");
  el.setAttributeNS(null, "data-local", "local");
  const stages = [
    el.getAttributeNS("urn:moli:test", "flag"),
    el.hasAttributeNS("urn:moli:test", "flag"),
    el.getAttributeNS(null, "data-local"),
    el.hasAttributeNS("", "data-local"),
    el.getAttribute("lm:flag"),
    probe(() => el.setAttributeNS("urn:moli:test", "bogus name", "v")),
    probe(() => el.setAttributeNS(null, "lm:bad", "v"))
  ];
  el.removeAttributeNS("urn:moli:test", "flag");
  el.removeAttributeNS(null, "data-local");
  stages.push(el.hasAttributeNS("urn:moli:test", "flag"));
  stages.push(el.hasAttributeNS(null, "data-local"));
  return stages.join("|");
})()
"#,
        )
        .expect("detached Element NS attribute methods should evaluate");

    assert_eq!(
        result,
        "on|true|local|true|on|throw:InvalidCharacterError|throw:NamespaceError|false|false"
    );
}

#[test]
fn detached_html_document_all_matches_chromium_htmldda_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const div = doc.createElement("div");
              div.id = "probe";
              doc.body.appendChild(div);
              const allDescriptor = Object.getOwnPropertyDescriptor(Document.prototype, "all");
              return JSON.stringify({
                ownAll: Object.prototype.hasOwnProperty.call(doc, "all"),
                protoGetter: typeof allDescriptor?.get,
                protoGetterTag: Object.prototype.toString.call(allDescriptor.get.call(doc)),
                allType: typeof doc.all,
                loose: doc.all == undefined,
                strict: doc.all === undefined,
                bool: !!doc.all,
                string: String(doc.all),
                tag: Object.prototype.toString.call(doc.all),
                ctorDirect: doc.all.constructor && doc.all.constructor.name,
                calledType: typeof doc.all(),
                calledNull: doc.all() === null,
                callByIndex: doc.all(doc.all.length - 1) === div,
                namedHit: doc.all("probe") === div,
                itemMethodNull: doc.all.item(999) === null,
                namedMethodNull: doc.all.namedItem("missing") === null
              });
            })()
            "#,
        )
        .expect("detached HTMLDocument.all probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownAll":false,"protoGetter":"function","protoGetterTag":"[object HTMLAllCollection]","allType":"undefined","loose":true,"strict":false,"bool":false,"string":"[object HTMLAllCollection]","tag":"[object HTMLAllCollection]","ctorDirect":"HTMLAllCollection","calledType":"object","calledNull":true,"callByIndex":true,"namedHit":true,"itemMethodNull":true,"namedMethodNull":true}"#
    );
}

#[test]
fn detached_document_all_declared_members_ignore_public_data_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const div = doc.createElement("div");
              div.id = "probe";
              doc.body.appendChild(div);
              const all = doc.all;
              const summarize = name => {
                const descriptor = Object.getOwnPropertyDescriptor(all, name);
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const summarizePrototype = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  HTMLAllCollection.prototype,
                  name
                );
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const beforeNames = Object.getOwnPropertyNames(all).includes("data");
              all.data = {
                items: [],
                named: { probe: null }
              };
              return [
                summarize("length"),
                summarize("item"),
                summarize("namedItem"),
                summarizePrototype(Symbol.iterator),
                Object.prototype.hasOwnProperty.call(all, Symbol.iterator),
                beforeNames,
                Object.prototype.hasOwnProperty.call(all, "data"),
                all.data && Array.isArray(all.data.items),
                all.item(all.length - 1) === div,
                all.namedItem("probe") === div,
                all("probe") === div,
                typeof all[Symbol.iterator]
              ].join("|");
            })()
            "#,
        )
        .expect("detached document.all declared surface spoofing probe should evaluate");

    assert_eq!(
        result,
        "true:false:true:false:number|true:false:true:true:function|true:false:true:true:function|true:false:true:true:function|false|false|true|true|true|true|true|function"
    );
}

#[test]
fn detached_collection_declared_iterators_ignore_public_data_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const div = doc.createElement("div");
              div.id = "probe";
              doc.body.appendChild(div);
              const nodeList = doc.querySelectorAll("div");
              const collection = doc.getElementsByTagName("div");
              const summarizePrototype = (object, key) => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  Object.getPrototypeOf(object),
                  key
                );
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const beforeNames = [
                Object.getOwnPropertyNames(nodeList).includes("data"),
                Object.getOwnPropertyNames(collection).includes("data")
              ].join(":");
              nodeList.data = { items: [] };
              collection.data = { items: [], named: { probe: null } };
              return [
                summarizePrototype(nodeList, Symbol.iterator),
                summarizePrototype(collection, Symbol.iterator),
                Object.prototype.hasOwnProperty.call(nodeList, Symbol.iterator),
                Object.prototype.hasOwnProperty.call(collection, Symbol.iterator),
                Object.getPrototypeOf(nodeList) === NodeList.prototype,
                Object.getPrototypeOf(collection) === HTMLCollection.prototype,
                beforeNames,
                Object.prototype.hasOwnProperty.call(nodeList, "data"),
                Object.prototype.hasOwnProperty.call(collection, "data"),
                Array.from(nodeList)[0] === div,
                Array.from(collection)[0] === div,
                nodeList.item(0) === div,
                collection.item(0) === div,
                collection.namedItem("probe") === div
              ].join("|");
            })()
            "#,
        )
        .expect("detached collection declared iterator spoofing probe should evaluate");

    assert_eq!(
        result,
        "true:false:true:true:function|true:false:true:true:function|false|false|true|true|false:false|true|true|true|true|true|true|true"
    );
}

#[test]
fn detached_html_document_all_includes_default_document_tree() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              return JSON.stringify({
                allType: typeof doc.all,
                loose: doc.all == undefined,
                strict: doc.all === undefined,
                calledNull: doc.all() === null,
                calledLength: doc.all.length
              });
            })()
            "#,
        )
        .expect("empty detached HTMLDocument.all probe should evaluate");

    assert_eq!(
        result,
        r#"{"allType":"undefined","loose":true,"strict":false,"calledNull":true,"calledLength":4}"#
    );
}

#[test]
fn detached_html_elements_expose_click_method() {
    let mut vm = new_storage_test_vm("https://detached-element-click.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const link = doc.createElement("a");
              let clicks = 0;
              link.addEventListener("click", event => {
                clicks += event.isTrusted ? 1 : 10;
              });
              link.click();
              return `${typeof link.click}|${clicks}`;
            })()
            "#,
        )
        .expect("detached HTML elements should expose synthetic click");

    assert_eq!(result, "function|10");
}

#[test]
fn detached_html_document_body_class_list_matches_domtokenlist_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const body = doc.body;
              body.className = "outer highlight";
              const list = body.classList;
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const stable = list === body.classList;
              const seen = [];
              const thisArg = { marker: "detached" };
              const initial = {
                tag: Object.prototype.toString.call(list),
                containsHighlight: list.contains("highlight"),
                containsMissing: list.contains("missing"),
                containsEmpty: list.contains(""),
                item0: list.item(0),
                item1: list.item(1),
                itemSymbol: probe(() => list.item(Symbol())),
                length: list.length,
                stable
              };
              list.forEach(function(value, index, owner) {
                seen.push(`${this.marker}:${value}:${index}:${owner === list}`);
              }, thisArg);
              list.remove("outer");
              list.add("processed");
              const replaced = list.replace("highlight", "done");
              return JSON.stringify({
                initial,
                seen,
                replaced,
                finalClassName: body.className,
                finalValue: list.value,
                containsDone: list.contains("done"),
                containsOuter: list.contains("outer"),
                toggledMissing: list.toggle("missing", false),
                containsSymbol: probe(() => list.contains(Symbol())),
                toggleSymbol: probe(() => list.toggle(Symbol())),
                replaceMissing: probe(() => list.replace("done")),
                forEachMissing: probe(() => list.forEach())
              });
            })()
            "#,
        )
        .expect("detached HTMLDocument body.classList should behave like DOMTokenList");

    assert_eq!(
        result,
        r#"{"initial":{"tag":"[object DOMTokenList]","containsHighlight":true,"containsMissing":false,"containsEmpty":false,"item0":"outer","item1":"highlight","itemSymbol":"throw:TypeError","length":2,"stable":true},"seen":["detached:outer:0:true","detached:highlight:1:true"],"replaced":true,"finalClassName":"done processed","finalValue":"done processed","containsDone":true,"containsOuter":false,"toggledMissing":false,"containsSymbol":"throw:TypeError","toggleSymbol":"throw:TypeError","replaceMissing":"throw:TypeError","forEachMissing":"throw:TypeError"}"#
    );
}

#[test]
fn detached_html_document_class_list_internal_slots_are_not_visible_to_js() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const body = doc.body;
              const list = body.classList;
              return JSON.stringify({
                bodyOwnNamesHasCacheSlot: Object.getOwnPropertyNames(body).includes("__moliDetachedClassList"),
                listOwnNamesHasTargetSlot: Object.getOwnPropertyNames(list).includes("__moliDetachedClassListTarget"),
                bodyOwnKeysHasCacheSlot: Reflect.ownKeys(body).includes("__moliDetachedClassList"),
                listOwnKeysHasTargetSlot: Reflect.ownKeys(list).includes("__moliDetachedClassListTarget"),
                bodyHasCacheSlot: "__moliDetachedClassList" in body,
                listHasTargetSlot: "__moliDetachedClassListTarget" in list,
                bodyCacheValueType: typeof body.__moliDetachedClassList,
                listTargetValueType: typeof list.__moliDetachedClassListTarget
              });
            })()
            "#,
        )
        .expect("detached classList private slots should stay hidden from JS inspection");

    assert_eq!(
        result,
        r#"{"bodyOwnNamesHasCacheSlot":false,"listOwnNamesHasTargetSlot":false,"bodyOwnKeysHasCacheSlot":false,"listOwnKeysHasTargetSlot":false,"bodyHasCacheSlot":false,"listHasTargetSlot":false,"bodyCacheValueType":"undefined","listTargetValueType":"undefined"}"#
    );
}

#[test]
fn detached_domparser_node_adoption_matches_chromium_parent_and_connected_semantics() {
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
              const before = {
                parentIsMain: node.parentNode === doc.querySelector('main'),
                parentTag: node.parentNode && (node.parentNode.tagName || node.parentNode.nodeName),
                isConnected: node.isConnected,
                ownerDocumentIsDetached: node.ownerDocument === doc,
                hostContains: host.contains(node)
              };
              host.appendChild(node);
              const after = {
                parentIsHost: node.parentNode === host,
                parentTag: node.parentNode && (node.parentNode.tagName || node.parentNode.nodeName),
                isConnected: node.isConnected,
                ownerDocumentIsLive: node.ownerDocument === document,
                hostContains: host.contains(node)
              };
              return JSON.stringify({ before, after });
            })()
            "#,
        )
        .expect("detached DOMParser node adoption should return a probe result");

    assert_eq!(
        result,
        r#"{"before":{"parentIsMain":true,"parentTag":"MAIN","isConnected":true,"ownerDocumentIsDetached":true,"hostContains":false},"after":{"parentIsHost":true,"parentTag":"BODY","isConnected":true,"ownerDocumentIsLive":true,"hostContains":true}}"#
    );
}

#[test]
fn detached_document_append_child_returns_materialized_child() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createDocument(null, "", null);
              const liveComment = document.createComment("before");
              const inserted = doc.appendChild(liveComment);
              return JSON.stringify({
                returnedIsStored: inserted === doc.firstChild,
                ownerIsDetached: inserted.ownerDocument === doc,
                originalAdopted: liveComment.ownerDocument === doc,
                childCount: doc.childNodes.length
              });
            })()
            "#,
        )
        .expect("detached appendChild return probe should evaluate");

    assert_eq!(
        result,
        r#"{"returnedIsStored":true,"ownerIsDetached":true,"originalAdopted":true,"childCount":1}"#
    );
}

#[test]
fn detached_domparser_adopted_nodes_follow_live_tree_for_children_text_and_mutation_methods() {
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
              const root = doc.querySelector('main');
              const heldChild = root.firstChild;
              document.body.appendChild(root);

              const liveRoot = document.body.firstChild;
              liveRoot.appendChild(document.createTextNode('y'));

              const beforeRemoval = {
                firstChildIsHeld: root.firstChild === heldChild,
                childParentIsForeignRoot: heldChild.parentNode === root,
                childNodesLength: root.childNodes.length,
                lastChildType: root.lastChild && root.lastChild.nodeType,
                textContent: root.textContent,
                containsHeldChild: root.contains(heldChild)
              };

              const removed = root.removeChild(heldChild);

              const afterRemoval = {
                removedIsHeld: removed === heldChild,
                removedParentIsNull: heldChild.parentNode === null,
                childNodesLength: root.childNodes.length,
                firstChildType: root.firstChild && root.firstChild.nodeType,
                textContent: root.textContent,
                liveBodyText: document.body.firstChild && document.body.firstChild.textContent
              };

              return JSON.stringify({ beforeRemoval, afterRemoval });
            })()
            "#,
        )
        .expect("adopted DOMParser nodes should keep tracking the live subtree");

    assert_eq!(
        result,
        r#"{"beforeRemoval":{"firstChildIsHeld":true,"childParentIsForeignRoot":true,"childNodesLength":2,"lastChildType":3,"textContent":"xy","containsHeldChild":true},"afterRemoval":{"removedIsHeld":true,"removedParentIsNull":true,"childNodesLength":1,"firstChildType":3,"textContent":"y","liveBodyText":"y"}}"#
    );
}

#[test]
fn detached_document_content_type_controls_element_creation_after_root_mutations_and_cloning() {
    let mut vm = new_storage_test_vm("https://document-create-element-content-type.test/");
    let fixture =
        include_str!("../../../../../tests/fixtures/document-create-element-content-type.js");
    let result = vm
        .eval(&format!("JSON.stringify({fixture})"))
        .expect("Document content type and element creation probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["checks"], 689);
}

#[test]
fn domparser_xml_preserves_content_type_and_document_interface_for_success_and_errors() {
    let mut vm = new_storage_test_vm("https://domparser-xml-content-type.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  return JSON.stringify([
    "text/xml",
    "application/xml",
    "application/xhtml+xml",
    "image/svg+xml"
  ].map(contentType => {
    const valid = parser.parseFromString("<root/>", contentType);
    const invalid = parser.parseFromString("", contentType);
    return [
      valid.contentType,
      invalid.contentType,
      invalid.documentElement.localName,
      Object.getPrototypeOf(valid) === Document.prototype,
      valid instanceof XMLDocument,
      Object.getPrototypeOf(invalid) === Document.prototype,
      invalid instanceof XMLDocument
    ];
  }));
})()
"#,
        )
        .expect("DOMParser XML content type probe should evaluate");

    assert_eq!(
        result,
        r#"[["text/xml","text/xml","parsererror",true,false,true,false],["application/xml","application/xml","parsererror",true,false,true,false],["application/xhtml+xml","application/xhtml+xml","parsererror",true,false,true,false],["image/svg+xml","image/svg+xml","parsererror",true,false,true,false]]"#
    );
}

#[test]
fn body_legacy_colors_treat_null_as_empty_before_reflection() {
    let mut vm = new_parsed_test_vm(
        "https://body-color-null-reflection.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const detachedDocument = document.implementation.createHTMLDocument("");
  for (const body of [document.body, document.createElement("body"), detachedDocument.body]) {
    for (const name of ["text", "link", "vLink", "aLink", "bgColor"]) {
      const attribute = name.toLowerCase();
      body[name] = null;
      assert(body[name] === "" && body.getAttribute(attribute) === "", `${name}: null`);
      body[name] = undefined;
      assert(body[name] === "undefined", `${name}: undefined`);
      let conversions = 0;
      body[name] = {toString() { conversions++; return "red"; }};
      assert(conversions === 1 && body.getAttribute(attribute) === "red", `${name}: conversion`);
      const failure = new Error("conversion failure");
      let caught;
      try { body[name] = {toString() {throw failure; }}; } catch (error) { caught = error; }
      assert(caught === failure && body[name] === "red", `${name}: exception`);
      const setter = Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, name).set;
      for (const invalid of [{}, document.createElement("div"), Object.create(body), new Proxy(body, {})]) {
        conversions = 0;
        caught = undefined;
        try {
          setter.call(invalid, {toString() {conversions++; return "blue"; }});
        } catch (error) { caught = error; }
        assert(caught instanceof TypeError && conversions === 0, `${name}: receiver before conversion`);
      }
    }
    body.background = null;
    assert(body.getAttribute("background") === "null", "background uses ordinary DOMString conversion");
  }
  return "ok";
})()
"#,
        )
        .expect("legacy body colors should preserve WebIDL conversion semantics");
    assert_eq!(result, "ok");
}
