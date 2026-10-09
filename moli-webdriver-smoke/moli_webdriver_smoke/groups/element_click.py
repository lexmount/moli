from __future__ import annotations

import asyncio
from typing import Any
from urllib.parse import quote

from selenium import webdriver
from selenium.common.exceptions import ElementClickInterceptedException
from selenium.webdriver.common.by import By

from ..assertions import assert_equal, assert_true, record_contract
from ..config import WebDriverTarget
from ..selenium_options import create_selenium_options


CASES = (
    ("multiline", "<div style='width:240px;text-indent:180px;line-height:48px'><a id='target' href='#'>AAAA BBBB CCCC</a></div>", False),
    ("fragmented", "<a id='target' href='#' style='line-height:48px'>first<br><span style='margin-left:180px'>second</span></a>", False),
    ("viewport-clipped", "<button id='target' style='position:fixed;left:-150px;top:20px;width:200px;height:50px'>go</button>", False),
    ("fractional", "<button id='target' style='position:fixed;left:20.25px;top:70.75px;width:101.5px;height:31.25px'>go</button>", False),
    ("first-fragment-obscured", "<div style='width:240px;text-indent:180px;line-height:48px'><a id='target' href='#'>AAAA BBBB CCCC</a></div><div style='position:fixed;left:180px;top:0;width:60px;height:48px;z-index:2'>cover</div>", True),
)


async def run_element_click_group(
    target: WebDriverTarget,
    _fixture: str,
    results: list[dict[str, Any]],
    _continue_on_failure: bool = False,
) -> None:
    await asyncio.to_thread(_run, target, results)


def _run(target: WebDriverTarget, results: list[dict[str, Any]]) -> None:
    with webdriver.Remote(target.endpoint, options=create_selenium_options(target)) as driver:
        for name, markup, intercepted in CASES:
            html = """<!doctype html><style>body { margin:0; font:16px monospace }</style>""" + markup + """
              <script>
              window.events = [];
              const target = document.getElementById('target');
              for (const type of ['pointermove','pointerdown','mousedown','pointerup','mouseup','click']) {
                target.addEventListener(type, e => {
                  events.push([e.type, e.clientX, e.clientY, e.isTrusted]);
                  if (e.type === 'click') e.preventDefault();
                });
              }
              </script>"""
            driver.get("data:text/html," + quote(html))
            element = driver.find_element(By.ID, "target")
            geometry = driver.execute_script("""
              const rects = arguments[0].getClientRects(), r = rects[0];
              const left = Math.max(0, Math.min(r.x, r.x + r.width));
              const right = Math.min(innerWidth, Math.max(r.x, r.x + r.width));
              const top = Math.max(0, Math.min(r.y, r.y + r.height));
              const bottom = Math.min(innerHeight, Math.max(r.y, r.y + r.height));
              const b = arguments[0].getBoundingClientRect();
              return {count: rects.length, point: [Math.floor((left+right)/2), Math.floor((top+bottom)/2)],
                      boundingCenter: [(b.left+b.right)/2, (b.top+b.bottom)/2]};
            """, element)
            if name in ("multiline", "fragmented", "first-fragment-obscured"):
                assert_true(geometry["count"] > 1, f"{name} has multiple fragments")
            assert_true(geometry["point"] != geometry["boundingCenter"], f"{name} distinguishes the bounding center")
            if intercepted:
                try:
                    element.click()
                except ElementClickInterceptedException:
                    pass
                else:
                    raise AssertionError("the first fragment is covered")
                assert_equal(driver.execute_script("return events"), [], "intercepted click dispatches no events")
            else:
                element.click()
                events = driver.execute_script("return events")
                assert_equal([e[0] for e in events],
                             ['pointermove','pointerdown','mousedown','pointerup','mouseup','click'],
                             f"{name} trusted pointer sequence")
                for event in events:
                    assert_equal(event[1:3], geometry["point"], f"{name} {event[0]} uses the in-view center")
                    assert_equal(event[3], True, f"{name} {event[0]} is trusted")
            record_contract(results, f"webdriver_element_click_{name}",
                            contract="Element Click hit-tests and dispatches at the floored viewport intersection of the first client rect.",
                            source="https://www.w3.org/TR/webdriver2/#dfn-in-view-center-point",
                            commands=["POST /element/{elementId}/click", "POST /execute/sync"],
                            observed={**geometry, "intercepted": intercepted})
