"""Chrome for Testing 156.0.8078.73, with default desktop Site Isolation."""

from __future__ import annotations

import asyncio
import base64
import json
import platform
from pathlib import Path
from typing import Any
from urllib.parse import urlencode, urlsplit

from playwright.async_api import async_playwright

from ..assertions import SmokeError, assert_equal, record
from ..config import is_moli_browser
from ..diagnostics import new_cdp_session
from ..fixture import FixtureServer
from ..serve import BrowserProcess, start_browser, stop_browser, wait_for_browser_endpoint
from ..web_security_fixture import READ
from .web_security_loaders import observe_loaders


ORACLE_VERSION = '156.0.8078.73'
CONTRACTS = Path(__file__).with_name('web_security_contracts.json')


def browser_args(fixture: str, disabled: bool) -> tuple[str, ...]:
    if is_moli_browser():
        port = urlsplit(fixture).port
        args = ('--http-host-resolve', f'one.test:{port}:127.0.0.1',
                '--http-host-resolve', f'two.test:{port}:127.0.0.1')
    else:
        args = ('--host-resolver-rules=MAP one.test 127.0.0.1,MAP two.test 127.0.0.1',)
    return (*args, '--disable-web-security') if disabled else args


def url(origin: str, route: str, case: str, **query: Any) -> str:
    return f'{origin}/web-security/{route}?' + urlencode({'case':case, **query})


def normalize(value: Any, source: str, target: str) -> Any:
    if isinstance(value, str):
        return (value.replace(source, '{source}').replace(target, '{target}')
                .replace(source.replace('127.0.0.1','one.test'), '{remote-source}')
                .replace(source.replace('127.0.0.1','two.test'), '{remote-target}'))
    if isinstance(value, list):
        return [normalize(item, source, target) for item in value]
    if isinstance(value, dict):
        return {key:normalize(item, source, target) for key,item in value.items()}
    return value


async def read(page: Any, target: str, api: str = 'fetch', init: dict[str, Any] | None = None) -> Any:
    return await page.evaluate('async args => {' + READ + ';return read(...args)}', [target, api, init or {}])


async def worker_read(page: Any, source: str, target: str, api: str, kind: str = 'classic',
                      init: dict[str, Any] | None = None) -> Any:
    script = source + '/web-security/' + ('shared.js' if kind == 'shared' else 'worker.js')
    return await page.evaluate('''args => new Promise((resolve,reject) => {
      const [script,url,api,kind,init] = args;
      let worker;
      try {worker = kind === 'shared' ? new SharedWorker(script) : new Worker(script,{type:kind});}
      catch(error) {resolve({error:error.name});return;}
      const port = kind === 'shared' ? worker.port : worker;
      const finish = value => {clearTimeout(timer);kind === 'shared' ? port.close() : worker.terminate();resolve(value);};
      const timer = setTimeout(()=>reject(Error('worker did not complete')),8000);
      worker.onerror = event => {event.preventDefault();finish({error:'WorkerError'});};
      port.onmessage = event => finish(event.data);
      if(kind === 'shared') port.start();
      port.postMessage([url,api,init]);
    })''', [script,target,api,kind,init or {}])


async def module_worker(page: Any, source: str, dependency: str, kind: str) -> Any:
    script = url(source, 'module-worker.js', 'worker-'+kind, dependency=dependency, kind=kind)
    return await page.evaluate('''script => new Promise((resolve,reject) => {
      const worker = new Worker(script,{type:'module'});
      const finish = value => {clearTimeout(timer);worker.terminate();resolve(value);};
      const timer = setTimeout(()=>reject(Error('module worker did not complete')),8000);
      worker.onerror=e=>{e.preventDefault();finish({error:'WorkerError'});};
      worker.onmessage=e=>finish(e.data);worker.postMessage(null);
    })''', script)


