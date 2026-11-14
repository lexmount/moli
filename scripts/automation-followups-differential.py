#!/usr/bin/env python3
"""Cross-check automation follow-ups against Chromium or a Moli binary.

Run with the moli-cdp-smoke virtualenv, e.g.:
  moli-cdp-smoke/.venv/bin/python scripts/automation-followups-differential.py chromium reference
  moli-cdp-smoke/.venv/bin/python scripts/automation-followups-differential.py target/debug/moli candidate

Each run checks explicit expectations and exits nonzero on any failure.
Touch geometry mutations distinguish Chromium's live layout from Moli's
frozen snapshot contract; other cases share expectations. Reports include
unnormalized events and server-observed requests.
"""

import argparse
import asyncio
import base64
import json
import os
import socket
import threading
import time
import traceback
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

from playwright.async_api import async_playwright

OUT = Path("target/automation-followups")
CHROMIUM = "/usr/bin/chromium"
REDIRECT_BODY_RELEASE = threading.Event()
REDIRECT_HEADERS_SENT = threading.Event()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_OPTIONS(self):
        self.reply(200, b"")

    def reply(self, status, body, extra=None):
        self.send_response(status)
        for k, v in {
            "content-type": "text/html",
            "access-control-allow-origin": "*",
            "access-control-allow-methods": "*",
            "access-control-allow-headers": "*",
            **(extra or {}),
        }.items():
            self.send_header(k, v)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.handle_request()

    def do_POST(self):
        self.handle_request()

    def do_PUT(self):
        self.handle_request()

    def handle_request(self):
        body = self.rfile.read(int(self.headers.get("content-length", "0"))).decode(
            errors="replace"
        )
        path = urlsplit(self.path).path
        if path == "/held-start":
            self.send_response(302)
            self.send_header("Location", "/held-final")
            self.send_header("Content-Length", "1")
            self.end_headers()
            self.wfile.flush()
            REDIRECT_HEADERS_SENT.set()
            REDIRECT_BODY_RELEASE.wait(10)
            try:
                self.wfile.write(b"x")
            except (BrokenPipeError, ConnectionResetError):
                pass
            return
        if path == "/held-final":
            return self.reply(200, b"final")
        if path == "/auth-start":
            return self.reply(307, b"", {"location": "/auth-protected"})
        if path == "/auth-protected":
            if self.headers.get("authorization") != "Basic dXNlcjpwYXNz":
                return self.reply(
                    401, b"auth", {"www-authenticate": 'Basic realm="followups"'}
                )
            return self.reply(200, b"ok")
        if path.startswith("/redirect-"):
            dest = "/echo" if path == "/redirect-same" else CROSS + "/echo"
            return self.reply(302, b"", {"location": dest})
        if path.endswith(("/echo", "/destination")):
            return self.reply(
                200,
                json.dumps(
                    {
                        "path": path,
                        "method": self.command,
                        "body": body,
                        "headers": {k.lower(): v for k, v in self.headers.items()},
                    }
                ).encode(),
                {"content-type": "application/json"},
            )
        if path == "/frames":
            return self.reply(
                200, b'<!doctype html><iframe name="uno" src="/child/one"></iframe>'
            )
        return self.reply(200, b"<!doctype html><html><body>fixture</body></html>")


SERVERS = []
BASE = ""
CROSS = ""


def start_servers():
    global BASE, CROSS
    for _ in range(2):
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        SERVERS.append(server)
    BASE = f"http://127.0.0.1:{SERVERS[0].server_port}"
    CROSS = f"http://127.0.0.1:{SERVERS[1].server_port}"


async def page_fixture(browser, html=None, touch=False):
    context = await browser.new_context(has_touch=touch)
    page = await context.new_page()
    page.set_default_timeout(8000)
    await page.goto(BASE + "/main")
    if html is not None:
        await page.set_content(html)
    cdp = await context.new_cdp_session(page)
    return context, page, cdp


async def screenshot(cdp):
    await cdp.send("Page.captureScreenshot", {"format": "png"})


async def tap(cdp, x=20, y=20):
    await cdp.send(
        "Input.dispatchTouchEvent",
        {"type": "touchStart", "touchPoints": [{"x": x, "y": y, "id": 0}]},
    )
    await cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    await cdp.send(
        "Runtime.evaluate",
        {
            "expression": "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))",
            "awaitPromise": True,
            "returnByValue": True,
        },
    )


