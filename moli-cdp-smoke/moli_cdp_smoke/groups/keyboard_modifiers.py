from __future__ import annotations

from typing import Any

from ..assertions import assert_equal
from ..diagnostics import new_cdp_session
from ..state import SmokeState


def _key_event(
    kind: str, key: str, code: str, modifiers: int, *, repeat: bool = False
) -> dict[str, Any]:
    flags = [bool(modifiers & bit) for bit in (1, 2, 4, 8)]
    return {
        "type": kind,
        "key": key,
        "code": code,
        "altKey": flags[0],
        "ctrlKey": flags[1],
        "metaKey": flags[2],
        "shiftKey": flags[3],
        "states": flags,
        "repeat": repeat,
        "trusted": True,
    }


async def run_keyboard_modifier_workflow(state: SmokeState) -> None:
    page = await state.context.new_page()
    session = None
    try:
        session = await new_cdp_session(state.context, page)
        for control in (
            '<input id="field">',
            '<textarea id="field"></textarea>',
            '<div id="field" contenteditable="true"></div>',
        ):
            await page.set_content(control)
            await page.evaluate("""() => {
                const field = document.getElementById('field');
                window.__modifierKeys = [];
                window.__modifierEdits = [];
                for (const type of ['keydown', 'keypress', 'keyup']) {
                    field.addEventListener(type, event => __modifierKeys.push({
                        type, key: event.key, code: event.code,
                        altKey: event.altKey, ctrlKey: event.ctrlKey,
                        metaKey: event.metaKey, shiftKey: event.shiftKey,
                        states: ['Alt', 'Control', 'Meta', 'Shift'].map(
                            name => event.getModifierState(name)),
                        repeat: event.repeat, trusted: event.isTrusted,
                    }));
                }
                for (const type of ['beforeinput', 'input']) {
                    field.addEventListener(type, event => __modifierEdits.push({
                        type, data: event.data, trusted: event.isTrusted,
                    }));
                }
            }""")

            async def reset(value: str = "") -> None:
                await page.evaluate(
                    """value => {
                    const field = document.getElementById('field');
                    if ('value' in field) field.value = value;
                    else field.textContent = value;
                    field.focus();
                    if ('setSelectionRange' in field) field.setSelectionRange(2, 2);
                    window.__modifierKeys = [];
                    window.__modifierEdits = [];
                }""",
                    value,
                )

            async def check(
                value: str, keys: list[dict[str, Any]], text: str | None, label: str
            ) -> None:
                actual = await page.evaluate("""() => {
                    const field = document.getElementById('field');
                    return {value: field.value ?? field.textContent,
                        keys: __modifierKeys, edits: __modifierEdits};
                }""")
                edits = (
                    []
                    if text is None
                    else [
                        {"type": kind, "data": text, "trusted": True}
                        for kind in ("beforeinput", "input")
                    ]
                )
                assert_equal(
                    actual, {"value": value, "keys": keys, "edits": edits}, label
                )

            for modifier, code, mask in (
                ("Alt", "AltLeft", 1),
                ("Control", "ControlLeft", 2),
                ("Meta", "MetaLeft", 4),
                ("Shift", "ShiftLeft", 8),
            ):
                await reset()
                await page.keyboard.press(f"{modifier}+a")
                # Playwright's literal "a" stays lowercase even with Shift;
                # the physical KeyA spelling below exercises shifted layout.
                key = "a"
                keys = [
                    _key_event("keydown", modifier, code, mask),
                    _key_event("keydown", key, "KeyA", mask),
                ]
                if modifier == "Shift":
                    keys.append(_key_event("keypress", key, "KeyA", mask))
                keys.extend(
                    [
                        _key_event("keyup", key, "KeyA", mask),
                        _key_event("keyup", modifier, code, 0),
                    ]
                )
                inserts = modifier == "Shift"
                await check(
                    "a" if inserts else "",
                    keys,
                    "a" if inserts else None,
                    f"{control}: {modifier}+a preserves each event and modifier",
                )

            await reset()
            await page.keyboard.press("Shift+KeyA")
            await check(
                "A",
                [
                    _key_event("keydown", "Shift", "ShiftLeft", 8),
                    _key_event("keydown", "A", "KeyA", 8),
                    _key_event("keypress", "A", "KeyA", 8),
                    _key_event("keyup", "A", "KeyA", 8),
                    _key_event("keyup", "Shift", "ShiftLeft", 0),
                ],
                "A",
                "Shift+KeyA uses the shifted physical key description",
            )

            await reset("abc")
            await page.keyboard.press("Shift+ArrowLeft")
            await check(
                "abc",
                [
                    _key_event("keydown", "Shift", "ShiftLeft", 8),
                    _key_event("keydown", "ArrowLeft", "ArrowLeft", 8),
                    _key_event("keyup", "ArrowLeft", "ArrowLeft", 8),
                    _key_event("keyup", "Shift", "ShiftLeft", 0),
                ],
                None,
                f"{control}: Shift keydown carries shiftKey without duplicate events",
            )
            if not control.startswith("<div"):
                assert_equal(
                    await page.evaluate("""() => {
                    const field = document.getElementById('field');
                    return [field.selectionStart, field.selectionEnd, field.selectionDirection];
                }"""),
                    [1, 2, "backward"],
                    "Shift+ArrowLeft extends the text selection",
                )

            await reset()
            await page.keyboard.down("Shift")
            await page.keyboard.down("Shift")
            await page.keyboard.up("Shift")
            await check(
                "",
                [
                    _key_event("keydown", "Shift", "ShiftLeft", 8),
                    _key_event("keydown", "Shift", "ShiftLeft", 8, repeat=True),
                    _key_event("keyup", "Shift", "ShiftLeft", 0),
                ],
                None,
                "intentional modifier autorepeat is retained",
            )

            # Explicit CDP character input differs from a Playwright shortcut.
            # Do not fix Alt+a by dropping every character carrying altKey.
            for kind in ("rawKeyDown", "keyDown", "char"):
                await reset()
                await session.send(
                    "Input.dispatchKeyEvent",
                    {
                        "type": kind,
                        "key": "a",
                        "code": "KeyA",
                        "text": "a",
                        "windowsVirtualKeyCode": 65,
                        "modifiers": 1,
                    },
                )
                await session.send(
                    "Input.dispatchKeyEvent",
                    {
                        "type": "keyUp",
                        "key": "a",
                        "code": "KeyA",
                        "windowsVirtualKeyCode": 65,
                    },
                )
                keys = []
                if kind != "char":
                    keys.append(_key_event("keydown", "a", "KeyA", 1))
                inserts = kind != "rawKeyDown"
                if inserts:
                    keys.append(_key_event("keypress", "a", "KeyA", 1))
                keys.append(_key_event("keyup", "a", "KeyA", 0))
                await check(
                    "a" if inserts else "",
                    keys,
                    "a" if inserts else None,
                    f"explicit CDP {kind} preserves Alt character-phase semantics",
                )

            await reset()
            await page.keyboard.down("Alt")
            await session.send("Input.insertText", {"text": "@"})
            await page.keyboard.up("Alt")
            await check(
                "@",
                [
                    _key_event("keydown", "Alt", "AltLeft", 1),
                    _key_event("keyup", "Alt", "AltLeft", 0),
                ],
                "@",
                "insertText still works while Alt is held",
            )

        state.record("keyboard_modifier_shortcuts_and_character_phases")
    finally:
        if session is not None:
            await session.detach()
        await page.close()
