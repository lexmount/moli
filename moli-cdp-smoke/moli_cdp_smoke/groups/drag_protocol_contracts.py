"""Drag payload, event-view, and pointer contracts checked against Chromium."""

from __future__ import annotations

import asyncio
from contextlib import AsyncExitStack, asynccontextmanager
from pathlib import Path
from tempfile import TemporaryDirectory

from ..assertions import assert_equal
from ..diagnostics import new_cdp_session
from ..helpers import capture_layout
from . import SmokeState


_BOARD = """<!doctype html><style>
  html,body{margin:0;height:320px}
  #source,#a,#b{position:absolute;width:100px;height:80px}
  #source{left:20px;top:20px}#a{left:220px;top:20px}#b{left:420px;top:20px}
</style><div id=source draggable=true>source</div><div id=a>A</div><div id=b>B</div>"""

_INSTALL = """options => {
  const contract = window.__dragContract = {events:[], views:[], stringCallbacks:[]};
  const identities = new WeakMap();
  let nextIdentity = 1;
  const identity = value => {
    if (!identities.has(value)) identities.set(value, nextIdentity++);
    return identities.get(value);
  };
  const node = value => value ? (value.id || value.nodeName) : null;
  const oldView = view => ({
    type:view.type, data:view.dt.getData('text/plain'), types:Array.from(view.dt.types),
    itemCount:view.items.length, fileNames:Array.from(view.files,file=>file.name),
    savedTypes:Array.from(view.types), kind:view.item?.kind ?? null,
    fileKind:view.fileItem?.kind ?? null, file:view.fileItem?.getAsFile()?.name ?? null,
  });
  contract.sourceState = () => {
    const view=contract.sourceView;
    if(!view) return null;
    return {data:view.dt.getData('text/plain'),nativeData:view.dt.getData('application/x-native'),
      types:Array.from(view.dt.types),items:Array.from(view.items,item=>({kind:item.kind,type:item.type})),
      files:Array.from(view.dt.files,file=>file.name),savedTypes:Array.from(view.types),
      savedItem:{kind:view.item.kind,type:view.item.type},
      savedFile:view.fileItem ? {kind:view.fileItem.kind,type:view.fileItem.type,
        name:view.fileItem.getAsFile()?.name ?? null} : null};
  };
  for (const type of ['dragstart','drag','dragenter','dragover','dragleave','drop','dragend']) {
    document.addEventListener(type, event => {
      const dt=event.dataTransfer;
      const initialEffect=dt.dropEffect;
      if(type==='dragstart') {
        dt.setData('text/plain','native-secret');
        dt.setData('application/x-native','native-only');
        if(options.sourceFile) dt.items.add(new File(['native-file-payload'],'source-file.txt',{type:'text/plain'}));
        dt.effectAllowed='copyMove';
      }
      if(type==='dragover' && ['a','b'].includes(event.target.id)) {
        if(options.overEffect) dt.dropEffect=options.overEffect;
        event.preventDefault();
      }
      if(type==='drop') event.preventDefault();
      const view={type,dt,items:dt.items,types:dt.types,files:dt.files,item:dt.items[0],
        fileItem:Array.from(dt.items).find(item=>item.kind==='file')};
      if(type==='dragstart') contract.sourceView=view;
      const record={type,target:node(event.target),relatedTarget:node(event.relatedTarget),
        cancelable:event.cancelable,dt:identity(dt),data:dt.getData('text/plain'),
        types:Array.from(dt.types),allowed:dt.effectAllowed,initialEffect,effect:dt.dropEffect};
      if(contract.sourceView) record.source=contract.sourceState();
      if(type==='drop') {
        record.oldViews=contract.views.filter(view=>['dragenter','dragover'].includes(view.type)).map(oldView);
        contract.dropFiles=Array.from(dt.files);
        contract.dropView=view;
        if(options.strings) {
          for(const previous of contract.views) if(previous.item) previous.item.getAsString(data =>
            contract.stringCallbacks.push({type:'old:'+previous.type,data}));
          contract.dropString = new Promise(resolve => {
            view.item.getAsString(data => resolve({data,kind:view.item.kind,type:view.item.type}));
          });
        }
      }
      if(options.strings && type!=='drop' && view.item) view.item.getAsString(data =>
        contract.stringCallbacks.push({type,data}));
      contract.views.push(view);
      contract.events.push(record);
    });
  }
  for (const type of ['pointerdown','pointermove','pointerup','pointercancel',
                      'gotpointercapture','lostpointercapture','mousedown','mousemove','mouseup']) {
    document.addEventListener(type,event => {
      contract.events.push({type,target:node(event.target),buttons:event.buttons,cancelable:event.cancelable});
      if(options.capture && type==='pointerdown' && event.target.id==='source')
        event.target.setPointerCapture(event.pointerId);
    });
  }
}"""


