"""Native scrollbar visibility through the CLI and public CDP endpoint."""

from __future__ import annotations

import json
import os
from typing import Any
from urllib.parse import quote

from ..assertions import SmokeError, assert_equal, record
from ..config import clear_proxy_env, moli_binary
from ..png_image import decode_png
from ..process import run_captured_process
from ..raw_cdp import RawCdpClient, connect_raw_cdp
from ..serve import start_moli_serve, stop_moli_serve, wait_for_moli_endpoint
from .layout_screenshot import _capture, _open_target, _response_for_id, _success, _wait_for_load


REPRO = """<!doctype html><style>
html { scrollbar-color: red blue }
body { margin: 0 }
</style><div style="width:100vw;height:2000px;background:lime"></div>"""
HTML = REPRO + """
<style>
.scroller { position:absolute;left:20px;width:200px;height:100px;
            overflow:scroll;scrollbar-color:red blue;scrollbar-width:auto!important }
.scroller > div { width:400px;height:300px;background:lime }
#scroller { top:20px }
#stable { top:140px;scrollbar-gutter:stable both-edges }
iframe { position:absolute;left:300px;top:20px;width:240px;height:180px;border:0 }
</style>
<div id="scroller" class="scroller"><div id="content"></div></div>
<div id="stable" class="scroller"><div></div></div>
<iframe id="frame" srcdoc="<!doctype html><style>
html { scrollbar-color:red blue } body { margin:0 }
</style><div style='width:100vw;height:1000px;background:lime'>
<div id='child' style='width:100px;height:60px;overflow:scroll;scrollbar-color:red blue'>
<div style='width:200px;height:180px;background:lime'></div></div></div>"></iframe>
<script>
window.firstClient = document.documentElement.clientWidth;
window.clicks = [];
document.addEventListener('click', event => clicks.push(event.target.id));
</script>"""
URL = "data:text/html," + quote(HTML)
REPRO_URL = "data:text/html," + quote(REPRO)
ROOT_METRICS = "[innerWidth,document.documentElement.clientWidth,document.documentElement.scrollWidth]"
METRICS = """(() => {
  const scroller = document.getElementById('scroller');
  const stable = document.getElementById('stable');
  const frame = document.getElementById('frame');
  const child = frame.contentDocument.getElementById('child');
  return {
    root: [innerWidth, document.documentElement.clientWidth, document.documentElement.scrollWidth],
    element: [scroller.clientWidth, scroller.clientHeight, scroller.scrollWidth, scroller.scrollHeight],
    stable: [stable.clientWidth, stable.clientHeight, stable.clientLeft],
    css: [getComputedStyle(scroller).scrollbarWidth, getComputedStyle(stable).scrollbarGutter],
    frame: [frame.contentWindow.innerWidth, frame.contentDocument.documentElement.clientWidth,
            child.clientWidth, child.clientHeight, child.scrollWidth, child.scrollHeight],
  };
})()"""
CASES = (
    ("default", (), None, False),
    ("flag", ("--scrollbars",), None, True),
    ("env-true", (), "true", True),
    ("env-one", (), "1", True),
    ("env-false", (), "false", False),
    ("env-zero", (), "0", False),
    ("flag-overrides-false", ("--scrollbars",), "false", True),
    ("flag-overrides-zero", ("--scrollbars",), "0", True),
)


async def _evaluate(client: RawCdpClient, session: str, expression: str) -> Any:
    result, _, _ = await _success(client, "Runtime.evaluate", {
        "expression": expression, "returnByValue": True,
    }, session_id=session)
    if "exceptionDetails" in result:
        raise SmokeError(f"scrollbar evaluation failed: {result!r}")
    return result["result"].get("value")


async def _assert_metrics(
    client: RawCdpClient, session: str, visible: bool, width: int = 800,
) -> dict[str, Any]:
    metrics = await _evaluate(client, session, METRICS)
    assert_equal(metrics, {
        "root": [width, width - (15 if visible else 0), width],
        "element": [185, 85, 400, 300] if visible else [200, 100, 400, 300],
        "stable": [170, 85 if visible else 100, 15],
        "css": ["auto", "stable both-edges"],
        "frame": [240, 225, 85, 45, 200, 180] if visible else [240, 240, 100, 60, 200, 180],
    }, "scrollbar visibility, authored gutters and embedded frame geometry")
    return metrics


