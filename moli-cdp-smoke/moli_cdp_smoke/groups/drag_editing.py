"""Native selection moves use a destination caret independent of Selection."""
import asyncio

from ..assertions import assert_equal
from ..helpers import capture_layout
from . import SmokeState
from .native_drag_semantics import _mouse, _session_page


async def run_drag_editing(state: SmokeState) -> None:
    cases = (
        ("move", 19, "ABEFCD"), ("copy-mask-only", 1, "ABCDEF"),
        ("copy-key", 19, "ABCDEF CD"), ("start", 19, "CDABEF"),
        ("inside", 19, "ABCDEF"), ("cancel-delete", 19, "ABCDEFCD"),
        ("cancel-insert", 19, "ABEF"), ("unicode", 19, "😀ABEFCD"),
        ("rich", 19, "😀DABC"), ("disable-delete", 19, "ABCDEF"),
        ("remove-delete", 19, "ABCDEF"), ("remove-insert", 19, "ABEF"),
    )
    for case, mask, expected in cases:
        async with _session_page(state) as (page, session, received, ready):
            markup = "<b>😀AB</b><em>CD</em>" if case == "rich" else (
                "😀ABCDEF" if case == "unicode" else "ABCDEF")
            await page.set_content("<!doctype html><body style='margin:0'>"
                                   "<div id=editor contenteditable style='position:absolute;left:20px;top:20px;"
                                   "width:400px;height:100px;font:20px/30px monospace'>" + markup + "</div>")
            points = await page.evaluate("""kind => {
              const editor=document.getElementById('editor'),text=editor.firstChild;window.__editor=editor;
              const range=document.createRange();
              if(kind==='rich'){range.setStart(text.firstChild,2);range.setEnd(editor.lastChild.firstChild,1);}
              else {const offset=kind==='unicode'?2:0;range.setStart(text,offset+2);range.setEnd(text,offset+4);}
              editor.focus();getSelection().removeAllRanges();getSelection().addRange(range);
              const source=range.getBoundingClientRect(),start=document.createRange(),end=document.createRange();
              const first=kind==='rich'?text.firstChild:text,last=kind==='rich'?editor.lastChild.firstChild:text;
              start.setStart(first,0);start.setEnd(first,kind==='unicode'||kind==='rich'?2:1);
              end.setStart(last,last.length-1);end.setEnd(last,last.length);
              window.__editEvents=[];window.__endEffects=[];
              editor.addEventListener('beforeinput',event=>{
                __editEvents.push([event.type,event.inputType]);
                if(kind==='disable-delete'&&event.inputType==='deleteByDrag')editor.contentEditable='false';
                if((kind==='remove-delete'&&event.inputType==='deleteByDrag')||
                   (kind==='remove-insert'&&event.inputType==='insertFromDrop'))editor.remove();
                if((kind==='cancel-delete'&&event.inputType==='deleteByDrag')||
                   (kind==='cancel-insert'&&event.inputType==='insertFromDrop'))event.preventDefault();
              });
              editor.addEventListener('input',event=>__editEvents.push([event.type,event.inputType]));
              editor.addEventListener('dragend',event=>__endEffects.push(event.dataTransfer.dropEffect));
              return {x:source.left+2,y:source.top+2,start:start.getBoundingClientRect().left+1,
                end:end.getBoundingClientRect().right+2,copyKey:/Mac/.test(navigator.platform)?1:2};
            }""", case)
            await capture_layout(page)
            await session.send("Input.setInterceptDrags", {"enabled": True})
            await _mouse(session, "mouseMoved", points["x"], points["y"], 0)
            await _mouse(session, "mousePressed", points["x"], points["y"], 1)
            # smoke-timing: intentional selection drag after macOS kTextDragDelay.
            await asyncio.sleep(0.25)
            await _mouse(session, "mouseMoved", points["x"] + 15, points["y"], 1)
            await asyncio.wait_for(ready.wait(), timeout=5)
            data = {**received[0], "dragOperationsMask": mask}
            x = points["start"] if case == "start" else points["x"] if case == "inside" else points["end"]
            for kind in ("dragEnter", "dragOver", "drop"):
                await session.send("Input.dispatchDragEvent", {
                    "type": kind, "x": x, "y": points["y"], "data": data,
                    "modifiers": points["copyKey"] if case == "copy-key" else 0,
                })
            actual = await page.evaluate("""() => ({text:__editor.textContent,events:__editEvents,ends:__endEffects,
              bold:Array.from(__editor.querySelectorAll('b'),node=>node.textContent),
              italic:Array.from(__editor.querySelectorAll('em'),node=>node.textContent)})""")
            assert_equal(actual["text"], expected, f"{case}: edit at drop caret, independently of source Selection")
            events = []
            if case not in ("copy-mask-only", "copy-key", "inside"):
                events.append(["beforeinput", "deleteByDrag"])
                if case not in ("cancel-delete", "disable-delete", "remove-delete"):
                    events.append(["input", "deleteByDrag"])
            if case not in ("inside", "copy-mask-only", "remove-delete"):
                events.append(["beforeinput", "insertFromDrop"])
                if case not in ("cancel-insert", "disable-delete", "remove-insert"):
                    events.append(["input", "insertFromDrop"])
            assert_equal(actual["events"], events, f"{case}: deletion and insertion honor independent beforeinput cancellation")
            effect = "none" if case in ("inside", "copy-mask-only") else "copy" if case == "copy-key" else "move"
            assert_equal(actual["ends"], [effect], f"{case}: an accepted editing drop retains its negotiated operation")
            if case == "rich":
                assert_equal(actual["bold"], ["😀", "AB"], "moving rich selection preserves bold fragments")
                assert_equal(actual["italic"], ["D", "C"], "moving rich selection preserves italic fragments")
    state.record("native_drag_editing_move_copy_caret_and_cancellation")
    await _run_canceled_editing_drops(state)