def _data(value: str | None, mask: int = 1, mime: str = "text/plain") -> dict:
    return {
        "items": [] if value is None else [{"mimeType": mime, "data": value}],
        "dragOperationsMask": mask,
    }


async def _mouse(session, kind: str, x: int, y: int, buttons: int) -> None:
    await session.send("Input.dispatchMouseEvent", {
        "type": kind, "x": x, "y": y, "buttons": buttons,
        "button": "none" if kind == "mouseMoved" and not buttons else "left",
    })


async def _drag(session, kind: str, data: dict, x: int = 250) -> None:
    await session.send("Input.dispatchDragEvent", {"type": kind, "x": x, "y": 40, "data": data})


async def _events(page) -> list[dict]:
    return await page.evaluate("() => window.__dragContract.events")


async def _drag_step(page, session, kind: str, data: dict, x: int = 250) -> list[dict]:
    start = len(await _events(page))
    await _drag(session, kind, data, x)
    return [event for event in (await _events(page))[start:]
            if event["type"] in ("dragenter", "dragover", "dragleave", "drop")]


@asynccontextmanager
async def _board(state: SmokeState, *, native: bool = True, capture: bool = False,
                 strings: bool = False, source_file: bool = False,
                 over_effect: str | None = None, page=None):
    page = state.page if page is None else page
    await page.set_content(_BOARD)
    await page.evaluate(_INSTALL, {"capture": capture, "strings": strings,
                                  "sourceFile": source_file, "overEffect": over_effect})
    await capture_layout(page)
    session = await new_cdp_session(page.context, page)
    intercepted = []
    ready = asyncio.Event()

    def on_intercepted(event: dict) -> None:
        intercepted.append(event)
        ready.set()

    session.on("Input.dragIntercepted", on_intercepted)
    try:
        await session.send("Input.setInterceptDrags", {"enabled": native})
        if native:
            await _mouse(session, "mouseMoved", 40, 40, 0)
            await _mouse(session, "mousePressed", 40, 40, 1)
            await _mouse(session, "mouseMoved", 55, 50, 1)
            await asyncio.wait_for(ready.wait(), timeout=5)
        yield session, intercepted
    finally:
        # dragCancel really ends intercepted native drags in Chromium. Its
        # cancelDragging command can leave that particular source pending.
        await _drag(session, "dragCancel", _data(None))
        await _mouse(session, "mouseReleased", 250, 40, 0)
        await session.send("Input.setInterceptDrags", {"enabled": False})
        await session.detach()


async def _payload_contracts(state: SmokeState) -> None:
    page = state.page
    async with _board(state) as (session, intercepted):
        assert_equal(intercepted[0]["data"]["items"][0]["data"], "native-secret", "source interception payload")
        enter = await _drag_step(page, session, "dragEnter", _data(None))
        assert_equal([(event["type"], event["types"]) for event in enter],
                     [("dragenter", [])], "an explicitly empty enter replaces the native secret")
        drop = await _drag_step(page, session, "drop", _data("drop-replacement"))
        assert_equal([(event["type"], event["types"], event["data"]) for event in drop],
                     [("dragover", [], ""), ("drop", ["text/plain"], "drop-replacement")],
                     "drop pre-over keeps enter data; drop consumes its own payload")

    async with _board(state) as (session, _):
        await _drag_step(page, session, "dragEnter", _data("enter-secret"))
        over = await _drag_step(page, session, "dragOver", _data("ignored-over", 16, "application/x-over"))
        assert_equal([(event["type"], event["types"], event["allowed"]) for event in over],
                     [("dragover", ["text/plain"], "move")], "over updates mask without replacing enter items")
        drop = await _drag_step(page, session, "drop", _data("drop-replacement", 1))
        assert_equal([(event["type"], event["types"], event["allowed"], event["data"]) for event in drop],
                     [("dragover", ["text/plain"], "copy", ""),
                      ("drop", ["text/plain"], "copy", "drop-replacement")],
                     "drop updates the pre-over mask and replaces data only for drop")

    async with _board(state) as (session, _):
        await _drag_step(page, session, "dragEnter", _data("enter-secret"))
        drop = await _drag_step(page, session, "drop", _data(None))
        assert_equal([(event["type"], event["types"], event["data"]) for event in drop],
                     [("dragover", ["text/plain"], ""), ("drop", [], "")],
                     "an explicitly empty drop cannot reuse enter payload")
    state.record("drag_command_payload_and_operation_replacement")


