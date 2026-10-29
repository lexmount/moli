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
  if(mode.startsWith('custom-')){
    box.style.cssText='--phase:base;--w:20px;width:var(--w)';
    let a;
    if(mode==='custom-css'){
      sheet('@keyframes custom{from{--phase:old}to{--phase:new}}');box.style.animation='custom 1s linear both paused';a=box.getAnimations()[0];
    }else{
      if(mode==='custom-universal')CSS.registerProperty({name:'--phase',syntax:'*',inherits:true,initialValue:'old'});
      a=box.animate(mode==='custom-dependent'?{'--w':['0px','100px']}:[{'--phase':'old'},{'--phase':'new'}],{duration:1000,fill:'both'});a.pause();
    }
    return sample(a,[0,250,500,1000],s=>mode==='custom-dependent'?parseFloat(s.width):s.getPropertyValue('--phase').trim());
  }
  if(mode==='css-typed-timing'){
    sheet('@keyframes "a,b"{from{opacity:0}to{opacity:1}}@keyframes plain{from{left:0px}to{left:100px}}');
    box.style.cssText='position:relative;animation-name:none,"a,b",plain;animation-duration:.25s,.5s;animation-delay:-.125s,.25s;animation-iteration-count:1.5,2;animation-fill-mode:backwards,both;animation-direction:alternate,reverse;animation-timing-function:steps(2,end),linear;animation-play-state:running,paused';
    const animations=box.getAnimations();
    const timings=animations.map(a=>{const t=a.effect.getTiming();return[a.animationName,t.duration,t.delay,t.iterations,t.fill,t.direction,t.easing,a.playState]});
    animations[0].currentTime=500;animations[1].currentTime=-62.5;
    const style=getComputedStyle(box);return[timings,+style.opacity,parseFloat(style.left)];
  }
  if(mode.startsWith('css-read-')){
    if(mode==='css-read-frame'){
      const frame=document.createElement('iframe');
      await new Promise(resolve=>{frame.onload=resolve;frame.srcdoc='<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}</style><div id=owner></div>';document.body.append(frame)});
      const owner=frame.contentDocument.getElementById('owner');
      owner.style.animation='fade 1s linear both paused';
      const a=owner.getAnimations()[0],effect=a.effect;effect.target=box;
      owner.style.animationDuration='2s';const duration=KeyframeEffect.prototype.getTiming.call(effect).duration;
      owner.style.animationName='none';return[duration,a.playState,effect===a.effect];
    }
    sheet('@keyframes fade{from{opacity:0}to{opacity:1}}');
    box.style.animation='fade 1s linear both paused';
    const a=box.getAnimations()[0],effect=a.effect;
    if(mode==='css-read-state'){
      box.style.animationPlayState='running';const running=a.playState;
      box.style.animationName='none';return[running,a.playState];
    }
    if(mode==='css-read-retarget')effect.target=document.body.appendChild(document.createElement('div'));
    if(mode==='css-read-null')effect.target=null;
    box.style.animationDuration='2s';const duration=effect.getTiming().duration;
    box.style.animationName='none';return[duration,a.playState,effect===a.effect];
  }
  if(mode==='css-timing-override'){
    sheet('@keyframes fade{from{opacity:0}to{opacity:1}}');
    box.style.animation='fade 1s linear both paused';
    const a=box.getAnimations()[0],effect=a.effect;
    effect.updateTiming({duration:3000});box.style.animationDuration='2s';
    return[effect.getTiming().duration,effect.getComputedTiming().activeDuration];
  }
  if(mode.startsWith('orphan-order-')){
    sheet('@keyframes fixed{from,to{opacity:.8}}');
    const retarget=mode.endsWith('-retarget');
    const target=retarget?document.body.appendChild(document.createElement('div')):box;
    const makeScript=()=>{const a=target.animate({opacity:[.2,.2]},{duration:1000,fill:'both'});a.pause();a.currentTime=500;return a};
    const script=makeScript();
    box.style.animation='fixed 1s linear both paused';
    const retained=box.getAnimations().find(a=>a instanceof CSSAnimation);
    box.style.animation='none';getComputedStyle(box).opacity;
    if(retarget)retained.effect.target=target;
    retained.play();retained.pause();retained.currentTime=500;
    const label=a=>a===retained?'retained':a===script?'script':'later';
    const replay=[+getComputedStyle(target).opacity,target.getAnimations().map(label)];
    const later=makeScript();
    retained.cancel();retained.play();retained.pause();retained.currentTime=500;
    return[retained instanceof CSSAnimation,replay,+getComputedStyle(target).opacity,target.getAnimations().map(a=>a===later?'later':label(a))];
  }
  if(mode==='commit-styles-lower-endpoint'){
    const results=[];
    for(const time of [999,1000,1001]){
      box.style.opacity='.2';
      const lower=box.animate({opacity:[0,1]},1000);lower.pause();lower.currentTime=time;
      const upper=box.animate({opacity:['.8']},{duration:2000,fill:'both'});upper.pause();upper.currentTime=1000;
      const higher=box.animate({opacity:[.1,.1]},{duration:1000,fill:'both'});higher.pause();higher.currentTime=500;
      upper.commitStyles();const committed=+box.style.opacity;
      lower.cancel();upper.cancel();higher.cancel();
      results.push([committed,+getComputedStyle(box).opacity]);
    }
    return results;
  }
  if(mode==='zero-rate-play-ready'){
    const out=[];
    for(const rate of [0,1]){
      const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});await a.ready;
      a.playbackRate=rate;a.currentTime=500;
      const oldReady=a.ready;a.play();
      const state=[a.ready===oldReady,a.pending,a.startTime===null,a.currentTime];
      const result=a.ready.then(()=> 'resolved',error=>error.name);
      a.cancel();out.push([state,await result]);
    }
    return out;
  }
  if(mode==='position-equivalence'){
    const pairs=[];
    for(const rate of [1,2,-1,-2])for(const local of [-500,0,500,1000,2000]){
      const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});
      const b=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});
      await Promise.all([a.ready,b.ready]);
      a.playbackRate=rate;b.playbackRate=rate;
      a.currentTime=local;b.startTime=document.timeline.currentTime-local/rate;
      pairs.push([Math.round(a.currentTime),Math.round(b.currentTime),a.playState===b.playState]);
      a.cancel();b.cancel();
    }
    return pairs;
  }
  if(mode.startsWith('start-')){
    const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});await a.ready;
    const done=new Promise(resolve=>a.onfinish=resolve);
    if(mode==='start-reverse'){
      a.playbackRate=-1;a.currentTime=500;a.startTime=document.timeline.currentTime-500;
    }else{
      a.startTime=document.timeline.currentTime-2000;
      if(mode==='start-then-seek')a.currentTime=2500;
      if(mode==='start-then-finish'){a.finish();a.currentTime=2500;}
    }
    const current=Math.round(a.currentTime),state=a.playState,event=await done;
    return[current,state,Math.round(event.currentTime),Number.isFinite(event.timelineTime)];
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
 'adopt-into-child': [True, False, True],
 'custom-css': ['old', 'old', 'new', 'new'],
 'custom-script': ['old', 'old', 'new', 'new'],
 'custom-dependent': [0, 0, 100, 100],
 'custom-universal': ['old', 'old', 'new', 'new'],
 'css-typed-timing': [[['a,b', 500, 250, 2, 'both', 'reverse', 'linear', 'paused'],
                       ['plain', 250, -125, 1.5, 'backwards', 'alternate', 'linear', 'running']], 0.5, 0],
 'css-read-timing': [2000, 'idle', True],
 'css-read-retarget': [2000, 'idle', True],
 'css-read-null': [2000, 'idle', True],
 'css-read-state': ['running', 'idle'],
 'css-read-frame': [2000, 'idle', True],
 'css-timing-override': [3000, 3000],
 # Renderer tests also cover CSS-first admission, where Chromium 156 keeps its
 # creation sequence instead of the CSS Animations 2 global-list admission order.
 'orphan-order-script-first': [True, [0.8, ['script', 'retained']], 0.2, ['script', 'retained', 'later']],
 'orphan-order-script-first-retarget': [True, [0.8, ['script', 'retained']], 0.2, ['script', 'retained', 'later']],
 'commit-styles-lower-endpoint': [[0.8995, 0.8995], [0.5, 0.5], [0.5, 0.5]],
 'zero-rate-play-ready': [[[False, True, True, 500], 'AbortError'], [[True, False, False, 500], 'resolved']],
 'position-equivalence': [[local, local, True] for rate in (1, 2, -1, -2) for local in (-500, 0, 500, 1000, 2000)],
 'start-forward': [2000, 'finished', 2000, True],
 'start-reverse': [-500, 'finished', -500, True],
 'start-then-seek': [2500, 'finished', 2500, True],
 'start-then-finish': [2500, 'finished', 1000, True]}


async def run_animation_input_contracts(page: Page) -> dict[str, Any]:
    observed = {}
    for name, expected in _ANIMATION_INPUT_EXPECTED.items():
        await page.set_content('<!doctype html><html><head></head><body></body></html>')
        actual = await page.evaluate(_ANIMATION_INPUT_SCRIPT, name)
        assert_equal(actual, expected, f'Animation input contract {name}')
        observed[name] = actual
    return observed
