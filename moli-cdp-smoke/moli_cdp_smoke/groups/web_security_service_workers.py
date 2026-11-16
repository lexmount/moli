"""Stylesheet redirects through network and Service Worker loading paths."""

from __future__ import annotations

from typing import Any
from urllib.parse import urlencode

from ..fixture import FixtureServer
from .web_security_loaders import resource


async def observe_stylesheet_redirect(page: Any, context: Any, source: str,
                                     peer: FixtureServer, mode: str, *,
                                     service_worker: bool) -> dict[str, Any]:
    await context.clear_cookies()
    observed: dict[str, Any] = {}
    for same_origin in (False, True):
        name = ('sw-style-redirect' if service_worker else 'style-redirect') + ('-same-origin' if same_origin else '')
        key = mode + '-' + name
        destination = resource(source if same_origin else peer.url, 'style', key, cors=1)
        redirect = resource(source, 'style' if service_worker else 'redirect', key + '-initial',
                            **({'sw': 'redirect'} if service_worker else {'status': 307}), to=destination)
        result = await page.evaluate('''url=>new Promise((resolve,reject)=>{
          const link=document.createElement('link');link.rel='stylesheet';link.crossOrigin='anonymous';
          const timer=setTimeout(()=>reject(Error('redirected stylesheet did not complete')),8000);
          const finish=value=>{clearTimeout(timer);link.remove();resolve(value);};
          link.onload=()=>finish(getComputedStyle(document.body).getPropertyValue('--security-probe').trim());
          link.onerror=()=>finish('error');link.href=url;document.head.append(link);
        })''', redirect)
        response = await context.request.get(source + '/web-security/requests?' + urlencode({'for': key + '-initial'}))
        initial = await response.json()
        if same_origin:
            response = await context.request.get(source + '/web-security/requests?' + urlencode({'for': key}))
            requests = await response.json()
        else:
            requests = peer.web_security.requests(key)
        observed[name] = {'result': result, 'initialRequests': initial, 'requests': requests}
    return observed
