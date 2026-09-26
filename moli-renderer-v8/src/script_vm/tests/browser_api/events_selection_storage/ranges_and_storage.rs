use super::*;

#[test]
fn range_clone_contents_returns_fragment_for_ancestor_to_descendant_boundary() {
    let mut vm = new_storage_test_vm("https://range-clone-ancestor-boundary.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const line = document.createElement("div");
              line.className = "ace-line";
              line.innerHTML = '<span data-marker="1">Alpha</span><span data-hit="1">Beta</span>';
              (document.body || document.documentElement || document).append(line);

              const range = document.createRange();
              range.setStart(line, 0);
              range.setEnd(line.querySelector("[data-hit]").firstChild, 2);
              const fragment = range.cloneContents();

              return JSON.stringify({
                tag: Object.prototype.toString.call(fragment),
                instance: fragment instanceof DocumentFragment,
                hasQuerySelectorAll: typeof fragment.querySelectorAll === "function",
                text: fragment.textContent,
                hitCount: fragment.querySelectorAll("[data-hit]").length,
                hitText: fragment.querySelector("[data-hit]").textContent,
                originalText: line.textContent
              });
            })()
            "#,
        )
        .expect("ancestor-to-descendant Range.cloneContents probe should evaluate");

    assert_eq!(
        result,
        r#"{"tag":"[object DocumentFragment]","instance":true,"hasQuerySelectorAll":true,"text":"AlphaBe","hitCount":1,"hitText":"Be","originalText":"AlphaBeta"}"#
    );
}

#[test]
fn range_clone_contents_preserves_partial_boundary_structure() {
    let mut vm = new_storage_test_vm("https://range-clone-partial-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              let parent = document.body || document.documentElement;
              if (!parent) {
                parent = document.createElement("main");
                document.appendChild(parent);
              }

              const ancestorEnd = document.createElement("div");
              ancestorEnd.innerHTML =
                '<span data-left="1">Alpha</span><span data-mid="1">Beta</span><span data-right="1">Gamma</span>';
              parent.append(ancestorEnd);
              const ancestorRange = document.createRange();
              ancestorRange.setStart(ancestorEnd.querySelector("[data-left]").firstChild, 2);
              ancestorRange.setEnd(ancestorEnd, 2);
              const ancestorFragment = ancestorRange.cloneContents();

              const cross = document.createElement("div");
              cross.innerHTML =
                '<section data-left-branch="1"><b>Alpha</b><i>Aft</i></section>' +
                '<p data-middle="1">Middle</p>' +
                '<section data-right-branch="1"><b>Omega</b><i>Tail</i></section>';
              parent.append(cross);
              const crossRange = document.createRange();
              crossRange.setStart(cross.querySelector("[data-left-branch] b").firstChild, 2);
              crossRange.setEnd(cross.querySelector("[data-right-branch] b").firstChild, 2);
              const crossFragment = crossRange.cloneContents();

              const childOffset = document.createElement("div");
              childOffset.innerHTML = '<a>One</a><b>Two</b><c>Three</c>';
              parent.append(childOffset);
              const childOffsetRange = document.createRange();
              childOffsetRange.setStart(childOffset, 1);
              childOffsetRange.setEnd(childOffset, 2);
              const childOffsetFragment = childOffsetRange.cloneContents();

              return JSON.stringify({
                ancestorText: ancestorFragment.textContent,
                ancestorLeftText: ancestorFragment.querySelector("[data-left]").textContent,
                ancestorMidCount: ancestorFragment.querySelectorAll("[data-mid]").length,
                ancestorRightCount: ancestorFragment.querySelectorAll("[data-right]").length,
                ancestorOriginal: ancestorEnd.textContent,
                crossText: crossFragment.textContent,
                crossLeftText: crossFragment.querySelector("[data-left-branch]").textContent,
                crossMiddleCount: crossFragment.querySelectorAll("[data-middle]").length,
                crossRightText: crossFragment.querySelector("[data-right-branch]").textContent,
                crossOriginal: cross.textContent,
                childOffsetText: childOffsetFragment.textContent,
                childOffsetChildNames: Array.from(childOffsetFragment.childNodes)
                  .map((node) => node.localName)
                  .join(",")
              });
            })()
            "#,
        )
        .expect("partial-boundary Range.cloneContents probes should evaluate");

    assert_eq!(
        result,
        r#"{"ancestorText":"phaBeta","ancestorLeftText":"pha","ancestorMidCount":1,"ancestorRightCount":0,"ancestorOriginal":"AlphaBetaGamma","crossText":"phaAftMiddleOm","crossLeftText":"phaAft","crossMiddleCount":1,"crossRightText":"Om","crossOriginal":"AlphaAftMiddleOmegaTail","childOffsetText":"Two","childOffsetChildNames":"b"}"#
    );
}