async def boundary(browser, kind):
    context, page, cdp = await page_fixture(
        browser,
        """<style>body{margin:0}#menu{position:absolute;left:0;top:0;width:200px;height:100px}button{position:absolute;top:10px;width:60px;height:40px}#item{left:10px}#other{left:80px}</style><div id=menu><button id=item>Item</button><button id=other>Other</button></div><script>window.log=[];window.clicks=0;window.menuClosed=false;const menuEl=document.getElementById('menu'),itemEl=document.getElementById('item'),otherEl=document.getElementById('other');itemEl.onclick=()=>clicks++;for(const t of ['mouseover','mouseout','mouseenter','mouseleave'])for(const e of [menuEl,itemEl,otherEl])e.addEventListener(t,x=>{if(x.target===e)log.push(x.type+'@'+e.id)});</script>""",
        True,
    )
    try:
        await screenshot(cdp)
        if kind in ("mouse-parent-child", "tap-parent-child"):
            if kind.startswith("mouse"):
                await page.mouse.move(190, 90)
            else:
                await tap(cdp, 190, 90)
            await page.evaluate(
                "log=[];document.getElementById('menu').onmouseleave=()=>{menuClosed=true;document.getElementById('menu').remove()};'ready'"
            )
            await tap(cdp)
        elif kind == "child-parent":
            await tap(cdp)
            await page.evaluate("log=[]")
            await tap(cdp, 190, 90)
        else:
            await tap(cdp)
            await page.evaluate("log=[]")
            await tap(cdp, 100, 20)
        return await page.evaluate("({log,clicks,closed:menuClosed})")
    finally:
        await context.close()


async def moved_target(browser, event, mutation):
    context, page, cdp = await page_fixture(
        browser,
        """<style>body{margin:0}#parent{width:400px;height:120px}#item{position:absolute;left:0;top:0;width:80px;height:80px}#cover{display:none;position:absolute;left:0;top:0;width:80px;height:80px;z-index:10}</style><div id=parent><button id=item>Item</button><div id=cover>Cover</div></div><script>window.log=[];for(const t of ['mousemove','mousedown','mouseup','click','focus'])for(const e of ['item','parent','cover'].map(id=>document.getElementById(id)))e.addEventListener(t,x=>{if(x.target===e)log.push(x.type+'@'+e.id)});</script>""",
        True,
    )
    try:
        mutations = {
            "move": "document.getElementById('item').style.left='200px'",
            "hide": "document.getElementById('item').style.display='none'",
            "remove-root": "document.documentElement.remove()",
            "cover": "document.getElementById('cover').style.display='block'",
        }
        await page.evaluate(
            f"document.getElementById('item').addEventListener('{event}',()=>{{{mutations[mutation]}}},{{once:true}})"
        )
        await screenshot(cdp)
        await tap(cdp)
        return await page.evaluate("log")
    finally:
        await context.close()


async def empty_text(browser, kind, cancel):
    html = f"<{kind} id=i" + (
        " contenteditable=true>abc</div>"
        if kind == "div"
        else ">" + ("abc</textarea>" if kind == "textarea" else "")
    )
    if kind == "input":
        html = "<input id=i value=abc>"
    context, page, cdp = await page_fixture(browser, html)
    try:
        await page.evaluate(
            """kind=>{i.focus();if(kind==='div'){const r=document.createRange();r.setStart(i.firstChild,1);r.setEnd(i.firstChild,2);getSelection().removeAllRanges();getSelection().addRange(r);}else i.setSelectionRange(1,2);window.log=[];for(const t of ['beforeinput','textInput','input'])i.addEventListener(t,e=>log.push([e.type,e.data,e.inputType??null]));}""",
            kind,
        )
        if cancel:
            await page.evaluate(
                "i.addEventListener('beforeinput',e=>e.preventDefault())"
            )
        await cdp.send("Input.insertText", {"text": ""})
        return await page.evaluate("({text:i.value??i.textContent,log})")
    finally:
        await context.close()