async def _view_contracts(state: SmokeState) -> None:
    page = state.page
    source_types = ["text/plain", "application/x-native", "Files"]
    protected_source = {
        "data": "", "nativeData": "", "types": source_types,
        "items": [{"kind": "string", "type": "text/plain"},
                  {"kind": "string", "type": "application/x-native"},
                  {"kind": "file", "type": "text/plain"}],
        "files": [], "savedTypes": source_types,
        "savedItem": {"kind": "string", "type": "text/plain"},
        "savedFile": {"kind": "file", "type": "text/plain", "name": None},
    }
    async with _board(state, source_file=True) as (session, _):
        assert_equal(await page.evaluate("() => window.__dragContract.sourceState()"), protected_source,
                     "the retained source view exposes metadata but protects data and files after dragstart")
        packet = _data("target-data", 17)
        for kind in ("dragEnter", "dragOver", "drop"):
            await _drag_step(page, session, kind, packet)
        events = await _events(page)
        start = next(event for event in events if event["type"] == "dragstart")
        end = next(event for event in events if event["type"] == "dragend")
        target = [event for event in events if event["type"] in ("dragenter", "dragover", "drop")]
        assert_equal(start["dt"], end["dt"], "source dragend retains the source DataTransfer identity")
        assert_equal(end["effect"], "move", "accepted copyMove without explicit dropEffect prefers move")
        assert_equal(start["source"], {**protected_source, "data": "native-secret", "nativeData": "native-only",
                     "files": ["source-file.txt"],
                     "savedFile": {"kind": "file", "type": "text/plain", "name": "source-file.txt"}},
                     "the source is writable and its File is readable inside dragstart")
        assert_equal(len({event["dt"] for event in target}), len(target),
                     "distinct target updates use distinct DataTransfer views")
        assert_equal(start["dt"] in {event["dt"] for event in target}, False,
                     "target views do not borrow the source's writable object")
        for event in events:
            if "source" in event and event["type"] != "dragstart":
                assert_equal(event["source"], protected_source,
                             f"retained source metadata survives {event['type']} without exposing native data")
            if event["type"] in ("dragstart", "drag", "dragend"):
                assert_equal(event["cancelable"], event["type"] != "dragend",
                             f"source {event['type']} cancellation contract")
        assert_equal(await page.evaluate("() => window.__dragContract.sourceState()"),
                     {"data": "", "nativeData": "", "types": [], "items": [], "files": [],
                      "savedTypes": source_types, "savedItem": {"kind": "", "type": ""},
                      "savedFile": {"kind": "", "type": "", "name": None}},
                     "the source becomes disabled only after dragend dispatch finishes")

    with TemporaryDirectory(prefix="moli-drag-views-") as directory:
        file = Path(directory) / "payload.txt"
        file.write_text("retained-file-payload", encoding="utf-8")
        async with _board(state, native=False, strings=True) as (session, _):
            packet = {**_data("enter-data"), "files": [str(file)]}
            for kind in ("dragEnter", "dragOver"):
                await _drag_step(page, session, kind, packet)
            await _drag_step(page, session, "drop", {**packet, "items": _data("drop-data")["items"]})
            callback = await page.evaluate("async () => await window.__dragContract.dropString")
            assert_equal(callback, {"data": "drop-data", "kind": "", "type": ""},
                         "queued getAsString reads its captured value after the item becomes disabled")
            drop = next(event for event in await _events(page) if event["type"] == "drop")
            for view in drop["oldViews"]:
                assert_equal({key: view[key] for key in ("data", "types", "itemCount", "fileNames", "kind", "fileKind", "file")},
                             {"data": "", "types": [], "itemCount": 0, "fileNames": [],
                              "kind": "", "fileKind": "", "file": None},
                             "saved protected target views cannot regain access during drop")
                assert_equal(view["savedTypes"], ["text/plain", "Files"], "saved types array is a snapshot")
            result = await page.evaluate("""async () => {
              const contract=window.__dragContract;
              return {protectedCallbacks:contract.stringCallbacks,
                capturedFiles:await Promise.all(contract.dropFiles.map(async file=>({name:file.name,data:await file.text()}))),
                data:contract.dropView.dt.getData('text/plain'),types:Array.from(contract.dropView.dt.types),
                items:contract.dropView.dt.items.length,files:contract.dropView.dt.files.length};
            }""")
            assert_equal(result, {"protectedCallbacks": [],
                         "capturedFiles": [{"name": "payload.txt", "data": "retained-file-payload"}],
                         "data": "", "types": [], "items": 0, "files": 0},
                         "completed stores stay disabled while captured File objects remain readable")
    state.record("drag_data_transfer_view_access_and_async_callbacks")


