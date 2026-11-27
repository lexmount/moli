"""Browser-observable ports of Chromium's disabled-web-security tests.

Source revision and exclusions live in web_security_upstream_sources.json.
Expected results come from the pinned desktop Chrome, never from Moli. This
opt-in conformance group reports observable differences as test failures.
"""

from __future__ import annotations

import asyncio
import json
import logging
import platform
import tempfile
from pathlib import Path
from typing import Any

from playwright.async_api import Error as PlaywrightError, async_playwright

from ..assertions import SmokeError, assert_equal, record
from ..config import is_moli_browser
from ..diagnostics import new_cdp_session
from ..fixture import FixtureServer, TRANSPARENT_PNG
from ..png_image import decode_png
from ..serve import BrowserProcess, start_browser, stop_browser, wait_for_browser_endpoint
from .web_security import ORACLE_VERSION, browser_args, normalize, url


SOURCES = Path(__file__).with_name('web_security_upstream_sources.json')
CONTRACTS = Path(__file__).with_name('web_security_upstream_contracts.json')

XHR = """args => new Promise(resolve => {
  const xhr = new XMLHttpRequest();
  const finish = error => resolve({status:xhr.status, body:xhr.responseText, error});
  try {
    xhr.open('GET',args.url);
    if(args.preflight) xhr.setRequestHeader('X-NotSimple','1');
    xhr.onload = () => finish(null);
    xhr.onerror = () => finish('NetworkError');
    xhr.send();
  } catch(error) {finish(error.name)}
})"""

IMAGE = """async args => {
  const image = new Image();
  if(args.cors) image.crossOrigin = 'anonymous';
  const loaded = await new Promise(resolve => {
    image.onload = () => resolve(true);image.onerror = () => resolve(false);image.src = args.url;
  });
  if(!loaded) return {loaded};
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  canvas.getContext('2d').drawImage(image,0,0);
  const checks = {};
  for(const [name,read] of Object.entries({
    pixels:()=>canvas.getContext('2d').getImageData(0,0,1,1),
    dataURL:()=>canvas.toDataURL(), blob:()=>canvas.toBlob(()=>{}),
  })) {
    try {read();checks[name] = 'readable'} catch(error) {checks[name] = error.name}
  }
  return {loaded,...checks};
}"""


