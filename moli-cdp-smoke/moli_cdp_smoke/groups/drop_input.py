"""Native editing events, driven through CDP drag input rather than dispatchEvent."""
from __future__ import annotations

from . import SmokeState
from ..assertions import assert_equal
from ..diagnostics import new_cdp_session


_DROP_EDITING_SETUP = """options => {
    const editor = document.getElementById('editor');
    editor.textContent = options.text;
    editor.focus();
    const selection = getSelection();
    const range = document.createRange();
    range.selectNodeContents(editor);
    selection.removeAllRanges();
    selection.addRange(range);
    const outer = document.getElementById('outer');
    const check = window.__dropEditing = {editor, events: [], transfers: []};
    const listenerTarget = options.action === 'inherit-plaintext-only' ? outer : editor;
    const NativeInputEvent = window.__originalInputEvent || InputEvent;
    check.selection = () => ({
        focusText: selection.focusNode?.nodeType === 3 ? selection.focusNode.data : null,
        focusOffset: selection.focusOffset, rangeCount: selection.rangeCount,
        anchorInside: !!selection.anchorNode && editor.contains(selection.anchorNode),
        focusInside: !!selection.focusNode && editor.contains(selection.focusNode),
        text: selection.toString(),
    });
    for (const type of ['drop', 'beforeinput', 'input']) {
        listenerTarget.addEventListener(type, event => {
            if (type === 'drop') check.dropTransfer = event.dataTransfer;
            if (type === 'beforeinput' && options.start !== undefined) {
                // Native drops position their caret from coordinates first. Set
                // the replacement range here to isolate UTF-16 editing semantics.
                selection.setBaseAndExtent(editor.firstChild, options.start,
                                           editor.firstChild, options.end);
                check.replacementSelection = check.selection();
            }
            const transfer = event.dataTransfer;
            check.events.push({type, target: event.target.id,
                nativeInput: event instanceof NativeInputEvent, trusted: event.isTrusted,
                cancelable: event.cancelable, inputType: event.inputType ?? null,
                data: event.data ?? null, transfer: transfer ? transfer.getData('text/plain') : null});
            if (type !== 'drop') check.transfers.push(transfer);
            if (type === 'beforeinput') {
                if (options.action === 'remove') editor.remove();
                else if (options.action === 'disable') editor.contentEditable = 'false';
                else if (options.action === 'adopt') {
                    check.foreign = document.implementation.createHTMLDocument('other');
                    check.foreign.adoptNode(editor);
                    check.foreign.body.appendChild(editor);
                } else if (options.action === 'plaintext-only') editor.contentEditable = 'plaintext-only';
                else if (options.action === 'inherit-plaintext-only') {
                    outer.contentEditable = 'plaintext-only';
                    editor.removeAttribute('contenteditable');
                }
                check.mutationSelection = check.selection();
            }
        });
    }
}"""

_DROP_EDITING_RESULT = """() => {
    const check = window.__dropEditing, editor = check.editor;
    return {events: check.events, content: editor.textContent, html: editor.innerHTML,
        connected: editor.isConnected, editable: editor.isContentEditable,
        sameDocument: editor.ownerDocument === document,
        inForeign: !!check.foreign && editor.parentNode === check.foreign.body,
        selection: check.selection(), mutationSelection: check.mutationSelection,
        replacementSelection: check.replacementSelection,
        transfers: check.transfers.map(transfer => transfer ? transfer.getData('text/plain') : null),
        dropAfter: [check.dropTransfer.getData('text/plain'), check.dropTransfer.types.length,
                    check.dropTransfer.items.length]};
}"""


async def _prepare_drop_editor(page, kind: str, text: str, **options) -> None:
    mode = "plaintext-only" if kind == "plaintext-only" else "true"
    control = f'<div id="editor" contenteditable="{mode}"></div>'
    if options.get("action") == "inherit-plaintext-only":
        control = f'<div id="outer">{control}</div>'
    await page.set_content(
        '<style>#editor {font:20px monospace;width:400px;height:100px;margin:20px;}</style>'
        + control
    )
    await page.evaluate(_DROP_EDITING_SETUP, {"text": text, **options})