#[test]
fn range_extract_delete_contents_preserve_partial_boundary_structure() {
    let mut vm = new_storage_test_vm("https://range-extract-delete-partial-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              let parent = document.body || document.documentElement;
              if (!parent) {
                parent = document.createElement("main");
                document.appendChild(parent);
              }

              const ancestorEnd = document.createElement("div");
              ancestorEnd.innerHTML =
                '<span data-left="1">Alpha</span><span data-mid="1">Beta</span><span data-right="1">Gamma</span>';
              parent.append(ancestorEnd);
              const ancestorRange = document.createRange();
              ancestorRange.setStart(ancestorEnd.querySelector("[data-left]").firstChild, 2);
              ancestorRange.setEnd(ancestorEnd, 2);
              const ancestorFragment = ancestorRange.extractContents();

              const cross = document.createElement("div");
              cross.innerHTML =
                '<section data-left-branch="1"><b>Alpha</b><i>Aft</i></section>' +
                '<p data-middle="1">Middle</p>' +
                '<section data-right-branch="1"><b>Omega</b><i>Tail</i></section>';
              parent.append(cross);
              const crossRange = document.createRange();
              crossRange.setStart(cross.querySelector("[data-left-branch] b").firstChild, 2);
              crossRange.setEnd(cross.querySelector("[data-right-branch] b").firstChild, 2);
              const crossFragment = crossRange.extractContents();

              const childOffset = document.createElement("div");
              childOffset.innerHTML = '<a>One</a><b>Two</b><c>Three</c>';
              parent.append(childOffset);
              const childOffsetRange = document.createRange();
              childOffsetRange.setStart(childOffset, 1);
              childOffsetRange.setEnd(childOffset, 2);
              const childOffsetFragment = childOffsetRange.extractContents();

              const deletion = document.createElement("div");
              deletion.innerHTML = '<span data-left="1">Alpha</span><span data-hit="1">Beta</span>';
              parent.append(deletion);
              const deleteRange = document.createRange();
              deleteRange.setStart(deletion, 0);
              deleteRange.setEnd(deletion.querySelector("[data-hit]").firstChild, 2);
              const deleteReturn = deleteRange.deleteContents();

              return JSON.stringify({
                ancestorFragmentText: ancestorFragment.textContent,
                ancestorFragmentLeftText: ancestorFragment.querySelector("[data-left]").textContent,
                ancestorFragmentMidCount: ancestorFragment.querySelectorAll("[data-mid]").length,
                ancestorOriginal: ancestorEnd.textContent,
                ancestorCollapsed: [
                  ancestorRange.collapsed,
                  ancestorRange.startContainer === ancestorEnd,
                  ancestorRange.startOffset
                ].join(":"),
                crossFragmentText: crossFragment.textContent,
                crossLeftText: crossFragment.querySelector("[data-left-branch]").textContent,
                crossMiddleCount: crossFragment.querySelectorAll("[data-middle]").length,
                crossRightText: crossFragment.querySelector("[data-right-branch]").textContent,
                crossOriginal: cross.textContent,
                crossCollapsed: [
                  crossRange.collapsed,
                  crossRange.startContainer === cross,
                  crossRange.startOffset
                ].join(":"),
                childOffsetFragmentText: childOffsetFragment.textContent,
                childOffsetOriginal: childOffset.textContent,
                childOffsetCollapsed: [
                  childOffsetRange.collapsed,
                  childOffsetRange.startContainer === childOffset,
                  childOffsetRange.startOffset
                ].join(":"),
                deleteReturn: String(deleteReturn),
                deleteOriginal: deletion.textContent,
                deleteHitText: deletion.querySelector("[data-hit]").textContent,
                deleteCollapsed: [
                  deleteRange.collapsed,
                  deleteRange.startContainer === deletion,
                  deleteRange.startOffset
                ].join(":")
              });
            })()
            "#,
        )
        .expect("partial-boundary Range.extractContents/deleteContents probes should evaluate");

    assert_eq!(
        result,
        r#"{"ancestorFragmentText":"phaBeta","ancestorFragmentLeftText":"pha","ancestorFragmentMidCount":1,"ancestorOriginal":"AlGamma","ancestorCollapsed":"true:true:1","crossFragmentText":"phaAftMiddleOm","crossLeftText":"phaAft","crossMiddleCount":1,"crossRightText":"Om","crossOriginal":"AlegaTail","crossCollapsed":"true:true:1","childOffsetFragmentText":"Two","childOffsetOriginal":"OneThree","childOffsetCollapsed":"true:true:1","deleteReturn":"undefined","deleteOriginal":"ta","deleteHitText":"ta","deleteCollapsed":"true:true:0"}"#
    );
}

#[test]
fn range_contents_handles_cdata_pi_foreign_text_and_doctype_edges() {
    let mut vm = new_storage_test_vm("https://range-contents-character-data-edges.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const thrownName = (callback) => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error && error.name}:${error && error.code}`;
                }
              };

              const xmlDoc = document.implementation.createDocument(null, "root");
              const cdataForClone = xmlDoc.createCDATASection("1234");
              xmlDoc.documentElement.appendChild(cdataForClone);
              const cdataCloneRange = xmlDoc.createRange();
              cdataCloneRange.setStart(cdataForClone, 1);
              cdataCloneRange.setEnd(cdataForClone, 3);
              const cdataClone = cdataCloneRange.cloneContents().firstChild;

              const cdataForDelete = xmlDoc.createCDATASection("5678");
              xmlDoc.documentElement.appendChild(cdataForDelete);
              const cdataDeleteRange = xmlDoc.createRange();
              cdataDeleteRange.setStart(cdataForDelete, 1);
              cdataDeleteRange.setEnd(cdataForDelete, 3);
              cdataDeleteRange.deleteContents();

              const piForClone = xmlDoc.createProcessingInstruction("somePI", "abcdef");
              xmlDoc.documentElement.appendChild(piForClone);
              const piCloneRange = xmlDoc.createRange();
              piCloneRange.setStart(piForClone, 1);
              piCloneRange.setEnd(piForClone, 4);
              const piClone = piCloneRange.cloneContents().firstChild;

              const piForExtract = xmlDoc.createProcessingInstruction("otherPI", "uvwxyz");
              xmlDoc.documentElement.appendChild(piForExtract);
              const piExtractRange = xmlDoc.createRange();
              piExtractRange.setStart(piForExtract, 2);
              piExtractRange.setEnd(piForExtract, 5);
              const piExtract = piExtractRange.extractContents().firstChild;

              const foreignDoc = document.implementation.createHTMLDocument("");
              const foreignText = foreignDoc.createTextNode("Efghijkl");
              foreignDoc.body.appendChild(foreignText);
              const foreignRange = foreignDoc.createRange();
              foreignRange.setStart(foreignText, 2);
              foreignRange.setEnd(foreignText, 8);
              const foreignFragment = foreignRange.extractContents();

              const doctypeDoc = document.implementation.createHTMLDocument("");
              if (!doctypeDoc.doctype) {
                doctypeDoc.insertBefore(
                  document.implementation.createDocumentType("html", "", ""),
                  doctypeDoc.firstChild
                );
              }
              const doctypeCloneRange = doctypeDoc.createRange();
              doctypeCloneRange.setStart(doctypeDoc, 0);
              doctypeCloneRange.setEnd(doctypeDoc, 1);
              const doctypeExtractRange = doctypeDoc.createRange();
              doctypeExtractRange.setStart(doctypeDoc, 0);
              doctypeExtractRange.setEnd(doctypeDoc, 1);

              return JSON.stringify({
                cdataClone: [
                  cdataClone.nodeType,
                  cdataClone.nodeName,
                  cdataClone.data,
                  cdataForClone.data
                ],
                cdataDelete: [
                  cdataForDelete.data,
                  cdataDeleteRange.collapsed,
                  cdataDeleteRange.startContainer === cdataForDelete,
                  cdataDeleteRange.startOffset
                ],
                piClone: [
                  piClone.nodeType,
                  piClone.target,
                  piClone.data,
                  piForClone.data
                ],
                piExtract: [
                  piExtract.nodeType,
                  piExtract.target,
                  piExtract.data,
                  piForExtract.data,
                  piExtractRange.collapsed,
                  piExtractRange.startContainer === piForExtract,
                  piExtractRange.startOffset
                ],
                foreignText: [
                  foreignFragment.firstChild.data,
                  foreignText.data,
                  foreignRange.collapsed,
                  foreignRange.startContainer === foreignText,
                  foreignRange.startOffset
                ],
                doctype: [
                  thrownName(() => doctypeCloneRange.cloneContents()),
                  thrownName(() => doctypeExtractRange.extractContents()),
                  doctypeDoc.doctype.parentNode === doctypeDoc
                ]
              });
            })()
            "#,
        )
        .expect("Range contents character data edge probe should evaluate");

    assert_eq!(
        result,
        r##"{"cdataClone":[4,"#cdata-section","23","1234"],"cdataDelete":["58",true,true,1],"piClone":[7,"somePI","bcd","abcdef"],"piExtract":[7,"otherPI","wxy","uvz",true,true,2],"foreignText":["ghijkl","Ef",true,true,2],"doctype":["HierarchyRequestError:3","HierarchyRequestError:3",true]}"##
    );
}

