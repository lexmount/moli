"""Exercise Chromium's bundled DevTools against two remote pages per engine."""
from __future__ import annotations

import argparse
import asyncio
import base64
import json
from pathlib import Path

from .multipage_cdp_fuzz import Case, Lab, ROOT, http_get, write_json


CAPTURE = """(() => {
  window.__cdpTrace=[];
  const original = WebSocket.prototype.send;
  WebSocket.prototype.send = function(data) {
    if (!this.__recording) {
      this.__recording=true;
      this.addEventListener('message', e=>__cdpTrace.push({direction:'received',data:e.data}));
    }
    __cdpTrace.push({direction:'sent',data});
    return original.call(this,data);
  };
})()"""
SNAPSHOT = """(async () => {
  const SDK = await import('devtools://devtools/bundled/core/sdk/sdk.js');
  const manager = SDK.TargetManager.TargetManager.instance();
  const targets = manager.targets();
  return {primary:manager.primaryPageTarget()?.inspectedURL(),
    targets:await Promise.all(targets.map(async target => {
      const dom=target.model(SDK.DOMModel.DOMModel);
      const root=await dom?.requestDocument();
      const runtime=target.model(SDK.RuntimeModel.RuntimeModel);
      const resource=target.model(SDK.ResourceTreeModel.ResourceTreeModel);
      return {id:target.id(),name:target.name(),type:target.type(),url:target.inspectedURL(),
        domURL:root?.documentURL,frameURL:resource?.mainFrame?.url,
        contexts:runtime?.executionContexts().map(c=>({name:c.name,origin:c.origin,isDefault:c.isDefault}))};
    })), body:document.body.innerText.slice(0,1200)};
})()"""


async def run(args: argparse.Namespace) -> None:
    output = Path(args.output).resolve()
    lab = Lab(output, Path(args.moli).resolve(), args.chromium)
    try:
        await lab.start()
        for engine in ("chromium", "moli"):
            remote = Case(lab.endpoints[engine], lab.base, output / engine / "remote")
            frontend = Case(lab.endpoints["chromium"], lab.base, output / engine / "frontend")
            states = []
            try:
                await remote.connect()
                await frontend.connect()
                for label in ("A", "B"):
                    await remote.action({"op": "create", "page": label})
                    listing = await asyncio.to_thread(http_get, remote.endpoint + "/json/list")
                    ws = next(t["webSocketDebuggerUrl"] for t in listing if t["id"] == remote.targets[label])
                    created = await frontend.command("Target.createTarget", {"url": "about:blank"})
                    frontend.targets[label] = created["targetId"]
                    frontend.sessions[label] = await frontend.attach_extra("main", label)
                    sid = frontend.sessions[label]
                    await frontend.command("Page.addScriptToEvaluateOnNewDocument", {"source": CAPTURE}, sid)
                    await frontend.command("Page.navigate", {"url": "devtools://devtools/bundled/inspector.html?ws=" + ws.removeprefix("ws://")}, sid)
                for step in [
                    {"op": "delay", "seconds": 2},
                    {"op": "navigate", "page": "A", "path": "/A-frontend-next"},
                    {"op": "activate", "page": "B"},
                    {"op": "hash", "page": "B", "value": "frontend-fragment"},
                    {"op": "navigate", "page": "B", "path": "/B-frontend-cross", "cross": True},
                    {"op": "close", "page": "A"},
                ]:
                    await remote.action(step)
                    await asyncio.sleep(0.5)
                    state = {"action": step, "frontends": {}}
                    for label, sid in frontend.sessions.items():
                        response = await frontend.command("Runtime.evaluate", {
                            "expression": SNAPSHOT, "awaitPromise": True, "returnByValue": True}, sid)
                        state["frontends"][label] = response
                    states.append(state)
                    write_json(output / engine / "frontend-states.json", states)
                    print(json.dumps({"engine": engine, "step": step, "frontends": {
                        p: r.get("result", {}).get("value", r) for p, r in state["frontends"].items()}}), flush=True)
                for label, sid in frontend.sessions.items():
                    capture = await frontend.command("Runtime.evaluate", {"expression": "window.__cdpTrace", "returnByValue": True}, sid)
                    write_json(output / engine / f"{label}-frontend-wire.json", capture.get("result", {}).get("value", capture))
                    shot = await frontend.command("Page.captureScreenshot", {"format": "png"}, sid)
                    if "data" in shot:
                        (output / engine / f"{label}-frontend.png").write_bytes(base64.b64decode(shot["data"]))
            finally:
                await frontend.close()
                await remote.close()
    finally:
        lab.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli", default=str(ROOT / "target/debug/moli"))
    parser.add_argument("--chromium", default="/usr/bin/chromium")
    parser.add_argument("--output", required=True)
    asyncio.run(run(parser.parse_args()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
