use super::*;

#[test]
fn document_open_preserves_document_identity_and_detaches_the_replaced_tree() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-identity.test/",
        "<!doctype html><html><body><main id=\"old\">old text</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const oldDocument = document;
              const oldNode = document.getElementById("old");
              const oldBody = document.body;
              const oldText = oldNode.firstChild;
              const oldClassList = oldNode.classList;
              const oldDataset = oldNode.dataset;
              const oldStyle = oldNode.style;
              const oldDocumentMains = document.getElementsByTagName("main");
              const oldBodyChildren = oldBody.children;
              const oldShadowHost = document.createElement("section");
              oldBody.append(oldShadowHost);
              const oldShadow = oldShadowHost.attachShadow({ mode: "open" });
              oldShadow.innerHTML = "<span>shadow text</span>";
              const oldShadowChild = oldShadow.firstChild;
              const preDetached = document.createElement("aside");
              const listenerRuns = {
                node: 0,
                document: 0,
                window: 0,
                handler: 0,
                preDetachedNode: 0,
                preDetachedHandler: 0
              };
              oldNode.addEventListener("click", () => listenerRuns.node++);
              oldNode.onclick = () => listenerRuns.handler++;
              preDetached.addEventListener("click", () => listenerRuns.preDetachedNode++);
              preDetached.onclick = () => listenerRuns.preDetachedHandler++;
              document.addEventListener("replacement-probe", () => listenerRuns.document++);
              window.addEventListener("replacement-probe", () => listenerRuns.window++);

              document.open();
              document.write("<!doctype html><html><body><main id='new'>new text</main></body></html>");
              document.close();

              oldNode.dispatchEvent(new Event("click"));
              preDetached.dispatchEvent(new Event("click"));
              document.dispatchEvent(new Event("replacement-probe", { bubbles: true }));
              window.dispatchEvent(new Event("replacement-probe"));

              return JSON.stringify({
                sameDocument: document === oldDocument,
                oldNodeConnected: oldNode.isConnected,
                oldNodeText: oldNode.textContent,
                oldNodeOwnerPreserved: oldNode.ownerDocument === document,
                oldNodeParentPreserved: oldNode.parentNode === oldBody,
                oldTextIdentityPreserved: oldNode.firstChild === oldText,
                oldClassListIdentityPreserved: oldNode.classList === oldClassList,
                oldDatasetIdentityPreserved: oldNode.dataset === oldDataset,
                oldStyleIdentityPreserved: oldNode.style === oldStyle,
                documentCollectionIdentityPreserved:
                  document.getElementsByTagName("main") === oldDocumentMains,
                documentCollectionTracksReplacement:
                  Array.from(oldDocumentMains, node => node.id).join(","),
                detachedCollectionIdentityPreserved:
                  oldBody.children === oldBodyChildren,
                detachedCollectionKeepsOldTree:
                  Array.from(oldBodyChildren, node => node.id || node.localName).join(","),
                oldNodeStillMatches: oldNode.matches('#old'),
                oldBodyConnected: oldBody.isConnected,
                oldShadowIdentityPreserved: oldShadowHost.shadowRoot === oldShadow,
                oldShadowChildIdentityPreserved: oldShadow.firstChild === oldShadowChild,
                oldShadowText: oldShadowChild.textContent,
                oldShadowConnected: oldShadow.isConnected,
                listenerRuns,
                oldLookupMissing: document.getElementById("old") === null,
                newText: document.getElementById("new")?.textContent
              });
            })()
            "#,
        )
        .expect("document.open identity and detached-tree probe should evaluate");

    assert_eq!(
        result,
        r#"{"sameDocument":true,"oldNodeConnected":false,"oldNodeText":"old text","oldNodeOwnerPreserved":true,"oldNodeParentPreserved":true,"oldTextIdentityPreserved":true,"oldClassListIdentityPreserved":true,"oldDatasetIdentityPreserved":true,"oldStyleIdentityPreserved":true,"documentCollectionIdentityPreserved":true,"documentCollectionTracksReplacement":"new","detachedCollectionIdentityPreserved":true,"detachedCollectionKeepsOldTree":"old,section","oldNodeStillMatches":true,"oldBodyConnected":false,"oldShadowIdentityPreserved":true,"oldShadowChildIdentityPreserved":true,"oldShadowText":"shadow text","oldShadowConnected":false,"listenerRuns":{"node":0,"document":0,"window":0,"handler":0,"preDetachedNode":1,"preDetachedHandler":1},"oldLookupMissing":true,"newText":"new text"}"#
    );
}

