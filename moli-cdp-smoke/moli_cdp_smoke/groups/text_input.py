from __future__ import annotations

import asyncio
import json
import traceback
from typing import Any, Awaitable

from ..assertions import SmokeError, assert_equal, record_contract
from ..config import REPO_ROOT
from ..diagnostics import new_cdp_session


class TextInputPageCrashed(SmokeError):
    pass


async def _input_command(page: Any, operation: Awaitable[Any]) -> Any:
    # A renderer crash can leave Input.insertText without a CDP reply. Race
    # against the actual page-lifetime event, rather than hanging or retrying.
    crashed = asyncio.get_running_loop().create_future()

    def on_crash(*_: Any) -> None:
        if not crashed.done():
            crashed.set_result(None)

    page.on("crash", on_crash)
    task = asyncio.ensure_future(operation)
    try:
        done, _ = await asyncio.wait([task, crashed], return_when=asyncio.FIRST_COMPLETED)
        if crashed in done:
            raise TextInputPageCrashed("browser page crashed during native text input")
        return await task
    finally:
        page.remove_listener("crash", on_crash)
        crashed.cancel()
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)


def _text_event(target: str, original: str, data: str, value: str = "") -> dict[str, Any]:
    return {"type": "textInput", "target": target, "originalTarget": original,
            "data": data, "native": True, "ui": True, "input": False, "trusted": True,
            "bubbles": True, "cancelable": True, "composed": True, "view": True,
            "detail": 0, "value": value}


def _sequence(route: str, target: str) -> list[str]:
    kinds = (["keydown"] if route in {"keyDown", "raw-char", "type"} else [])
    if route != "insertText":
        kinds.append("keypress")
    kinds.extend(["beforeinput", "textInput", "input"])
    if route != "insertText":
        kinds.append("keyup")
    return [f"{kind}:{target}" for kind in kinds]