async def iframe_navigation(browser, routed=False):
    context, page, cdp = await page_fixture(browser)
    try:
        await page.goto(BASE + "/frames")
        frame = page.frame(name="uno")
        assert frame is not None
        if routed:
            await page.route("**/child/**", lambda route: route.continue_())
        events = []
        await cdp.send("Page.enable")
        await cdp.send("Network.enable")
        for m in [
            "Page.frameRequestedNavigation",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Page.frameNavigated",
            "Page.frameAttached",
        ]:
            cdp.on(
                m,
                lambda e, m=m: events.append(
                    {
                        "method": m,
                        "frame": e.get("frameId", e.get("frame", {}).get("id")),
                        "url": e.get(
                            "url",
                            e.get("request", e.get("response", e.get("frame", {}))).get(
                                "url"
                            ),
                        ),
                    }
                ),
            )
        async with frame.expect_navigation(wait_until="load") as wait:
            await page.locator("iframe").evaluate(
                "(e,url)=>e.src=url", BASE + "/child/two"
            )
        response = await wait.value
        return {
            "response": None
            if response is None
            else {"url": response.url, "status": response.status},
            "events": events,
        }
    finally:
        await context.close()


async def held_redirect(browser):
    REDIRECT_BODY_RELEASE.clear()
    REDIRECT_HEADERS_SENT.clear()
    context, page, cdp = await page_fixture(browser)
    try:
        pauses = asyncio.Queue()
        cdp.on("Fetch.requestPaused", pauses.put_nowait)
        await cdp.send("Fetch.enable", {"patterns": [{"urlPattern": "*/held-*"}]})
        await page.evaluate("globalThis.heldResult=fetch('/held-start').then(r=>r.text());'started'")
        first = await asyncio.wait_for(pauses.get(), 5)
        await cdp.send("Fetch.continueRequest", {"requestId": first["requestId"]})
        assert await asyncio.to_thread(REDIRECT_HEADERS_SENT.wait, 5), "server did not send headers"
        try:
            next_hop = await asyncio.wait_for(pauses.get(), 5)
            advanced_before_body = not REDIRECT_BODY_RELEASE.is_set()
        except asyncio.TimeoutError:
            next_hop = None
            advanced_before_body = False
        # Release even on failure so cleanup cannot conceal the failed ordering.
        REDIRECT_BODY_RELEASE.set()
        if next_hop is None:
            next_hop = await asyncio.wait_for(pauses.get(), 5)
        await cdp.send("Fetch.continueRequest", {"requestId": next_hop["requestId"]})
        return {
            "advancedBeforeBody": advanced_before_body,
            "nextPath": urlsplit(next_hop["request"]["url"]).path,
            "body": await page.evaluate("heldResult"),
        }
    finally:
        REDIRECT_BODY_RELEASE.set()
        await context.close()


async def selection_change(browser):
    context, page, cdp = await page_fixture(
        browser, "<div id=i contenteditable>abc</div>"
    )
    try:
        await cdp.send("Page.enable")
        tree = await cdp.send("Page.getFrameTree")
        fid = tree["frameTree"]["frame"]["id"]
        world = await cdp.send(
            "Page.createIsolatedWorld", {"frameId": fid, "worldName": "probe-selection"}
        )
        await page.evaluate(
            "window.changes=0;window.changeDone=new Promise(resolve=>document.addEventListener('selectionchange',()=>{changes++;resolve(changes)}));'ready'"
        )
        r = await cdp.send(
            "Runtime.evaluate",
            {
                "contextId": world["executionContextId"],
                "expression": "getSelection().selectAllChildren(document.getElementById('i'));'done'",
                "returnByValue": True,
            },
        )
        # A distinct queued user-interaction task lets both engines drain selectionchange.
        try:
            await asyncio.wait_for(page.evaluate("changeDone"), 2)
            notified = True
        except TimeoutError:
            notified = False
        return {
            "selected": await page.evaluate("String(getSelection())"),
            "notified": notified,
            "changes": await page.evaluate("changes"),
            "worldResult": r.get("exceptionDetails"),
        }
    finally:
        await context.close()