// Ported from WPT opening-the-input-stream/active.window.js and
// mutation-observer.window.js. Document.open() performs the all-children
// removal synchronously; the replacement parser must not pre-create an HTML
// shell before it consumes input.
#[test]
fn document_open_immediately_empties_document_and_reports_one_removal_batch() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-active.test/",
        "<html><body><main id=old>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const oldDocument = document;
              const oldHtml = document.documentElement;
              const observer = new MutationObserver(() => {});
              observer.observe(document, { childList: true, subtree: true });

              const returned = document.open();
              const firstRecords = observer.takeRecords().map(record => ({
                target: record.target.nodeName,
                added: Array.from(record.addedNodes, node => node.nodeName),
                removed: Array.from(record.removedNodes, node => node.nodeName),
                removedOldHtml: record.removedNodes[0] === oldHtml,
              }));
              const firstState = {
                returnedSameDocument: returned === oldDocument,
                childCount: document.childNodes.length,
                documentElementIsNull: document.documentElement === null,
                bodyIsNull: document.body === null,
                readyState: document.readyState,
                firstRecords,
              };

              document.open();
              const secondRecords = observer.takeRecords().length;
              const secondChildCount = document.childNodes.length;
              document.close();

              return JSON.stringify({
                ...firstState,
                secondRecords,
                secondChildCount,
              });
            })()
            "#,
        )
        .expect("document.open active-state probe should evaluate");

    assert_eq!(
        result,
        r##"{"returnedSameDocument":true,"childCount":0,"documentElementIsNull":true,"bodyIsNull":true,"readyState":"loading","firstRecords":[{"target":"#document","added":[],"removed":["HTML"],"removedOldHtml":true}],"secondRecords":0,"secondChildCount":0}"##
    );
}

