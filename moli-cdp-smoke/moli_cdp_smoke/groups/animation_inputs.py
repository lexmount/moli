"""Typed CSS animation inputs, notification snapshots and cross-document bindings."""
from __future__ import annotations

from typing import Any
from playwright.async_api import Page
from ..assertions import assert_equal

_ANIMATION_INPUT_SCRIPT = r"""async mode => {
  const box=document.body.appendChild(document.createElement('div'));
  const sheet=css=>document.head.appendChild(document.createElement('style')).textContent=css;
  const sample=(a,times,read)=>times.map(t=>{a.currentTime=t;return read(getComputedStyle(box))});
  if(mode.startsWith('logical-')) {
    const mixed=mode==='logical-mixed', rtl=mode==='logical-rtl', vertical=mode==='logical-vertical';
    const property=mode==='logical-inset'?'inset-inline-start':'margin-inline-start';
    const from=mixed?'from{margin-left:20px}':'';
    const css=mode==='logical-collision'?'from{margin-inline-start:10px;margin-left:20px}to{margin-inline-start:100px}':from+'to{'+property+':100px}';
    sheet('@keyframes move{'+css+'}');
    box.style.cssText='position:relative;margin-left:'+(mixed?'0':'20')+'px;margin-right:30px;margin-top:40px;left:20px;'+(rtl?'direction:rtl;':'')+(vertical?'writing-mode:vertical-lr;':'')+'animation:move 1s linear both paused';
    const a=box.getAnimations()[0];
    if(mode==='logical-writing-change'){
      a.currentTime=500;const before=parseFloat(getComputedStyle(box).marginInlineStart);
      box.style.direction='rtl';return[before,parseFloat(getComputedStyle(box).marginInlineStart)];
    }
    return sample(a,[0,500,1000],s=>parseFloat(mode==='logical-inset'?s.insetInlineStart:s.marginInlineStart));
  }
  if(mode.startsWith('name-')) {
    if(mode==='name-escaped'){
      sheet(String.raw`@keyframes \31 abc{from{opacity:0}to{opacity:1}}`);
      box.style.cssText=String.raw`animation:\31 abc 1s linear both paused`;
    }else{
      sheet('@keyframes "a,b"{from{opacity:0}to{opacity:1}}@keyframes "none"{from{opacity:0}to{opacity:1}}@keyframes plain{from{left:0px}to{left:100px}}');
      box.style.cssText='position:relative;animation-name:'+(mode==='name-quoted-none'?'none,"none",plain':'"a,b",plain')+';animation-duration:1s,2s,3s;animation-timing-function:linear;animation-fill-mode:both;animation-play-state:paused';
    }
    const animations=box.getAnimations();animations.forEach(a=>a.currentTime=500);
    return[animations.map(a=>[a.animationName,a.effect.getTiming().duration]),+getComputedStyle(box).opacity];
  }
  if(mode.startsWith('easing-')) {
    const keyword=mode.slice(7);
    sheet('@keyframes fade{from{opacity:0;animation-timing-function:'+keyword+'}to{opacity:1}}');
    box.style.cssText='animation:fade 1s linear both paused';
    box.getAnimations()[0].currentTime=500;return +getComputedStyle(box).opacity;
  }
  if(mode.startsWith('finish-')){
    const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});await a.ready;
    const done=new Promise(resolve=>a.onfinish=resolve);
    if(mode==='finish-explicit')a.finish();else a.currentTime=1000;
    a.currentTime=2000;
    const e=await done;return[e.currentTime,Number.isFinite(e.timelineTime),e.target===a];
  }
  if(mode.startsWith('adopt-')){
    await box.animate([],{duration:40}).finished;
    const frame=document.createElement('iframe');
    await new Promise(resolve=>{frame.onload=resolve;frame.srcdoc='<!doctype html><html><head></head><body></body></html>';document.body.append(frame)});
    const source=mode==='adopt-into-child'?document:frame.contentDocument;
    const target=mode==='adopt-into-child'?frame.contentDocument:document;
    const el=target.adoptNode(source.createElement('div'));target.body.append(el);
    target.head.appendChild(target.createElement('style')).textContent='@keyframes fade{from{opacity:0}to{opacity:1}}';
    el.style.animation='fade 100s linear both';
    const a=el.getAnimations()[0];
    const before=a.currentTime;a.timeline=a.timeline;const after=a.currentTime;
    return[a.timeline===target.timeline,a.timeline===source.timeline,Math.abs(after-before)<.01];
  }
  throw new Error(mode);
}"""

_ANIMATION_INPUT_EXPECTED = {'logical-missing': [20, 60, 100],
 'logical-mixed': [20, 60, 100],
 'logical-rtl': [30, 65, 100],
 'logical-vertical': [40, 70, 100],
 'logical-inset': [20, 60, 100],
 'logical-collision': [20, 60, 100],
 'logical-writing-change': [60, 65],
 'name-escaped': [[['1abc', 1000]], 0.5],
 'name-comma': [[['a,b', 1000], ['plain', 2000]], 0.5],
 'name-quoted-none': [[['none', 2000], ['plain', 3000]], 0.25],
 'easing-initial': 0.802403,
 'easing-unset': 0.802403,
 'easing-inherit': 0.802403,
 'finish-seek': [2000, True, True],
 'finish-explicit': [1000, True, True],
 'adopt-into-main': [True, False, True],
 'adopt-into-child': [True, False, True]}


async def run_animation_input_contracts(page: Page) -> dict[str, Any]:
    observed = {}
    for name, expected in _ANIMATION_INPUT_EXPECTED.items():
        await page.set_content('<!doctype html><html><head></head><body></body></html>')
        actual = await page.evaluate(_ANIMATION_INPUT_SCRIPT, name)
        assert_equal(actual, expected, f'Animation input contract {name}')
        observed[name] = actual
    return observed