async def chained_fulfill(browser, variant):
    context, page, first = await page_fixture(browser)
    second = await context.new_cdp_session(page)
    q = asyncio.Queue()
    trace = []
    try:
        for sid, cdp in enumerate([first, second]):
            cdp.on("Fetch.requestPaused", lambda e, sid=sid: q.put_nowait((sid, e)))
            await cdp.send(
                "Fetch.enable",
                {
                    "patterns": [
                        {
                            "urlPattern": "*",
                            "resourceType": "Fetch",
                            "requestStage": "Request",
                        }
                    ]
                },
            )
        fulfilled = False
        stage = 0

        async def consume():
            nonlocal fulfilled, stage
            while True:
                sid, e = await q.get()
                r = e["request"]
                trace.append(
                    {
                        "session": sid,
                        "url": r["url"],
                        "method": r["method"],
                        "body": r.get("postData"),
                    }
                )
                cdp = [first, second][sid]
                if not fulfilled and stage == 0:
                    stage = 1
                    params = {
                        "requestId": e["requestId"],
                        "postData": base64.b64encode(b"new").decode(),
                    }
                    if variant == "method":
                        params["method"] = "PUT"
                    if variant == "relative":
                        params["url"] = BASE + "/rewritten/base/source"
                    await cdp.send("Fetch.continueRequest", params)
                elif not fulfilled:
                    fulfilled = True
                    status = 308 if variant == "308" else 307
                    await cdp.send(
                        "Fetch.fulfillRequest",
                        {
                            "requestId": e["requestId"],
                            "responseCode": status,
                            "responseHeaders": [
                                {
                                    "name": "Location",
                                    "value": "destination"
                                    if variant == "relative"
                                    else "/destination",
                                }
                            ],
                            "body": "",
                        },
                    )
                else:
                    await cdp.send(
                        "Fetch.continueRequest", {"requestId": e["requestId"]}
                    )

        actor = asyncio.create_task(consume())
        try:
            result = await asyncio.wait_for(
                page.evaluate(
                    "async()=>{const r=await fetch('/source',{method:'POST',body:'old'});return {status:r.status,url:r.url,text:await r.text()}}"
                ),
                10,
            )
        finally:
            actor.cancel()
            await asyncio.gather(actor, return_exceptions=True)
        result["server"] = json.loads(result["text"])
        return {"trace": trace, "result": result}
    finally:
        await context.close()


async def redirect_headers(browser, kind):
    context, page, _cdp = await page_fixture(browser)
    try:

        async def handle(route):
            if "/redirect-" in route.request.url:
                h = dict(route.request.headers)
                h.pop("x-delete", None)
                h["x-added"] = "new"
                await route.continue_(headers=h)
            else:
                await route.continue_()

        await page.route("**/*", handle)
        result = await page.evaluate(
            "async path=>{const r=await fetch(path,{headers:{'x-delete':'old','x-keep':'keep'}});return {status:r.status,url:r.url,text:await r.text()}}",
            "/redirect-" + kind,
        )
        result["server"] = json.loads(result["text"])
        return result
    finally:
        await context.close()


async def auth_redirect(browser):
    context, page, cdp = await page_fixture(browser)
    queue = asyncio.Queue()
    events = []
    try:
        cdp.on(
            "Fetch.requestPaused", lambda event: queue.put_nowait(("request", event))
        )
        cdp.on("Fetch.authRequired", lambda event: queue.put_nowait(("auth", event)))
        await cdp.send(
            "Fetch.enable",
            {
                "handleAuthRequests": True,
                "patterns": [{"urlPattern": "*", "requestStage": "Request"}],
            },
        )

        async def consume():
            while True:
                kind, event = await queue.get()
                events.append(kind + "@" + urlsplit(event["request"]["url"]).path)
                if kind == "request":
                    await cdp.send(
                        "Fetch.continueRequest", {"requestId": event["requestId"]}
                    )
                else:
                    await cdp.send(
                        "Fetch.continueWithAuth",
                        {
                            "requestId": event["requestId"],
                            "authChallengeResponse": {
                                "response": "ProvideCredentials",
                                "username": "user",
                                "password": "pass",
                            },
                        },
                    )

        actor = asyncio.create_task(consume())
        evaluation = asyncio.create_task(
            page.evaluate("fetch('/auth-start').then(r=>r.text())")
        )
        try:
            done, _ = await asyncio.wait(
                {actor, evaluation}, return_when=asyncio.FIRST_COMPLETED
            )
            if actor in done:
                await actor
                raise AssertionError(
                    "interception consumer ended before fetch completion"
                )
            body = await evaluation
        finally:
            actor.cancel()
            evaluation.cancel()
            await asyncio.gather(actor, evaluation, return_exceptions=True)
        return {"body": body, "events": events}
    finally:
        await context.close()