// Ported from WPT opening-the-input-stream/active.window.js. A script-created
// document parser mutates the live Document as each write is consumed; waiting
// until close() and swapping in a completed foreign tree is observably wrong.
#[test]
fn document_write_incrementally_updates_the_live_document_before_close() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-incremental.test/",
        "<html><body><main>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              const afterOpen = document.childNodes.length;
              document.write('<!doctype html>');
              const afterDoctypeWrite = {
                childCount: document.childNodes.length,
                firstType: document.firstChild?.nodeType,
                firstName: document.firstChild?.nodeName,
              };
              document.close();
              const afterClose = {
                childCount: document.childNodes.length,
                hasHtml: document.documentElement?.nodeName,
              };

              document.write();
              const afterImplicitOpen = document.childNodes.length;
              document.write('<!doctype html>');
              const afterSecondDoctypeWrite = document.childNodes.length;
              document.close();
              return JSON.stringify({
                afterOpen,
                afterDoctypeWrite,
                afterClose,
                afterImplicitOpen,
                afterSecondDoctypeWrite,
                finalChildCount: document.childNodes.length,
              });
            })()
            "#,
        )
        .expect("incremental document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"afterOpen":0,"afterDoctypeWrite":{"childCount":1,"firstType":10,"firstName":"html"},"afterClose":{"childCount":2,"hasHtml":"HTML"},"afterImplicitOpen":0,"afterSecondDoctypeWrite":1,"finalChildCount":2}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/003.html,
// 004.html, 009.html, and 015.html. Each write continues the same tokenizer;
// it must not parse the calls as independent fragments.
#[test]
fn document_write_keeps_tokenizer_state_across_split_tag_and_attribute_input() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-split-tag.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<');
              document.write('i id');
              document.write("='test'");
              document.write(" class='a'>Filler");
              document.write(' Text</');
              document.write('i>');
              const element = document.body.firstChild;
              const snapshot = {
                name: element.localName,
                id: element.id,
                className: element.className,
                text: element.textContent,
                childCount: document.body.childNodes.length,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("split tag and attribute document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"name":"i","id":"test","className":"a","text":"Filler Text","childCount":1}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/018.html,
// 042.html, and 044-046.html. These tokenizer states intentionally span
// separate document.write calls.
#[test]
fn document_write_keeps_comment_character_reference_and_rcdata_state_across_calls() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-split-tokenizer-states.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<body><!');
              document.write('--com');
              document.write('ment-->');
              document.write('<span>&not');
              document.write('in;abc</span>');
              document.write('<textarea><span>');
              document.write('Filler</span></text');
              document.write('area>');
              const nodes = document.body.childNodes;
              const snapshot = {
                commentType: nodes[0].nodeType,
                comment: nodes[0].data,
                entity: nodes[1].textContent,
                textareaText: nodes[2].textContent,
                textareaChildren: nodes[2].childNodes.length,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("split tokenizer state document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"commentType":8,"comment":"comment","entity":"∉abc","textareaText":"<span>Filler</span>","textareaChildren":1}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/034-036.html.
#[test]
fn document_write_keeps_foreign_content_cdata_state_across_calls() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-split-cdata.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<body><svg><!');
              document.write('[CDATA[Filler');
              document.write(' Text]]></svg>');
              const svg = document.body.firstChild;
              const snapshot = {
                name: svg.localName,
                namespace: svg.namespaceURI,
                text: svg.textContent,
                childType: svg.firstChild.nodeType,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("split foreign-content CDATA document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"name":"svg","namespace":"http://www.w3.org/2000/svg","text":"Filler Text","childType":3}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/051.html. CRLF
// preprocessing must retain the pending CR across parser input chunks.
#[test]
fn document_write_normalizes_newlines_across_call_boundaries() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-newline-boundary.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<body>');
              document.write('\r');
              document.write('\nA');
              document.write('\rB');
              document.close();
              return JSON.stringify(document.body.textContent);
            })()
            "#,
        )
        .expect("cross-call newline normalization probe should evaluate");

    assert_eq!(result, r##""\nA\nB""##);
}

// Ported from WPT opening-the-input-stream/quirks.window.js. open() itself
// resets the mode synchronously; the tokenizer changes it only after a full
// doctype token has been consumed, even when the token spans writes.
#[test]
fn document_open_resets_compat_mode_and_parser_updates_it_incrementally() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-quirks.test/",
        "<html><body>quirks</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const modes = [document.compatMode];
              document.open();
              modes.push(document.compatMode);
              document.write('<!doctype html public');
              modes.push(document.compatMode);
              document.write(' "-//IETF//DTD HTML 3//"');
              modes.push(document.compatMode);
              document.write('>');
              modes.push(document.compatMode);
              document.close();
              modes.push(document.compatMode);

              document.open();
              modes.push(document.compatMode);
              document.write('<!doctype html');
              modes.push(document.compatMode);
              document.write('>');
              modes.push(document.compatMode);
              document.close();
              modes.push(document.compatMode);
              return modes.join('|');
            })()
            "#,
        )
        .expect("document.open quirks-mode probe should evaluate");

    assert_eq!(
        result,
        "BackCompat|CSS1Compat|CSS1Compat|CSS1Compat|BackCompat|BackCompat|CSS1Compat|CSS1Compat|CSS1Compat|CSS1Compat"
    );
}

// Ported from WPT opening-the-input-stream/custom-element.window.js. The
// dynamic-markup counter is a parser construction guard, not a blanket custom
// element construction guard.
#[test]
fn document_open_is_allowed_in_create_element_custom_element_constructor() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-create-element-constructor.test/",
        "<!doctype html><html><body><main>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              let returnedDocument = null;
              let errorName = null;
              class OpenFromConstructor extends HTMLElement {
                constructor() {
                  super();
                  try {
                    returnedDocument = document.open();
                  } catch (error) {
                    errorName = error.name;
                  }
                }
              }
              customElements.define('x-open-from-constructor', OpenFromConstructor);
              const element = document.createElement('x-open-from-constructor');
              const snapshot = {
                errorName,
                returnedSameDocument: returnedDocument === document,
                constructed: element instanceof OpenFromConstructor,
                childCountAfterOpen: document.childNodes.length,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("createElement custom element document.open probe should evaluate");

    assert_eq!(
        result,
        r##"{"errorName":null,"returnedSameDocument":true,"constructed":true,"childCountAfterOpen":0}"##
    );
}

