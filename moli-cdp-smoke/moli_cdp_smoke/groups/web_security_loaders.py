"""Resource-loader and Worker-origin witnesses for the fixed Chrome oracle."""

from __future__ import annotations

from typing import Any
from urllib.parse import urlencode

from ..fixture import FixtureServer


def resource(origin: str, route: str, key: str, **query: Any) -> str:
    return origin + '/web-security/' + route + '?' + urlencode({'case': key, **query})


async def observe_loaders(page: Any, context: Any, source: str, peer: FixtureServer,
                          mode: str) -> dict[str, Any]:
    target = peer.url
    observed: dict[str, Any] = {}
    for kind in ('form', 'style', 'image', 'font'):
        await context.clear_cookies()
        key = mode + '-origin-' + kind
        destination = resource(target, 'echo' if kind == 'form' else kind, key)
        if kind == 'form':
            async with page.expect_navigation(wait_until='load'):
                await page.evaluate('''url=>{
                  const form=document.createElement('form');form.method='POST';form.action=url;
                  const input=document.createElement('input');input.name='probe';input.value='value';
                  form.append(input);document.body.append(form);form.submit();
                }''', destination)
            result = 'submitted'
            await page.goto(source + '/web-security/page', wait_until='load')
        else:
            result = await page.evaluate('''async ([kind,url])=>{
              if(kind==='font') {
                try {await new FontFace('SecurityProbe',`url("${url}")`).load();return 'load';}
                catch(e) {return e.name;}
              }
              return await new Promise((resolve,reject)=>{
                const node=document.createElement(kind==='style'?'link':'img');
                const timer=setTimeout(()=>reject(Error('resource did not complete')),8000);
                const finish=value=>{clearTimeout(timer);node.remove();resolve(value);};
                node.crossOrigin='anonymous';node.onload=()=>finish('load');node.onerror=()=>finish('error');
                if(kind==='style'){node.rel='stylesheet';node.href=url;}else node.src=url;
                document.head.append(node);
              });
            }''', [kind, destination])
        observed['origin-' + kind] = {'result': result, 'requests': peer.web_security.requests(key)}

    for policy in ('no-referrer', 'same-origin'):
        for framed in (False, True):
            await context.clear_cookies()
            key = f'{mode}-form-{policy}-' + ('iframe' if framed else 'top')
            await page.goto(resource(source, 'page', key + '-page', policy=policy), wait_until='load')
            if framed:
                await page.evaluate('''()=>new Promise(resolve=>{
                  const frame=document.createElement('iframe');frame.name='formTarget';
                  frame.onload=resolve;document.body.append(frame);
                })''')
            async with page.expect_event('framenavigated', predicate=lambda frame:
                    frame.name == 'formTarget' if framed else frame == page.main_frame):
                await page.evaluate('''([url,framed])=>{
                  const form=document.createElement('form');form.method='POST';form.action=url;
                  if(framed)form.target='formTarget';document.body.append(form);form.submit();
                }''', [resource(target, 'echo', key), framed])
            observed[key.removeprefix(mode + '-')] = peer.web_security.requests(key)
    await page.goto(source + '/web-security/page', wait_until='load')

    for kind in ('audio', 'video'):
        await context.clear_cookies()
        key = mode + '-origin-' + kind
        result = await page.evaluate('''([kind,url])=>new Promise((resolve,reject)=>{
          const media=document.createElement(kind);media.crossOrigin='anonymous';media.preload='metadata';
          const timer=setTimeout(()=>reject(Error('media did not complete')),8000);
          const finish=value=>{clearTimeout(timer);media.remove();resolve(value);};
          media.onloadedmetadata=()=>finish('load');media.onerror=()=>finish('error');
          media.src=url;document.body.append(media);
        })''', [kind, resource(target, kind, key, cors=1)])
        observed['origin-' + kind] = {'result': result, 'requests': peer.web_security.requests(key)}

    await context.clear_cookies()
    await context.add_cookies([{'name': 'target_cookie', 'value': 'present', 'url': source}])
    key = mode + '-nested-worker'
    child = resource(source, 'nested-child.js', key)
    parent = resource(target, 'nested-parent.js', key, child=child)
    result = await page.evaluate('''url=>new Promise((resolve,reject)=>{
      let worker;
      const finish=value=>{clearTimeout(timer);worker?.terminate();resolve(value);};
      const timer=setTimeout(()=>reject(Error('nested Worker did not complete')),8000);
      try {worker=new Worker(url);}catch(e){finish({error:e.name});return;}
      worker.onerror=e=>{e.preventDefault();finish({error:'WorkerError'});};
      worker.onmessage=e=>finish(e.data);worker.postMessage(null);
    })''', parent)
    response = await context.request.get(source + '/web-security/requests?' + urlencode({'for': key}))
    observed['nested-worker'] = {'result': result, 'parentRequests': peer.web_security.requests(key),
                                 'childRequests': await response.json()}

    for kind in ('shared', 'service'):
        await context.clear_cookies()
        key = mode + '-module-' + kind + '-static'
        dependency = resource(target, 'module.js', key, cors=1)
        script = resource(source, 'module-worker.js', key + '-root', dependency=dependency, workerKind=kind)
        result = await page.evaluate('''async ([kind,script])=>{
          if(kind==='shared') return await new Promise((resolve,reject)=>{
            const worker=new SharedWorker(script,{type:'module'});
            const timer=setTimeout(()=>reject(Error('SharedWorker module did not complete')),8000);
            const finish=value=>{clearTimeout(timer);worker.port.close();resolve(value);};
            worker.onerror=e=>{e.preventDefault();finish({error:'WorkerError'});};
            worker.port.onmessage=e=>finish(e.data);worker.port.start();
          });
          const registration=await navigator.serviceWorker.register(script,{type:'module',scope:'/web-security/module-scope/'});
          try {
            const worker=registration.installing||registration.waiting||registration.active;
            if(worker.state!=='activated') await new Promise((resolve,reject)=>{
              worker.onstatechange=()=>{if(worker.state==='activated')resolve();
                else if(worker.state==='redundant')reject(Error('Service Worker module failed'));};
            });
            const channel=new MessageChannel();
            return await new Promise((resolve,reject)=>{
              const timer=setTimeout(()=>reject(Error('Service Worker module did not reply')),8000);
              channel.port1.onmessage=e=>{clearTimeout(timer);channel.port1.close();resolve(e.data);};
              worker.postMessage(null,[channel.port2]);
            });
          } finally {await registration.unregister();}
        }''', [kind, script])
        observed['module-' + kind + '-static'] = {'result': result, 'requests': peer.web_security.requests(key)}

    key = mode + '-text-track'
    result = await page.evaluate('''url=>new Promise((resolve,reject)=>{
      const video=document.createElement('video'),track=document.createElement('track');
      const timer=setTimeout(()=>reject(Error('track did not complete')),8000);
      const finish=()=>{clearTimeout(timer);const value={readyState:track.readyState,cues:track.track.cues?.length||0};video.remove();resolve(value);};
      track.onload=finish;track.onerror=finish;track.src=url;track.default=true;
      video.append(track);document.body.append(video);track.track.mode='showing';
    })''', resource(target, 'track', key))
    observed['text-track'] = {'result': result, 'requests': peer.web_security.requests(key)}

    key = mode + '-resource-timing'
    result = await page.evaluate('''async url=>{
      const entry=new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>reject(Error('Resource Timing did not complete')),8000);
        const observer=new PerformanceObserver(list=>{for(const e of list.getEntries())if(e.name===url){
          clearTimeout(timer);observer.disconnect();resolve([e.transferSize,e.encodedBodySize,e.decodedBodySize]);}});
        observer.observe({type:'resource'});
      });
      await (await fetch(url)).text();return await entry;
    }''', resource(source, 'timing', key))
    observed['resource-timing'] = result
    return observed