async def ime_caret(browser):
    context, page, cdp = await page_fixture(browser, "<input id=i value=abcd>")
    try:
        await page.evaluate(
            "i.focus();i.setSelectionRange(1,2);i.addEventListener('beforeinput',()=>i.setSelectionRange(3,4));"
        )
        await cdp.send("Input.insertText", {"text": "X"})
        return await page.evaluate("[i.value,i.selectionStart,i.selectionEnd]")
    finally:
        await context.close()


async def ime_focus(browser):
    context, page, cdp = await page_fixture(
        browser, "<input id=i value=abcd><input id=j value=wxyz>"
    )
    try:
        await page.evaluate(
            "i.focus();i.setSelectionRange(1,2);j.setSelectionRange(2,3);i.addEventListener('beforeinput',()=>j.focus());'ready'"
        )
        await cdp.send("Input.insertText", {"text": "X"})
        return await page.evaluate(
            "[i.value,i.selectionStart,i.selectionEnd,j.value,j.selectionStart,j.selectionEnd,document.activeElement.id]"
        )
    finally:
        await context.close()


async def frame_editor_fixture(browser):
    context, page, cdp = await page_fixture(
        browser,
        '<iframe name="editor-frame" srcdoc="<div id=first contenteditable>abcd</div><div id=second contenteditable>wxyz</div>"></iframe>',
    )
    frame = page.frame(name="editor-frame")
    assert frame is not None
    return context, page, cdp, frame


async def frame_world_editing(browser):
    context, page, cdp, frame = await frame_editor_fixture(browser)
    try:
        await frame.locator("#first").fill("replace")
        filled = await frame.locator("#first").text_content()
        await frame.locator("#first").fill("")
        empty = await frame.locator("#first").text_content()
        await frame.evaluate("document.getElementById('first').textContent='abcd'")
        tree = await cdp.send("Page.getFrameTree")
        frame_id = tree["frameTree"]["childFrames"][0]["frame"]["id"]
        world = await cdp.send(
            "Page.createIsolatedWorld",
            {"frameId": frame_id, "worldName": "followup-editing"},
        )
        for expression, text in [
            (
                "const e=document.getElementById('first');e.focus();getSelection().setBaseAndExtent(e.firstChild,1,e.firstChild,2);'ready'",
                "X",
            ),
            ("getSelection().selectAllChildren(e);'ready'", ""),
        ]:
            response = await cdp.send(
                "Runtime.evaluate",
                {"contextId": world["executionContextId"], "expression": expression},
            )
            assert "exceptionDetails" not in response, response
            await cdp.send("Input.insertText", {"text": text})
            if text:
                replaced = await frame.locator("#first").text_content()
        return {
            "fill": filled,
            "emptyFill": empty,
            "replaceSelection": replaced,
            "emptySelection": await frame.locator("#first").text_content(),
        }
    finally:
        await context.close()


async def frame_editing_callback(browser, phase, effect):
    context, page, cdp, frame = await frame_editor_fixture(browser)
    try:
        await frame.evaluate(
            """({phase,effect})=>{
                const first=document.getElementById('first'),second=document.getElementById('second');
                first.focus();getSelection().setBaseAndExtent(first.firstChild,1,first.firstChild,2);
                globalThis.editProbe={first,second,log:[]};
                for(const node of [first,second])for(const type of ['beforeinput','textInput','input'])
                    node.addEventListener(type,e=>editProbe.log.push([type,e.target.id,e.data]));
                first.addEventListener(phase,()=>{
                    if(effect==='selection')getSelection().setBaseAndExtent(first.firstChild,3,first.firstChild,4);
                    if(effect==='focus')second.focus();
                    if(effect==='remove')first.remove();
                    if(effect==='adopt')parent.document.body.append(parent.document.adoptNode(first));
                },{once:true});
            }""",
            {"phase": phase, "effect": effect},
        )
        await cdp.send("Input.insertText", {"text": "X"})
        return await frame.evaluate(
            """()=>({first:editProbe.first.textContent,second:editProbe.second.textContent,
                childOwner:editProbe.first.ownerDocument===document,
                connected:editProbe.first.isConnected,log:editProbe.log})"""
        )
    finally:
        await context.close()


