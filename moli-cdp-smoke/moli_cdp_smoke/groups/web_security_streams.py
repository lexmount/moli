"""Worker response delivery with EOF held by an explicit HTTP barrier."""

from __future__ import annotations

import asyncio
from typing import Any

from ..assertions import assert_equal
from ..fixture import FixtureServer
from .web_security_loaders import resource


async def observe_streams(page: Any, context: Any, source: str, peer: FixtureServer,
                          mode: str, disabled: bool) -> dict[str, Any]:
    observed: dict[str, Any] = {}
    for kind in ('classic', 'module', 'shared'):
        for cross in (False, True):
            name = f'{kind}-no-cors-stream-' + ('cross' if cross else 'same')
            key = mode + '-' + name
            origin = source if cross else peer.url
            await context.clear_cookies()
            await page.goto(origin + '/web-security/page', wait_until='load')
            gate = peer.web_security.hold_stream(key)
            try:
                await page.evaluate('''([script,url,kind])=>{
                  globalThis.securityStreamWorker=kind==='shared'?new SharedWorker(script):new Worker(script,{type:kind});
                  const port=kind==='shared'?securityStreamWorker.port:securityStreamWorker;
                  globalThis.securityStreamEvents=[];
                  let first,done;
                  globalThis.securityStreamFirst=new Promise(resolve=>first=resolve);
                  globalThis.securityStreamDone=new Promise(resolve=>done=resolve);
                  port.onmessage=({data})=>{
                    securityStreamEvents.push(data);
                    if(data.phase==='first')first(true);
                    if(data.phase==='error')first(false);
                    if(data.phase==='error'||data.phase==='done')done(securityStreamEvents);
                  };
                  if(kind==='shared')port.start();
                  port.postMessage(url);
                }''', [resource(origin, 'stream-worker.js', key + '-worker'),
                       resource(peer.url, 'stream', key), kind])
                assert_equal(await asyncio.to_thread(gate.started.wait, 10), True,
                             name + ' server sent the first chunk')
                before_eof = None
                if disabled or not cross:
                    try:
                        async with asyncio.timeout(5):
                            before_eof = await page.evaluate('securityStreamFirst')
                    except TimeoutError:
                        before_eof = False
                gate.release.set()
                async with asyncio.timeout(10):
                    events = await page.evaluate('securityStreamDone')
                observed[name] = {'beforeEOF': before_eof, 'events': events,
                                  'requests': peer.web_security.requests(key)}
            finally:
                gate.release.set()
                await page.evaluate('''kind=>{
                  if(kind==='shared')securityStreamWorker.port.close();
                  else securityStreamWorker.terminate();
                }''', kind)
    await page.goto(source + '/web-security/page', wait_until='load')
    return observed