async def _target_contracts(state: SmokeState) -> None:
    page = state.page
    async with _board(state, native=False) as (session, _):
        packet = _data("target-data", 17)
        steps = []
        for kind, x in (("dragEnter", 250), ("dragOver", 250), ("dragOver", 450),
                        ("dragOver", 250), ("drop", 250)):
            steps.append(await _drag_step(page, session, kind, packet, x))
        expected = [
            [("dragenter", "a", None)], [("dragover", "a", None)],
            [("dragenter", "b", "a"), ("dragleave", "a", "b")],
            [("dragenter", "a", "b"), ("dragleave", "b", "a")],
            [("dragover", "a", None), ("drop", "a", None)],
        ]
        assert_equal([[(event["type"], event["target"], event["relatedTarget"]) for event in step] for step in steps],
                     expected, "target changes enter new before leaving old, without an extra over")
        for step in steps[2:4]:
            assert_equal(step[0]["dt"], step[1]["dt"], "one target transition shares one event view")
        events = [event for step in steps for event in step]
        for event in events:
            assert_equal(event["cancelable"], event["type"] != "dragleave", f"{event['type']} cancellation contract")
            assert_equal(event["effect"], "none" if event["type"] == "dragleave" else "move",
                         f"copyMove initializes {event['type']} with its phase's operation")

    # Chrome 148 introduced source-operation initialization for enter/over;
    # drop receives the operation negotiated by the preceding accepted over.
    # See https://developer.chrome.com/release-notes/148.
    for mask, allowed, effect in ((0, "none", "none"), (1, "copy", "copy"),
                                  (2, "link", "link"), (16, "move", "move"),
                                  (3, "copyLink", "copy"), (17, "copyMove", "move"),
                                  (18, "linkMove", "move"), (19, "all", "copy"),
                                  (-1, "all", "copy")):
        async with _board(state, native=False) as (session, _):
            packet = _data("operations", mask)
            enter = await _drag_step(page, session, "dragEnter", packet)
            over = await _drag_step(page, session, "dragOver", packet)
            assert_equal([(event["type"], event["allowed"], event["initialEffect"]) for event in enter + over],
                         [("dragenter", allowed, effect), ("dragover", allowed, effect)],
                         f"mask {mask} initializes target operations from effectAllowed")

    for operation in ("copy", "link", "move"):
        async with _board(state, over_effect=operation) as (session, _):
            packet = _data("negotiated", 19)
            await _drag_step(page, session, "dragEnter", packet)
            over = await _drag_step(page, session, "dragOver", packet)
            assert_equal([(event["initialEffect"], event["effect"]) for event in over],
                         [("copy", operation)], f"dragover can negotiate {operation} from all operations")
            drop = await _drag_step(page, session, "drop", packet)
            assert_equal([(event["type"], event["initialEffect"], event["effect"]) for event in drop],
                         [("dragover", "copy", operation), ("drop", operation, operation)],
                         f"drop preserves the accepted {operation} operation")
            assert_equal([event["effect"] for event in await _events(page) if event["type"] == "dragend"],
                         [operation], f"a handled {operation} drop reports its negotiated source result")
    state.record("drag_target_transition_order_related_target_and_views")


async def _pointer_contracts(state: SmokeState) -> None:
    fixture = (Path(__file__).parents[1] / "fixtures/drag-pointer-suppression.html").read_text(encoding="utf-8")
    for capture in (False, True):
        for resume_same_target in (True, False):
            await _pointer_contract_case(state, fixture, capture, resume_same_target)
    state.record("native_drag_pointer_cancel_capture_and_stream_recovery")