// Ported from WPT opening-the-input-stream/tasks.window.js. Document-owned
// parser/script continuations are invalidated by open(), but an ordinary task
// already queued on the associated Window remains queued.
#[tokio::test]
async fn document_open_keeps_already_queued_window_timer_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://document-open-keeps-window-task.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              globalThis.__documentOpenTaskLog = [];
              setTimeout(() => {
                __documentOpenTaskLog.push(document.getElementById('replacement')?.textContent);
              }, 0);
              document.open();
              document.write('<main id=replacement>new document</main>');
              document.close();
              return __documentOpenTaskLog.length;
            })()
            "#,
        )
        .expect("document.open queued timer setup should evaluate"),
        "0"
    );

    for _ in 0..4 {
        let _ = vm
            .run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("the exact timer body should advance the pre-open Window timer");
        if vm
            .eval("__documentOpenTaskLog.length")
            .expect("document.open task count should evaluate")
            == "1"
        {
            break;
        }
    }

    assert_eq!(
        vm.eval("__documentOpenTaskLog.join('|')")
            .expect("document.open task result should evaluate"),
        "new document"
    );
}

#[test]
fn document_open_coalesces_doctype_and_element_removal_into_one_record() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-remove-all.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const oldChildren = Array.from(document.childNodes);
              const observer = new MutationObserver(() => {});
              observer.observe(document, { childList: true });
              document.open();
              const records = observer.takeRecords();
              const result = {
                oldChildCount: oldChildren.length,
                recordCount: records.length,
                removedCount: records[0]?.removedNodes.length,
                exactIdentityAndOrder: Array.from(
                  records[0]?.removedNodes || [],
                  (node, index) => node === oldChildren[index]
                ).every(Boolean),
                addedCount: records[0]?.addedNodes.length,
                childCount: document.childNodes.length,
              };
              document.close();
              return JSON.stringify(result);
            })()
            "#,
        )
        .expect("document.open all-children mutation probe should evaluate");

    assert_eq!(
        result,
        r#"{"oldChildCount":2,"recordCount":1,"removedCount":2,"exactIdentityAndOrder":true,"addedCount":0,"childCount":0}"#
    );
}

#[test]
fn document_writeln_uses_document_prototype_and_appends_newline() {
    let mut vm = new_storage_test_vm("https://document-writeln-webidl.test/");

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
              const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "writeln");
              document.open();
              document.writeln(
                "<main id='",
                { toString() { return "line"; } },
                "'>value</main>"
              );
              document.close();
              return [
                typeof descriptor.value,
                descriptor.value.length,
                descriptor.enumerable,
                descriptor.configurable,
                Object.prototype.hasOwnProperty.call(document, "writeln"),
                document.getElementById("line")?.textContent,
                JSON.stringify(document.body.innerHTML),
                probe(() => document.writeln(Symbol("chunk"))),
                document.getElementById("line")?.textContent
              ].join("|");
            })()
            "#,
        )
        .expect("Document.writeln WebIDL variadic argument probe should evaluate");

    assert_eq!(
        result,
        "function|0|true|true|false|value|\"<main id=\\\"line\\\">value</main>\\n\"|TypeError|value"
    );
}