async def observe_mode(endpoint: str, source: str, peer: FixtureServer,
                       results: list[dict[str, Any]], disabled: bool) -> dict[str, Any]:
    target = peer.url
    observed: dict[str, Any] = {}
    mode = 'disabled' if disabled else 'default'
    async with async_playwright() as playwright:
        browser = await playwright.chromium.connect_over_cdp(endpoint)
        root = await browser.new_browser_cdp_session()
        version = await root.send('Browser.getVersion')
        if not is_moli_browser():
            assert_equal(version['product'].split('/')[-1], ORACLE_VERSION, 'web-security oracle version')
        record(results, 'web_security_'+mode+'_environment', {
            'version':version, 'platform':platform.platform(), 'machine':platform.machine(),
        })
        context = await browser.new_context()
        try:
            page = await context.new_page()
            await page.goto(source+'/web-security/page', wait_until='load')

            async def case(name: str, api: str = 'fetch', route: str = 'echo',
                           init: dict[str, Any] | None = None, **query: Any) -> None:
                await context.clear_cookies()
                await context.add_cookies([{'name':'target_cookie','value':'present','url':target}])
                key = mode+'-'+name
                result = await read(page, url(target,route,key,**query),api,init)
                observed[name] = {'result':result,'requests':peer.web_security.requests(key)}

            for api in ('fetch','xhr','sync-xhr'):
                await case(api+'-get',api)
                await case(api+'-put',api,init={'method':'PUT','headers':{'X-Author':'kept'},'body':'request-body'})
                for credentials in ('omit','same-origin','include') if api == 'fetch' else ('same-origin','include'):
                    await case(api+'-cookie-'+credentials,api,init={'credentials':credentials},cors=1)
            for request_mode in ('cors','no-cors','same-origin'):
                await case('mode-'+request_mode,init={'mode':request_mode})
            for redirect in ('follow','manual','error'):
                for location in ('valid','missing','invalid'):
                    key = mode+'-redirect-'+redirect+'-'+location
                    query = {} if location == 'missing' else {'to':
                        url(target,'echo',key) if location == 'valid' else 'http://['}
                    await case('redirect-'+redirect+'-'+location,route='redirect',
                               init={'redirect':redirect},**query)
            await case('corp',route='corp',init={'mode':'no-cors'})
            await case('orb',route='orb',init={'mode':'no-cors'})
            await page.goto(source+'/web-security/csp',wait_until='load')
            await case('csp')
            await page.goto(source+'/web-security/page',wait_until='load')

            # Both a final response and a redirect are read twice in one context.
            # Server witnesses distinguish a cache hit from a second network request.
            for route in ('echo','redirect'):
                key = mode+'-cached-'+route
                query = {'cache':1,'cors':1}
                if route == 'redirect':
                    query['to'] = url(target,'echo',key,cache=1,cors=1)
                resource = url(target,route,key,**query)
                first, second = await read(page,resource), await read(page,resource)
                observed['cached-'+route] = {'first':first,'second':second,
                                             'requests':peer.web_security.requests(key)}

            for kind in ('classic','module','shared'):
                for api in ('fetch','xhr','sync-xhr'):
                    for method in ('GET','PUT'):
                        key = mode+'-'+kind+'-'+api+'-'+method
                        init = {} if method == 'GET' else {'method':'PUT','headers':{'X-Author':'kept'},'body':'request-body'}
                        result = await worker_read(page,source,url(target,'echo',key),api,kind,init)
                        observed[kind+'-'+api+'-'+method] = {'result':result,'requests':peer.web_security.requests(key)}
                key = mode+'-cross-worker-'+kind
                result = await worker_read(page,target,url(target,'echo',key),'fetch',kind,{'probeOrigin':True})
                observed['cross-worker-'+kind] = {'result':result,'requests':peer.web_security.requests(key)}
            for kind in ('static','dynamic'):
                key = mode+'-module-worker-'+kind
                result = await module_worker(page,source,url(target,'module.js',key),kind)
                observed['module-worker-'+kind] = {'result':result,'requests':peer.web_security.requests(key)}
                key = mode+'-window-import-'+kind
                dependency = url(target,'module.js',key)
                if kind == 'dynamic':
                    result = await page.evaluate('async url=>{try{return (await import(url)).value}catch(e){return {error:e.name}}}',dependency)
                else:
                    result = await page.evaluate('''url=>new Promise((resolve,reject)=>{
                      const script=document.createElement('script');script.type='module';
                      const timer=setTimeout(()=>reject(Error('module did not complete')),8000);
                      const finish=value=>{clearTimeout(timer);resolve(value)};
                      script.src=url;
                      script.onload=()=>finish(globalThis.importValue);script.onerror=()=>finish({error:'ModuleError'});
                      document.head.append(script);
                    })''',url(source,'window-module.js',key,dependency=dependency))
                observed['window-import-'+kind] = {'result':result,'requests':peer.web_security.requests(key)}

            cdp = await new_cdp_session(context,page)
            await cdp.send('Network.enable')
            for stage, action in (('Request','continue'),('Request','fulfill'),('Response','continue'),('Response','fulfill')):
                name = 'intercept-'+stage.lower()+'-'+action
                key = mode+'-'+name
                result = await intercepted_read(page,cdp,url(target,'echo',key),stage,action)
                observed[name] = {'result':result,'requests':peer.web_security.requests(key)}

            for sandbox in (False,True):
                frame = await frame_case(page,source,target,sandbox)
                observed['sandbox-frame' if sandbox else 'local-frame'] = frame
            observed['navigated-proxy'] = await page.evaluate('''source=>new Promise(resolve=>{
              const frame=document.querySelector('iframe');
              globalThis.savedProxy=frame.contentWindow;
              frame.onload=()=>{try{resolve({same:savedProxy===frame.contentWindow,
                origin:savedProxy.location.origin,body:savedProxy.document.querySelector('main').textContent})}
                catch(e){resolve({error:e.name})}};
              frame.removeAttribute('sandbox');frame.src=source+'/web-security/page?navigated=1';
            })''',source)
            observed['initial-about'] = await page.evaluate('''()=>{
              const frame=document.createElement('iframe');document.body.append(frame);
              const w=frame.contentWindow;return {url:w.location.href,origin:w.origin,
                readable:!!w.document,ownOrigin:location.origin};
            }''')
            async with page.expect_popup() as opened:
                await page.evaluate("url=>{globalThis.probePopup=window.open(url)}", target+'/web-security/page')
            popup = await opened.value
            try:
                await popup.wait_for_load_state('load')
                observed['popup'] = await page.evaluate('''()=>{try{
                  return {body:probePopup.document.querySelector('main').textContent,origin:probePopup.location.origin}
                  }catch(e){return {error:e.name}}}''')
            finally:
                await popup.close()

            observed.update(await observe_loaders(page,context,source,peer,mode))

            # Distinct registrable domains exercise default Site Isolation.
            remote_source = source.replace('127.0.0.1','one.test')
            remote_target = source.replace('127.0.0.1','two.test')
            await page.goto(remote_source+'/web-security/page',wait_until='load')
            observed['remote-frame'] = await frame_case(page,remote_source,remote_target,False)
            targets = await root.send('Target.getTargets')
            record(results,'web_security_'+mode+'_frame_targets',{'targets':targets['targetInfos']})
            await page.goto(source+'/web-security/page',wait_until='load')

            await page.evaluate('''async()=>{
              await navigator.serviceWorker.register('/web-security/sw.js');
              await navigator.serviceWorker.ready;
              if(!navigator.serviceWorker.controller) await new Promise(resolve=>
                navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true}));
            }''')
            for request_mode in ('cors','no-cors','same-origin'):
                for sw in ('fallback','synthetic'):
                    await case('sw-'+sw+'-'+request_mode,init={'mode':request_mode},sw=sw)
            key = mode+'-sw-redirect-same-origin'
            destination = url(target,'echo',key)
            result = await read(page,url(source,'echo',key,sw='redirect',to=destination),init={'mode':'same-origin'})
            observed['sw-redirect-same-origin'] = {'result':result,'requests':peer.web_security.requests(key)}
        finally:
            await context.close()
            await browser.close()
    normalized = normalize(observed,source,target)
    record(results,'web_security_'+mode+'_observations',{'observed':normalized})
    return normalized