async def _pointer_contract_case(state: SmokeState, fixture: str,
                                 capture: bool, resume_same_target: bool) -> None:
    label = f"capture={capture}, resume={'same' if resume_same_target else 'different'}"
    async with AsyncExitStack() as cleanup:
        page = await state.page.context.new_page()
        cleanup.push_async_callback(page.close)
        await page.set_content(fixture)
        await page.evaluate("options => window.installPointerSuppression(options)", {"capture": capture})
        await page.evaluate("""() => {
          const state=window.__pointerSuppression;
          const name=node=>node ? node.id || node.nodeName : null;
          for(const type of ['pointerover','pointerenter','mouseover','mouseenter','mouseout','mouseleave'])
            document.addEventListener(type,event=>state.events.push({
              type,target:name(event.target),related:name(event.relatedTarget),pointerId:event.pointerId ?? null,
              buttons:event.buttons,bubbles:event.bubbles,cancelable:event.cancelable,
              trusted:event.isTrusted,dragging:state.started,
            }),true);
          state.endEffects=[];
          document.addEventListener('dragend',event=>state.endEffects.push(event.dataTransfer.dropEffect));
        }""")
        await capture_layout(page)
        session = await new_cdp_session(page.context, page)
        cleanup.push_async_callback(session.detach)
        intercepted = []
        ready = asyncio.Event()

        def on_intercepted(event: dict) -> None:
            intercepted.append(event)
            ready.set()

        session.on("Input.dragIntercepted", on_intercepted)
        await session.send("Input.setInterceptDrags", {"enabled": True})
        try:
            await _mouse(session, "mouseMoved", 40, 40, 0)
            await _mouse(session, "mousePressed", 40, 40, 1)
            await _mouse(session, "mouseMoved", 55, 55, 1)
            await asyncio.wait_for(ready.wait(), timeout=5)
            await page.wait_for_function(
                "() => window.__pointerSuppression.events.some(event => event.dragging && "
                "event.type === 'pointerleave' && event.target === 'HTML')",
                timeout=5_000,
            )
            initial = await page.evaluate("() => window.__pointerSuppression.events")
            target = "source" if capture else "leaf"
            leave_chain = ["source", "outer", "BODY", "HTML"]
            if not capture:
                leave_chain.insert(0, "leaf")
            # Assert the complete element ancestor chain. Document event targets
            # are outside this element-boundary contract.
            suppression = [event for event in initial if event["dragging"]
                           and event["target"] != "#document" and event["type"] != "dragstart"]
            expected = [("pointercancel", target)]
            if capture:
                expected.append(("lostpointercapture", target))
            expected.append(("pointerout", target))
            expected.extend(("pointerleave", ancestor) for ancestor in leave_chain)
            assert_equal([(event["type"], event["target"]) for event in suppression], expected,
                         f"{label}: native startup suppresses the pointer before any source dragleave")
            pointer_id = next(event["pointerId"] for event in initial if event["type"] == "pointerdown")
            assert_equal([(event["buttons"], event["bubbles"], event["cancelable"],
                           event["trusted"], event["related"], event["pointerId"]) for event in suppression],
                         [(0, kind != "pointerleave", kind == "pointerout", True, None, pointer_id)
                          for kind, _ in expected],
                         f"{label}: suppressed events retain pointer identity and native flags")
            assert_equal(any(event["type"] == "gotpointercapture" for event in initial), capture,
                         f"{label}: capture was acquired only in the capture scenario")

            # Replay the native packet at the original hit, then leave the
            # source. The fixture checks suppression was already observable.
            packet = intercepted[0]["data"]
            await session.send("Input.dispatchDragEvent", {
                "type": "dragEnter", "x": 55, "y": 55, "data": packet,
            })
            before = await page.evaluate("() => window.__pointerSuppression.events.length")
            await _mouse(session, "mouseMoved", 60, 50, 1)
            await _mouse(session, "mouseMoved", 250, 40, 1)
            active = await page.evaluate("start => window.__pointerSuppression.events.slice(start)", before)
            assert_equal([event["type"] for event in active
                          if event["type"].startswith(("pointer", "mouse"))], [],
                         f"{label}: an active native drag suppresses pointer and mouse input")
            await _drag(session, "dragOver", packet)
            assert_equal(await page.evaluate("() => window.__pointerSuppression.dragLeaveSawSuppression"), True,
                         f"{label}: source dragleave observes startup pointer suppression")
            await _drag(session, "dragCancel", packet)
            ended = await page.evaluate("() => window.__pointerSuppression")
            assert_equal(ended["endEffects"], ["none"], f"{label}: cancellation ends the source exactly once")
            assert_equal([(event["bubbles"], event["cancelable"]) for event in ended["events"]
                          if event["type"] == "dragend"], [(True, False)],
                         f"{label}: dragend is bubbling and noncancelable")

            x, y = (60, 50) if resume_same_target else (250, 40)
            resumed_target = "leaf" if resume_same_target else "target"
            before = len(ended["events"])
            await _mouse(session, "mouseMoved", x, y, 1)
            await _mouse(session, "mouseReleased", x, y, 0)
            resumed = await page.evaluate("start => window.__pointerSuppression.events.slice(start)", before)
            enter_chain = (["HTML", "BODY", "outer", "source", "leaf"] if resume_same_target
                           else ["HTML", "BODY", "target"])
            assert_equal([(event["type"], event["target"], event["related"]) for event in resumed
                          if event["type"] in ("pointerover", "pointerenter", "pointerout", "pointerleave")
                          and event["target"] != "#document"],
                         [("pointerover", resumed_target, None)]
                         + [("pointerenter", ancestor, None) for ancestor in enter_chain],
                         f"{label}: the recovered pointer enters from outside without repeating old leaves")
            if resume_same_target:
                mouse_boundaries = ([("mouseout", "source", "leaf"), ("mouseover", "leaf", "source"),
                                     ("mouseenter", "leaf", "source")] if capture else [])
            else:
                mouse_boundaries = [("mouseout", target, "target"), ("mouseleave", target, "target"),
                                    ("mouseover", "target", target), ("mouseenter", "target", target)]
            # Legacy mouse ancestor dispatch is a separate hover contract.
            # Here retain exact old/new target boundaries and reject duplicates;
            # the pointer contract above still checks every element ancestor.
            assert_equal([(event["type"], event["target"], event["related"]) for event in resumed
                          if event["type"] in ("mouseover", "mouseenter", "mouseout", "mouseleave")
                          and event["target"] in (target, resumed_target)],
                         mouse_boundaries, f"{label}: suppression preserves the separate legacy mouse hover")
            assert_equal([event["type"] for event in resumed
                          if event["type"] in ("pointermove", "mousemove", "pointerup", "mouseup")],
                         ["pointermove", "mousemove", "pointerup", "mouseup"],
                         f"{label}: completed cancellation restores held movement and release")
            assert_equal(len(intercepted), 1, f"{label}: held movement cannot start another native drag")
        finally:
            await _drag(session, "dragCancel", _data(None))
            await _mouse(session, "mouseReleased", 250, 40, 0)
            await session.send("Input.setInterceptDrags", {"enabled": False})


