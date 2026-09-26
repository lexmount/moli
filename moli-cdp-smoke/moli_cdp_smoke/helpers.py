from __future__ import annotations

import base64
from typing import Any


async def capture_layout(page: Any) -> bytes:
    """Publish fixture layout for geometry and input assertions.

    Screenshot tests must call page.screenshot() directly on a fresh page;
    prewarming here would bypass Playwright's getLayoutMetrics preflight.
    """
    client = await page.context.new_cdp_session(page)
    try:
        result = await client.send("Page.captureScreenshot", {"format": "png"})
        return base64.b64decode(result["data"], validate=True)
    finally:
        await client.detach()


def attach_cdp_event_collector(client: Any, methods: list[str]) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for method in methods:
        client.on(method, lambda params, method=method: events.append({"method": method, "params": params}))
    return events


async def run_worker_command(page: Any, payload: Any, timeout_ms: int = 10_000) -> Any:
    return await page.evaluate(
        """
        async ({ payload, timeout }) => {
          return await new Promise((resolve, reject) => {
            const worker = new Worker('/worker.js');
            const timer = setTimeout(() => {
              worker.terminate();
              reject(new Error(`worker command timed out after ${timeout}ms`));
            }, timeout);
            worker.onmessage = event => {
              clearTimeout(timer);
              worker.terminate();
              resolve(event.data);
            };
            worker.onerror = event => {
              clearTimeout(timer);
              worker.terminate();
              reject(new Error(event.message || 'worker error'));
            };
            worker.postMessage(payload);
          });
        }
        """,
        {"payload": payload, "timeout": timeout_ms},
    )


async def evaluate_xhr(page: Any, url: str, method: str = "GET", body: str | None = None) -> Any:
    return await page.evaluate(
        """
        async ({ url, method, body }) => {
          return await new Promise(resolve => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('abort', () => events.push('abort'));
            xhr.addEventListener('loadend', () => {
              resolve({
                events,
                phase: events.includes('load') ? 'load' : events.includes('error') ? 'error' : 'other',
                status: xhr.status,
                readyState: xhr.readyState,
                text: xhr.responseText,
              });
            });
            xhr.open(method, url, true);
            xhr.send(body);
          });
        }
        """,
        {"url": url, "method": method, "body": body},
    )
