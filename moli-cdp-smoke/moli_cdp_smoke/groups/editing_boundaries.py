from __future__ import annotations

from ..assertions import assert_equal
from ..helpers import capture_layout
from ..state import SmokeState
from .editing_shortcuts import _editor


async def run_editing_boundary_workflow(state: SmokeState) -> None:
    for name, opening, closing in [
        ("p", "<p>", "</p>"),
        ("div", "<div>", "</div>"),
        ("css", '<span style="display:block">', "</span>"),
    ]:
        for backward in (False, True):
            markup = f'<div id="e" contenteditable>{opening}ab{closing}\n  {opening}cd{closing}</div>'
            async with _editor(state, markup) as page:
                await page.evaluate("""() => {
                    globalThis.original = e.innerHTML;
                    globalThis.firstBlock = e.firstElementChild;
                    globalThis.secondBlock = e.lastElementChild;
                    e.focus();
                }""")
                await page.evaluate(
                    "getSelection().collapse(secondBlock.firstChild, 0)" if backward
                    else "getSelection().collapse(firstBlock.firstChild, 2)"
                )
                await page.keyboard.press("Backspace" if backward else "Delete")
                assert_equal(await page.evaluate("[e.textContent, e.children.length]"), ["abcd", 1], "join block boundary without deleting text")
                await page.keyboard.press("Control+z")
                assert_equal(
                    await page.evaluate("e.innerHTML === original && e.firstElementChild === firstBlock && e.lastElementChild === secondBlock"),
                    True, "undo preserves paragraph identities",
                )
                state.record(f"editing_block_{name}_{'backward' if backward else 'forward'}")

    for name, attributes in [
        ("hidden", "hidden"),
        ("display", 'style="display:none"'),
        ("visibility", 'style="visibility:hidden"'),
    ]:
        async with _editor(state, f'<div id="e" contenteditable><b>ab</b><span id="h" {attributes}>SECRET</span><i>cd</i></div>') as page:
            await page.evaluate("globalThis.hiddenText=h.firstChild;globalThis.hiddenNode=h;e.focus();getSelection().collapse(e.firstChild.firstChild,2)")
            await page.keyboard.press("Delete")
            assert_equal(await page.evaluate("[e.textContent, hiddenText.data, hiddenNode.isConnected]"), ["abd", "SECRET", False], "delete range crosses hidden subtree")
            await page.keyboard.press("Control+z")
            assert_equal(await page.evaluate("[e.textContent, e.querySelector('#h') === hiddenNode]"), ["abSECRETcd", True], "undo restores hidden subtree")
            state.record(f"editing_hidden_boundary_{name}")

    async with _editor(state, '<input id="a" value="abc"><input id="b" value="xyz">') as page:
        await page.evaluate("""() => {
            a.focus(); a.setSelectionRange(0, 2);
            a.oncut = () => { b.focus(); b.setSelectionRange(0, 2); };
            globalThis.targets = [];
            for (const kind of ['cut', 'beforeinput', 'input'])
                document.addEventListener(kind, event => targets.push([kind, event.target.id]));
        }""")
        await page.keyboard.press("Control+x")
        assert_equal(await page.evaluate("[a.value, b.value, targets]"), ["abc", "z", [["cut", "a"], ["beforeinput", "b"], ["input", "b"]]], "cut callback retargets editing default")
        assert_equal(await page.evaluate("navigator.clipboard.readText()"), "xy", "cut callback clipboard")
        state.record("editing_cut_focus_callback")

    for kind, markup in [
        ("input", '<input id="a">'),
        ("textarea", '<textarea id="a"></textarea>'),
        ("editable", '<div id="a" contenteditable></div>'),
    ]:
        for movement in ("focus", "arrows", "selection"):
            async with _editor(state, markup + '<input id="b">') as page:
                await page.evaluate("a.focus()")
                await page.keyboard.type("abc")
                if movement == "focus":
                    await page.evaluate("b.focus(); a.focus()")
                elif movement == "arrows":
                    await page.keyboard.press("ArrowLeft")
                    await page.keyboard.press("ArrowRight")
                else:
                    await page.evaluate("""() => {
                        if (a.setSelectionRange) {
                            a.setSelectionRange(2, 2); a.setSelectionRange(3, 3);
                        } else {
                            getSelection().collapse(a.firstChild, 2);
                            getSelection().collapse(a.firstChild, 3);
                        }
                    }""")
                await page.keyboard.type("d")
                await page.keyboard.press("Control+z")
                assert_equal(await page.evaluate("a.value ?? a.textContent"), "abc", "typing group ends after selection/focus movement")
                state.record(f"editing_typing_group_{kind}_{movement}")

    async with _editor(state, '<textarea id="a"></textarea><textarea id="b"></textarea>') as page:
        await page.evaluate("a.focus()")
        await page.keyboard.type("first")
        await page.evaluate("""() => {
            a.addEventListener('beforeinput', function once(event) {
                if (event.inputType !== 'historyUndo') return;
                a.removeEventListener('beforeinput', once);
                b.focus(); document.execCommand('insertText', false, 'second'); a.focus();
            });
        }""")
        for expected in (["first", "", "b"], ["", "", "a"]):
            await page.keyboard.press("Control+z")
            assert_equal(await page.evaluate("[a.value, b.value, document.activeElement.id]"), expected, "undo uses callback-created history")
        await page.keyboard.press("Control+Shift+z")
        await page.keyboard.press("Control+Shift+z")
        assert_equal(await page.evaluate("[a.value, b.value]"), ["first", "second"], "redo retains both callback and original edit")
        state.record("editing_undo_reentrant_beforeinput")

    async with _editor(state, '<textarea id="a"></textarea>') as page:
        await page.evaluate("a.focus()")
        await page.keyboard.type("first")
        await page.evaluate("""() => {
            a.addEventListener('beforeinput', function once(event) {
                if (event.inputType !== 'historyUndo') return;
                a.removeEventListener('beforeinput', once);
                document.execCommand('insertText', false, 'second');
            });
        }""")
        await page.keyboard.press("Control+z")
        assert_equal(await page.input_value("#a"), "", "same-control callback edit remains in the continuous typing group")
        await page.keyboard.press("Control+z")
        assert_equal(await page.input_value("#a"), "", "original edit remains undoable")
        state.record("editing_undo_reentrant_same_control")

    async with _editor(state, '<textarea id="a"></textarea><textarea id="b"></textarea>') as page:
        await page.evaluate("a.focus()")
        await page.keyboard.type("first")
        await page.evaluate("""() => {
            a.addEventListener('beforeinput', function once(event) {
                if (event.inputType !== 'historyUndo') return;
                a.removeEventListener('beforeinput', once);
                b.focus(); document.execCommand('insertText', false, 'second'); a.focus();
                event.preventDefault();
            });
        }""")
        await page.keyboard.press("Control+z")
        assert_equal(await page.evaluate("[a.value, b.value]"), ["first", "second"], "cancel undo preserves callback edit")
        for expected in (["first", ""], ["", ""]):
            await page.keyboard.press("Control+z")
            assert_equal(await page.evaluate("[a.value, b.value]"), expected, "canceled undo does not discard history")
        state.record("editing_undo_reentrant_canceled")

    async with _editor(state, '<textarea id="a"></textarea><textarea id="b"></textarea>') as page:
        await page.evaluate("a.focus()")
        await page.keyboard.type("first")
        await page.evaluate("""() => {
            b.focus();
            a.addEventListener('focus', function once() {
                a.removeEventListener('focus', once);
                b.focus(); document.execCommand('insertText', false, 'second'); a.focus();
            });
        }""")
        await page.keyboard.press("Control+z")
        assert_equal(await page.evaluate("[a.value, b.value]"), ["", "second"], "focus callback follows the restored value")
        await page.keyboard.press("Control+z")
        assert_equal(await page.evaluate("[a.value, b.value]"), ["", ""], "focus callback history survives stack consumption")
        state.record("editing_undo_reentrant_focus")

    for name, markup, expected in [
        ("div", "<div>first</div><div>second</div>", "first\nsecond"),
        ("p", "<p>first</p><p>second</p>", "first\n\nsecond"),
        ("nested", "<div><div>first</div></div><div>second</div>", "first\nsecond"),
        ("css", '<span style="display:block">first</span><span style="display:block">second</span>', "first\nsecond"),
        ("inline", '<div style="display:inline">first</div><div style="display:inline">second</div>', "firstsecond"),
    ]:
        async with _editor(state, f'<div id="e" contenteditable>{markup}</div><textarea id="o"></textarea>') as page:
            await capture_layout(page)
            await page.evaluate("e.focus(); getSelection().selectAllChildren(e)")
            assert_equal(await page.evaluate("getSelection().getRangeAt(0).toString()"), "firstsecond", "Range text remains DOM character data")
            await page.keyboard.press("Control+c")
            await page.evaluate("o.focus()")
            await page.keyboard.press("Control+v")
            assert_equal(await page.input_value("#o"), expected, "copy rendered block separators")
            state.record(f"editing_copy_block_{name}")