async def run_text_input_group(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    script = (REPO_ROOT / "moli-renderer-v8/tests/fixtures/text-input-events.js").read_text()
    cases: list[dict[str, Any]] = []
    for control in ["input", "textarea", "editor", "shadow", "frame"]:
        for route in ["keyDown", "char", "raw-char", "insertText", "type"]:
            # CDP dispatchKeyEvent accepts at most three UTF-16 code units;
            # insertText has no keyboard-event buffer limit.
            for text in (["a"] if route == "type" else ["a", "😀ab" if route == "insertText" else "😀a"]):
                cases.append({"control": control, "route": route, "text": text})
    for control in ["input", "textarea", "editor"]:
        for route in ["keyDown", "char", "insertText"]:
            for cancel in ["beforeinput", "textInput"]:
                cases.append({"control": control, "route": route, "options": {"cancel": cancel}})
    for control, effects in [("input", ["remove", "readonly", "adopt", "selection"]),
                             ("textarea", ["remove", "readonly", "adopt", "selection"]),
                             ("editor", ["remove", "disable", "adopt"]), ("frame", ["remove-frame"])]:
        for route in ["keyDown", "insertText"]:
            for effect in effects:
                cases.append({"control": control, "route": route, "text": "X",
                              "options": {"phase": "textInput", "effect": effect}})
    for control in ["input", "textarea", "editor"]:
        for route in ["keyDown", "insertText"]:
            for phase in ["beforeinput", "textInput"]:
                cases.append({"control": control, "route": route, "text": "X",
                              "options": {"phase": phase, "effect": "focus"}})
    for control in ["input", "shadow", "frame"]:
        cases.append({"control": control, "route": "keyDown", "options": {"poison": True}})
    for control in ["input", "textarea"]:
        for route in ["keyDown", "insertText"]:
            cases.append({"control": control, "route": route, "text": "XYZ", "options": {"maxLength": 1}})
    for control in ["input", "textarea", "editor"]:
        cases.append({"control": control, "route": "enter"})
    for control in ["readonly", "plain"]:
        for route in ["keyDown", "insertText"]:
            cases.append({"control": control, "route": route})
    for route in ["empty", "rawKeyDown", "backspace", "exec", "unicode-type"]:
        cases.append({"control": "input", "route": route})
    context = await browser.new_context()
    failures = []
    try:
        for case in cases:
            kind, route, text = case["control"], case["route"], case.get("text", "a")
            options = case.get("options", {})
            label = f"{kind}/{route}/{text}/{json.dumps(options, sort_keys=True)}"
            page = await context.new_page()
            session = None
            crashed = False
            try:
                await page.goto(f"{fixture}/plain", wait_until="load")
                await page.evaluate(f"{script}\ninstallTextInputProbe({json.dumps(kind)}, {json.dumps(options)})")
                session = await new_cdp_session(context, page)
                async def send(params: dict[str, Any]) -> None:
                    await _input_command(page, session.send("Input.dispatchKeyEvent", params))

                if route in {"insertText", "empty"}:
                    await _input_command(page, session.send("Input.insertText", {"text": "" if route == "empty" else text}))
                elif route in {"type", "unicode-type"}:
                    await _input_command(page, page.keyboard.type("a😀b" if route == "unicode-type" else text))
                elif route == "exec":
                    await page.evaluate("document.execCommand('insertText',false,'a')")
                elif route == "backspace":
                    await page.evaluate("textInputProbe.field.value='abc';textInputProbe.field.setSelectionRange(2,2)")
                    await _input_command(page, page.keyboard.press("Backspace"))
                elif route == "enter":
                    await _input_command(page, page.keyboard.press("Enter"))
                else:
                    if route == "raw-char":
                        await send({"type": "rawKeyDown", "key": "a", "code": "KeyA", "text": text, "windowsVirtualKeyCode": 65})
                    await send({"type": "keyDown" if route == "keyDown" else "rawKeyDown" if route == "rawKeyDown" else "char",
                                                               "key": "a", "code": "KeyA", "text": text, "windowsVirtualKeyCode": 65})
                    await send({"type": "keyUp", "key": "a", "code": "KeyA", "windowsVirtualKeyCode": 65})
                observed = await page.evaluate("readTextInputProbe()")
                target = "host" if kind == "shadow" else "field"
                effect, cancel = options.get("effect"), options.get("cancel")
                if effect == "focus":
                    legacy_target = "other" if route == "insertText" and options["phase"] == "beforeinput" else "field"
                    assert_equal(observed["textEvents"], [_text_event(legacy_target, legacy_target, text)], label)
                    other = legacy_target == "other" or kind == "editor"
                    assert_equal([observed["value"], observed["other"]], ["", text] if other else [text, ""], label)
                    active = await page.evaluate("textInputProbe.field.ownerDocument.activeElement.id")
                    assert_equal(active, "other" if other else "field", label + " final focus")
                elif effect:
                    assert_equal(len(observed["textEvents"]), 1, label)
                    if effect == "selection":
                        assert_equal(observed["value"], "aXc", label)
                        caret = 1 if route == "insertText" else 2
                        assert_equal(observed["selection"], [caret, caret], label)
                    else:
                        assert_equal(observed["value"], "", label)
                        if any(event.startswith("input:") for event in observed["events"]):
                            raise SmokeError(label + ": invalidated target received input")
                elif cancel:
                    sequence = _sequence(route, target)
                    index = sequence.index(f"{cancel}:{target}")
                    sequence = sequence[:index + 1] + ([f"keyup:{target}"] if route != "insertText" else [])
                    assert_equal(observed["events"], sequence, label)
                    assert_equal(observed["value"], "", label)
                    assert_equal(observed["textEvents"], [_text_event(target, "field", text)] if cancel == "textInput" else [], label)
                elif route == "enter":
                    assert_equal(observed["textEvents"], [] if kind == "input" else [_text_event("field", "field", "\n")], label)
                    if kind != "input":
                        input_type = "insertLineBreak" if kind == "textarea" else "insertParagraph"
                        assert_equal(observed["editEvents"], [{"type": type_, "inputType": input_type, "data": None} for type_ in ["beforeinput", "input"]], label)
                elif route == "unicode-type":
                    assert_equal(observed["value"], "a😀b", label)
                    assert_equal(observed["textEvents"], [_text_event("field", "field", data, value)
                                                        for data, value in [("a", ""), ("😀", "a"), ("b", "a😀")]], label)
                elif route in {"empty", "rawKeyDown", "backspace", "exec"} or kind == "plain":
                    assert_equal(observed["textEvents"], [], label)
                    assert_equal(observed["value"], "ac" if route == "backspace" else "a" if route == "exec" else "", label)
                    if kind == "plain" and route == "insertText":
                        assert_equal(observed["events"], [], label)
                else:
                    assert_equal(observed["textEvents"], [_text_event(target, "field", text)], label)
                    value = "" if kind in {"readonly", "plain"} else text[:1] if "maxLength" in options else text
                    assert_equal(observed["value"], value, label)
                    sequence = _sequence(route, target)
                    if kind in {"readonly", "plain"}:
                        sequence.remove(f"input:{target}")
                    assert_equal(observed["events"], sequence, label)
                record_contract(results, "text-input/" + label, observed=observed,
                                contract="Native text insertion dispatches a trusted, cancelable TextEvent after beforeinput and before its editing default.",
                                source="Chromium 145.0.7632.116 executable comparison (2026-11-13)",
                                commands=["Input.dispatchKeyEvent", "Input.insertText", "Runtime.evaluate"])
            except Exception as error:
                crashed = isinstance(error, TextInputPageCrashed)
                failures.append(f"{label}: {error}")
                results.append({"name": "text-input/" + label, "ok": False,
                                "error": str(error), "traceback": traceback.format_exc()})
            finally:
                if session is not None and not crashed:
                    await session.detach()
                await page.close()
    finally:
        await context.close()
    if failures:
        raise SmokeError("textInput failures:\n" + "\n".join(failures))