#[test]
fn document_write_replacement_style_sources_drive_has_invalidation() {
    let mut vm = new_storage_test_vm("https://document-write-style-source.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write(`
                <!doctype html>
                <style>body:has(.marker) .target { color: rgb(1, 2, 3); }</style>
                <main><div id="target" class="target"></div></main>
              `);
              document.close();
              const target = document.getElementById("target");
              const before = getComputedStyle(target).color;
              const marker = document.createElement("span");
              marker.className = "marker";
              document.body.appendChild(marker);
              return `${before}|${getComputedStyle(target).color}`;
            })()
            "#,
        )
        .expect("document.write replacement style invalidation probe should evaluate");

    assert_eq!(result, "rgb(0, 0, 0)|rgb(1, 2, 3)");
}

#[test]
fn inline_style_has_not_any_link_invalidates_on_plain_child_insertion() {
    let mut vm = new_storage_test_vm("https://link-pseudo-has-invalidation.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write(`
                <!doctype html>
                <style>
                  #parent { color: rgb(0, 0, 255); }
                  #grandparent { color: rgb(0, 0, 255); }
                  #parent:has(> :not(:link)) { color: rgb(128, 128, 128); }
                  #parent:has(> :link) { color: rgb(0, 128, 0); }
                  #parent:has(> :visited) { color: rgb(255, 0, 0); }
                  #grandparent:has(:not(:any-link)) { color: rgb(128, 128, 128); }
                  #grandparent:has(:any-link) { color: rgb(0, 128, 0); }
                </style>
                <div id="grandparent"></div>
              `);
              document.close();
              const grandparent = document.getElementById("grandparent");
              const before = getComputedStyle(grandparent).color;
              const parent = document.createElement("div");
              parent.id = "parent";
              grandparent.appendChild(parent);
              return [
                before,
                getComputedStyle(grandparent).color,
                getComputedStyle(parent).color
              ].join("|");
            })()
            "#,
        )
        .expect(":not(:any-link) invalidation probe should evaluate");

    assert_eq!(result, "rgb(0, 0, 255)|rgb(128, 128, 128)|rgb(0, 0, 255)");
}

#[test]
fn document_visibility_and_default_view_follow_receiver_across_realms() {
    let mut vm = new_storage_test_vm("https://document-visibility-realms.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "visibility-live-frame";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("visibility child frame should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "document visibility and defaultView child Realm",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const errors = [];
  const check = (condition, message) => { if (!condition) errors.push(message); };
  const childWindow = document.getElementById("visibility-live-frame").contentWindow;
  const child = childWindow.document;
  const parentView = Object.getOwnPropertyDescriptor(Document.prototype, "defaultView").get;
  const childView = Object.getOwnPropertyDescriptor(childWindow.Document.prototype, "defaultView").get;
  const parentHidden = Object.getOwnPropertyDescriptor(Document.prototype, "hidden").get;
  const childHidden = Object.getOwnPropertyDescriptor(childWindow.Document.prototype, "hidden").get;
  const parentVisibility = Object.getOwnPropertyDescriptor(Document.prototype, "visibilityState").get;
  const childVisibility = Object.getOwnPropertyDescriptor(childWindow.Document.prototype, "visibilityState").get;
  check(parentView.call(document) === window, "parent defaultView");
  check(parentView.call(child) === childWindow, "parent getter on child defaultView");
  check(childView.call(document) === window, "child getter on parent defaultView");
  check(childView.call(child) === childWindow, "child defaultView");
  check(!parentHidden.call(child) && parentVisibility.call(child) === "visible", "child is visible through parent getters");
  check(!childHidden.call(document) && childVisibility.call(document) === "visible", "parent is visible through child getters");
  const windowless = child.implementation.createHTMLDocument("");
  check(childView.call(windowless) === null, "windowless defaultView through child getter");
  check(parentHidden.call(windowless) && parentVisibility.call(windowless) === "hidden", "windowless remains hidden through parent getters");
  const popup = window.open("about:blank", "visibility-popup");
  check(popup !== null, "popup created");
  try {
    check(!popup.document.hidden && popup.document.visibilityState === "visible", "live popup is visible");
    check(parentView.call(popup.document) === popup, "popup defaultView through parent getter");
  } finally {
    popup.close();
  }
  return JSON.stringify(errors);
})()
"#,
        )
        .expect("cross-realm document visibility and view probe should evaluate");

    assert_eq!(result, "[]");
}
