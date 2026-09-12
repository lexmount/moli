"""Native editing events, driven through CDP drag input rather than dispatchEvent."""
from __future__ import annotations

from . import SmokeState
from ..assertions import assert_equal


async def run_drop_input_workflow(state: SmokeState) -> None:
    page = await state.context.new_page()
    session = None
    try:
        session = await state.context.new_cdp_session(page)
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
    finally:
        if session is not None:
            await session.detach()
        await page.close()
