"""Native drag payloads and frame/event contracts calibrated with Chromium 156."""

from __future__ import annotations

import asyncio
import html
from contextlib import asynccontextmanager
from html.parser import HTMLParser

from ..assertions import assert_equal
from ..config import is_moli_browser
from ..diagnostics import new_cdp_session
from ..helpers import capture_layout
from . import SmokeState


_BASE_URL = "https://drag-source.test/assets/"
_LINK_URL = "https://drag-source.test/details?x=1&y=2"
_OVERRIDE_URL = "https://override.test/changed"
_OVERRIDE_HTML = "<em>author changed</em>"
_BLOCKED_BASE_URL = "about:blank#blocked"

_BOARD_STYLE = """<style>
  html,body{margin:0;height:500px;font:16px Arial}
  #source{position:absolute;left:20px;top:20px;width:200px;height:60px}
  #target,#other{position:absolute;left:300px;top:20px;width:200px;height:100px}
  #other{left:550px}
</style>"""

_INSTALL_LOG = """options => {
  window.__nativeSemantics={events:[]};
  const state=window.__nativeSemantics;
  if(options.trackSource) {
    state.sourceViews=[];state.sourceDragCount=0;
    state.sourceState=()=>({types:Array.from(state.source.types),items:state.source.items.length,
      data:state.source.getData('text/plain'),effect:state.source.dropEffect,
      savedItem:{kind:state.sourceItem.kind,type:state.sourceItem.type},
      savedTypes:Array.from(state.sourceTypes),distinctViews:new Set(state.sourceViews).size});
  }
  const name=value=>value?(value.id||value.nodeName):null;
  for(const type of ['dragstart','drag','dragenter','dragover','dragleave','drop','dragend']) {
    document.addEventListener(type,event=>{
      const transfer=event.dataTransfer;
      const initialEffect=transfer.dropEffect;
      if(type==='dragstart') {
        if(options.allowed)transfer.effectAllowed=options.allowed;
        if(options.sourcePayload)transfer.setData('text/plain','source-payload');
        if(options.override) {
          transfer.setData('text/uri-list','https://override.test/changed');
          transfer.setData('text/html','<em>author changed</em>');
        }
        if(options.trackSource) {
          state.source=transfer;state.sourceItem=transfer.items[0];state.sourceTypes=transfer.types;
        }
      }
      const sourceEvent=['dragstart','drag','dragend'].includes(type);
      if(options.trackSource && sourceEvent)state.sourceViews.push(transfer);
      if(options.trackSource && type==='drag' && state.sourceDragCount++===0)transfer.dropEffect='link';
      if(type==='drag' && options.cancelDrag)event.preventDefault();
      if(!options.defaultEditor && (type==='dragenter'||type==='dragover'))event.preventDefault();
      if(type==='drop' && options.cancelDrop)event.preventDefault();
      window.__nativeSemantics.events.push({
        type,target:name(event.target),relatedTarget:name(event.relatedTarget),
        client:[event.clientX,event.clientY],screen:[event.screenX,event.screenY],
        allowed:transfer.effectAllowed,effect:transfer.dropEffect,
        canceled:event.defaultPrevented,cancelable:event.cancelable,
        types:Array.from(transfer.types),
        strings:Object.fromEntries(Array.from(transfer.types).filter(type=>type!=='Files')
          .map(type=>[type,transfer.getData(type)])),
        ...(options.trackSource ? {initialEffect,source:state.sourceState(),
          ...(sourceEvent ? {sameSource:transfer===state.source} : {})} : {}),
      });
    },true);
  }
}"""