async def _intrinsic_constructor_contracts(state: SmokeState) -> None:
    for mode in ("replace", "delete", "getter"):
        page = await state.page.context.new_page()
        session = await new_cdp_session(page.context, page)
        intercepted = []
        ready = asyncio.Event()

        def on_intercepted(event: dict) -> None:
            intercepted.append(event)
            ready.set()

        session.on("Input.dragIntercepted", on_intercepted)
        try:
            await page.set_content(_BOARD)
            await page.evaluate("""mode => {
              const IntrinsicDragEvent=DragEvent;
              window.__events=[];window.__constructorReads=0;
              for(const type of ['dragstart','dragend'])document.addEventListener(type,event=>{
                __events.push([type,event instanceof IntrinsicDragEvent,event.isTrusted]);
                if(type==='dragstart')event.dataTransfer.setData('text/plain','payload');
              });
              if(mode==='replace')window.DragEvent=function(){
                __constructorReads++;throw Error('author constructor');
              };
              if(mode==='delete')delete window.DragEvent;
              if(mode==='getter')Object.defineProperty(window,'DragEvent',{configurable:true,get(){
                __constructorReads++;throw Error('author getter');
              }});
            }""", mode)
            await capture_layout(page)
            await session.send("Input.setInterceptDrags", {"enabled": True})
            await _mouse(session, "mouseMoved", 40, 40, 0)
            await _mouse(session, "mousePressed", 40, 40, 1)
            await _mouse(session, "mouseMoved", 55, 50, 1)
            await asyncio.wait_for(ready.wait(), timeout=5)
            await _drag(session, "dragCancel", _data(None))
            assert_equal(len(intercepted), 1, f"{mode}: native drag is intercepted once")
            assert_equal(intercepted[0]["data"]["items"][0]["data"], "payload",
                         f"{mode}: the native dragstart can populate its transfer")
            assert_equal(await page.evaluate("() => [__constructorReads,__events]"),
                         [0, [["dragstart", True, True], ["dragend", True, True]]],
                         f"{mode}: native events use intrinsic constructors without reading public properties")
        finally:
            await _drag(session, "dragCancel", _data(None))
            await _mouse(session, "mouseReleased", 55, 50, 0)
            await session.detach()
            await page.close()
        state.record(f"drag_intrinsic_constructor_{mode}")