#[test]
fn selection_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://selection-prototype-methods.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
            (function* () {
              const methods = [
                ["getRangeAt", 1],
                ["addRange", 1],
                ["removeRange", 1],
                ["removeAllRanges", 0],
                ["empty", 0],
                ["collapse", 1],
                ["setPosition", 1],
                ["collapseToStart", 0],
                ["collapseToEnd", 0],
                ["extend", 1],
                ["selectAllChildren", 1],
                ["setBaseAndExtent", 4],
                ["containsNode", 1],
                ["deleteFromDocument", 0],
                ["modify", 0],
                ["toString", 0]
              ];
              const accessors = [
                "anchorNode",
                "anchorOffset",
                "focusNode",
                "focusOffset",
                "isCollapsed",
                "rangeCount",
                "type",
                "direction"
              ];
              const stringify = (value) =>
                value === undefined ? "undefined" : String(value);
              const selection = getSelection();
              const descriptors = methods.map(([name, length]) => {
                const descriptor = Object.getOwnPropertyDescriptor(Selection.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable,
                  Object.hasOwn(selection, name),
                  descriptor?.value === selection[name],
                  descriptor?.value?.length === length
                ].join(":");
              });
              const accessorDescriptors = accessors.map((name) => {
                const descriptor = Object.getOwnPropertyDescriptor(Selection.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable,
                  Object.hasOwn(selection, name)
                ].join(":");
              });
              const enumerableMethods = Object.keys(Selection.prototype)
                .filter((name) => methods.some(([method]) => method === name))
                .join(",");
              const enumerableAccessors = Object.keys(Selection.prototype)
                .filter((name) => accessors.includes(name))
                .join(",");

              const host = document.createElement("div");
              const text = document.createTextNode("abcdef");
              host.appendChild(text);
              let parent = document.body || document.documentElement;
              if (!parent) {
                parent = document.createElement("html");
                document.appendChild(parent);
              }
              parent.appendChild(host);
              const range = document.createRange();
              range.setStart(text, 1);
              range.setEnd(text, 4);
              selection.removeAllRanges();
              selection.addRange(range);
              const ownSlots = Object.getOwnPropertyNames(selection)
                .filter((name) => name.startsWith("__moliSelection"))
                .sort();
              for (const slot of [
                "__moliSelectionRange",
                "__moliSelectionAnchorNode",
                "__moliSelectionAnchorOffset",
                "__moliSelectionFocusNode",
                "__moliSelectionFocusOffset",
                "__moliSelectionDirection"
              ]) {
                Selection.prototype[slot] = "prototype-spoof";
                selection[slot] = "own-spoof";
              }
              yield; // Publish this scene before reading its geometry.
const behavior = [
                selection.getRangeAt(0) === range,
                selection.toString(),
                selection.containsNode(text, true),
                selection.rangeCount
              ].join(":");
              const attributeValues = [
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset,
                selection.isCollapsed,
                selection.rangeCount,
                selection.type,
                selection.direction
              ].join(":");
              const fake = Object.create(Selection.prototype);
              const fakeValues = [
                fake.anchorNode,
                fake.anchorOffset,
                fake.focusNode,
                fake.focusOffset,
                fake.isCollapsed,
                fake.rangeCount,
                fake.type,
                fake.direction
              ].map(stringify).join(":");
              selection.empty();
              return JSON.stringify({
                descriptors,
                accessorDescriptors,
                enumerableMethods,
                enumerableAccessors,
                behavior,
                attributeValues,
                fakeValues,
                ownSlots,
                afterEmpty: [
                  selection.rangeCount,
                  selection.anchorNode === null,
                  selection.focusNode === null
                ].join(":")
              });
            })()
            "#,
    )
    .expect("Selection prototype method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["getRangeAt:true:function:getRangeAt:1:true:true:true:false:true:true","addRange:true:function:addRange:1:true:true:true:false:true:true","removeRange:true:function:removeRange:1:true:true:true:false:true:true","removeAllRanges:true:function:removeAllRanges:0:true:true:true:false:true:true","empty:true:function:empty:0:true:true:true:false:true:true","collapse:true:function:collapse:1:true:true:true:false:true:true","setPosition:true:function:setPosition:1:true:true:true:false:true:true","collapseToStart:true:function:collapseToStart:0:true:true:true:false:true:true","collapseToEnd:true:function:collapseToEnd:0:true:true:true:false:true:true","extend:true:function:extend:1:true:true:true:false:true:true","selectAllChildren:true:function:selectAllChildren:1:true:true:true:false:true:true","setBaseAndExtent:true:function:setBaseAndExtent:4:true:true:true:false:true:true","containsNode:true:function:containsNode:1:true:true:true:false:true:true","deleteFromDocument:true:function:deleteFromDocument:0:true:true:true:false:true:true","modify:true:function:modify:0:true:true:true:false:true:true","toString:true:function:toString:0:true:true:true:false:true:true"],"accessorDescriptors":["anchorNode:true:function:get anchorNode:0:undefined:true:true:false","anchorOffset:true:function:get anchorOffset:0:undefined:true:true:false","focusNode:true:function:get focusNode:0:undefined:true:true:false","focusOffset:true:function:get focusOffset:0:undefined:true:true:false","isCollapsed:true:function:get isCollapsed:0:undefined:true:true:false","rangeCount:true:function:get rangeCount:0:undefined:true:true:false","type:true:function:get type:0:undefined:true:true:false","direction:true:function:get direction:0:undefined:true:true:false"],"enumerableMethods":"getRangeAt,addRange,removeRange,removeAllRanges,empty,collapse,setPosition,collapseToStart,collapseToEnd,extend,selectAllChildren,setBaseAndExtent,containsNode,deleteFromDocument,modify,toString","enumerableAccessors":"anchorNode,anchorOffset,focusNode,focusOffset,isCollapsed,rangeCount,type,direction","behavior":"true:bcd:true:1","attributeValues":"true:1:true:4:false:1:Range:forward","fakeValues":"null:0:null:0:true:0:None:undefined","ownSlots":[],"afterEmpty":"0:true:true"}"#
    );
}

