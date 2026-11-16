"""Form policy selection and redirect privacy, witnessed by the HTTP server."""

from __future__ import annotations

from typing import Any

from ..fixture import FixtureServer
from .web_security_loaders import resource


async def observe_forms(page: Any, context: Any, source: str, peer: FixtureServer,
                        mode: str) -> dict[str, Any]:
    observed: dict[str, Any] = {}
    for framed in (False, True):
        for initial, changed in (('origin', 'no-referrer'), ('no-referrer', 'origin')):
            name = f'formdata-policy-{initial}-to-{changed}-' + ('iframe' if framed else 'top')
            key = mode + '-' + name
            await context.clear_cookies()
            await page.goto(resource(source, 'page', key + '-page', policy=initial), wait_until='load')
            destination = resource(peer.url, 'echo', key, witness_referrer=1)
            await submit(page, destination, framed, changed)
            observed[name] = peer.web_security.requests(key)

        for status in (307, 308):
            name = f'form-hidden-origin-{status}-' + ('iframe' if framed else 'top')
            key = mode + '-' + name
            await context.clear_cookies()
            await page.goto(resource(source, 'page', key + '-page', policy='no-referrer'), wait_until='load')
            destination = resource(peer.url, 'echo', key, witness_referrer=1)
            redirect = resource(peer.url, 'redirect', key, status=status, to=destination,
                                policy='unsafe-url', witness_referrer=1)
            await submit(page, redirect, framed)
            observed[name] = peer.web_security.requests(key)
    await page.goto(source + '/web-security/page', wait_until='load')
    return observed


async def submit(page: Any, destination: str, framed: bool, policy: str | None = None) -> None:
    if framed:
        await page.evaluate('''()=>new Promise(resolve=>{
          const frame=document.createElement('iframe');frame.name='policyTarget';
          frame.onload=resolve;document.body.append(frame);
        })''')
    async with page.expect_event('framenavigated', predicate=lambda frame:
            frame.name == 'policyTarget' if framed else frame == page.main_frame):
        await page.evaluate('''([url,framed,policy])=>{
          const form=document.createElement('form');form.method='POST';form.action=url;
          if(framed)form.target='policyTarget';
          const input=document.createElement('input');input.name='probe';input.value='value';form.append(input);
          if(policy)form.addEventListener('formdata',()=>{
            const meta=document.createElement('meta');meta.name='referrer';meta.content=policy;
            document.head.append(meta);
          });
          document.body.append(form);form.submit();
        }''', [destination, framed, policy])