class _FragmentSummary(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.tags: list[str] = []
        self.text: list[str] = []
        self.bold: list[str] = []
        self.italic: list[str] = []
        self.links: list[dict] = []
        self.images: list[dict] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if tag == "a":
            self.links.append({"href": values.get("href"), "title": values.get("title")})
        if tag == "img":
            self.images.append({"src": values.get("src"), "alt": values.get("alt"),
                                "title": values.get("title")})
        if tag not in {"area", "base", "br", "col", "embed", "hr", "img", "input",
                       "link", "meta", "param", "source", "track", "wbr"}:
            self.tags.append(tag)

    def handle_endtag(self, tag: str) -> None:
        if tag in self.tags:
            self.tags = self.tags[:len(self.tags) - 1 - self.tags[::-1].index(tag)]

    def handle_data(self, data: str) -> None:
        self.text.append(data)
        if any(tag in self.tags for tag in ("b", "strong")):
            self.bold.append(data)
        if any(tag in self.tags for tag in ("i", "em")):
            self.italic.append(data)


def _fragment_summary(markup: str) -> dict:
    fragment = _FragmentSummary()
    fragment.feed(markup)
    return {"text": "".join(fragment.text), "bold": "".join(fragment.bold),
            "italic": "".join(fragment.italic), "links": fragment.links,
            "images": fragment.images}


def _assert_payload(kind: str, start: dict, packet: dict, drop: dict,
                    document_url: str, image_url: str, *, overridden: bool = False) -> None:
    label = f"native {kind}{' author override' if overridden else ''}"
    assert_equal(packet["dragOperationsMask"], -1, f"{label}: all operations use the wire value -1")
    items = packet["items"]
    by_type = {item["mimeType"]: item for item in items}
    if overridden:
        expected_types = (["text/plain"] if kind != "linked-image" else []) + ["text/uri-list", "text/html"]
        # Chromium replaces strings by removing and appending. Moli retains
        # their position, as setData's standard contract requires. Both must
        # export the exact order their dragstart event actually exposes.
        assert_equal(sorted(item["mimeType"] for item in items), sorted(expected_types),
                     f"{label}: replacing strings retains the complete format set")
        assert_equal(by_type["text/uri-list"], {"mimeType": "text/uri-list", "data": _OVERRIDE_URL},
                     f"{label}: author URI replacement clears the native title")
        assert_equal(by_type["text/html"], {"mimeType": "text/html", "data": _OVERRIDE_HTML,
                                           "baseURL": _BLOCKED_BASE_URL},
                     f"{label}: author HTML replacement clears the native base URL")
    elif kind == "selection":
        assert_equal([item["mimeType"] for item in items], ["text/html", "text/plain"],
                     f"{label}: selection supplies HTML before plain text")
        assert_equal(by_type["text/plain"]["data"], "Rich text", f"{label}: selected visible text")
        assert_equal(by_type["text/html"].get("baseURL"), document_url,
                     f"{label}: selection HTML base is the Document URL, independently of base href")
        summary = _fragment_summary(by_type["text/html"]["data"])
        assert_equal((summary["text"], summary["bold"], summary["italic"]),
                     ("Rich text", "Rich", " text"), f"{label}: selected inline formatting survives")
    elif kind == "link":
        assert_equal([item["mimeType"] for item in items], ["text/uri-list", "text/plain", "text/html"],
                     f"{label}: native link string formats")
        assert_equal(by_type["text/uri-list"], {"mimeType": "text/uri-list", "data": _LINK_URL,
                                               "title": "Link label"},
                     f"{label}: URI title uses the visible link label")
        assert_equal(by_type["text/plain"]["data"], _LINK_URL, f"{label}: resolved URL plain text")
        assert_equal(by_type["text/html"].get("baseURL"), _LINK_URL, f"{label}: link HTML base URL")
        summary = _fragment_summary(by_type["text/html"]["data"])
        assert_equal((summary["text"], summary["bold"], summary["links"]),
                     ("Link label", "label", [{"href": _LINK_URL, "title": "Anchor title"}]),
                     f"{label}: resolved anchor markup retains its content and attributes")
    else:
        assert_equal([item["mimeType"] for item in items], ["text/uri-list", "text/html"],
                     f"{label}: linked image has no invented plain-text payload")
        assert_equal(by_type["text/uri-list"], {"mimeType": "text/uri-list", "data": _LINK_URL,
                                               "title": "Picture alt"},
                     f"{label}: linked image uses the enclosing href and image alt")
        assert_equal(by_type["text/html"].get("baseURL"), _BLOCKED_BASE_URL,
                     f"{label}: image HTML has the browser's empty-base normalization")
        summary = _fragment_summary(by_type["text/html"]["data"])
        assert_equal(summary["links"], [], f"{label}: image HTML does not wrap the enclosing link")
        assert_equal(summary["images"], [{"src": image_url, "alt": "Picture alt", "title": "Image title"}],
                     f"{label}: image HTML resolves src and preserves image metadata")
    exported = {item["mimeType"]: item["data"] for item in items}
    assert_equal([item["mimeType"] for item in items], [kind for kind in start["types"] if kind != "Files"],
                 f"{label}: raw item order follows this backend's source DataTransfer")
    assert_equal(sorted(drop["types"]), sorted(exported), f"{label}: drop retains every exported string format")
    assert_equal(start["strings"], exported, f"{label}: raw interception exports dragstart's actual strings")
    assert_equal(drop["strings"], exported, f"{label}: replay retains all string payloads")


async def _mouse(session, kind: str, x: float, y: float, buttons: int) -> None:
    await session.send("Input.dispatchMouseEvent", {
        "type": kind, "x": x, "y": y, "buttons": buttons,
        "button": "left" if buttons or kind == "mouseReleased" else "none",
    })


async def _drag(session, kind: str, packet: dict, x: float = 330, y: float = 40) -> None:
    await session.send("Input.dispatchDragEvent", {"type": kind, "x": x, "y": y, "data": packet})


async def _events(page) -> list[dict]:
    return await page.evaluate("() => window.__nativeSemantics.events")


@asynccontextmanager
async def _session_page(state: SmokeState):
    page = await state.context.new_page()
    session = await new_cdp_session(state.context, page)
    received: list[dict] = []
    ready = asyncio.Event()

    def intercepted(event: dict) -> None:
        received.append(event["data"])
        ready.set()

    session.on("Input.dragIntercepted", intercepted)
    try:
        yield page, session, received, ready
    finally:
        await _drag(session, "dragCancel", {"items": [], "dragOperationsMask": 0})
        await _mouse(session, "mouseReleased", 330, 40, 0)
        await session.send("Input.setInterceptDrags", {"enabled": False})
        await session.detach()
        await page.close()


async def _start(page, session, ready: asyncio.Event, *, selection: bool = False) -> None:
    await capture_layout(page)
    await session.send("Input.setInterceptDrags", {"enabled": True})
    await _mouse(session, "mouseMoved", 30, 28, 0)
    await _mouse(session, "mousePressed", 30, 28, 1)
    if selection:
        # smoke-timing: Blink's kTextDragDelay distinguishes dragging a selection from extending it.
        await asyncio.sleep(0.25)
    await _mouse(session, "mouseMoved", 45, 33, 1)
    await asyncio.wait_for(ready.wait(), timeout=5)


async def _default_payloads(state: SmokeState) -> None:
    image_url = f"{state.fixture}/chromium-resource-image.png"
    for kind in ("selection", "link", "linked-image"):
        for overridden in (False, True):
            async with _session_page(state) as (page, session, received, ready):
                await page.goto(state.fixture)
                document_url = page.url
                if kind == "selection":
                    source = "<div id=source><b>Rich</b><i> text</i></div>"
                elif kind == "link":
                    source = '<a id=source href="../details?x=1&amp;y=2" title="Anchor title">Link <b>label</b></a>'
                else:
                    source = ('<a id=link href="../details?x=1&amp;y=2" title="Anchor title">'
                              f'<img id=source src="{html.escape(image_url, quote=True)}" '
                              'alt="Picture alt" title="Image title"></a>')
                await page.set_content(f'<!doctype html><base href="{_BASE_URL}">{_BOARD_STYLE}' + source
                                       + "<div id=target contenteditable=true></div>")
                await page.evaluate(_INSTALL_LOG, {"override": overridden, "defaultEditor": True})
                if kind == "linked-image":
                    await page.locator("#source").evaluate("image => image.decode()")
                if kind == "selection":
                    await page.evaluate("""() => {
                      const range=document.createRange();range.selectNodeContents(document.getElementById('source'));
                      const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
                    }""")
                await _start(page, session, ready, selection=kind == "selection")
                assert_equal(len(received), 1, f"{kind}: exactly one native interception")
                packet = received[0]
                for command in ("dragEnter", "dragOver", "drop"):
                    await _drag(session, command, packet)
                events = await _events(page)
                starts = [event for event in events if event["type"] == "dragstart"]
                drops = [event for event in events if event["type"] == "drop"]
                assert_equal((len(starts), len(drops)), (1, 1), f"{kind}: native start and replay drop")
                _assert_payload(kind, starts[0], packet, drops[0], document_url, image_url, overridden=overridden)
                inserted = _fragment_summary(await page.locator("#target").inner_html())
                expected = _fragment_summary(next(item["data"] for item in packet["items"]
                                                  if item["mimeType"] == "text/html"))
                assert_equal(inserted, expected, f"{kind}: the default editor inserts the rich HTML payload")
                assert_equal([event["effect"] for event in events if event["type"] == "dragend"],
                             ["copy"], f"{kind}: rich default insertion completes with copy")
            state.record(f"native_drag_{kind.replace('-', '_')}_{'author_override' if overridden else 'default'}_payload")


async def _wire_operation_masks(state: SmokeState) -> None:
    for allowed, expected in ((None, -1), ("uninitialized", -1), ("all", -1), ("copyMove", 17)):
        async with _session_page(state) as (page, session, received, ready):
            await page.set_content(_BOARD_STYLE + "<div id=source draggable=true>source</div><div id=target></div>")
            await page.evaluate(_INSTALL_LOG, {"allowed": allowed, "sourcePayload": True})
            await _start(page, session, ready)
            assert_equal(received[0]["dragOperationsMask"], expected,
                         f"effectAllowed={allowed}: exact native CDP operation mask")
            await _drag(session, "dragCancel", received[0])
        state.record(f"native_drag_wire_mask_{allowed or 'default'}")


_FRAME_BOARD = """<!doctype html><style>
  html,body{margin:0;height:700px}
  #parent{position:absolute;left:20px;top:20px;width:100px;height:70px}
  iframe{position:absolute;left:200px;top:100px;width:240px;height:180px;border:0;
    transform:scale(.5);transform-origin:0 0}
  #second{left:400px;top:200px;transform:scale(2)}
</style><div id=parent>parent</div><iframe id=first></iframe><iframe id=second></iframe>"""

_INSTALL_FRAMES = """() => {
  window.__nativeSemantics={events:[]};
  const add=(doc,name)=>{
    doc.documentElement.style.cssText='margin:0;padding:0';doc.body.style.cssText='margin:0;padding:0';
    for(const type of ['dragstart','drag','dragenter','dragover','dragleave','drop','dragend'])
      doc.addEventListener(type,event=>{
        if(type==='dragstart')event.dataTransfer.setData('text/plain','frame-source');
        if(type==='dragenter'||type==='dragover'||type==='drop')event.preventDefault();
        window.__nativeSemantics.events.push({document:name,type,target:event.target.id||event.target.nodeName,
          relatedTarget:event.relatedTarget?(event.relatedTarget.id||event.relatedTarget.nodeName):null,
          client:[event.clientX,event.clientY],screen:[event.screenX,event.screenY]});
      },true);
  };
  add(document,'parent');
  for(const name of ['first','second']) {
    const doc=document.getElementById(name).contentDocument;
    doc.body.innerHTML='<div id='+name+'-a style="position:absolute;left:0;top:0;width:100px;height:100px">A</div>'
      +'<div id='+name+'-b style="position:absolute;left:120px;top:0;width:100px;height:100px">B</div>';
    add(doc,name);
  }
}"""


def _frame_event(document: str, kind: str, target: str, related: str | None,
                 client: list[int], screen: list[int]) -> dict:
    return {"document": document, "type": kind, "target": target,
            "relatedTarget": related, "client": client, "screen": screen}


async def _cross_document_targets(state: SmokeState) -> None:
    async with _session_page(state) as (page, session, _, _):
        await page.set_content(_FRAME_BOARD)
        await page.evaluate(_INSTALL_FRAMES)
        await capture_layout(page)
        packet = {"items": [{"mimeType": "text/plain", "data": "frame-payload"}], "dragOperationsMask": 1}
        cases = [
            ("dragEnter", 40, 40, [_frame_event("parent", "dragenter", "parent", None, [40, 40], [40, 40])]),
            ("dragOver", 210, 115, [
                _frame_event("first", "dragenter", "first-a", None, [20, 30], [210, 115]),
                _frame_event("parent", "dragleave", "parent", "first", [210, 115], [210, 115]),
            ]),
            ("dragOver", 270, 115, [
                _frame_event("first", "dragenter", "first-b", "first-a", [140, 30], [270, 115]),
                _frame_event("first", "dragleave", "first-a", "first-b", [140, 30], [270, 115]),
            ]),
            ("dragOver", 420, 240, [
                _frame_event("second", "dragenter", "second-a", None, [10, 20], [420, 240]),
                _frame_event("first", "dragleave", "first-b", None, [440, 280], [420, 240]),
            ]),
            ("dragOver", 40, 40, [
                _frame_event("parent", "dragenter", "parent", "second", [40, 40], [40, 40]),
                _frame_event("second", "dragleave", "second-a", None, [-180, -80], [40, 40]),
            ]),
            ("drop", 210, 115, [
                _frame_event("first", "dragenter", "first-a", None, [20, 30], [210, 115]),
                _frame_event("parent", "dragleave", "parent", "first", [210, 115], [210, 115]),
                _frame_event("first", "drop", "first-a", None, [20, 30], [210, 115]),
            ]),
        ]
        for command, x, y, expected in cases:
            before = len(await _events(page))
            await _drag(session, command, packet, x, y)
            assert_equal((await _events(page))[before:], expected,
                         f"{command}@{x},{y}: target clients and related targets stay in their Documents")
    state.record("native_drag_cross_document_target_coordinates_and_related_target")


async def _cross_document_source_coordinates(state: SmokeState) -> None:
    async with _session_page(state) as (page, session, received, ready):
        await page.set_content(_FRAME_BOARD)
        await page.evaluate(_INSTALL_FRAMES)
        await page.evaluate("() => document.getElementById('first').contentDocument.getElementById('first-a').draggable=true")
        await capture_layout(page)
        await session.send("Input.setInterceptDrags", {"enabled": True})
        await _mouse(session, "mouseMoved", 210, 110, 0)
        await _mouse(session, "mousePressed", 210, 110, 1)
        await _mouse(session, "mouseMoved", 215, 115, 1)
        await asyncio.wait_for(ready.wait(), timeout=5)
        for command in ("dragEnter", "dragOver", "drop"):
            await _drag(session, command, received[0], 420, 240)
        source = [event for event in await _events(page) if event["type"] in ("dragstart", "drag", "dragend")]
        assert_equal([event["type"] for event in source], ["dragstart", "drag", "drag", "dragend"],
                     "cross-frame source lifecycle dispatches at the known update boundaries")
        assert_equal(source[0]["client"], [20, 20], "source dragstart retains the original press position")
        # Chromium 156 constructs these source events in the current target's
        # MouseEventManager; Moli deliberately keeps the source Document space.
        expected_client = [440, 280] if is_moli_browser() else [10, 20]
        for event in source[1:]:
            assert_equal(event["client"], expected_client,
                         f"{event['type']}: the selected backend's source-coordinate contract")
            assert_equal(event["screen"], [420, 240], f"{event['type']}: root screen position")
    state.record("native_drag_cross_document_source_coordinates", {
        "sourceClientSpace": "source-document" if is_moli_browser() else "current-target-manager",
    })


async def _source_event_cadence(state: SmokeState) -> None:
    for canceled in (False, True):
        async with _session_page(state) as (page, session, received, ready):
            await page.set_content(_BOARD_STYLE + "<div id=source draggable=true>source</div><div id=target></div><div id=other></div>")
            await page.evaluate(_INSTALL_LOG, {"sourcePayload": True, "cancelDrop": True,
                                             "cancelDrag": canceled, "trackSource": True})
            await _start(page, session, ready)
            packet = {**received[0], "items": [
                {"mimeType": "text/plain", "data": "target-replacement"},
                {"mimeType": "application/x-target", "data": "target-only"},
            ]}
            assert_equal([event["type"] for event in await _events(page)], ["dragstart"],
                         "intercepted source pauses before the first target update")
            for x in (60, 70):
                await _mouse(session, "mouseMoved", x, 40, 1)
            # smoke-timing: observe two conventional 350ms drag periods; CDP interception has no autonomous drag timer.
            await asyncio.sleep(0.7)
            assert_equal([event["type"] for event in await _events(page)], ["dragstart"],
                         "held mouse updates and elapsed time do not resume an intercepted source")
            steps = [
                ("dragEnter", 330, ["drag", "dragenter"]),
                ("dragOver", 330, ["dragover"]),
                ("dragOver", 330, ["drag", "dragover"]),
                ("dragOver", 580, ["drag", "dragenter", "dragleave"]),
                ("dragOver", 580, ["dragover"]),
            ]
            for command, x, expected in steps:
                before = len(await _events(page))
                await _drag(session, command, packet, x)
                assert_equal([event["type"] for event in (await _events(page))[before:]], expected,
                             f"{command}@{x}: source drag follows Blink's enter/next-over state")
            before = await _events(page)
            # smoke-timing: an entered target also remains idle while the CDP client sends no drag update.
            await asyncio.sleep(0.7)
            assert_equal(await _events(page), before, "an entered intercepted target has no periodic source drag")
            await _drag(session, "drop", packet, 580)
            events = await _events(page)
            assert_equal([event["type"] for event in events[len(before):]],
                         ["drag", "dragover", "drop", "dragend"],
                         "drop's final over runs the source update and completes once")
            drags = [event for event in events if event["type"] == "drag"]
            assert_equal([event["canceled"] for event in drags], [canceled] * len(drags),
                         "source drag cancellation is observable")
            assert_equal([event["effect"] for event in events if event["type"] == "dragend"], ["copy"],
                         "Chromium ignores source drag preventDefault and still completes the accepted drop")
            source_events = [event for event in events if event["type"] in ("dragstart", "drag", "dragend")]
            assert_equal([event["sameSource"] for event in source_events], [True] * len(source_events),
                         "dragstart, every source drag, and dragend retain one DataTransfer identity")
            assert_equal([event["initialEffect"] for event in drags], ["none"] + ["link"] * (len(drags) - 1),
                         "the first source drag's dropEffect mutation survives subsequent source drags")
            protected = {"types": ["text/plain"], "items": 1, "data": "", "effect": "link",
                         "savedItem": {"kind": "string", "type": "text/plain"},
                         "savedTypes": ["text/plain"], "distinctViews": 1}
            for event in events:
                if event["type"] in ("dragenter", "dragover", "dragleave", "drop"):
                    assert_equal(event["source"], protected,
                                 f"{event['type']}: saved source views keep protected original metadata")
            ended = await page.evaluate("() => window.__nativeSemantics.sourceState()")
            assert_equal(ended, {"types": [], "items": 0, "data": "", "effect": "copy",
                                 "savedItem": {"kind": "", "type": ""},
                                 "savedTypes": ["text/plain"], "distinctViews": 1},
                         "source completion disables every saved view and item while preserving the FrozenArray")
        state.record(f"native_drag_source_command_cadence_{'canceled' if canceled else 'normal'}")


async def _cancel_contracts(state: SmokeState) -> None:
    for method in ("dragCancel", "cancelDragging", "native-cancelDragging"):
        async with _session_page(state) as (page, session, received, ready):
            await page.set_content(_BOARD_STYLE + "<div id=source draggable=true>source</div><div id=target></div>")
            await page.evaluate(_INSTALL_LOG, {"sourcePayload": True, "cancelDrop": True})
            if method == "native-cancelDragging":
                await capture_layout(page)
                await session.send("Input.setInterceptDrags", {"enabled": False})
                await _mouse(session, "mouseMoved", 30, 28, 0)
                await _mouse(session, "mousePressed", 30, 28, 1)
                await _mouse(session, "mouseMoved", 45, 33, 1)
                await _mouse(session, "mouseMoved", 330, 40, 1)
            else:
                await _start(page, session, ready)
                await _drag(session, "dragEnter", received[0])
            before = len(await _events(page))
            if method == "dragCancel":
                await _drag(session, method, received[0])
            else:
                await session.send("Input.cancelDragging")
            changes = (await _events(page))[before:]
            # Moli intentionally closes intercepted sessions for both cancel
            # APIs. Chromium's explicit replay cancel ends only the source;
            # its cancelDragging command cannot reach an intercepted source.
            if method == "native-cancelDragging":
                expected = ["drag", "dragleave", "dragend"]
            elif is_moli_browser():
                expected = ["dragleave", "dragend"]
            else:
                expected = ["dragend"] if method == "dragCancel" else []
            assert_equal([event["type"] for event in changes], expected,
                         f"{method}: the selected backend's explicit cancellation contract")
            assert_equal([event["effect"] for event in changes if event["type"] == "dragend"],
                         ["none"] if expected else [], f"{method}: canceled source operation")
            assert_equal([event for event in changes if event["type"] == "drop"], [],
                         f"{method}: cancellation never dispatches a drop")
        state.record(f"native_drag_cancel_contract_{method}", {"backend": "moli" if is_moli_browser() else "chromium"})


async def run_native_drag_semantics(state: SmokeState) -> None:
    await _default_payloads(state)
    await _wire_operation_masks(state)
    await _cross_document_targets(state)
    await _cross_document_source_coordinates(state)
    await _source_event_cadence(state)
    await _cancel_contracts(state)