#[test]
fn selection_delete_from_document_uses_utf16_range_offsets() {
    let mut vm = new_storage_test_vm("https://selection-delete-utf16.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("a\uD83D\uDE00b");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);

              const range = document.createRange();
              range.setStart(text, 1);
              range.setEnd(text, 3);
              const selection = getSelection();
              selection.removeAllRanges();
              selection.addRange(range);
              selection.deleteFromDocument();

              const selectedRange = selection.getRangeAt(0);
              return [
                text.data,
                text.data.length,
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset,
                selectedRange.startContainer === text,
                selectedRange.startOffset,
                selectedRange.collapsed
              ].join("|");
            })()
            "#,
        )
        .expect("Selection.deleteFromDocument should use UTF-16 offsets");

    assert_eq!(result, "ab|2|true|1|true|1|true|1|true");
}

#[test]
fn child_window_range_constructors_match_child_document_ranges() {
    let mut vm = new_storage_test_vm("https://child-window-range-constructors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const iframe = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(iframe);
              const childWindow = iframe.contentWindow;
              const childDocument = iframe.contentDocument;

              const range = childDocument.createRange();
              range.selectNodeContents(childDocument.body);
              const constructedRange = new childWindow.Range();
              const staticRange = new childWindow.StaticRange({
                startContainer: childDocument,
                startOffset: 0,
                endContainer: childDocument,
                endOffset: childDocument.childNodes.length
              });

              return [
                typeof childWindow.AbstractRange,
                typeof childWindow.Range,
                typeof childWindow.StaticRange,
                childWindow.Range.length,
                childWindow.StaticRange.length,
                range instanceof childWindow.Range,
                range instanceof childWindow.AbstractRange,
                constructedRange.startContainer === childDocument,
                constructedRange.endContainer === childDocument,
                constructedRange.collapsed,
                staticRange instanceof childWindow.StaticRange,
                staticRange instanceof childWindow.AbstractRange,
                staticRange.startContainer === childDocument,
                staticRange.endOffset === childDocument.childNodes.length
              ].join("|");
            })()
            "#,
        )
        .expect("child window Range constructors should evaluate");

    assert_eq!(
        result,
        "function|function|function|0|1|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn detached_document_remove_child_updates_live_range_boundaries() {
    let mut vm = new_storage_test_vm("https://detached-document-range-remove.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const documentRange = doc.createRange();
              documentRange.setStart(doc, 0);
              documentRange.setEnd(doc, doc.childNodes.length);
              doc.removeChild(doc.documentElement);
              const expectedDocumentEnd = doc.childNodes.length;

              const descendantDoc = document.implementation.createHTMLDocument("");
              const child = descendantDoc.createElement("span");
              const text = descendantDoc.createTextNode("x");
              child.appendChild(text);
              descendantDoc.body.appendChild(child);
              const descendantRange = descendantDoc.createRange();
              descendantRange.setStart(text, 0);
              descendantRange.setEnd(child, 1);
              descendantDoc.body.removeChild(child);

              return [
                documentRange.startContainer === doc,
                documentRange.startOffset,
                documentRange.endContainer === doc,
                documentRange.endOffset,
                expectedDocumentEnd,
                descendantRange.startContainer === descendantDoc.body,
                descendantRange.startOffset,
                descendantRange.endContainer === descendantDoc.body,
                descendantRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("detached document removeChild should update live ranges");

    assert_eq!(result, "true|0|true|1|1|true|0|true|0");
}

#[test]
fn detached_document_adopted_live_container_keeps_range_mutation_updates() {
    let mut vm = new_storage_test_vm("https://range-adopt-live-container.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function createRangeWithUnparentedContainerOfSingleElement() {
                const range = document.createRange();
                const container = document.createElement("container");
                const element = document.createElement("element");
                container.appendChild(element);
                range.selectNode(element);
                return range;
              }
              function nestRangeInOuterContainer(range) {
                range.startContainer.ownerDocument.createElement("outer").appendChild(range.startContainer);
              }
              function moveNodeToNewlyCreatedDocumentWithAppendChild(node) {
                document.implementation.createDocument(null, null).appendChild(node);
              }

              const direct = createRangeWithUnparentedContainerOfSingleElement();
              let directError = "";
              try { direct.startContainer.removeChild(direct.startContainer.firstChild); } catch (error) { directError = error.name; }

              const parentedMoved = createRangeWithUnparentedContainerOfSingleElement();
              nestRangeInOuterContainer(parentedMoved);
              let parentedMovedError = "";
              try { moveNodeToNewlyCreatedDocumentWithAppendChild(parentedMoved.startContainer); } catch (error) { parentedMovedError = error.name; }

              const parentlessMoved = createRangeWithUnparentedContainerOfSingleElement();
              let parentlessMovedError = "";
              let parentlessRemoveError = "";
              try { moveNodeToNewlyCreatedDocumentWithAppendChild(parentlessMoved.startContainer); } catch (error) { parentlessMovedError = error.name; }
              try { parentlessMoved.startContainer.removeChild(parentlessMoved.startContainer.firstChild); } catch (error) { parentlessRemoveError = error.name; }

              const outerMoved = createRangeWithUnparentedContainerOfSingleElement();
              nestRangeInOuterContainer(outerMoved);
              let outerMovedError = "";
              let outerRemoveError = "";
              try { moveNodeToNewlyCreatedDocumentWithAppendChild(outerMoved.startContainer.parentNode); } catch (error) { outerMovedError = error.name; }
              try { outerMoved.startContainer.removeChild(outerMoved.startContainer.firstChild); } catch (error) { outerRemoveError = error.name; }

              const errors = [
                directError,
                parentedMovedError,
                parentlessMovedError,
                parentlessRemoveError,
                outerMovedError,
                outerRemoveError
              ].filter(Boolean).join(",");

              return [
                errors,
                direct.endOffset,
                parentedMoved.endOffset,
                parentlessMoved.endOffset,
                outerMoved.endOffset,
                parentlessMoved.endContainer === parentlessMoved.startContainer,
                outerMoved.endContainer === outerMoved.startContainer
              ].join("|");
            })()
            "#,
        )
        .expect("adopted live container range mutation checks should evaluate");

    assert_eq!(result, "|0|0|0|0|true|true");
}