EDITOR_CONTENTS = {
    "plain": "visible",
    "nested": "<span>visible</span>",
    "hidden": '<span style="display:none">hidden</span>visible',
    "invisible": '<span style="visibility:hidden">hidden</span>visible',
    "noneditable": "<span contenteditable=false>skip</span>visible",
    "inert": "<span inert>skip</span>visible",
    "empty": "<span></span>visible",
    "comment": "<!--comment-->visible",
    "break": "<br>visible",
    "empty-host": "",
    "inline-containment": '<span style="content-visibility:hidden">hidden</span>visible',
    "visible-override": '<span style="visibility:hidden">hidden<b style="visibility:visible">start</b></span>visible',
}


async def editor_start(browser, kind):
    context, page, cdp = await page_fixture(
        browser, '<div id=editor contenteditable>' + EDITOR_CONTENTS[kind] + "</div>"
    )
    try:
        await page.locator("#editor").focus()
        await cdp.send("Input.insertText", {"text": "X"})
        # Publish rendering before observing innerText, including newly inserted
        # text nodes. Do not treat a stale layout projection as the edit result.
        await screenshot(cdp)
        return await page.locator("#editor").inner_text()
    finally:
        await context.close()


def verify(case):
    name = case["name"]
    assert "error" not in case, case
    value = case["value"]
    if name.startswith("boundary-"):
        expected = {
            "mouse-parent-child": [
                "mouseout@menu",
                "mouseover@item",
                "mouseenter@item",
            ],
            "tap-parent-child": ["mouseout@menu", "mouseover@item", "mouseenter@item"],
            "child-parent": ["mouseout@item", "mouseleave@item", "mouseover@menu"],
            "siblings": [
                "mouseout@item",
                "mouseleave@item",
                "mouseover@other",
                "mouseenter@other",
            ],
        }[name.removeprefix("boundary-")]
        assert value["log"] == expected, value
        assert value["closed"] is False, value
        assert value["clicks"] == 1, value
    elif name.startswith("retarget-"):
        phase, mutation = name.removeprefix("retarget-").split("-", 1)
        released = "cover" if mutation == "cover" else "parent"
        expected = ["mousemove@item"]
        if mutation == "remove-root":
            if phase != "mousemove":
                expected.append("mousedown@item")
            if phase == "focus":
                expected.append("focus@item")
        elif phase == "mousemove":
            expected += [
                "mousedown@" + released,
                "mouseup@" + released,
                "click@" + released,
            ]
        else:
            expected += ["mousedown@item"]
            if phase == "focus" or mutation != "hide":
                expected += ["focus@item"]
            expected += ["mouseup@" + released, "click@parent"]
        assert value == expected, value
    elif name.startswith("empty-"):
        canceled = name.endswith("-canceled")
        assert value["text"] == ("abc" if canceled else "ac"), value
        expected = [["beforeinput", "", "insertText"]]
        if not canceled:
            expected += [["textInput", "", None], ["input", "", "insertText"]]
        assert value["log"] == expected, value
    elif name.startswith("iframe-navigation"):
        assert value["response"] and value["response"]["status"] == 200, value
        methods = [
            event["method"]
            for event in value["events"]
            if event["url"] == BASE + "/child/two"
        ]
        assert methods == [
            "Page.frameRequestedNavigation",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Page.frameNavigated",
        ], value
    elif name == "held-redirect":
        assert value == {"advancedBeforeBody": True, "nextPath": "/held-final", "body": "final"}, value
    elif name == "selectionchange-world":
        assert (
            value["selected"] == "abc" and value["notified"] and value["changes"] == 1
        ), value
    elif name.startswith("chained-fulfill-"):
        variant = name.removeprefix("chained-fulfill-")
        method = "PUT" if variant == "method" else "POST"
        destination = (
            "/rewritten/base/destination" if variant == "relative" else "/destination"
        )
        trace = value["trace"]
        assert len(trace) == 4 and len({event["session"] for event in trace}) == 2, (
            value
        )
        assert [event["body"] for event in trace] == ["old", "new", "new", "new"], value
        assert [event["method"] for event in trace] == [
            "POST",
            method,
            method,
            method,
        ], value
        assert all(urlsplit(event["url"]).path == destination for event in trace[2:]), (
            value
        )
        server = value["result"]["server"]
        assert (server["method"], server["body"], server["path"]) == (
            method,
            "new",
            destination,
        ), value
        assert value["result"]["status"] == 200, value
    elif name.startswith("redirect-headers-"):
        assert value["status"] == 200, value
        headers = value["server"]["headers"]
        assert (
            headers.get("x-added") == "new"
            and headers.get("x-keep") == "keep"
            and "x-delete" not in headers
        ), value
    elif name == "auth-redirect":
        assert value == {
            "body": "ok",
            "events": [
                "request@/auth-start",
                "request@/auth-protected",
                "auth@/auth-protected",
            ],
        }, value
    elif name == "ime-caret-beforeinput":
        assert value == ["abcX", 2, 2], value
    elif name == "ime-focus-beforeinput":
        assert value == ["abcd", 1, 2, "wxXz", 2, 2, "j"], value
    elif name == "frame-world-editing":
        assert value == {
            "fill": "replace",
            "emptyFill": "",
            "replaceSelection": "aXcd",
            "emptySelection": "",
        }, value
    elif name.startswith("frame-editing-"):
        phase, effect = name.removeprefix("frame-editing-").split("-", 1)
        events = [["beforeinput", "first", "X"]]
        removed = effect in ("remove", "adopt")
        if phase == "textInput" or not removed:
            target = "second" if effect == "focus" and phase == "beforeinput" else "first"
            events.append(["textInput", target, "X"])
        if not removed:
            events.append(["input", "second" if effect == "focus" else "first", "X"])
        assert value == {
            "first": "abcX" if effect == "selection" else "abcd",
            "second": "Xwxyz" if effect == "focus" else "wxyz",
            "childOwner": effect != "adopt",
            "connected": effect != "remove",
            "log": events,
        }, value
    elif name.startswith("editor-start-"):
        kind = name.removeprefix("editor-start-")
        assert value == {
            "noneditable": "Xskipvisible",
            "inert": "Xskipvisible",
            "inline-containment": "Xhiddenvisible",
            "visible-override": "Xstartvisible",
            "break": "X\nvisible",
            "empty-host": "X",
        }.get(kind, "Xvisible"), value
    else:
        raise AssertionError("Unvalidated case: " + name)