async def intercepted_read(page: Any, cdp: Any, target: str, stage: str, action: str) -> Any:
    queue: asyncio.Queue[Any] = asyncio.Queue()
    cdp.on('Fetch.requestPaused',queue.put_nowait)
    await cdp.send('Fetch.enable',{'patterns':[{'urlPattern':target,'requestStage':stage}]})
    operation = asyncio.create_task(read(page,target))
    paused = None
    try:
        async with asyncio.timeout(10):
            while not operation.done():
                paused = asyncio.create_task(queue.get())
                done,_ = await asyncio.wait((operation,paused),return_when=asyncio.FIRST_COMPLETED)
                if operation in done:
                    break
                event = paused.result()
                if action == 'fulfill':
                    await cdp.send('Fetch.fulfillRequest',{'requestId':event['requestId'],
                        'responseCode':200,'responseHeaders':[{'name':'X-Secret','value':'replacement'}],
                        'body':base64.b64encode(b'replacement').decode()})
                else:
                    await cdp.send('Fetch.continueResponse' if stage == 'Response' else 'Fetch.continueRequest',
                                   {'requestId':event['requestId']})
            return await operation
    finally:
        if paused is not None:
            paused.cancel()
            await asyncio.gather(paused,return_exceptions=True)
        operation.cancel()
        await asyncio.gather(operation,return_exceptions=True)
        await cdp.send('Fetch.disable')
        cdp.remove_listener('Fetch.requestPaused',queue.put_nowait)


