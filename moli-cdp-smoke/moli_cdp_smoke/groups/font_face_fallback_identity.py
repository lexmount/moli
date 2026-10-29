from __future__ import annotations

import asyncio
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from ..assertions import assert_equal, record_contract
from ..config import REPO_ROOT


async def run_font_fallback_identity_contract(browser: Any, results: list[dict[str, Any]]) -> None:
    fixtures = REPO_ROOT / "moli-layout/tests/fixtures"
    fonts = {name: (fixtures / f"moli-ahem-{width}.ttf").read_bytes()
             for name, width in [("a", "narrow"), ("b", "wide")]}
    requests: list[str] = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            requests.append(self.path)
            if self.path.endswith("style.css"):
                body = b"@font-face{font-family:FallbackIdentity;src:url(data:font/woff2;base64,YmFk),url(font.ttf)}"
                content_type = "text/css"
            elif self.path.endswith("font.ttf"):
                body = fonts[self.path.split("/")[-2]]
                content_type = "font/ttf"
            else:
                body = b"<!doctype html><html><head></head><body></body></html>"
                content_type = "text/html"
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_: Any) -> None:
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    observed = []
    try:
        for index, order in enumerate([["a", "b"], ["b", "a"]]):
            context = await browser.new_context()
            try:
                page = await context.new_page()
                await page.goto(f"http://127.0.0.1:{server.server_port}/round-{index}/page", wait_until="load")
                observed.append(await page.evaluate("""async order => {
                    const target=document.createElement('span');
                    target.textContent='AAAA';
                    target.style.cssText='display:inline-block;font:40px FallbackIdentity';
                    document.body.append(target);
                    const widths=[], counts=[];
                    for (const directory of order) {
                        const link=document.createElement('link');link.rel='stylesheet';
                        link.href=directory+'/style.css';
                        const loaded=new Promise((resolve,reject)=>{link.onload=resolve;link.onerror=reject});
                        document.head.append(link);await loaded;
                        const faces=[...document.fonts].filter(face=>face.family==='FallbackIdentity');
                        counts.push(faces.length);
                        await Promise.all(faces.map(face=>face.load()));
                        widths.push(target.getBoundingClientRect().width);
                    }
                    return {widths,counts};
                }""", order))
            finally:
                await context.close()
    finally:
        await asyncio.to_thread(server.shutdown)
        server.server_close()
        thread.join()
    assert_equal(observed, [{"widths": [80, 160], "counts": [1, 2]},
                            {"widths": [160, 80], "counts": [1, 2]}],
                 "relative fallbacks in identical stylesheet text keep independent resources")
    font_requests = sorted(path for path in requests if path.endswith("font.ttf"))
    assert_equal(font_requests, [f"/round-{index}/{directory}/font.ttf"
                                for index in range(2) for directory in ["a", "b"]],
                 "each stylesheet requests its own resolved fallback")
    record_contract(results, "font_face_resolved_fallback_identity",
                    contract="Font resource identity includes the ordered resolved fallback list.",
                    source="CSS Fonts source fallback and stylesheet URL resolution",
                    commands=["Runtime.evaluate"], observed={"geometry": observed, "requests": font_requests})
