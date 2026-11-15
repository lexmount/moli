"""Local HTTP witnesses for the fixed Chromium web-security contract."""

from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler
from typing import Any
from urllib.parse import parse_qs, urlsplit

from .config import REPO_ROOT


READ = """async function read(url, api='fetch', init={}) {
  try {
    if (api === 'fetch') {
      const request = new Request(url, init);
      const response = await fetch(request);
      const copy = response.clone();
      return {mode:request.mode, type:response.type, status:response.status,
        redirected:response.redirected, body:await response.text(), clone:await copy.text(),
        secret:response.headers.get('x-secret'), cookie:response.headers.get('set-cookie')};
    }
    const xhr = new XMLHttpRequest();
    xhr.open(init.method || 'GET', url, api !== 'sync-xhr');
    xhr.withCredentials = init.credentials === 'include';
    for (const [name,value] of Object.entries(init.headers || {})) xhr.setRequestHeader(name,value);
    const result = () => ({status:xhr.status,body:xhr.responseText,
      secret:xhr.getResponseHeader('x-secret'),cookie:xhr.getResponseHeader('set-cookie')});
    if (api === 'sync-xhr') { xhr.send(init.body || null); return result(); }
    return await new Promise(resolve => {
      xhr.onload = () => resolve(result());
      xhr.onerror = () => resolve({error:'NetworkError'});
      xhr.send(init.body || null);
    });
  } catch(error) { return {error:error.name}; }
}"""