def verify_frozen_touch_geometry(case):
    """Moli reuses published geometry; live focus eligibility still applies."""
    assert "error" not in case, case
    phase, mutation = case["name"].removeprefix("retarget-").split("-", 1)
    assert phase in ("mousemove", "mousedown", "focus"), case
    assert mutation in ("move", "hide", "cover"), case
    expected = ["mousemove@item", "mousedown@item"]
    if phase == "focus" or mutation != "hide":
        expected.append("focus@item")
    expected += ["mouseup@item", "click@item"]
    assert case["value"] == expected, case


async def run(browser):
    results = []

    async def probe(name, fn):
        try:
            value = await asyncio.wait_for(fn(), 20)
            results.append({"name": name, "value": value})
        except Exception as e:  # noqa: BLE001 - report every probe failure without skipping later cases
            results.append(
                {"name": name, "error": repr(e), "traceback": traceback.format_exc()}
            )
        print(json.dumps(results[-1]), flush=True)

    for k in ["mouse-parent-child", "tap-parent-child", "child-parent", "siblings"]:
        await probe("boundary-" + k, lambda k=k: boundary(browser, k))
    for e in ["mousemove", "mousedown", "focus"]:
        for m in ["move", "hide", "cover", "remove-root"]:
            await probe(
                "retarget-" + e + "-" + m, lambda e=e, m=m: moved_target(browser, e, m)
            )
    for k in ["input", "textarea", "div"]:
        for cancel in [False, True]:
            await probe(
                "empty-" + k + ("-canceled" if cancel else ""),
                lambda k=k, cancel=cancel: empty_text(browser, k, cancel),
            )
    await probe("iframe-navigation", lambda: iframe_navigation(browser))
    await probe("iframe-navigation-routed", lambda: iframe_navigation(browser, True))
    await probe("held-redirect", lambda: held_redirect(browser))
    await probe("selectionchange-world", lambda: selection_change(browser))
    for variant in ["307", "308", "method", "relative"]:
        await probe(
            "chained-fulfill-" + variant,
            lambda variant=variant: chained_fulfill(browser, variant),
        )
    for kind in ["same", "cross"]:
        await probe(
            "redirect-headers-" + kind,
            lambda kind=kind: redirect_headers(browser, kind),
        )
    await probe("auth-redirect", lambda: auth_redirect(browser))
    await probe("ime-caret-beforeinput", lambda: ime_caret(browser))
    await probe("ime-focus-beforeinput", lambda: ime_focus(browser))
    await probe("frame-world-editing", lambda: frame_world_editing(browser))
    for phase in ["beforeinput", "textInput"]:
        for effect in ["selection", "focus", "remove", "adopt"]:
            await probe(
                "frame-editing-" + phase + "-" + effect,
                lambda phase=phase, effect=effect: frame_editing_callback(browser, phase, effect),
            )
    for kind in EDITOR_CONTENTS:
        await probe("editor-start-" + kind, lambda kind=kind: editor_start(browser, kind))
    return results