_REALM_FRAME = """<!doctype html><style>
  body{margin:0}#source,#target{position:absolute;left:0;width:100px;height:80px}
  #source{top:0}#target{top:100px}
</style><div id=source draggable=true>source</div><div id=target>target</div><script>
  window.__events=[];window.__views=[];window.__constructorReads=0;
  const IntrinsicDragEvent=DragEvent;
  Object.defineProperty(window,'DragEvent',{configurable:true,get(){
    __constructorReads++;throw Error('child public constructor');
  }});
  for(const type of ['dragstart','drag','dragenter','dragover','dragleave','drop','dragend']){
    document.addEventListener(type,event=>{
      const dt=event.dataTransfer;
      if(type==='dragstart'){dt.setData('text/plain','child-payload');dt.effectAllowed='copy';}
      if(type==='dragover'||type==='drop')event.preventDefault();
      const file=dt.files[0];
      __events.push({type,event:event instanceof IntrinsicDragEvent,
        transfer:dt instanceof DataTransfer,items:dt.items instanceof DataTransferItemList,
        item:dt.items[0] instanceof DataTransferItem,files:dt.files instanceof FileList,
        types:dt.types instanceof Array&&Object.isFrozen(dt.types),view:event.view===window,
        stable:dt===event.dataTransfer,
        text:dt.getData('text/plain'),file:file?[file instanceof File,file.name,file.size,dt.items[1].getAsFile() instanceof File]:null});
      if(type==='drop')window.__retainedFile=file;
      __views.push(dt);
    });
  }
</script>"""