#[test]
fn removing_shadow_host_keeps_shadow_range_boundaries() {
    let mut vm = new_storage_test_vm("https://range-shadow-host-remove.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<div id="in-shadow">ABC</div>';
              const shadowChild = root.firstChild;
              (document.body || document.documentElement || document).appendChild(host);
              const hostRange = document.createRange();
              hostRange.setStart(shadowChild, 1);
              host.remove();

              const wrapper = document.createElement("div");
              const nestedHost = document.createElement("div");
              const nestedRoot = nestedHost.attachShadow({ mode: "open" });
              nestedRoot.innerHTML = '<div id="in-shadow">ABC</div>';
              const nestedShadowChild = nestedRoot.firstChild;
              wrapper.appendChild(nestedHost);
              (document.body || document.documentElement || document).appendChild(wrapper);
              const wrapperRange = document.createRange();
              wrapperRange.setStart(nestedShadowChild, 1);
              wrapper.remove();

              return [
                hostRange.startContainer === shadowChild,
                hostRange.startOffset,
                wrapperRange.startContainer === nestedShadowChild,
                wrapperRange.startOffset
              ].join("|");
            })()
            "#,
        )
        .expect("shadow host removal range checks should evaluate");

    assert_eq!(result, "true|1|true|1");
}

#[test]
fn pre_insert_updates_live_ranges_for_moved_nodes() {
    let mut vm = new_storage_test_vm("https://range-pre-insert-move.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const first = document.createElement("a");
              const moved = document.createElement("b");
              const last = document.createElement("c");
              host.append(first, moved, last);
              (document.body || document.documentElement || document).appendChild(host);

              const movedRange = document.createRange();
              movedRange.setStart(moved, 0);
              movedRange.setEnd(host, host.childNodes.length);
              host.appendChild(moved);

              const detachedDoc = document.implementation.createHTMLDocument("");
              const parent = detachedDoc.createElement("div");
              const left = detachedDoc.createElement("l");
              const right = detachedDoc.createElement("r");
              parent.append(left, right);
              detachedDoc.body.appendChild(parent);
              const detachedRange = detachedDoc.createRange();
              detachedRange.setStart(parent, 1);
              detachedRange.setEnd(parent, 2);
              parent.insertBefore(detachedDoc.createElement("x"), left);

              return [
                Array.from(host.childNodes).map(node => node.localName).join(","),
                movedRange.startContainer === host,
                movedRange.startOffset,
                movedRange.endContainer === host,
                movedRange.endOffset,
                Array.from(parent.childNodes).map(node => node.localName).join(","),
                detachedRange.startContainer === parent,
                detachedRange.startOffset,
                detachedRange.endContainer === parent,
                detachedRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("pre-insert live range checks should evaluate");

    assert_eq!(result, "a,c,b|true|1|true|2|x,l,r|true|2|true|3");
}

#[test]
fn detached_document_accepts_live_node_insert_and_updates_range() {
    let mut vm = new_storage_test_vm("https://range-pre-insert-live-to-detached.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const detachedDoc = document.implementation.createHTMLDocument("");
              const detachedRange = detachedDoc.createRange();
              detachedRange.setStart(detachedDoc, 0);
              detachedRange.setEnd(detachedDoc, detachedDoc.childNodes.length);
              const liveComment = document.createComment("live");
              let detachedInsert = "no-throw";
              try {
                detachedDoc.insertBefore(liveComment, detachedDoc.documentElement);
              } catch (error) {
                detachedInsert = `${error.name}:${error.code}`;
              }

              return [
                detachedInsert,
                detachedDoc.childNodes[1] === liveComment,
                liveComment.parentNode === detachedDoc,
                liveComment.ownerDocument === detachedDoc,
                detachedRange.startContainer === detachedDoc,
                detachedRange.startOffset,
                detachedRange.endContainer === detachedDoc,
                detachedRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("live node to detached document insert should evaluate");

    assert_eq!(result, "no-throw|true|true|true|true|0|true|3");
}

#[test]
fn live_document_adopts_detached_node_insert_and_updates_range() {
    let mut vm = new_storage_test_vm("https://range-pre-insert-detached-to-live.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const first = document.createTextNode("first");
              host.appendChild(first);
              (document.body || document.documentElement || document).appendChild(host);

              const xmlDoc = document.implementation.createDocument(null, null, null);
              const xmlElement = xmlDoc.createElement("x");
              const xmlText = xmlDoc.createTextNode("foreign");
              xmlElement.appendChild(xmlText);
              xmlDoc.appendChild(xmlElement);

              const range = document.createRange();
              range.setStart(host, 0);
              range.setEnd(host, 1);
              let thrown = "no-throw";
              try {
                host.insertBefore(xmlText, first);
              } catch (error) {
                thrown = `${error.name}:${error.code}`;
              }

              return [
                thrown,
                xmlText.parentNode === xmlElement,
                host.firstChild === first,
                range.startContainer === host,
                range.startOffset,
                range.endContainer === host,
                range.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("detached node to live document insert should evaluate");

    assert_eq!(result, "no-throw|false|false|true|0|true|2");
}

#[test]
fn replace_child_updates_live_ranges_in_remove_then_insert_order() {
    let mut vm = new_storage_test_vm("https://range-replace-child-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const first = document.createElement("a");
              const second = document.createElement("b");
              const oldText = document.createTextNode("old");
              first.appendChild(oldText);
              host.append(first, second);
              (document.body || document.documentElement || document).appendChild(host);

              const sameRange = document.createRange();
              sameRange.setStart(first, 0);
              sameRange.setEnd(first, 1);
              host.replaceChild(first, first);

              const movingHost = document.createElement("div");
              const movingOld = document.createTextNode("old");
              const movingNew = document.createElement("n");
              const spare = document.createElement("s");
              movingHost.append(movingOld, spare);
              const staging = document.createElement("div");
              staging.appendChild(movingNew);
              (document.body || document.documentElement || document).appendChild(staging);
              (document.body || document.documentElement || document).appendChild(movingHost);
              const movingRange = document.createRange();
              movingRange.setStart(movingHost, 0);
              movingRange.setEnd(movingHost, 1);
              movingHost.replaceChild(movingNew, movingOld);

              const xmlDoc = document.implementation.createDocument(null, null, null);
              const xmlElement = xmlDoc.createElement("root");
              const xmlText = xmlDoc.createTextNode("xml");
              xmlElement.appendChild(xmlText);
              xmlDoc.appendChild(xmlElement);
              const foreignTextHost = document.createElement("p");
              foreignTextHost.appendChild(document.createTextNode("old"));
              (document.body || document.documentElement || document).appendChild(foreignTextHost);
              const foreignTextRange = document.createRange();
              foreignTextRange.setStart(foreignTextHost, 0);
              foreignTextRange.setEnd(foreignTextHost, 1);
              foreignTextHost.replaceChild(xmlText, foreignTextHost.firstChild);

              const foreignDoc = document.implementation.createHTMLDocument("");
              const invalidHost = document.createElement("p");
              invalidHost.appendChild(document.createTextNode("old"));
              (document.body || document.documentElement || document).appendChild(invalidHost);
              const invalidRange = document.createRange();
              invalidRange.setStart(invalidHost, 0);
              invalidRange.setEnd(invalidHost, 1);
              let invalidThrown = "no-throw";
              try {
                invalidHost.replaceChild(foreignDoc, invalidHost.firstChild);
              } catch (error) {
                invalidThrown = error.name;
              }

              return [
                sameRange.startContainer === host,
                sameRange.startOffset,
                sameRange.endContainer === host,
                sameRange.endOffset,
                movingHost.firstChild === movingNew,
                movingOld.parentNode === null,
                movingNew.parentNode === movingHost,
                movingRange.startContainer === movingHost,
                movingRange.startOffset,
                movingRange.endContainer === movingHost,
                movingRange.endOffset,
                foreignTextHost.firstChild.data,
                foreignTextHost.childNodes.length,
                foreignTextRange.startContainer === foreignTextHost,
                foreignTextRange.startOffset,
                foreignTextRange.endContainer === foreignTextHost,
                foreignTextRange.endOffset,
                invalidThrown,
                invalidHost.firstChild.nodeValue,
                invalidRange.startContainer === invalidHost,
                invalidRange.startOffset,
                invalidRange.endContainer === invalidHost,
                invalidRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("replaceChild live range order should evaluate");

    assert_eq!(
        result,
        "true|0|true|0|true|true|true|true|0|true|0|xml|1|true|0|true|0|HierarchyRequestError|old|true|0|true|1"
    );
}

#[test]
fn element_append_child_rejects_document_type_without_mutating_selection_range() {
    let mut vm = new_parsed_test_vm(
        "https://range-pre-insert-doctype.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abc");
              host.appendChild(text);
              document.body.appendChild(host);
              const doctype = document.doctype;
              const range = document.createRange();
              range.setStart(host, 0);
              range.setEnd(host, 1);
              getSelection().removeAllRanges();
              getSelection().addRange(range);
              const selectedRange = getSelection().getRangeAt(0);
              let thrown = "no";
              try {
                host.appendChild(doctype);
              } catch (error) {
                thrown = `${error.name}:${error.code}`;
              }
              return [
                thrown,
                document.doctype === doctype,
                doctype.parentNode === document,
                selectedRange.startContainer === host,
                selectedRange.startOffset,
                selectedRange.endContainer === host,
                selectedRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("doctype insertion rejection should evaluate");

    assert_eq!(result, "HierarchyRequestError:3|true|true|true|0|true|1");
}

#[tokio::test]
async fn selectionchange_is_queued_and_coalesced_per_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-coalesce.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              const selection = getSelection();
              globalThis.__selectionChangeLog = [];
              globalThis.__selectionChangeCount = 0;
              document.addEventListener("selectionchange", () => {
                globalThis.__selectionChangeCount += 1;
                globalThis.__selectionChangeLog.push(
                  `event:${globalThis.__selectionChangeCount}:${selection.anchorOffset}:${selection.focusOffset}`
                );
              });
              selection.collapse(text, 1);
              globalThis.__selectionChangeLog.push(`after-collapse:${globalThis.__selectionChangeCount}`);
              selection.extend(text, 2);
              globalThis.__selectionChangeLog.push(`after-extend:${globalThis.__selectionChangeCount}`);
              return `${globalThis.__selectionChangeLog.join("|")}|count:${globalThis.__selectionChangeCount}`;
            })()
            "#,
        )
        .expect("selectionchange setup should evaluate");

    assert_eq!(
        result, "after-collapse:0|after-extend:0|count:0",
        "selectionchange must not fire synchronously"
    );

    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance queued selectionchange task")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionChangeLog.join('|')}|count:${globalThis.__selectionChangeCount}`")
            .expect("selectionchange task result should evaluate"),
        "after-collapse:0|after-extend:0|event:1:1:2|count:1"
    );
}

#[tokio::test]
async fn storage_mutations_queue_events_to_child_window_body_handler() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `<body onstorage="
    parent.__storageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      url: event.url,
      storageArea: event.storageArea === localStorage,
      instance: event instanceof StorageEvent,
      tag: Object.prototype.toString.call(event)
    });
  "></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("storage child event setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    assert_eq!(
        vm.eval("localStorage.setItem('k', 'v'); __storageEvents.length")
            .expect("storage mutation should evaluate"),
        "0",
        "storage events must be queued instead of dispatched synchronously"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance queued storage event")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__storageEvents)")
            .expect("storage event result should evaluate"),
        r#"[{"key":"k","oldValue":null,"newValue":"v","url":"https://storage-events.test/page","storageArea":true,"instance":true,"tag":"[object StorageEvent]"}]"#
    );

    vm.eval("localStorage.setItem('k', 'v')")
        .expect("same-value storage mutation should evaluate");
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should observe no same-value storage event")
    );
    assert_eq!(
        vm.eval("__storageEvents.length")
            .expect("same-value storage event count should evaluate"),
        "1"
    );
}

#[tokio::test]
async fn queued_storage_event_init_object_ignores_object_prototype_setters() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-event-init-data-property.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  globalThis.__storageInitSetterHits = [];
  globalThis.__captureStorageInitSetters = false;
  for (const name of ["key", "oldValue", "newValue", "url", "storageArea"]) {
    Object.defineProperty(Object.prototype, name, {
      configurable: true,
      get() { return undefined; },
      set(value) {
        if (globalThis.__captureStorageInitSetters) {
          const receiverKind = this instanceof StorageEvent ? "event" : "plain";
          globalThis.__storageInitSetterHits.push(`${receiverKind}:${name}`);
        }
        Object.defineProperty(this, name, {
          configurable: true,
          enumerable: true,
          writable: true,
          value
        });
      }
    });
  }
  const frame = document.createElement('iframe');
  frame.srcdoc = `<body onstorage="
    parent.__storageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      url: event.url,
      storageArea: event.storageArea === localStorage,
      instance: event instanceof StorageEvent
    });
  "></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("storage event init data-property setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__captureStorageInitSetters = true;
  localStorage.setItem("__storage-init-key", "__storage-init-value");
  return __storageEvents.length;
})()
"#
        )
        .expect("storage mutation with Object.prototype setters should evaluate"),
        "0",
        "storage events must remain queued"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance queued storage event")
    );

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__captureStorageInitSetters = false;
  return JSON.stringify({
    events: globalThis.__storageEvents,
    plainSetterHits: globalThis.__storageInitSetterHits.filter(hit => hit.startsWith("plain:"))
  });
})()
"#
        )
        .expect("storage event init data-property result should evaluate"),
        r#"{"events":[{"key":"__storage-init-key","oldValue":null,"newValue":"__storage-init-value","url":"https://storage-event-init-data-property.test/page","storageArea":true,"instance":true}],"plainSetterHits":[]}"#
    );
}

