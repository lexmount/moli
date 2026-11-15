"""Stylesheet redirects through network and Service Worker loading paths."""

from __future__ import annotations

from typing import Any

from ..fixture import FixtureServer
from .web_security_loaders import resource


async def observe_stylesheet_redirect(page: Any, context: Any, source: str,
                                     peer: FixtureServer, mode: str, *,
                                     service_worker: bool) -> dict[str, Any]:
    await context.clear_cookies()
    name = 'sw-style-redirect' if service_worker else 'style-redirect'
    key = mode + '-' + name
    destination = resource(peer.url, 'style', key, cors=1)
    redirect = resource(source, 'style' if service_worker else 'redirect', key + '-initial',
                        **({'sw': 'redirect'} if service_worker else {}), to=destination)
    result = await page.evaluate('''url=>new Promise((resolve,reject)=>{
      const link=document.createElement('link');link.rel='stylesheet';link.crossOrigin='anonymous';
      const timer=setTimeout(()=>reject(Error('redirected stylesheet did not complete')),8000);
      const finish=value=>{clearTimeout(timer);link.remove();resolve(value);};
      link.onload=()=>finish(getComputedStyle(document.body).getPropertyValue('--security-probe').trim());
      link.onerror=()=>finish('error');link.href=url;document.head.append(link);
    })''', redirect)
    return {name: {'result': result, 'requests': peer.web_security.requests(key)}}