async def _iframe_realm_contracts(state: SmokeState) -> None:
    for native in (False, True):
        page = await state.page.context.new_page()
        session = await new_cdp_session(page.context, page)
        label = "native" if native else "replay"
        try:
            await page.set_content("""<!doctype html><body style='margin:0'>
              <iframe id=realmFrame name=realmFrame
                style='position:absolute;left:100px;top:120px;width:300px;height:300px;border:0'></iframe>""")
            await page.evaluate("""html => {
              window.__constructorReads=0;
              window.__IntrinsicDragEvent=DragEvent;
              Object.defineProperty(window,'DragEvent',{configurable:true,get(){
                __constructorReads++;throw Error('parent public constructor');
              }});
              document.getElementById('realmFrame').srcdoc=html;
            }""", _REALM_FRAME)
            await page.wait_for_function(
                "() => Array.isArray(document.getElementById('realmFrame').contentWindow.__events)")
            child = next(frame for frame in page.frames if frame.name == "realmFrame")
            await capture_layout(page)
            if native:
                for kind, y, buttons in (("mouseMoved", 140, 0), ("mousePressed", 140, 1),
                                         ("mouseMoved", 240, 1), ("mouseMoved", 245, 1),
                                         ("mouseReleased", 245, 0)):
                    await _mouse(session, kind, 120, y, buttons)
            else:
                with TemporaryDirectory(prefix="moli-drag-realm-") as directory:
                    file = Path(directory) / "payload.txt"
                    file.write_bytes(b"file-payload")
                    packet = {**_data("external-payload"), "files": [str(file)]}
                    for kind in ("dragEnter", "dragOver", "drop"):
                        await session.send("Input.dispatchDragEvent", {
                            "type": kind, "x": 120, "y": 245, "data": packet,
                        })
                    retained_text = await child.evaluate("async () => await __retainedFile.text()")
            result = await child.evaluate("""() => ({events:__events,reads:__constructorReads,
              closed:__views.every(dt=>dt.types.length===0&&dt.items.length===0&&
                dt.files.length===0&&dt.getData('text/plain')==='')})""")
            assert_equal(bool(result["events"]), True, f"{label}: the child's own script receives drag events")
            for event in result["events"]:
                for field in ("event", "transfer", "items", "item", "files", "types", "view", "stable"):
                    assert_equal(event[field], True, f"{label} {event['type']}: {field} belongs to the child realm")
            drop = [event for event in result["events"] if event["type"] == "drop"]
            assert_equal(len(drop), 1, f"{label}: the child receives one drop")
            assert_equal(drop[0]["text"], "child-payload" if native else "external-payload",
                         f"{label}: realm-local views retain the shared store's payload")
            if native:
                assert_equal([result["events"][0]["type"], result["events"][-1]["type"]],
                             ["dragstart", "dragend"], "source events also belong to the child's Window")
            else:
                assert_equal(drop[0]["file"], [True, "payload.txt", 12, True],
                             "FileList and getAsFile return Files in the child realm with the original metadata")
                assert_equal(retained_text, "file-payload",
                             "a retained File remains readable after its DataTransfer view closes")
            assert_equal(result["closed"], True, f"{label}: finishing closes all views across realms")
            assert_equal([result["reads"], await page.evaluate("() => __constructorReads")], [0, 0],
                         f"{label}: neither Window's public DragEvent constructor is consulted")
            if not native:
                await page.evaluate("""() => {
                  window.__cross=[];let next=0;const ids=new WeakMap();
                  window.__dragIdentity=dt=>{if(!ids.has(dt))ids.set(dt,++next);return ids.get(dt);};
                  const outer=document.createElement('div');outer.id='outer';
                  outer.style='position:absolute;left:500px;top:100px;width:200px;height:200px';
                  document.body.append(outer);
                  for(const type of ['dragenter','dragover','dragleave','drop']){
                    document.addEventListener(type,event=>{
                      const dt=event.dataTransfer;
                      __cross.push([type,__dragIdentity(dt),event instanceof __IntrinsicDragEvent,
                        dt instanceof DataTransfer,dt.items instanceof DataTransferItemList,event.view===window]);
                      if(type==='dragover'||type==='drop')event.preventDefault();
                    });
                  }
                }""")
                await child.evaluate("""() => {
                  window.__cross=[];
                  for(const type of ['dragenter','dragover','dragleave','drop']){
                    document.addEventListener(type,event=>{
                      const dt=event.dataTransfer;
                      __cross.push([type,parent.__dragIdentity(dt),event instanceof IntrinsicDragEvent,
                        dt instanceof DataTransfer,dt.items instanceof DataTransferItemList,event.view===window]);
                    });
                  }
                }""")
                await capture_layout(page)
                for kind, x, y in (("dragEnter", 120, 240), ("dragOver", 520, 150),
                                   ("dragOver", 120, 245), ("drop", 120, 245)):
                    await session.send("Input.dispatchDragEvent", {
                        "type": kind, "x": x, "y": y, "data": _data("cross-payload"),
                    })
                assert_equal(await page.evaluate("() => __cross"),
                             [["dragenter", 2, True, True, True, True],
                              ["dragleave", 3, True, False, False, True]],
                             "cross-document enter/leave share a view belonging to the new Window")
                assert_equal(await child.evaluate("() => __cross"),
                             [["dragenter", 1, True, True, True, True],
                              ["dragleave", 2, True, False, False, True],
                              ["dragenter", 3, True, True, True, True],
                              ["dragover", 4, True, True, True, True],
                              ["drop", 5, True, True, True, True]],
                             "each Event keeps its receiving Window while the transition view stays shared")
                state.record("drag_cross_document_transition_realms")
        finally:
            await _drag(session, "dragCancel", _data(None))
            await _mouse(session, "mouseReleased", 120, 245, 0)
            await session.detach()
            await page.close()
        state.record(f"drag_iframe_realm_{label}")


async def run_drag_protocol_contracts(state: SmokeState) -> None:
    await _payload_contracts(state)
    await _view_contracts(state)
    await _target_contracts(state)
    await _pointer_contracts(state)
    await _intrinsic_constructor_contracts(state)
    await _iframe_realm_contracts(state)