async def observe_upstream(endpoint: str, source: str, peer: FixtureServer,
                           results: list[dict[str, Any]], disabled: bool) -> dict[str, Any]:
    mode = 'disabled' if disabled else 'default'
    observed: dict[str, Any] = {}
    async with async_playwright() as playwright:
        browser = await playwright.chromium.connect_over_cdp(endpoint)
        version = await (await browser.new_browser_cdp_session()).send('Browser.getVersion')
        if not is_moli_browser():
            assert_equal(version['product'].split('/')[-1], ORACLE_VERSION, 'upstream oracle version')
        record(results, 'web_security_upstream_environment_' + mode, {
            'version': version, 'platform': platform.platform(),
            'source': json.loads(SOURCES.read_text())['chromium'],
        })
        context = await browser.new_context()
        try:
            with tempfile.TemporaryDirectory(prefix='web-security-upstream-') as directory:
                files = Path(directory)
                (files / 'index.html').write_text('<!doctype html><title>file probe</title><body>file-page</body>')
                (files / 'child.html').write_text('<!doctype html><body>file-child</body>')
                (files / 'get.txt').write_text('file-payload')
                (files / 'image.png').write_bytes(TRANSPARENT_PNG)

                async def case(name: str, action: Any, *, location: str | None = None) -> None:
                    page = await context.new_page()
                    try:
                        async with asyncio.timeout(12):
                            await page.goto(location or source + '/web-security/page', wait_until='load')
                            value = await action(page)
                        observed[name] = normalize(value, source, peer.url)
                    except (PlaywrightError, TimeoutError) as error:
                        # An unavailable loader/API is a failing observation. Keep
                        # collecting so one unsupported capability cannot hide others.
                        observed[name] = {'harnessError': type(error).__name__}
                        record(results, 'web_security_upstream_diagnostic_' + mode + '_' + name,
                               {'error': str(error)})
                    finally:
                        logging.getLogger(__name__).info("%s/%s: %s", mode, name, observed.get(name))
                        await page.close()

                for allowed in ('null', 'file://'):
                    key = mode + '-file-xhr-' + allowed
                    async def file_http(page: Any, key: str = key, allowed: str = allowed) -> Any:
                        result = await page.evaluate(XHR, {
                            'url': url(peer.url, 'echo', key, allow=allowed), 'preflight': True,
                        })
                        return {'result': result, 'requests': peer.web_security.requests(key)}
                    await case('file-xhr-' + ('null' if allowed == 'null' else 'file'), file_http,
                               location=(files / 'index.html').as_uri())
                for route in ('index.html', 'get.txt'):
                    await case('file-xhr-' + ('self' if route == 'index.html' else 'other'),
                               lambda page, route=route: page.evaluate(XHR, {'url': route}),
                               location=(files / 'index.html').as_uri())
                for file_page in (False, True):
                    for cors in (False, True):
                        name = ('file' if file_page else 'http') + '-canvas-' + ('cors' if cors else 'no-cors')
                        image = (files / 'image.png').as_uri() if file_page else url(peer.url, 'image', mode + name)
                        await case(name, lambda page, image=image, cors=cors:
                                   page.evaluate(IMAGE, {'url': image, 'cors': cors}),
                                   location=(files / 'index.html').as_uri() if file_page else None)

                async def sandbox_file(page: Any) -> Any:
                    await page.evaluate("""url => new Promise(resolve => {
                      const frame=document.createElement('iframe');frame.sandbox='allow-scripts';
                      frame.onload=resolve;frame.src=url;document.body.append(frame);
                    })""", (files / 'child.html').as_uri())
                    child = page.frames[-1]
                    return await child.evaluate(IMAGE, {'url': (files / 'image.png').as_uri(), 'cors': False})
                await case('sandbox-file-image', sandbox_file, location=(files / 'index.html').as_uri())

                async def service_worker(page: Any, operation: str) -> Any:
                    script = url(peer.url, 'sw.js', mode + '-upstream-' + operation)
                    if operation == 'get-cross':
                        return await page.evaluate("""async url => {
                          try {return {found:!!(await navigator.serviceWorker.getRegistration(url))}}
                          catch(error) {return {error:error.name}}
                        }""", peer.url + '/web-security/')
                    if operation == 'register-cross':
                        return await page.evaluate("""async script => {
                          try {
                            const registration=await navigator.serviceWorker.register(script);
                            const result={scope:registration.scope};await registration.unregister();return result;
                          } catch(error) {return {error:error.name}}
                        }""", script)
                    target_page = await context.new_page()
                    try:
                        await target_page.goto(peer.url + '/web-security/page')
                        await target_page.evaluate("""async script => {
                          await navigator.serviceWorker.register(script);await navigator.serviceWorker.ready;
                        }""", script)
                        if operation == 'storage-key':
                            # Query from the other origin, then confirm that the
                            # registration remains in its real origin's partition.
                            result = await page.evaluate("""async target => {
                              try {
                                const registration=await navigator.serviceWorker.getRegistration(target);
                                return {scope:registration?.scope??null,
                                  own:(await navigator.serviceWorker.getRegistrations()).map(r=>r.scope)};
                              } catch(error) {return {error:error.name}}
                            }""", peer.url + '/web-security/')
                            result['target'] = await target_page.evaluate("""async()=>
                              (await navigator.serviceWorker.getRegistrations()).map(r=>r.scope)""")
                            return result
                        await page.evaluate("""url => new Promise(resolve => {
                          const frame=document.createElement('iframe');frame.onload=resolve;
                          frame.src=url;document.body.append(frame);
                        })""", peer.url + '/web-security/page')
                        # Install/run in the child's own Realm; parent access is
                        # not required by the upstream update/unregister regressions.
                        child = page.frames[-1]
                        return await child.evaluate("""async operation => {
                          try {
                            const registration=await navigator.serviceWorker.ready;
                            if(operation==='update') {await registration.update();return 'updated'}
                            return await registration.unregister();
                          } catch(error) {return {error:error.name}}
                        }""", operation)
                    finally:
                        await target_page.evaluate("""async()=>{
                          for(const r of await navigator.serviceWorker.getRegistrations()) await r.unregister();
                        }""")
                        await target_page.close()
                for operation in ('get-cross', 'register-cross', 'storage-key', 'unregister', 'update'):
                    await case('sw-' + operation, lambda page, operation=operation:
                               service_worker(page, operation))
                for scheme in ('filesystem:', 'file:', 'data:'):
                    await case('sw-reject-' + scheme[:-1], lambda page, scheme=scheme:
                               page.evaluate("""async scheme => {
                      try {await navigator.serviceWorker.register(scheme+'///not-a-worker.js');return 'registered'}
                      catch(error) {return {error:error.name}}
                    }""", scheme))
                await case('sw-insecure-document', lambda page: page.evaluate("""()=>({
                  secure:isSecureContext, serviceWorker:typeof navigator.serviceWorker
                })"""), location=source.replace('127.0.0.1', 'one.test') + '/web-security/page')

                async def orb(page: Any) -> Any:
                    key = mode + '-upstream-orb'
                    result = await page.evaluate(XHR, {'url': url(peer.url, 'orb', key)})
                    return {'result': result, 'requests': peer.web_security.requests(key)}
                # The upstream test observes the internal ORB decision; the
                # original web-security group separately covers JS filtering.
                await case('orb-disable-blocking', orb)

                async def frame_access(page: Any, location: str) -> Any:
                    await page.evaluate("""url=>new Promise(resolve=>{
                      const f=document.createElement('iframe');f.onload=resolve;
                      f.src=url;document.body.append(f);
                    })""", location)
                    bounds = await page.locator('iframe').bounding_box()
                    if bounds is None:
                        raise SmokeError('SVG iframe has no layout bounds')
                    async def pixel() -> list[int]:
                        image = decode_png(await page.screenshot(clip={
                            'x': bounds['x'] + 5, 'y': bounds['y'] + 5, 'width': 1, 'height': 1,
                        }))
                        return list(image.pixel(0, 0))
                    before = await pixel()
                    result = await page.evaluate("""()=>{
                      const f=document.querySelector('iframe');
                      try {
                        const doc=f.contentDocument;
                        if(!doc) return {access:false};
                        doc.documentElement.style.background='green';
                        return {access:true,background:doc.documentElement.style.background};
                      } catch(error){return {error:error.name}}
                    }""")
                    return {**result, 'before': before, 'after': await pixel()}
                await case('data-svg-background', lambda page: frame_access(page,
                           'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" style="background:red"/>'))
                await case('origin-agent-cluster', lambda page: observe_oac(page, source, peer.url))

                # Browser checks that survive disabled renderer same-origin checks.
                for scheme in ('chrome', 'chrome-untrusted', 'chrome-extension', 'filesystem', 'view-source'):
                    target = (scheme + '://moli-upstream-probe/not-a-resource'
                              if scheme not in ('filesystem', 'view-source') else
                              scheme + ':' + source + '/web-security/page')
                    await case('reject-fetch-' + scheme, lambda page, target=target:
                               page.evaluate(XHR, {'url': target}))
                await case('reject-script-chrome-untrusted', lambda page:
                           page.evaluate("""()=>new Promise(resolve=>{
                      const script=document.createElement('script');script.onload=()=>resolve('loaded');
                      script.onerror=()=>resolve('blocked');script.src='chrome-untrusted://resources/mojo/mojo/public/js/bindings.js';
                      document.body.append(script);
                    })"""))
                for scheme in ('chrome', 'chrome-untrusted', 'file', 'view-source'):
                    target = ((files / 'index.html').as_uri() if scheme == 'file' else
                              'view-source:' + source + '/web-security/page' if scheme == 'view-source' else
                              scheme + '://moli-upstream-probe/not-a-resource')
                    for kind in ('iframe', 'popup', 'self'):
                        await case('reject-navigation-' + kind + '-' + scheme,
                                   lambda page, target=target, kind=kind:
                                   observe_navigation(page, target, kind))
                await case('local-network-request', lambda page: observe_local_request(page, source, peer, mode))
        finally:
            await context.close()
    return observed