async def _assert_snapshot(
    client: RawCdpClient, session: str, visible: bool,
    width: int = 800, height: int = 600,
) -> dict[str, Any]:
    png, _ = await _capture(client, session)
    image = decode_png(png)
    assert_equal((image.width, image.height), (width, height), "scrollbar viewport PNG")
    color = (0, 0, 255, 255) if visible else (0, 255, 0, 255)
    for label, point in [
        ("viewport", (width - 8, height // 2)),
        ("element", (212, 80)),
        ("iframe", (532, 120)),
    ]:
        assert_equal(image.pixel(*point), color, f"{label} scrollbar paint or underlying content")
    return await _assert_metrics(client, session, visible, width)


async def _assert_input(client: RawCdpClient, session: str, visible: bool) -> None:
    offsets = await _evaluate(client, session, """(() => {
      const scroller = document.getElementById('scroller');
      const frame = document.getElementById('frame');
      const child = frame.contentDocument.getElementById('child');
      scroller.scrollLeft = 20; scroller.scrollTop = 60;
      child.scrollLeft = 10; child.scrollTop = 15;
      frame.contentWindow.scrollTo(0, 20); window.scrollTo(0, 100);
      return [scroller.scrollLeft, scroller.scrollTop, scrollY,
              child.scrollLeft, child.scrollTop, frame.contentWindow.scrollY];
    })()""")
    assert_equal(offsets, [20, 60, 100, 10, 15, 20], "hidden and visible script scrolling")
    await _evaluate(client, session, """(() => {
      window.scrollTo(0, 0); clicks.length = 0;
      document.getElementById('scroller').scrollLeft = 0;
    })()""")
    await _capture(client, session)
    # This is the classic vertical up button, or normal content when hidden.
    for event, buttons in [("mousePressed", 1), ("mouseReleased", 0)]:
        await _success(client, "Input.dispatchMouseEvent", {
            "type": event, "x": 212, "y": 25, "button": "left",
            "buttons": buttons, "clickCount": 1,
        }, session_id=session)
    await _capture(client, session)
    assert_equal(await _evaluate(client, session,
        "[document.getElementById('scroller').scrollTop, clicks]"),
        [20, []] if visible else [60, ["content"]],
        "native scrollbar hit or DOM content hit")
    await _evaluate(client, session, "document.getElementById('scroller').scrollTop = 0")
    await _capture(client, session)
    await _success(client, "Input.dispatchMouseEvent", {
        "type": "mouseWheel", "x": 80, "y": 50, "deltaX": 0, "deltaY": 30,
    }, session_id=session)
    await _capture(client, session)
    assert_equal(await _evaluate(client, session,
        "document.getElementById('scroller').scrollTop"), 30, "hidden and visible wheel scrolling")


async def _assert_emulation(
    client: RawCdpClient, target: str, session: str, startup_visible: bool,
) -> None:
    await _assert_snapshot(client, session, startup_visible)
    for params in [{}, {"hidden": "true"}]:
        message_id = await client.send("Emulation.setScrollbarsHidden", params, session_id=session)
        response, _ = await _response_for_id(client, message_id)
        assert_equal(response.get("error", {}).get("code"), -32602, "invalid scrollbar emulation parameters")
    await _assert_metrics(client, session, startup_visible)
    await _success(client, "Emulation.setScrollbarsHidden", {"hidden": True}, session_id=session)
    # Geometry is current at the command reply, before another output demand.
    await _assert_metrics(client, session, False)
    await _assert_snapshot(client, session, False)
    other, peer = await _open_target(client, URL)
    try:
        await _assert_snapshot(client, peer, startup_visible)
    finally:
        await _success(client, "Target.closeTarget", {"targetId": other})
    _, _, seen = await _success(client, "Page.navigate", {"url": URL}, session_id=session)
    await _wait_for_load(client, session, seen)
    assert_equal(await _evaluate(client, session, "firstClient"), 800, "hidden emulation before navigation scripts")
    await _assert_snapshot(client, session, False)
    await _success(client, "Emulation.setScrollbarsHidden", {"hidden": False}, session_id=session)
    await _assert_metrics(client, session, startup_visible)
    await _assert_snapshot(client, session, startup_visible)
    await _success(client, "Emulation.setScrollbarsHidden", {"hidden": True}, session_id=session)
    await _success(client, "Target.detachFromTarget", {"sessionId": session})
    attached, _, _ = await _success(client, "Target.attachToTarget", {"targetId": target, "flatten": True})
    session = attached["sessionId"]
    # Detach also clears the old session's device-metrics override.
    await _success(client, "Emulation.setDeviceMetricsOverride", {
        "width": 800, "height": 600, "deviceScaleFactor": 1, "mobile": False,
    }, session_id=session)
    await _assert_snapshot(client, session, startup_visible)
    await _success(client, "Emulation.setDeviceMetricsOverride", {
        "width": 640, "height": 480, "deviceScaleFactor": 1, "mobile": False,
    }, session_id=session)
    await _assert_snapshot(client, session, startup_visible, 640, 480)


async def _serve_case(
    name: str, args: tuple[str, ...], env_value: str | None,
    visible: bool, results: list[dict[str, Any]],
) -> None:
    serve = await start_moli_serve(extra_args=args, env_overrides={"MOLI_SCROLLBARS": env_value})
    client: RawCdpClient | None = None
    try:
        endpoint = await wait_for_moli_endpoint(serve)
        client = await connect_raw_cdp(endpoint)
        target, session = await _open_target(client, URL)
        metrics = await _assert_snapshot(client, session, visible)
        record(results, f"scrollbar_serve_{name}", {"visible": visible, "metrics": metrics})
        if name in {"default", "flag"}:
            await _assert_input(client, session, visible)
            record(results, f"scrollbar_input_{name}")
            # Reload clears the scroll offsets and DOM event probe.
            _, _, seen = await _success(client, "Page.navigate", {"url": URL}, session_id=session)
            await _wait_for_load(client, session, seen)
            await _assert_emulation(client, target, session, visible)
            record(results, f"scrollbar_emulation_{name}")
    finally:
        try:
            if client is not None:
                await client.websocket.close()
        finally:
            await stop_moli_serve(serve)


async def _fetch_case(
    name: str, args: tuple[str, ...], env_value: str | None,
    visible: bool, results: list[dict[str, Any]],
) -> None:
    env = clear_proxy_env(os.environ)
    env.pop("MOLI_LAYOUT", None)
    env.pop("MOLI_SCROLLBARS", None)
    if env_value is not None:
        env["MOLI_SCROLLBARS"] = env_value
    command = [str(moli_binary()), "fetch", "--layout", *args]
    for output_args in [("--eval", ROOT_METRICS), ("--dump", "screenshot")]:
        result = await run_captured_process([*command, *output_args, REPRO_URL], env=env, timeout_seconds=15)
        if result.returncode != 0:
            raise SmokeError(f"fetch {name} failed: {result.stderr.decode(errors='replace')}")
        if output_args[0] == "--eval":
            metrics = json.loads(result.stdout)
            width = metrics[0]
            assert_equal(metrics, [width, width - (15 if visible else 0), width], f"fetch {name} scrollbar geometry")
        else:
            image = decode_png(result.stdout)
            pixel = image.pixel(image.width - 8, image.height // 2)
            if visible:
                assert_equal(pixel in {(255, 0, 0, 255), (0, 0, 255, 255)}, True, f"fetch {name} paints a scrollbar")
            else:
                assert_equal(pixel, (0, 255, 0, 255), f"fetch {name} paints page content to the edge")
    record(results, f"scrollbar_fetch_{name}", {"visible": visible, "metrics": metrics})


async def _layout_boundaries(results: list[dict[str, Any]]) -> None:
    serve = await start_moli_serve(
        layout=False, env_overrides={"MOLI_LAYOUT": "0", "MOLI_SCROLLBARS": "0"},
    )
    client: RawCdpClient | None = None
    try:
        client = await connect_raw_cdp(await wait_for_moli_endpoint(serve))
        target, session = await _open_target(client, REPRO_URL)
        message_id = await client.send("Emulation.setScrollbarsHidden", {"hidden": True}, session_id=session)
        response, _ = await _response_for_id(client, message_id)
        assert_equal(response.get("error", {}).get("code"), -32000, "scrollbar emulation requires layout")
        assert_equal(response["error"]["message"], "Emulation.setScrollbarsHidden requires --layout", "scrollbar layout boundary")
        await _success(client, "Target.closeTarget", {"targetId": target})
        record(results, "scrollbar_mock_layout_boundary")
    finally:
        try:
            if client is not None:
                await client.websocket.close()
        finally:
            await stop_moli_serve(serve)

    env = clear_proxy_env(os.environ)
    env.pop("MOLI_LAYOUT", None)
    env.pop("MOLI_SCROLLBARS", None)
    for args, value in [(("--scrollbars",), None), ((), "true"), ((), "1")]:
        child_env = dict(env)
        if value is not None:
            child_env["MOLI_SCROLLBARS"] = value
        result = await run_captured_process(
            [str(moli_binary()), "fetch", *args, "--eval", ROOT_METRICS, REPRO_URL],
            env=child_env, timeout_seconds=15,
        )
        assert_equal(result.returncode != 0 and b"--layout" in result.stderr, True,
                     "enabling scrollbars requires layout")
    record(results, "scrollbar_cli_layout_dependency")


async def run_scrollbar_visibility_group(
    endpoint: str, fixture: str, results: list[dict[str, Any]],
) -> None:
    # Every startup case owns its process/environment; the caller's endpoint
    # cannot exercise CLI defaults or environment precedence.
    for name, args, env_value, visible in CASES:
        await _serve_case(name, args, env_value, visible, results)
        await _fetch_case(name, args, env_value, visible, results)
    await _layout_boundaries(results)
