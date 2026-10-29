"""A loaded image's File contains its original encoded response."""
import base64
from contextlib import AsyncExitStack

from ..assertions import assert_equal
from ..fixture import TRANSPARENT_PNG
from ..helpers import capture_layout
from ..diagnostics import new_cdp_session
from . import SmokeState
from .native_drag_semantics import _mouse


async def run_drag_image_files(state: SmokeState) -> None:
    for kind in ("image", "linked-image", "srcset", "cached", "cross-origin", "data"):
        async with AsyncExitStack() as cleanup:
            page=await state.context.new_page()
            cleanup.push_async_callback(page.close)
            session=await new_cdp_session(state.context,page)
            cleanup.push_async_callback(session.detach)
            await page.goto(state.fixture)
            image = state.fixture + "/chromium-resource-image.png"
            if kind == "data":
                image = "data:image/png;base64," + base64.b64encode(TRANSPARENT_PNG).decode("ascii")
            if kind == "cross-origin":
                image = image.replace("127.0.0.1", "localhost")
            await page.set_content("<!doctype html><body style='margin:0'>"
                                   "<style>#source{position:absolute;left:20px;top:20px;width:100px;height:80px}"
                                   "#target{position:absolute;left:300px;top:20px;width:100px;height:100px}</style>"
                                   "<div id=target></div>")
            await page.evaluate("""async options => {
              if(options.kind==='cached'){
                const cached=new Image();cached.src=options.url;document.body.append(cached);await cached.decode();
              }
              const image=new Image();image.id='source';
              if(options.kind==='srcset'){image.src='/unselected.png';image.srcset=options.url+' 1x';}
              else image.src=options.url;
              if(options.kind==='linked-image'){
                const link=document.createElement('a');link.href='/details';link.append(image);document.body.append(link);
              }else document.body.append(image);
              await image.decode();window.__imageFiles=[];
              const snapshot=async transfer=>{
                const file=transfer.files[0];
                return file?{file:file instanceof File,name:file.name,type:file.type,size:file.size,
                  items:Array.from(transfer.items,item=>[item.kind,item.type]),
                  bytes:btoa(String.fromCharCode(...new Uint8Array(await file.arrayBuffer())))}:null;
              };
              image.addEventListener('dragstart',event=>__imageFiles.push(snapshot(event.dataTransfer)));
              target.ondragover=event=>event.preventDefault();
              target.ondrop=event=>{event.preventDefault();__imageFiles.push(snapshot(event.dataTransfer));};
            }""", {"kind": kind, "url": image})
            await capture_layout(page)
            for event, x, buttons in (("mouseMoved", 30, 0), ("mousePressed", 30, 1),
                                      ("mouseMoved", 50, 1), ("mouseMoved", 320, 1),
                                      ("mouseMoved", 325, 1), ("mouseReleased", 325, 0)):
                await _mouse(session, event, x, 40, buttons)
            files = await page.evaluate("() => Promise.all(__imageFiles)")
            expected = None if kind == "cross-origin" else {
                "file": True, "name": image.rsplit("/", 1)[-1] if kind == "data" else "chromium-resource-image.png", "type": "image/png",
                "items": [["string", "text/uri-list"], ["file", "image/png"], ["string", "text/html"]],
                "size": len(TRANSPARENT_PNG), "bytes": base64.b64encode(TRANSPARENT_PNG).decode("ascii"),
            }
            dropped = None if expected is None else {
                **expected, "items": [["string", "text/uri-list"], ["string", "text/html"], ["file", "image/png"]],
            }
            assert_equal(files, [expected, dropped], f"{kind}: original image bytes, item order and file metadata survive native drop")
    state.record("native_drag_image_files_preserve_encoding_and_origin_access")