#[tokio::test]
async fn storage_event_promise_continuation_after_child_dispatch_keeps_top_scope() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-event-continuation.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  sessionStorage.clear();
  globalThis.__storageContinuationDone = false;
  globalThis.__storageContinuationLog = [];
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__storageContinuationFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("storage continuation frame setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    vm.eval(
        r#"
(() => {
  const frame = __storageContinuationFrame;
  const record = event => [
    event.key,
    event.oldValue,
    event.newValue,
    event.storageArea === frame.contentWindow.sessionStorage
  ].join(":");
  const waitForStorage = () => new Promise(resolve => {
    const listener = event => {
      frame.contentWindow.removeEventListener("storage", listener);
      resolve(event);
    };
    frame.contentWindow.addEventListener("storage", listener);
  });

  waitForStorage()
    .then(event => {
      __storageContinuationLog.push(record(event));
      return waitForStorage();
    })
    .then(event => {
      __storageContinuationLog.push(record(event));
      const next = waitForStorage().then(event => {
        __storageContinuationLog.push(record(event));
      });
      sessionStorage.removeItem("missing-continuation-key");
      sessionStorage.setItem("continuation-second", "foo");
      return next;
    })
    .then(
      () => { __storageContinuationDone = true; },
      error => { __storageContinuationDone = "error:" + (error && error.name); }
    );

  sessionStorage.setItem("continuation-first", "foo");
  sessionStorage.setItem("continuation-first", "foo");
  sessionStorage.setItem("continuation-first", "bar");
  return "started";
})()
"#,
    )
    .expect("storage continuation promise chain should evaluate");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__storageContinuationDone === true)")
            .expect("storage continuation done flag should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_dom_manipulation_task_executor_turn(
                PageDomManipulationTestFamily::StorageEvent,
                &loader,
            )
            .await
            .expect("storage continuation selected dispatcher should advance");
    }

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  done: globalThis.__storageContinuationDone,
  log: globalThis.__storageContinuationLog
})"#
        )
        .expect("storage continuation log should evaluate"),
        r#"{"done":true,"log":["continuation-first::foo:true","continuation-first:foo:bar:true","continuation-second::foo:true"]}"#
    );
}

