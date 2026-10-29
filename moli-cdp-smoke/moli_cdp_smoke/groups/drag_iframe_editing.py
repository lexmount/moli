"""Editor selection and FileSystemEntry callbacks belong to their Window."""
from tempfile import TemporaryDirectory
from pathlib import Path

from ..assertions import assert_equal
from ..helpers import capture_layout
from . import SmokeState
from .native_drag_semantics import _session_page


async def run_drag_iframe_editing(state: SmokeState) -> None:
    async with _session_page(state) as (page, session, _, __):
        await page.goto(state.fixture)
        await page.set_content("<!doctype html><body style='margin:0'><p id=parent>parent</p>"
                               "<iframe id=frame name=editorFrame style='position:absolute;left:100px;top:100px;width:300px;height:300px;border:0'></iframe>")
        await page.evaluate("""() => {
          getSelection().selectAllChildren(document.getElementById('parent'));
          frame.srcdoc=`<!doctype html><body style='margin:0'>
          <div id=editor contenteditable style='height:80px'>😀abc</div>
          <div id=target contenteditable style='height:80px'></div>
          <div id=files style='height:80px'></div><script>
          window.__ready=true;window.__edits=[];window.__files=[];
          for(const type of ['beforeinput','input'])document.addEventListener(type,e=>__edits.push([
            e.target.id,e.inputType,e instanceof InputEvent,e.dataTransfer===null||e.dataTransfer instanceof DataTransfer]));
          const readFile=entry=>new Promise((resolve,reject)=>entry.file(file=>resolve([file instanceof File,file.name,file.size]),reject));
          const readDirectory=entry=>new Promise((resolve,reject)=>entry.createReader().readEntries(resolve,reject));
          async function describe(entry){
            const common=[entry instanceof Object,
              Object.getPrototypeOf(entry) instanceof Object,
              entry.filesystem instanceof Object,entry.fullPath];
            if(entry.isFile)return [...common,await readFile(entry)];
            const reader=entry.createReader();
            const children=await new Promise((resolve,reject)=>reader.readEntries(resolve,reject));
            return [...common,reader instanceof Object,children instanceof Array,
              await Promise.all(children.map(describe))];
          }
          files.ondragover=e=>e.preventDefault();files.ondrop=e=>{
            e.preventDefault();window.__fileResults=Promise.all(Array.from(e.dataTransfer.items,item=>describe(item.webkitGetAsEntry())));
          };
          </scr`+`ipt>`;
        }""")
        await page.wait_for_function("() => frame.contentWindow.__ready")
        child = next(frame for frame in page.frames if frame.name == "editorFrame")
        await child.evaluate("""() => {
          const range=document.createRange();range.setStart(editor.firstChild,2);range.setEnd(editor.firstChild,3);
          editor.focus();getSelection().removeAllRanges();getSelection().addRange(range);
        }""")
        await session.send("Input.insertText", {"text": "X"})
        assert_equal(await child.locator("#editor").text_content(), "😀Xbc", "iframe typing uses the child UTF-16 selection")
        assert_equal(await page.evaluate("() => getSelection().toString()"), "parent", "iframe typing preserves the parent's selection")
        await capture_layout(page)
        data = {"items": [{"mimeType": "text/plain", "data": "drop"}], "dragOperationsMask": 1}
        for kind in ("dragEnter", "dragOver", "drop"):
            await session.send("Input.dispatchDragEvent", {"type": kind, "x": 120, "y": 200, "data": data})
        assert_equal(await child.locator("#target").text_content(), "drop", "iframe default drop edits the receiving Document")
        assert_equal(await child.evaluate("() => __edits"), [
            ["editor", "insertText", True, True], ["editor", "insertText", True, True],
            ["target", "insertFromDrop", True, True], ["target", "insertFromDrop", True, True],
        ], "iframe editing events and independent DataTransfer snapshots use the child Realm")
        with TemporaryDirectory(prefix="moli-drag-entry-realms-") as temporary:
            root = Path(temporary)
            (root / "one.txt").write_text("one", encoding="utf-8")
            data = {"items": [], "files": [str(root / "one.txt")], "dragOperationsMask": 1}
            for kind in ("dragEnter", "dragOver", "drop"):
                await session.send("Input.dispatchDragEvent", {"type": kind, "x": 120, "y": 280, "data": data})
            result = await child.evaluate("() => __fileResults")
            assert_equal(result, [
                [True, True, True, "/one.txt", [True, "one.txt", 3]],
            ], "iframe file entry, filesystem and callback File share the receiving Realm")
    state.record("native_drag_iframe_editing_and_file_system_entry_realms")
