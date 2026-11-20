from __future__ import annotations

from contextlib import asynccontextmanager
from typing import Any, AsyncIterator

from ..assertions import assert_equal
from ..helpers import capture_layout
from ..state import SmokeState


@asynccontextmanager
async def _editor(state: SmokeState, markup: str) -> AsyncIterator[Any]:
    # History and clipboard contents belong to a Document / BrowserContext.
    # Give each case fresh state, including when this group follows other input tests.
    context = await state.browser.new_context()
    try:
        await context.grant_permissions(
            ["clipboard-read", "clipboard-write"], origin=state.fixture
        )
        page = await context.new_page()
        await page.goto(state.fixture + "/plain", wait_until="load")
        await page.set_content(markup)
        await page.bring_to_front()
        yield page
    finally:
        await context.close()


async def run_editing_shortcuts_workflow(state: SmokeState) -> None:
    async with _editor(state, '<div id="e" contenteditable></div>') as page:
        await page.evaluate("e.focus()")
        await page.keyboard.type("a")
        await page.evaluate("e.textContent = ''")
        await page.keyboard.press("Shift+KeyA")
        assert_equal(await page.text_content("#e"), "A", "type after clearing editor")
        state.record("editing_clear_then_type")

    async with _editor(
        state,
        '<div id="e" contenteditable><iframe srcdoc="<p>child</p>"></iframe>'
        '<b>abc</b><i>def</i></div>',
    ) as page:
        await page.locator("iframe").content_frame.locator("p").wait_for(state="attached")
        await page.evaluate("""() => {
            globalThis.f = document.querySelector('iframe');
            globalThis.w = f.contentWindow;
            globalThis.d = f.contentDocument;
            w.marker = 42;
            e.focus();
            const range = document.createRange();
            range.selectNode(document.querySelector('b'));
            getSelection().removeAllRanges();
            getSelection().addRange(range);
        }""")
        await page.keyboard.type("X")
        for shortcut, text in [("Control+z", "abcdef"), ("Control+Shift+z", "Xdef")]:
            await page.keyboard.press(shortcut)
            assert_equal(
                await page.evaluate("""() => [e.textContent, f.contentWindow === w,
                    f.contentDocument === d, f.contentWindow.marker]"""),
                [text, True, True, 42],
                "history preserves unrelated iframe state",
            )
        state.record("editing_history_preserves_iframe")

    async with _editor(state, '<input id="i" value="ABCDEF">') as page:
        await page.evaluate("""() => {
            i.focus(); i.setSelectionRange(0, 2);
            i.oncut = () => i.setSelectionRange(2, 4);
        }""")
        await page.keyboard.press("Control+x")
        assert_equal(await page.input_value("#i"), "ABEF", "cut live selection")
        assert_equal(await page.evaluate("navigator.clipboard.readText()"), "CD", "copy live cut selection")
        state.record("editing_cut_callback_selection")

    async with _editor(state, '<input id="i" readonly value="ABC">') as page:
        await page.evaluate("navigator.clipboard.writeText('seed')")
        await page.evaluate("i.focus(); i.setSelectionRange(0, 2)")
        await page.keyboard.press("Control+x")
        assert_equal(await page.input_value("#i"), "ABC", "readonly cut value")
        assert_equal(await page.evaluate("navigator.clipboard.readText()"), "seed", "readonly cut clipboard")
        state.record("editing_readonly_cut")

    async with _editor(state, '<div contenteditable><input id="i" value="abc"></div>') as page:
        await page.evaluate("""() => {
            globalThis.targets = [];
            document.oncut = event => targets.push(event.target.id);
            i.focus(); i.setSelectionRange(0, 2);
        }""")
        await page.keyboard.press("Control+x")
        assert_equal(await page.evaluate("[i.value, targets]"), ["c", ["i"]], "nested input cut target")
        state.record("editing_nested_control_shortcut")

    async with _editor(state, '<input id="a" value="AAA"><input id="b" value="BBB">') as page:
        await page.evaluate("""() => {
            a.focus(); a.setSelectionRange(0, 2);
            a.onkeydown = event => {
                if (event.key === 'x') { b.focus(); b.setSelectionRange(0, 2); }
            };
        }""")
        await page.keyboard.press("Control+x")
        assert_equal(await page.evaluate("[a.value, b.value]"), ["AAA", "B"], "shortcut follows keydown focus")
        state.record("editing_shortcut_focus_callback")

    for removed in (False, True):
        async with _editor(state, '<input id="a"><input id="b">') as page:
            await page.evaluate("a.focus()")
            await page.keyboard.type("A")
            await page.evaluate("b.focus()")
            await page.keyboard.type("B")
            await page.evaluate("globalThis.savedB = b")
            if removed:
                await page.evaluate("savedB.remove()")
            await page.evaluate("a.focus()")
            await page.keyboard.press("Control+z")
            assert_equal(
                await page.evaluate("[a.value, savedB.value, document.activeElement.id]"),
                ["", "B", "a"] if removed else ["A", "", "b"],
                "Document history skips removed target" if removed else "Document history restores previous control",
            )
            state.record("editing_history_removed_target" if removed else "editing_history_cross_control")

    async with _editor(state, '<div id="e" contenteditable>abc</div><div id="outside"></div>') as page:
        await page.evaluate("e.focus(); getSelection().collapse(e.firstChild, 2)")
        await page.keyboard.type("X")
        await page.evaluate("outside.appendChild(e.firstChild); e.focus()")
        await page.keyboard.press("Control+z")
        assert_equal(await page.text_content("#outside"), "abXc", "history cannot edit a moved node")
        state.record("editing_history_moved_node")

    async with _editor(state, '<div id="e" contenteditable><b>ab</b><i>cd</i></div>') as page:
        await page.evaluate("e.focus(); getSelection().collapse(e.firstChild.firstChild, 2)")
        await page.keyboard.press("Delete")
        assert_equal(await page.text_content("#e"), "abd", "Delete crosses text node boundary")
        state.record("editing_forward_delete_boundary")

    for custom in (False, True):
        for control in (False, True):
            markup = '<textarea id="s">seed</textarea>' + (
                '<textarea id="e">keep</textarea>' if control
                else '<div id="e" contenteditable>keep</div>'
            )
            async with _editor(state, markup) as page:
                if custom:
                    await page.evaluate("""() => {
                        s.focus(); s.select();
                        s.oncopy = event => {
                            event.clipboardData.setData('application/x-test', 'custom');
                            event.preventDefault();
                        };
                    }""")
                    await page.keyboard.press("Control+c")
                else:
                    await page.evaluate("navigator.clipboard.writeText('')")
                await page.evaluate(
                    "e.focus(); e.select()" if control
                    else "e.focus(); getSelection().selectAllChildren(e)"
                )
                await page.keyboard.press("Control+v")
                assert_equal(
                    await page.evaluate("e.value" if control else "e.textContent"),
                    "" if control else "keep", "paste without usable plain text",
                )
                state.record(f"editing_paste_{'custom' if custom else 'empty'}_{'control' if control else 'editable'}")

    async with _editor(state, '<div id="e" contenteditable>a<br>b</div><textarea id="i"></textarea>') as page:
        await page.evaluate("e.focus(); getSelection().selectAllChildren(e)")
        await page.keyboard.press("Control+c")
        await page.evaluate("i.focus()")
        await page.keyboard.press("Control+v")
        assert_equal(await page.input_value("#i"), "a\nb", "copy visible line break")
        state.record("editing_copy_plain_line_break")

    async with _editor(state, '<textarea id="i">first\nsecond</textarea><div id="e" contenteditable></div>') as page:
        await page.evaluate("i.focus(); i.select()")
        await page.keyboard.press("Control+c")
        await page.evaluate("e.focus()")
        await page.keyboard.press("Control+v")
        # Moli deliberately retains frozen geometry across ordinary DOM edits.
        await capture_layout(page)
        assert_equal(await page.inner_text("#e"), "first\nsecond", "paste visible line break")
        state.record("editing_paste_plain_line_break")