async def _dispatch_editor_drop(page, session, items: list[dict]) -> None:
    box = await page.locator('#editor').bounding_box()
    for event_type in ("dragEnter", "dragOver", "drop"):
        await session.send("Input.dispatchDragEvent", {
            "type": event_type, "x": box["x"] + 40, "y": box["y"] + 10,
            "data": {"items": items, "dragOperationsMask": 1},
        })


def _editing_event(kind: str, event_type: str, payload: str, *, target: str = "editor") -> dict:
    plain_before = kind == "plaintext-only" and event_type == "beforeinput"
    return {"type": event_type, "target": target, "nativeInput": event_type != "drop",
            "trusted": True, "cancelable": event_type != "input",
            "inputType": None if event_type == "drop" else "insertFromDrop",
            "data": payload if plain_before else None,
            "transfer": None if plain_before else payload}


async def _run_drop_revalidation_workflows(state: SmokeState, page, session) -> None:
    items = [{"mimeType": "text/plain", "data": "replacement"},
             {"mimeType": "text/html", "data": "<b>replacement</b>"}]
    for kind in ("rich", "plaintext-only"):
        for action in ("remove", "disable", "adopt"):
            case = f"beforeinput-{kind}-{action}"
            await _prepare_drop_editor(page, kind, "original", action=action)
            await _dispatch_editor_drop(page, session, items)
            result = await page.evaluate(_DROP_EDITING_RESULT)
            assert_equal(result["events"], [_editing_event(kind, event_type, "replacement")
                         for event_type in ("drop", "beforeinput")], f"{case}: invalidated editor emits no input")
            assert_equal((result["content"], result["html"]), ("original", "original"),
                         f"{case}: invalidated editor keeps its original DOM")
            assert_equal({key: result[key] for key in ("connected", "editable", "sameDocument", "inForeign")},
                         {"connected": action != "remove", "editable": action != "disable",
                          "sameDocument": action != "adopt", "inForeign": action == "adopt"},
                         f"{case}: beforeinput actually changes the editor's native state")
            assert_equal(result["selection"], result["mutationSelection"],
                         f"{case}: aborted editing preserves the selection left by beforeinput")
            assert_equal(result["transfers"], [None if kind == "plaintext-only" else "replacement"],
                         f"{case}: the independent editing snapshot survives aborted insertion")
            assert_equal(result["dropAfter"], ["", 0, 0], f"{case}: the DOM drop view is closed")

    case = "beforeinput-rich-to-plaintext-only"
    await _prepare_drop_editor(page, "rich", "", action="plaintext-only")
    await _dispatch_editor_drop(page, session, [{"mimeType": "text/plain", "data": "hello"},
                                              {"mimeType": "text/html", "data": "<b>hello</b>"}])
    result = await page.evaluate(_DROP_EDITING_RESULT)
    assert_equal(result["events"], [_editing_event("rich", event_type, "hello")
                 for event_type in ("drop", "beforeinput", "input")],
                 f"{case}: editing events retain their original rich payload")
    assert_equal((result["content"], result["html"]), ("hello", "hello"),
                 f"{case}: the updated plain editing mode discards HTML formatting")
    assert_equal({key: result["selection"][key] for key in ("focusText", "focusOffset", "focusInside", "rangeCount")},
                 {"focusText": "hello", "focusOffset": 5, "focusInside": True, "rangeCount": 1},
                 f"{case}: the resulting selection ends after the inserted text")
    assert_equal(result["transfers"], ["hello", "hello"], f"{case}: the editing snapshot remains readable")
    assert_equal(result["dropAfter"], ["", 0, 0], f"{case}: the DOM drop view is closed")

    case = "beforeinput-rich-to-inherited-plaintext-only"
    await _prepare_drop_editor(page, "rich", "", action="inherit-plaintext-only")
    await _dispatch_editor_drop(page, session, [{"mimeType": "text/plain", "data": "hello"},
                                              {"mimeType": "text/html", "data": "<b>hello</b>"}])
    result = await page.evaluate(_DROP_EDITING_RESULT)
    assert_equal(result["events"], [_editing_event("rich", event_type, "hello",
                 target="outer" if event_type == "input" else "editor")
                 for event_type in ("drop", "beforeinput", "input")],
                 f"{case}: input targets the new editing host while retaining the original rich payload")
    assert_equal(await page.evaluate("""() => [document.getElementById('outer').contentEditable,
                 window.__dropEditing.editor.hasAttribute('contenteditable'),
                 window.__dropEditing.editor.isContentEditable]"""), ["plaintext-only", False, True],
                 f"{case}: beforeinput transfers editing ownership to the plaintext ancestor")
    assert_equal((result["content"], result["html"]), ("hello", "hello"),
                 f"{case}: inherited plaintext editing inserts text without bold markup")
    assert_equal({key: result["selection"][key] for key in ("focusText", "focusOffset", "focusInside", "rangeCount")},
                 {"focusText": "hello", "focusOffset": 5, "focusInside": True, "rangeCount": 1},
                 f"{case}: selection ends after the text inside the former editing host")
    assert_equal(result["transfers"], ["hello", "hello"], f"{case}: the editing snapshot remains readable")
    assert_equal(result["dropAfter"], ["", 0, 0], f"{case}: the DOM drop view is closed")
    state.record("native_drop_beforeinput_revalidation")