def wait_endpoint(endpoint):
    with urllib.request.urlopen(endpoint + "/json/version/", timeout=0.5):
        return


async def main():
    global OUT, CHROMIUM
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", help="chromium, or path to a Moli executable")
    parser.add_argument("label")
    parser.add_argument("--output-dir", type=Path, default=OUT)
    parser.add_argument("--chromium", default=CHROMIUM)
    args = parser.parse_args()
    engine, label = args.engine, args.label
    touch_geometry = "live" if engine == "chromium" else "frozen"
    OUT = args.output_dir
    OUT.mkdir(parents=True, exist_ok=True)
    CHROMIUM = args.chromium
    start_servers()
    proc = None
    log = None
    for key in list(os.environ):
        if key.lower() in ("http_proxy", "https_proxy", "all_proxy"):
            os.environ.pop(key)
    os.environ["NO_PROXY"] = "*"
    os.environ["no_proxy"] = "*"
    try:
        async with async_playwright() as p:
            if engine == "chromium":
                browser = await p.chromium.launch(
                    executable_path=CHROMIUM,
                    args=["--no-sandbox", "--disable-dev-shm-usage"],
                )
            else:
                with socket.socket() as s:
                    s.bind(("127.0.0.1", 0))
                    port = s.getsockname()[1]
                log = (OUT / (label + "-server.log")).open("w")
                proc = await asyncio.create_subprocess_exec(
                    engine,
                    "serve",
                    "--layout",
                    "--resource",
                    "--port",
                    str(port),
                    "--log-level",
                    "warn",
                    stdout=log,
                    stderr=log,
                )
                endpoint = f"http://127.0.0.1:{port}"
                deadline = time.monotonic() + 15
                while True:
                    try:
                        await asyncio.to_thread(wait_endpoint, endpoint)
                        break
                    except OSError:
                        if proc.returncode is not None or time.monotonic() > deadline:
                            raise
                        await asyncio.sleep(0.025)
                browser = await p.chromium.connect_over_cdp(endpoint)
            try:
                results = await run(browser)
                version = browser.version
            finally:
                await browser.close()
            failures = []
            for case in results:
                try:
                    if (
                        engine != "chromium"
                        and case["name"].startswith("retarget-")
                        and not case["name"].endswith("-remove-root")
                    ):
                        verify_frozen_touch_geometry(case)
                    else:
                        verify(case)
                except AssertionError as error:
                    failures.append({"name": case["name"], "detail": str(error)})
            (OUT / (label + "-results.json")).write_text(
                json.dumps(
                    {
                        "version": version,
                        "touch_geometry": touch_geometry,
                        "results": results,
                        "failures": failures,
                    },
                    indent=2,
                )
                + "\n"
            )
            print(
                f"{len(results) - len(failures)}/{len(results)} passed; {version}; touch geometry={touch_geometry}",
                flush=True,
            )
            if failures:
                raise SystemExit(1)
    finally:
        if proc is not None:
            proc.terminate()
            await asyncio.wait_for(proc.wait(), 10)
        if log:
            log.close()
        for s in SERVERS:
            s.shutdown()


if __name__ == "__main__":
    asyncio.run(main())
