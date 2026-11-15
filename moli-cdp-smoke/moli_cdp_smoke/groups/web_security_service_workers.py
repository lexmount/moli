"""Service Worker stylesheet redirect security witness."""

from __future__ import annotations

from typing import Any

from ..fixture import FixtureServer
from .web_security_loaders import resource


async def observe_service_workers(page: Any, context: Any, source: str,
                                  peer: FixtureServer, mode: str) -> dict[str, Any]:
    observed: dict[str, Any] = {}
    await context.clear_cookies()
    key = mode + '-sw-style-redirect'
    destination = resource(peer.url, 'style', key, cors=1)
    result = await page.evaluate('''url=>new Promise((resolve,reject)=>{
      const link=document.createElement('link');link.rel='stylesheet';link.crossOrigin='anonymous';
      const timer=setTimeout(()=>reject(Error('redirected stylesheet did not complete')),8000);
      const finish=value=>{clearTimeout(timer);link.remove();resolve(value);};
      link.onload=()=>finish(getComputedStyle(document.body).getPropertyValue('--security-probe').trim());
      link.onerror=()=>finish('error');link.href=url;document.head.append(link);
    })''', resource(source, 'style', key + '-initial', sw='redirect', to=destination))
    observed['sw-style-redirect'] = {'result': result, 'requests': peer.web_security.requests(key)}

    return observed