async def _run_drop_utf16_workflows(state: SmokeState, page, session) -> None:
    cases = [
        ("emoji-prefix", "😀abc", 2, 3, "X", "😀Xbc", 3, "a"),
        ("emoji-replacement", "a😀bc", 1, 3, "🦀", "a🦀bc", 3, "😀"),
        ("combining-to-zwj", "AéB", 1, 3, "👩‍💻", "A👩‍💻B", 6, "é"),
        ("combining-insertion", "A😀B", 1, 3, "é", "AéB", 3, "😀"),
        ("backward-selection", "😀abc", 3, 2, "X", "😀Xbc", 3, "a"),
        ("collapsed-non-bmp", "A😀B", 3, 3, "🦀", "A😀🦀B", 5, ""),
    ]
    for name, text, start, end, payload, expected, focus, selected in cases:
        case = f"utf16-{name}"
        await _prepare_drop_editor(page, "plaintext-only", text, start=start, end=end)
        await _dispatch_editor_drop(page, session, [{"mimeType": "text/plain", "data": payload}])
        result = await page.evaluate(_DROP_EDITING_RESULT)
        assert_equal(result["events"], [_editing_event("plaintext-only", event_type, payload)
                     for event_type in ("drop", "beforeinput", "input")], f"{case}: native Unicode drop events")
        assert_equal(result["replacementSelection"]["text"], selected,
                     f"{case}: beforeinput selects the requested UTF-16 replacement range")
        assert_equal(result["content"], expected, f"{case}: replacement preserves the exact Unicode text")
        # Chromium selects the inserted range while Moli collapses to its end.
        # Both must finish at the same UTF-16 offset in the same text content.
        assert_equal({key: result["selection"][key] for key in ("focusText", "focusOffset", "focusInside", "rangeCount")},
                     {"focusText": expected, "focusOffset": focus, "focusInside": True, "rangeCount": 1},
                     f"{case}: selection ends at the correct UTF-16 offset")
        assert_equal(result["transfers"], [None, payload], f"{case}: the input snapshot remains readable")
        assert_equal(result["dropAfter"], ["", 0, 0], f"{case}: the DOM drop view is closed")
    state.record("native_drop_utf16_selection")


