from __future__ import annotations

import asyncio
from contextlib import AsyncExitStack

from ..assertions import assert_equal
from ..diagnostics import new_cdp_session
from ..helpers import capture_layout
from . import SmokeState
from .drag_protocol_contracts import run_drag_protocol_contracts


async def run_drag_interception_workflows(state: SmokeState) -> None:
    page = state.page
    await page.set_content(
        """
        <style>
          html,body { margin:0; height:320px }
          #source,#target,#plain { position:absolute; width:100px; height:80px }
          #source { left:20px; top:20px; background:#ddd }
          #target { left:220px; top:20px; background:#eee }
          #plain { left:20px; top:140px; background:#ccc }
        </style>
        <div id=source draggable=true>source</div><div id=target>target</div><div id=plain></div>
        <script>
          window.__dragLog = [];
          window.__acceptDrop = true;
          for (const type of ['dragstart','dragenter','dragover','dragleave','drop','dragend']) {
            document.addEventListener(type, event => {
              if (type === 'dragstart') event.dataTransfer.setData('text/plain','native-payload');
              if (event.target.id === 'target' && type === 'dragover' && window.__acceptDrop)
                event.preventDefault();
              if (type === 'drop') event.preventDefault();
              window.__dragLog.push({type, target:event.target.id,
                allowed:event.dataTransfer.effectAllowed, effect:event.dataTransfer.dropEffect,
                types:Array.from(event.dataTransfer.types), data:event.dataTransfer.getData('text/plain')});
            });
          }
        </script>
        """
    )
    await capture_layout(page)

    await page.locator("#source").drag_to(page.locator("#target"), timeout=5_000)
    events = await page.evaluate("() => window.__dragLog")
    assert_equal([event["type"] for event in events if event["type"] in ("dragstart", "drop", "dragend")],
                 ["dragstart", "drop", "dragend"], "Playwright dragTo completes the native source lifecycle")
    assert_equal(next(event for event in events if event["type"] == "drop")["data"],
                 "native-payload", "Playwright dragTo transfers the native payload")
    assert_equal([event["effect"] for event in events if event["type"] in ("drop", "dragend")],
                 ["copy", "copy"], "a handled native drop retains its negotiated copy operation")
    await page.evaluate("() => window.__dragLog = []")
    state.record("locator_native_drag_to")

    async with AsyncExitStack() as cleanup:
        first = await new_cdp_session(page.context, page)
        cleanup.push_async_callback(first.detach)
        second = await new_cdp_session(page.context, page)
        cleanup.push_async_callback(second.detach)
        intercepted: dict[str, list[dict]] = {"first": [], "second": []}
        ready = {"first": asyncio.Event(), "second": asyncio.Event()}

        def capture(name: str, event: dict) -> None:
            intercepted[name].append(event)
            ready[name].set()

        first.on("Input.dragIntercepted", lambda event: capture("first", event))
        second.on("Input.dragIntercepted", lambda event: capture("second", event))
        await first.send("Input.setInterceptDrags", {"enabled": True})
        await second.send("Input.setInterceptDrags", {"enabled": False})

        async def mouse(session, kind: str, x: float, y: float, buttons: int) -> None:
            await session.send("Input.dispatchMouseEvent", {
                "type": kind, "x": x, "y": y, "buttons": buttons,
                "button": "none" if kind == "mouseMoved" and not buttons else "left",
            })

        async def log() -> list[dict]:
            return await page.evaluate("() => window.__dragLog")

        async def start(session, name: str) -> dict:
            ready[name].clear()
            await page.evaluate("() => window.__dragLog = []")
            await mouse(session, "mousePressed", 40, 40, 1)
            await mouse(session, "mouseMoved", 50, 50, 1)
            await asyncio.wait_for(ready[name].wait(), timeout=5)
            return intercepted[name][-1]["data"]

        async def replay(session, kind: str, data: dict) -> None:
            await session.send("Input.dispatchDragEvent", {
                "type": kind, "x": 250, "y": 40, "data": data,
            })

        # A plain element cannot start a native drag merely by moving the mouse.
        await mouse(first, "mousePressed", 40, 160, 1)
        await mouse(first, "mouseMoved", 90, 160, 1)
        await mouse(first, "mouseReleased", 90, 160, 0)
        assert_equal(await log(), [], "ordinary held mouse movement does not start a drag")
        assert_equal(intercepted["first"], [], "ordinary movement is not intercepted")

        # Movement inside the threshold preserves the candidate for a later move.
        await mouse(first, "mousePressed", 40, 40, 1)
        await mouse(first, "mouseMoved", 41, 41, 1)
        assert_equal(await log(), [], "sub-threshold movement does not start a drag")
        assert_equal(intercepted["first"], [], "sub-threshold movement is not intercepted")
        await mouse(first, "mouseMoved", 50, 50, 1)
        await asyncio.wait_for(ready["first"].wait(), timeout=5)
        data = intercepted["first"][-1]["data"]
        assert_equal(data["dragOperationsMask"], -1, "native default uses Chromium's all-operations wire value")
        starts = [event for event in await log() if event["type"] == "dragstart"]
        assert_equal(len(starts), 1, "threshold crossing starts one native drag")
        assert_equal(starts[0]["allowed"], "uninitialized", "native effectAllowed starts uninitialized")
        assert_equal(starts[0]["data"], "native-payload", "dragstart has read/write data access")
        assert_equal(intercepted["second"], [], "interception event stays with the enabled session")
        await replay(first, "dragCancel", data)
        await mouse(first, "mouseReleased", 50, 50, 0)
        assert_equal(
            [event["type"] for event in await log() if event["type"] in ("dragstart", "dragend")],
            ["dragstart", "dragend"], "cancel ends the source once",
        )

        data = await start(first, "first")
        for kind in ("dragEnter", "dragOver", "drop"):
            await replay(first, kind, data)
        events = await log()
        assert_equal(
            [event["type"] for event in events if event["type"] in ("dragstart", "drop", "dragend")],
            ["dragstart", "drop", "dragend"], "accepted replay completes the source lifecycle",
        )
        for event in events:
            if event["type"] in ("dragenter", "dragover"):
                assert_equal(event["data"], "", f"{event['type']} protects the payload")
                assert_equal(event["types"], ["text/plain"], f"{event['type']} exposes types")
                assert_equal(event["effect"], "copy", f"all operations initialize {event['type']} to copy")
        assert_equal(next(event for event in events if event["type"] == "drop")["data"],
                     "native-payload", "drop can read the original payload")
        assert_equal([event["effect"] for event in events if event["type"] in ("drop", "dragend")],
                     ["copy", "copy"], "handled replay preserves the accepted operation through source completion")
        await mouse(first, "mouseReleased", 250, 40, 0)

        data = await start(first, "first")
        data = {**data, "items": [{"mimeType": "text/plain", "data": "client-payload"}]}
        for kind in ("dragEnter", "drop"):
            await replay(first, kind, data)
        events = await log()
        assert_equal(next(event for event in events if event["type"] == "drop")["data"],
                     "client-payload", "replay consumes the client's explicit payload")
        assert_equal([event["type"] for event in events if event["type"] == "dragend"],
                     ["dragend"], "replacing replay data still ends the source once")
        await mouse(first, "mouseReleased", 250, 40, 0)

        # Drop performs a fresh dragover; a target that refuses it must see no drop.
        await page.evaluate("() => window.__acceptDrop = false")
        data = await start(first, "first")
        await replay(first, "dragEnter", data)
        await replay(first, "drop", data)
        events = await log()
        assert_equal([event for event in events if event["type"] == "drop"], [],
                     "a refusing target receives no drop")
        assert_equal([event["effect"] for event in events if event["type"] == "dragend"],
                     ["none"], "a rejected drop ends with no operation")
        await mouse(first, "mouseReleased", 250, 40, 0)

        # Disable one session without overriding the other session's configuration.
        await second.send("Input.setInterceptDrags", {"enabled": True})
        await first.send("Input.setInterceptDrags", {"enabled": False})
        first_count = len(intercepted["first"])
        data = await start(second, "second")
        assert_equal(len(intercepted["first"]), first_count, "other session receives no interception event")
        await replay(second, "dragCancel", data)
        await mouse(second, "mouseReleased", 50, 50, 0)
        await second.send("Input.setInterceptDrags", {"enabled": False})
    state.record("drag_interception_lifecycle_and_session_isolation")
    await run_native_text_drop_workflows(state)
    await run_drag_protocol_contracts(state)