async def observe_oac(page: Any, source: str, target: str) -> Any:
    # Ports differ, but the sites match, so this tests local cross-origin
    # bindings without switching off desktop Chromium's Site Isolation.
    await page.evaluate("""url=>new Promise(resolve=>{
      const f=document.createElement('iframe');f.onload=resolve;f.src=url;document.body.append(f);
    })""", target + '/web-security/page?oac=?1')
    child = page.frames[-1]
    return await child.evaluate("""()=>{
      let parentAccess;try{parentAccess=parent.document.querySelector('main').textContent}
      catch(error){parentAccess=error.name}
      return {cluster:self.originAgentCluster,parentAccess,origin:self.origin};
    }""")


async def observe_navigation(page: Any, target: str, kind: str) -> Any:
    root = await page.context.browser.new_browser_cdp_session()
    session = await new_cdp_session(page.context, page)
    before = {item['targetId'] for item in (await root.send('Target.getTargets'))['targetInfos']}
    try:
        # Blocked browser navigations need not fire load, or create a popup at
        # all. Observe the browser's committed URLs after the initiating task
        # and rendering opportunity, rather than waiting for a nonexistent event.
        opened = None
        try:
            opened = await page.evaluate("""async ({url,kind})=>{
              let opened=null;
              if(kind==='popup') opened=!!window.open(url,'_blank');
              else if(kind==='iframe') {
                const frame=document.createElement('iframe');frame.src=url;document.body.append(frame);
              } else location.href=url;
              await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
              return opened;
            }""", {'url': target, 'kind': kind})
        except PlaywrightError as error:
            if kind != 'self' or 'Execution context was destroyed' not in str(error):
                raise
        if kind == 'popup':
            popups = [item for item in (await root.send('Target.getTargets'))['targetInfos']
                      if item['type'] == 'page' and item['targetId'] not in before]
            try:
                return {'opened': opened, 'targets': len(popups),
                        'blocked': all(item['url'] != target for item in popups)}
            finally:
                for item in popups:
                    await root.send('Target.closeTarget', {'targetId': item['targetId']})
        tree = (await session.send('Page.getFrameTree'))['frameTree']
        frames = tree.get('childFrames', []) if kind == 'iframe' else [tree]
        return {'frames': len(frames), 'blocked': all(item['frame']['url'] != target for item in frames)}
    finally:
        await session.detach()
        await root.detach()