#[tokio::test]
async fn child_message_handler_can_reply_through_event_source_origin() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-message-source-origin.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__sourceOriginReplies = [];
  addEventListener("message", event => {
    __sourceOriginReplies.push(String(event.data));
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `<script>
    addEventListener("message", event => {
      event.source.postMessage("reply:" + event.origin + ":" + event.source.origin, event.source.origin);
    });
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__sourceOriginFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("source-origin frame setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    vm.eval(
        r#"
__sourceOriginFrame.contentWindow.postMessage(
  { command: "create ID" },
  __sourceOriginFrame.origin
);
"#,
    )
    .expect("source-origin postMessage should evaluate");

    for _ in 0..6 {
        if vm
            .eval("__sourceOriginReplies.length")
            .expect("source-origin reply length should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("source-origin wait driver should advance");
    }

    assert_eq!(
        vm.eval("__sourceOriginReplies.join('|')")
            .expect("source-origin reply should evaluate"),
        "reply:https://child-message-source-origin.test:https://child-message-source-origin.test"
    );
}

#[tokio::test]
async fn http_child_load_message_roundtrip_accepts_frame_origin_default() {
    let (child_url, server) = spawn_wpt_style_web_storage_message_child_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&child_url)
        .expect("child url")
        .join("/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__wptStyleRoundtrips = [];
  addEventListener("message", event => {{
    __wptStyleRoundtrips.push(JSON.stringify(event.data));
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  frame.addEventListener("load", () => {{
    frame.contentWindow.postMessage({{ command: "create ID", key: "userID" }}, frame.origin);
  }}, {{ once: true }});
  return "queued";
}})()
"#
    ))
    .expect("WPT-style child message setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__wptStyleRoundtrips.length",
        "1",
        "WPT-style child message roundtrip",
    )
    .await;

    assert_eq!(
        vm.eval("__wptStyleRoundtrips.join('|')")
            .expect("WPT-style roundtrip should evaluate"),
        r#"{"message":"ID created","userID":"created"}"#
    );
    let requests = server.await.expect("WPT-style child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn http_child_message_sent_before_navigation_commit_reaches_loaded_child() {
    let (child_url, server) = spawn_pending_child_message_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&child_url)
        .expect("child url")
        .join("/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__pendingChildMessages = [];
  addEventListener("message", event => {{
    __pendingChildMessages.push(String(event.data));
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentWindow.postMessage({{ type: "getmessages" }}, "*");
  return "queued";
}})()
"#
    ))
    .expect("pending child message setup should evaluate");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("pre-commit child setup should use only child selected tasks");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "pending-message child document completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("loaded child setup should use only child selected tasks");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__pendingChildMessages.length",
        "1",
        "message queued before child navigation commit",
    )
    .await;

    assert_eq!(
        vm.eval("__pendingChildMessages.join('|')")
            .expect("pending child message should evaluate"),
        "child:object:true"
    );
    let requests = server.await.expect("pending child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn captured_cross_origin_content_window_matches_message_source_after_child_navigation() {
    let (child_url, server) = spawn_pending_child_message_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let child_url_parsed = Url::parse(&child_url).expect("child url");
    let document_url = format!(
        "http://localhost:{}/parent.html",
        child_url_parsed
            .port()
            .expect("child url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__capturedWindowMessages = [];
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  const captured = frame.contentWindow;
  globalThis.__capturedCrossOriginWindow = captured;
  addEventListener("message", event => {{
    __capturedWindowMessages.push({{
      data: String(event.data),
      sourceIsCaptured: event.source === captured,
      sourceIsCurrent: event.source === frame.contentWindow,
      currentIsCaptured: frame.contentWindow === captured,
      sourceIsTop: event.source === globalThis,
      sourceIsNull: event.source === null
    }});
  }});
  return "queued";
}})()
"#
    ))
    .expect("captured contentWindow message setup should evaluate");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("captured-window child setup should use only child selected tasks");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "captured-window child document completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("captured-window loaded child should use only child selected tasks");
    assert_eq!(
        vm.eval("__capturedCrossOriginWindow === document.querySelector('iframe').contentWindow")
            .expect("captured WindowProxy identity should evaluate after navigation"),
        "true"
    );
    vm.eval("__capturedCrossOriginWindow.postMessage({ type: 'getmessages' }, '*')")
        .expect("captured WindowProxy should post to the replacement LocalWindow");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__capturedWindowMessages.length",
        "1",
        "captured WindowProxy message roundtrip",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(__capturedWindowMessages)")
            .expect("captured-window message should evaluate"),
        r#"[{"data":"child:object:true","sourceIsCaptured":true,"sourceIsCurrent":true,"currentIsCaptured":true,"sourceIsTop":false,"sourceIsNull":false}]"#
    );
    let requests = server.await.expect("captured child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn captured_cross_origin_content_window_keeps_safe_surface_during_realm_gap() {
    let (child_url, server) = spawn_pending_child_message_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let child_url_parsed = Url::parse(&child_url).expect("child url");
    let document_url = format!(
        "http://localhost:{}/parent.html",
        child_url_parsed
            .port()
            .expect("child url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__realmGapFrame = frame;
  globalThis.__realmGapWindow = frame.contentWindow;
  return "queued";
}})()
"#
    ))
    .expect("realm-gap child setup should evaluate");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("realm-gap child setup should use only child selected tasks");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "realm-gap child document completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("realm-gap loaded child should use only child selected tasks");

    let child_handle = {
        let host = vm._context_host.borrow();
        let handles = host.child_browsing_context_handles_in_document_order();
        assert_eq!(handles.len(), 1, "realm-gap fixture should have one child");
        handles[0]
    };
    vm.retire_child_frame_realm_for_test(child_handle);

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return `${error && error.name}:${error && error.message}`;
    }
  };
  return [
    __realmGapWindow === __realmGapFrame.contentWindow,
    probe(() => typeof __realmGapWindow.postMessage),
    probe(() => {
      __realmGapWindow.postMessage({ type: "watchcat" }, "*");
      return "no-throw";
    }),
    probe(() => __realmGapWindow.document)
  ].join("|");
})()
"#,
        )
        .expect("cross-origin WindowProxy realm-gap probes should evaluate");

    assert_eq!(
        result,
        concat!(
            "true|function|no-throw|",
            "SecurityError:Blocked a frame with a different origin from accessing a cross-origin frame."
        ),
        "a live browsing context must retain only its safe cross-origin WindowProxy surface while its LocalWindow realm is between generations"
    );
    let requests = server.await.expect("realm-gap child server should finish");
    assert_eq!(requests.len(), 1);
}