async def run_native_text_drop_workflows(state: SmokeState) -> None:
    page = state.page
    cases = [
        ("<input id=target>", "payload", "uninitialized"),
        ("<textarea id=target></textarea>", "payload", "uninitialized"),
        ("<div id=target contenteditable=true></div>", "payload", "uninitialized"),
        ("<input id=target readonly>", "", "uninitialized"),
        ("<textarea id=target readonly></textarea>", "", "uninitialized"),
        ("<input id=target disabled>", "", "uninitialized"),
        ("<div id=target></div>", "", "uninitialized"),
    ]
    for markup in ("<input id=target>", "<textarea id=target></textarea>",
                   "<div id=target contenteditable=true></div>"):
        cases.extend([(markup, "payload", "copyMove"), (markup, "", "linkMove")])
    for markup, expected, allowed in cases:
        label = f"{markup}, effectAllowed={allowed}"
        await page.set_content(
            """<style>
              #source,#target{position:absolute;width:100px;height:80px}
              #source{left:20px;top:20px}#target{left:220px;top:20px}
            </style><div id=source draggable=true>source</div>""" + markup + """<script>
              window.__defaultDropLog=[];
              document.getElementById('source').addEventListener('dragstart', event => {
                event.dataTransfer.setData('text/plain','payload');
                event.dataTransfer.effectAllowed='""" + allowed + """';
              });
              for(const type of ['dragover','drop','dragend']) document.addEventListener(type, event =>
                window.__defaultDropLog.push({type, canceled:event.defaultPrevented,
                  effect:event.dataTransfer.dropEffect, data:event.dataTransfer.getData('text/plain')}));
            </script>"""
        )
        await capture_layout(page)
        await page.locator("#source").drag_to(page.locator("#target"), timeout=5_000)
        result = await page.evaluate("""() => {
          const target=document.getElementById('target');
          return {events:window.__defaultDropLog, value:target.value ?? target.textContent};
        }""")
        assert_equal(result["value"], expected, f"default drop into {label}")
        drops = [event for event in result["events"] if event["type"] == "drop"]
        assert_equal(len(drops), int(bool(expected)), f"default acceptance for {label}")
        assert_equal([event["effect"] for event in result["events"] if event["type"] == "dragend"],
                     ["copy" if expected else "none"], f"source finishes with the default operation for {label}")
        for event in result["events"]:
            if event["type"] == "dragover":
                assert_equal(event["canceled"], False, f"author does not accept dragover for {label}")
                assert_equal(event["data"], "", f"dragover protects default drop data for {label}")
    state.record("native_text_drop_default_acceptance")
