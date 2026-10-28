"""CSS rule selection and animation event/lifetime contracts shared by both engines."""
from __future__ import annotations

from typing import Any

from playwright.async_api import Page

from ..assertions import assert_equal


_RULE_EVENT_SCRIPT = r"""async mode => {
  const box = document.body.appendChild(document.createElement('div'));
  box.style.opacity = '.2';
  const frame = async html => {
    const f = document.createElement('iframe');
    await new Promise(resolve => {
      f.onload = resolve; f.srcdoc = html; document.body.append(f);
    });
    return f;
  };
  const frames = async () => {
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
  };
  if (mode.startsWith('css-finish-')) {
    const child = mode === 'css-finish-child' ? await frame('<!doctype html><html><head></head><body></body></html>') : null;
    const doc = child ? child.contentDocument : document;
    const target = child ? doc.body.appendChild(doc.createElement('div')) : box;
    doc.head.appendChild(doc.createElement('style')).textContent = '@keyframes fade{from{opacity:0}to{opacity:1}}';
    const timing = mode === 'css-finish-negative' ? '100ms linear -200ms both' : mode === 'css-finish-ordinary' ? '20ms linear both' : '0s linear both';
    target.style.animation = 'fade ' + timing;
    (child ? child.contentWindow : window).getComputedStyle(target).opacity;
    const a = target.getAnimations()[0];
    const finished = new Promise(resolve => a.onfinish = resolve);
    if (mode === 'css-finish-detached') { a.finish(); a.timeline = null; }
    const e = await finished;
    return [e.currentTime,Number.isFinite(e.timelineTime),e.target === a,e.isTrusted,a.timeline === null];
  }
  if (mode.startsWith('frame-group-')) {
    const rules = {
      'frame-group-properties': 'from{opacity:0;animation-timing-function:steps(1,end)}from{left:0px;animation-timing-function:linear}to{opacity:1;left:100px}',
      'frame-group-intervals': 'from{opacity:0}50%{opacity:.5;animation-timing-function:steps(1,end)}50%{opacity:.7;animation-timing-function:linear}to{opacity:1}',
      'frame-group-default': 'from{opacity:0}50%{opacity:.5}50%{opacity:.7;animation-timing-function:linear}to{opacity:1}',
      'frame-group-cascade': 'from{opacity:0}to{opacity:.4}to{opacity:1}',
      'frame-group-alias': 'from{opacity:0}50%{opacity:.5;animation-timing-function:step-end}50%{opacity:.7;animation-timing-function:steps(1,end)}to{opacity:1}',
    };
    document.head.appendChild(document.createElement('style')).textContent =
      '@keyframes mixed{' + rules[mode] + '}';
    box.style.cssText = 'position:relative;animation:mixed 1s linear both paused';
    const a = box.getAnimations()[0];
    const times = mode === 'frame-group-properties' || mode === 'frame-group-cascade' ? [500] : [250,750];
    return times.map(t => {
      a.currentTime = t; const style = getComputedStyle(box);
      return [+style.opacity,parseFloat(style.left) || 0];
    });
  }
  if (mode.startsWith('keyframes-')) {
    const rules = {
      'keyframes-print': '@media print {@keyframes fade{from,to{opacity:.8}}}',
      'keyframes-width': '@media (min-width:999999px) {@keyframes fade{from,to{opacity:.8}}}',
      'keyframes-supports': '@supports (bogus:bogus) {@keyframes fade{from,to{opacity:.8}}}',
      'keyframes-case': '@keyframes FADE{from,to{opacity:.8}}',
      'keyframes-active': '@media screen {@supports(opacity:.5){@keyframes fade{from,to{opacity:.8}}}}',
      'keyframes-layer': '@layer first, last; @layer last {@keyframes fade{from,to{opacity:.8}}} @layer first {@keyframes fade{from,to{opacity:.4}}}',
    };
    document.head.appendChild(document.createElement('style')).textContent =
      '@keyframes fade{from,to{opacity:.2}}' + rules[mode];
    box.style.animation = 'fade 1s linear both paused';
    const a = box.getAnimations()[0];
    a.currentTime = 500;
    return +getComputedStyle(box).opacity;
  }
  if (mode === 'borrowed-command-promises') {
    const f = await frame('<!doctype html><body></body>'), w = f.contentWindow;
    const a = new w.Animation(new w.KeyframeEffect(box, {opacity:[0,1]},1000), document.timeline);
    Animation.prototype.play.call(a); const ready = a.ready;
    Animation.prototype.finish.call(a); const finished = a.finished;
    Animation.prototype.pause.call(a); a.currentTime = 100; const renewed = a.finished;
    return [ready.constructor === w.Promise, renewed.constructor === w.Promise,
      ready.constructor === Promise, renewed === finished];
  }
  if (mode === 'creator-finish') {
    const f = await frame('<!doctype html><body></body>'), w = f.contentWindow;
    const a = new w.Animation(new w.KeyframeEffect(box, {opacity:[0,1]},
      {duration:1000,fill:'both'}), document.timeline);
    let finish = 0, listener = 0;
    a.onfinish = () => ++finish;
    a.addEventListener('finish', () => ++listener);
    a.pause(); a.currentTime = 500;
    f.remove(); await frames(); a.finish(); await frames();
    return [finish, listener, a.playState, a.currentTime];
  }
  if (mode === 'creator-promises-and-events') {
    const f = await frame('<!doctype html><body></body>'), w = f.contentWindow;
    const a = new w.Animation(new w.KeyframeEffect(box, {opacity:[0,1]},1000), document.timeline);
    let ready = 'pending', finished = 'pending', event = 0;
    a.onfinish = () => ++event;
    a.finished.then(() => finished = 'resolved', e => finished = e.name);
    a.pause(); a.ready.then(() => ready = 'resolved', e => ready = e.name);
    f.remove(); await frames(); a.finish(); await frames();
    return [ready,finished,event,a.playState,a.currentTime];
  }
  if (mode === 'css-readmission') {
    const f = await frame('<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#x{animation:fade 1s linear both paused}</style><div id=x></div>');
    const a = f.contentDocument.getElementById('x').getAnimations()[0];
    f.remove(); await frames();
    const idle = [a.playState, a.currentTime];
    a.effect.target = box; a.timeline = document.timeline;
    a.play(); a.pause(); a.currentTime = 500;
    return [idle, a.playState, a.currentTime, +getComputedStyle(box).opacity,
      box.getAnimations().length, a.effect.target === box];
  }
  if (mode === 'finish-snapshot') {
    const a = box.animate({opacity:[0,1]},{duration:1000,fill:'both'});
    const event = new Promise(resolve => a.onfinish = e => resolve(e));
    a.finish(); a.currentTime = 100; a.timeline = null;
    const e = await event;
    return [e.constructor.name, e instanceof AnimationPlaybackEvent,
      e.currentTime, Number.isFinite(e.timelineTime), e.target === a, e.isTrusted, a.timeline === null];
  }
  if (mode === 'cancel-property') {
    const a = box.animate({opacity:[0,1]},1000);
    let property = 0, listener = 0, snapshot;
    a.oncancel = e => { ++property; snapshot = [e.constructor.name,e.currentTime,e.timelineTime !== null]; };
    const event = new Promise(resolve => a.addEventListener('cancel', e => {++listener;resolve(e)}));
    a.cancel(); await event; await frames();
    return [property,listener,snapshot];
  }
  if (mode === 'event-constructor') {
    const e = new AnimationPlaybackEvent('finish',{currentTime:1000,timelineTime:5});
    const n = new AnimationPlaybackEvent('cancel');
    return [e instanceof Event,e.currentTime,e.timelineTime,n.currentTime,n.timelineTime];
  }
  throw Error(mode);
}"""