class WebSecurityFixture:
    def __init__(self, image: bytes) -> None:
        self._image = image
        self._lock = threading.Lock()
        self._requests: dict[str, list[dict[str, Any]]] = {}

    def requests(self, case: str) -> list[dict[str, Any]]:
        with self._lock:
            return list(self._requests.get(case, []))

    def respond(self, handler: BaseHTTPRequestHandler, body: bytes = b"") -> None:
        parsed = urlsplit(handler.path)
        query = parse_qs(parsed.query, keep_blank_values=True)
        case = query.get('case', [''])[0]
        headers = {name.lower(): value for name,value in handler.headers.items()}
        cookies = dict(part.strip().split('=', 1) for part in headers.get('cookie','').split(';') if '=' in part)
        if case:
            with self._lock:
                observed = {
                    'path':parsed.path, 'method':handler.command, 'origin':headers.get('origin'),
                    'cookie':cookies, 'author':headers.get('x-author'),
                    'body':body.decode('utf-8'),
                }
                if parsed.path == '/web-security/module.js':
                    observed['destination'] = headers.get('sec-fetch-dest')
                self._requests.setdefault(case, []).append(observed)
        status, mime, payload = 200, 'text/plain', 'payload'
        extra = {'X-Secret':'visible', 'Set-Cookie':'hidden=1; Path=/; SameSite=Lax'}
        cache = 'public, max-age=3600' if query.get('cache') == ['1'] else 'no-store'
        if query.get('cache') == ['1']:
            extra.pop('Set-Cookie')
        route = parsed.path.removeprefix('/web-security/')
        if route == 'redirect':
            status = int(query.get('status', ['302'])[0])
            if 'to' in query:
                extra['Location'] = query['to'][0]
        elif route == 'corp':
            extra['Cross-Origin-Resource-Policy'] = 'same-origin'
        elif route == 'orb':
            mime, payload = 'application/json', '{"secret":"payload"}'
        elif route == 'requests':
            mime, payload = 'application/json', json.dumps(self.requests(query['for'][0]))
        elif route == 'image':
            mime, payload = 'image/png', self._image
        elif route == 'font':
            mime, payload = 'font/woff2', (REPO_ROOT / 'moli-layout/tests/fixtures/moli-ahem.woff2').read_bytes()
        elif route == 'style':
            mime, payload = 'text/css', 'body { --security-probe: loaded; }'
        elif route == 'track':
            mime, payload = 'text/vtt', 'WEBVTT\n\n00:00.000 --> 00:01.000\nprobe\n'
        elif route == 'timing':
            payload = '123456789'
        elif route in ('page', 'csp'):
            mime, payload = 'text/html', '<!doctype html><title>web security</title><main>payload</main>'
            if route == 'csp':
                extra['Content-Security-Policy'] = "connect-src 'self'"
        elif route == 'worker.js':
            mime = 'text/javascript'
            payload = READ + "\nonmessage=async e=>{const result=await read(...e.data);postMessage(e.data[2]?.probeOrigin?{...result,workerOrigin:self.origin,scriptOrigin:location.origin}:result);close();};"
        elif route == 'shared.js':
            mime = 'text/javascript'
            payload = READ + "\nonconnect=e=>{const p=e.ports[0];p.onmessage=async e=>{p.postMessage(await read(...e.data));p.close();close();};p.start();};"
        elif route == 'module.js':
            mime, payload = 'text/javascript', "export const value='module';"
        elif route == 'window-module.js':
            mime = 'text/javascript'
            dependency = query['dependency'][0]
            payload = f"import {{value}} from {json.dumps(dependency)};globalThis.importValue=value;"
        elif route == 'module-worker.js':
            mime = 'text/javascript'
            dependency = query['dependency'][0]
            if query.get('kind', ['static'])[0] == 'static':
                payload = f"import {{value}} from {json.dumps(dependency)};onmessage=()=>{{postMessage(value);close();}};"
                if query.get('workerKind') == ['shared']:
                    payload = f"import {{value}} from {json.dumps(dependency)};onconnect=({{ports:[p]}})=>{{p.postMessage(value);p.close();close();}};"
                elif query.get('workerKind') == ['service']:
                    payload = f"import {{value}} from {json.dumps(dependency)};oninstall=()=>skipWaiting();onactivate=e=>e.waitUntil(clients.claim());onmessage=e=>e.ports[0].postMessage(value);"
            else:
                payload = f"onmessage=async()=>{{try{{postMessage((await import({json.dumps(dependency)})).value)}}catch(e){{postMessage({{error:e.name}})}}close();}};"
        elif route == 'sw.js':
            mime = 'text/javascript'
            payload = """oninstall=e=>e.waitUntil(skipWaiting());
              onactivate=e=>e.waitUntil(clients.claim());
              onfetch=e=>{const q=new URL(e.request.url).searchParams;
                if(q.get('sw')==='synthetic') e.respondWith(new Response('synthetic',{headers:{'x-secret':'synthetic'}}));
                if(q.get('sw')==='redirect') e.respondWith(Response.redirect(q.get('to')));};"""
        elif route == 'nested-parent.js':
            mime = 'text/javascript'
            child = json.dumps(query['child'][0])
            payload = f"onmessage=()=>{{const worker=new Worker({child});worker.onmessage=e=>{{postMessage({{parentOrigin:self.origin,parentScriptOrigin:location.origin,child:e.data}});worker.terminate();close();}};worker.onerror=e=>{{e.preventDefault();postMessage({{error:'ChildWorkerError'}});close();}};}};"
        elif route == 'nested-child.js':
            mime, payload = 'text/javascript', "postMessage({origin:self.origin,scriptOrigin:location.origin});close();"
        if query.get('cors') == ['1']:
            extra.update({'Access-Control-Allow-Origin':headers.get('origin','*'),
                          'Access-Control-Allow-Credentials':'true',
                          'Access-Control-Allow-Methods':'GET, PUT',
                          'Access-Control-Allow-Headers':'x-author'})
        data = payload if isinstance(payload, bytes) else payload.encode()
        handler.send_response(status)
        for name,value in {'Content-Type':mime,'Content-Length':str(len(data)),
                           'Cache-Control':cache,**extra}.items():
            handler.send_header(name,value)
        handler.end_headers()
        if handler.command != 'HEAD':
            handler.wfile.write(data)