async def frame_case(page: Any, source: str, target: str, sandbox: bool) -> Any:
    await page.evaluate('''args=>new Promise(resolve=>{
      document.querySelectorAll('iframe').forEach(f=>f.remove());
      const frame=document.createElement('iframe');frame.id='probe';
      if(args[1])frame.sandbox='allow-scripts';frame.onload=()=>resolve();
      frame.src=args[0]+'/web-security/page';document.body.append(frame);
    })''',[target,sandbox])
    child = page.frames[-1]
    own = await page.evaluate("localStorage.setItem('marker','source');({origin:self.origin,urlOrigin:location.origin})")
    other = await child.evaluate('''()=>{let storage;try{localStorage.setItem('marker','target');storage=localStorage.getItem('marker')}
      catch(e){storage=e.name}return {origin:self.origin,urlOrigin:location.origin,storage}}''')
    access = await page.evaluate('''()=>{try{const w=document.querySelector('iframe').contentWindow;
      return {body:w.document.querySelector('main').textContent,origin:w.origin}}
      catch(e){return {error:e.name}}}''')
    marker = await page.evaluate("localStorage.getItem('marker')")
    return {'source':own,'target':other,'access':access,'sourceStorage':marker}


async def run_web_security_group(endpoint: str, fixture: str, results: list[dict[str, Any]],
                                 process: BrowserProcess | None) -> None:
    if process is None:
        raise SmokeError('web-security requires managed browser instances for both startup modes')
    peer = FixtureServer()
    peer.start()
    try:
        disabled_process = await start_browser(extra_args=browser_args(fixture,True))
        try:
            disabled_endpoint = await wait_for_browser_endpoint(disabled_process)
            defaults, enabled = await asyncio.gather(
                observe_mode(endpoint,fixture,peer,results,False),
                observe_mode(disabled_endpoint,fixture,peer,results,True),
            )
        finally:
            await stop_browser(disabled_process)
        expected = json.loads(CONTRACTS.read_text())
        compare_mode(defaults,expected['default'],False,results)
        compare_mode(enabled,expected['disabled'],True,results)
    finally:
        peer.stop()


def compare_mode(observed: dict[str, Any], oracle: dict[str, Any], disabled: bool,
                 results: list[dict[str, Any]]) -> None:
    """Keep named representation/standards differences visible and narrowly checked."""
    expected = json.loads(json.dumps(oracle))
    differences: dict[str, str] = {}
    if is_moli_browser():
        expected['sandbox-frame']['target']['urlOrigin'] = 'null'
        differences['sandbox-frame'] = 'Existing sandbox Location.origin uses the document opaque origin.'
        if disabled:
            expected['resource-timing'] = [309,9,9]
            differences['resource-timing'] = 'Moli retains network byte sizes authorized by TAO; the pinned disabled Chromium loader reports zero sizes.'
            for name,origin in [('sandbox-frame','null'),('remote-frame','{remote-target}')]:
                expected[name]['access'] = {'body':'payload','origin':origin}
                differences[name] = (differences.get(name,'') +
                    ' Moli represents this frame locally; Chromium isolates it in a remote renderer.')
            expected['redirect-manual-invalid']['result'] = dict(expected['redirect-manual-valid']['result'])
            expected['redirect-error-missing']['result'] = {'error':'TypeError'}
            for name in ('redirect-manual-invalid','redirect-error-missing'):
                differences[name] = 'Shared Fetch redirect behavior follows the standard as documented in #2964.'
        else:
            for kind in ('classic','module','shared'):
                name = 'cross-worker-'+kind
                expected[name]['result'] = {'error':'WorkerError'}
                differences[name] = 'Existing default Worker constructor reports cross-origin startup failure asynchronously.'
            expected['sw-synthetic-no-cors']['result'] = {'error':'TypeError'}
            differences['sw-synthetic-no-cors'] = 'Existing default Service Worker body security rechecks synthetic no-cors text.'
            expected['nested-worker']['result'] = {'error':'WorkerError'}
            differences['nested-worker'] = 'Existing default Worker constructor reports cross-origin startup failure asynchronously.'
    assert_equal(sorted(observed),sorted(expected),'web-security case coverage')
    for name,value in expected.items():
        assert_equal(observed[name],value,f'web-security {"disabled" if disabled else "default"}/{name}')
        if name in differences:
            record(results,f'web_security_known_difference_{disabled}_{name}',{
                'reason':differences[name],'chromium':oracle[name],'moli':observed[name],
            })