_RULE_EVENT_EXPECTED = {'keyframes-print': 0.2,
 'keyframes-width': 0.2,
 'keyframes-supports': 0.2,
 'keyframes-case': 0.2,
 'keyframes-active': 0.8,
 'keyframes-layer': 0.2,
 'frame-group-properties': [[0, 50]],
 'frame-group-intervals': [[0.25, 0], [0.85, 0]],
 'frame-group-default': [[0.35, 0], [0.85, 0]],
 'frame-group-cascade': [[0.5, 0]],
 'frame-group-alias': [[0.35, 0], [0.7, 0]],
 'css-finish-zero': [0, True, True, True, False],
 'css-finish-negative': [0, True, True, True, False],
 'css-finish-ordinary': [20, True, True, True, False],
 'css-finish-detached': [0, True, True, True, True],
 'css-finish-child': [0, True, True, True, False],
 'borrowed-command-promises': [True, True, False, False],
 'creator-finish': [1, 1, 'finished', 1000],
 'creator-promises-and-events': ['pending', 'pending', 1, 'finished', 1000],
 'css-readmission': [['idle', None], 'paused', 500, 0.5, 1, True],
 'finish-snapshot': ['AnimationPlaybackEvent', True, 1000, True, True, True, True],
 'cancel-property': [1, 1, ['AnimationPlaybackEvent', None, True]],
 'event-constructor': [True, 1000, 5, None, None]}


async def run_animation_rule_event_contracts(page: Page) -> dict[str, Any]:
    observed = {}
    for name, expected in _RULE_EVENT_EXPECTED.items():
        await page.set_content('<!doctype html><html><head></head><body></body></html>')
        actual = await page.evaluate(_RULE_EVENT_SCRIPT, name)
        assert_equal(actual, expected, f'Animation rule/event contract {name}')
        observed[name] = actual
    return observed