async def observe_local_request(page: Any, source: str, peer: FixtureServer, mode: str) -> Any:
    key = mode + '-upstream-local'
    result = await page.evaluate("""async url=>{
      try {return await (await fetch(url)).text()} catch(error){return {error:error.name}}
    }""", url(peer.url, 'echo', key, cors=1))
    return {'secure': await page.evaluate('isSecureContext'), 'result': result,
            'requests': peer.web_security.requests(key)}


def compare_upstream(observed: dict[str, Any], expected: dict[str, Any], mode: str,
                     results: list[dict[str, Any]]) -> list[str]:
    failures = []
    for name in sorted(observed.keys() | expected.keys()):
        actual, oracle = observed.get(name), expected.get(name)
        ok = name in observed and name in expected and actual == oracle
        results.append({'name': 'web_security_upstream_' + mode + '_' + name,
                        'ok': ok, 'chromium': oracle, 'observed': actual})
        if not ok:
            failures.append(mode + '/' + name)
    return failures


async def run_web_security_upstream_group(endpoint: str, fixture: str, results: list[dict[str, Any]],
                                          process: BrowserProcess | None) -> None:
    if process is None:
        raise SmokeError('web-security-upstream requires managed browser instances')
    peer = FixtureServer()
    peer.start()
    try:
        disabled_process = await start_browser(extra_args=browser_args(fixture, True))
        try:
            disabled_endpoint = await wait_for_browser_endpoint(disabled_process)
            defaults, disabled = await asyncio.gather(
                observe_upstream(endpoint, fixture, peer, results, False),
                observe_upstream(disabled_endpoint, fixture, peer, results, True),
            )
        finally:
            await stop_browser(disabled_process)
        expected = json.loads(CONTRACTS.read_text())
        failures = (compare_upstream(defaults, expected['default'], 'default', results) +
                    compare_upstream(disabled, expected['disabled'], 'disabled', results))
        if failures:
            raise SmokeError('Chromium upstream web-security differences: ' + ', '.join(failures))
    finally:
        peer.stop()