async def run_drop_input_workflow(state: SmokeState) -> None:
    page = await state.context.new_page()
    session = None
    try:
        session = await new_cdp_session(state.context, page)
        for kind in ["rich", "plain", "input", "textarea", "plaintext-only", "cancel"]:
            control = {
                "input": '<input id="editor">',
                "textarea": '<textarea id="editor"></textarea>',
                "plaintext-only": '<div id="editor" contenteditable="plaintext-only"></div>',
                "cancel": '<div id="editor" contenteditable="true">base</div>',
            }.get(kind, '<div id="editor" contenteditable="true"></div>')
            await page.set_content('<style>#editor {width:400px;height:100px;margin:20px;}</style>' + control)
            await page.evaluate("""kind => {
                const editor = document.getElementById('editor');
                editor.focus();
                if (!('value' in editor)) {
                    const range = document.createRange();
                    range.selectNodeContents(editor);
                    range.collapse(false);
                    getSelection().removeAllRanges();
                    getSelection().addRange(range);
                }
                window.__events = [];
                window.__transfers = [];
                window.__readonly = [];
                window.__dragTransfer = null;
                editor.addEventListener('drop', e => __dragTransfer = e.dataTransfer);
                const NativeInputEvent = window.__originalInputEvent ||= InputEvent;
                window.InputEvent = function() { throw new Error('page constructor must not run'); };
                for (const type of ['beforeinput', 'input']) {
                    editor.addEventListener(type, e => {
                        const t = e.dataTransfer;
                        __transfers.push(t);
                        __events.push({type, native: e instanceof NativeInputEvent, data: e.data,
                            inputType: e.inputType, composing: e.isComposing, trusted: e.isTrusted,
                            bubbles: e.bubbles, cancelable: e.cancelable, composed: e.composed,
                            transfer: t ? {text: t.getData('text/plain'), html: t.getData('text/html'),
                                types: Array.from(t.types), effect: t.effectAllowed, drop: t.dropEffect} : t});
                        if (t) {
                            const types = t.types;
                            t.setData('text/plain', 'changed');
                            t.clearData();
                            t.items.clear();
                            t.effectAllowed = 'move';
                            const added = t.items.add('duplicate', 'text/plain');
                            const file = t.items.add(new File(['x'], 'x.txt'));
                            let removed = '';
                            try { t.items.remove(0); } catch (e) { removed = e.name; }
                            __readonly.push({text: t.getData('text/plain'), html: t.getData('text/html'),
                                added, file, removed, sameTypes: t.types === types, effect: t.effectAllowed});
                        }
                        if (type === 'beforeinput' && kind === 'cancel') e.preventDefault();
                    });
                }
            }""", kind)
            box = await page.locator('#editor').bounding_box()
            html = "" if kind == "plain" else "<b>hello</b>"
            items = [{"mimeType": "text/plain", "data": "hello"}]
            if html:
                items.append({"mimeType": "text/html", "data": html})
            # Editable elements accept a native drop without canceling dragover.
            # Canceling it in Chromium suppresses this native editing path.
            for event_type in ["dragEnter", "dragOver", "drop"]:
                await session.send("Input.dispatchDragEvent", {
                    "type": event_type, "x": box["x"] + 40, "y": box["y"] + 10,
                    "data": {"items": items, "dragOperationsMask": 1},
                })
            result = await page.evaluate("""() => ({events: __events, readonly: __readonly,
                content: editor.value ?? editor.textContent, bold: !!editor.querySelector('b'),
                transfers: __transfers.map(t => t ? t.getData('text/plain') : null),
                independent: __transfers.every(t => t !== __dragTransfer),
                shared: __transfers.length === 2 && __transfers[0] === __transfers[1]})""")
            transfer = {"text": "hello", "html": html, "types": [item["mimeType"] for item in items],
                        "effect": "copy", "drop": "none"}
            expected = []
            for event_type in (["beforeinput"] if kind == "cancel" else ["beforeinput", "input"]):
                plain_data = kind in ["input", "textarea"] or (kind == "plaintext-only" and event_type == "beforeinput")
                expected.append({"type": event_type, "native": True, "data": "hello" if plain_data else None,
                    "inputType": "insertFromDrop", "composing": False, "trusted": True, "bubbles": True,
                    "cancelable": event_type == "beforeinput", "composed": True,
                    "transfer": None if plain_data else transfer})
            assert_equal(result["events"], expected, f"{kind}: native drop InputEvents")
            assert_equal(result["content"], "base" if kind == "cancel" else "hello", f"{kind}: drop content")
            assert_equal(result["bold"], kind == "rich", f"{kind}: rich vs plain insertion")
            with_transfer = [event for event in expected if event["transfer"] is not None]
            assert_equal(result["readonly"], [{"text": "hello", "html": html, "added": None,
                "file": None, "removed": "InvalidStateError", "sameTypes": True, "effect": "copy"}
                for _ in with_transfer], f"{kind}: read-only transfer mutations")
            assert_equal(result["transfers"], ["hello" if event["transfer"] else None for event in expected],
                         f"{kind}: transfer readable after dispatch")
            assert_equal(result["independent"], True, f"{kind}: editing transfer differs from drag transfer")
            if kind in ["rich", "plain"]:
                assert_equal(result["shared"], True, f"{kind}: beforeinput/input share transfer")
        state.record("native_drop_input_event_payloads")
        await _run_drop_revalidation_workflows(state, page, session)
        await _run_drop_utf16_workflows(state, page, session)
    finally:
        if session is not None:
            await session.detach()
        await page.close()