async def _run_canceled_editing_drops(state: SmokeState) -> None:
    for action in ("cancel", "disable", "adopt"):
        async with _session_page(state) as (page, session, received, ready):
            await page.set_content("<!doctype html><body style='margin:0'>"
                                   "<div id=source draggable=true style='position:absolute;left:20px;top:20px;width:100px;height:100px'>source</div>"
                                   "<div id=editor contenteditable style='position:absolute;left:20px;top:200px;width:400px;height:100px'>base</div>")
            await page.evaluate("""action => {
              const editor=document.getElementById('editor');window.__editor=editor;
              window.__ends=[];window.__inputs=[];
              source.ondragstart=e=>{e.dataTransfer.effectAllowed='copy';e.dataTransfer.setData('text/plain','CD');};
              source.ondragend=e=>__ends.push(e.dataTransfer.dropEffect);
              editor.onbeforeinput=e=>{
                __inputs.push([e.type,e.inputType]);
                if(action==='cancel')e.preventDefault();
                if(action==='disable')editor.contentEditable='false';
                if(action==='adopt')document.implementation.createHTMLDocument('other').body.append(editor);
              };
              editor.oninput=e=>__inputs.push([e.type,e.inputType]);
            }""", action)
            await capture_layout(page)
            await session.send("Input.setInterceptDrags", {"enabled": True})
            for kind, x, buttons in (("mouseMoved", 30, 0), ("mousePressed", 30, 1), ("mouseMoved", 50, 1)):
                await _mouse(session, kind, x, 30, buttons)
            await asyncio.wait_for(ready.wait(), timeout=5)
            for kind in ("dragEnter", "dragOver", "drop"):
                await session.send("Input.dispatchDragEvent", {"type": kind, "x": 320, "y": 230, "data": received[0]})
            actual = await page.evaluate("() => [__ends,__inputs,__editor.textContent]")
            assert_equal(actual, [["copy"], [["beforeinput", "insertFromDrop"]], "base"],
                         f"{action}: consuming a drop can cancel its edit without rejecting the accepted operation")
    state.record("native_drag_canceled_editor_drops_preserve_negotiated_operation")
