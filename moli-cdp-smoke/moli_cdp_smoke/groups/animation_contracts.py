"""Independent playback sequences and style invariants shared by both engines."""
from __future__ import annotations

from typing import Any

from playwright.async_api import Page

from ..assertions import assert_equal


# Ready expectations follow the Web Animations play/pause/seek/cancel algorithms.
# Identity numbers are assigned on observation, not read from engine internals.
_COMMANDS = {'idle-pause': {'records': [['new', False, 'idle', 'null', 0],
                            ['pause', True, 'paused', 'number', 1],
                            ['microtask', True, 'paused', 'number', 1],
                            ['ready', False, 'paused', 'number', 1]],
                'statuses': ['resolved', 'resolved']},
 'null-pause': {'records': [['new', False, 'idle', 'null', 0],
                            ['pause', True, 'paused', 'number', 1],
                            ['microtask', True, 'paused', 'number', 1],
                            ['seek', False, 'paused', 'number', 1],
                            ['ready', False, 'paused', 'number', 1]],
                'statuses': ['resolved', 'resolved']},
 'play-pause-seek': {'records': [['new', False, 'idle', 'null', 0],
                                 ['play', True, 'running', 'number', 1],
                                 ['pause', True, 'paused', 'number', 1],
                                 ['seek', False, 'paused', 'number', 1],
                                 ['ready', False, 'paused', 'number', 1]],
                     'statuses': ['resolved', 'resolved']},
 'pause-play': {'records': [['new', False, 'idle', 'null', 0],
                            ['pause', True, 'paused', 'number', 1],
                            ['play', True, 'running', 'number', 1],
                            ['ready', False, 'running', 'number', 1]],
                'statuses': ['resolved', 'resolved']},
 'play-pause-cancel': {'records': [['new', False, 'idle', 'null', 0],
                                   ['play', True, 'running', 'number', 1],
                                   ['pause', True, 'paused', 'number', 1],
                                   ['cancel', False, 'idle', 'null', 2],
                                   ['microtask', False, 'idle', 'null', 2]],
                       'statuses': ['resolved', 'AbortError']},
 'running-pause': {'records': [['new', False, 'idle', 'null', 0],
                               ['running', False, 'running', 'number', 1],
                               ['pause', True, 'paused', 'number', 2],
                               ['pause-again', True, 'paused', 'number', 2],
                               ['microtask', True, 'paused', 'number', 2],
                               ['ready', False, 'paused', 'number', 2]],
                   'statuses': ['resolved', 'resolved', 'resolved']},
 'null-pause-seek': {'records': [['new', False, 'idle', 'null', 0],
                                 ['pause', True, 'paused', 'number', 1],
                                 ['seek', False, 'paused', 'number', 1],
                                 ['ready', False, 'paused', 'number', 1]],
                     'statuses': ['resolved', 'resolved']},
 'play-pause-finish': {'records': [['new', False, 'idle', 'null', 0],
                                   ['play', True, 'running', 'number', 1],
                                   ['pause', True, 'paused', 'number', 1],
                                   ['finish', False, 'finished', 'number', 1],
                                   ['ready', False, 'finished', 'number', 1]],
                       'statuses': ['resolved', 'resolved']},
 'pause-rate': {'records': [['new', False, 'idle', 'null', 0],
                            ['play', True, 'running', 'number', 1],
                            ['pause', True, 'paused', 'number', 1],
                            ['rate', False, 'paused', 'number', 1],
                            ['ready', False, 'paused', 'number', 1]],
                'statuses': ['resolved', 'resolved']},
 'null-pause-start': {'records': [['new', False, 'idle', 'null', 0],
                                  ['pause', True, 'paused', 'number', 1],
                                  ['start', False, 'running', 'null', 1],
                                  ['ready', False, 'running', 'null', 1]],
                      'statuses': ['resolved', 'resolved']},
 'null-play-finish': {'records': [['new', False, 'idle', 'null', 0],
                                  ['pending', True, 'running', 'number', 1],
                                  ['finish', True, 'finished', 'number', 1],
                                  ['attached', True, 'finished', 'number', 1],
                                  ['ready', False, 'finished', 'number', 1]],
                      'statuses': ['resolved', 'resolved']},
 'null-pause-finish': {'records': [['new', False, 'idle', 'null', 0],
                                   ['pending', True, 'paused', 'number', 1],
                                   ['finish', True, 'paused', 'number', 1],
                                   ['attached', True, 'paused', 'number', 1],
                                   ['ready', False, 'paused', 'number', 1]],
                       'statuses': ['resolved', 'resolved']}}

_COMMAND_SCRIPT = r"""
async mode => {
  const el = document.body.appendChild(document.createElement('div'));
  const effect = new KeyframeEffect(el, {opacity:[0,1]}, {duration:10000,fill:'both'});
  const a = new Animation(effect, mode.startsWith('null-') ? null : document.timeline);
  a.finished.catch(() => {});
  const records = [], promises = [];
  const capture = label => {
    const ready = a.ready;
    let id = promises.indexOf(ready);
    if (id < 0) id = promises.push(ready) - 1;
    ready.catch(() => {});
    records.push([label, a.pending, a.playState, a.currentTime === null ? 'null' : 'number', id]);
  };
  capture('new');
  switch (mode) {
    case 'idle-pause':
    case 'null-pause':
      a.pause(); capture('pause');
      await Promise.resolve(); capture('microtask');
      if (mode === 'null-pause') { a.currentTime = 500; capture('seek'); }
      await a.ready; capture('ready');
      break;
    case 'play-pause-seek':
      a.play(); capture('play');
      a.pause(); capture('pause');
      a.currentTime = 500; capture('seek');
      await a.ready; capture('ready');
      break;
    case 'pause-play':
      a.pause(); capture('pause');
      a.play(); capture('play');
      await a.ready; capture('ready');
      break;
    case 'play-pause-cancel':
      a.play(); capture('play');
      a.pause(); capture('pause');
      a.cancel(); capture('cancel');
      await Promise.resolve(); capture('microtask');
      break;
    case 'running-pause':
      a.play(); await a.ready; capture('running');
      a.pause(); capture('pause');
      a.pause(); capture('pause-again');
      await Promise.resolve(); capture('microtask');
      await a.ready; capture('ready');
      break;
    case 'null-pause-seek':
      a.pause(); capture('pause');
      a.currentTime = 500; capture('seek');
      await a.ready; capture('ready');
      break;
    case 'pause-rate':
      a.currentTime = 500;
      a.play(); capture('play');
      a.pause(); capture('pause');
      a.playbackRate = 2; capture('rate');
      await a.ready; capture('ready');
      break;
    case 'null-pause-start':
      a.pause(); capture('pause');
      a.startTime = 0; capture('start');
      await a.ready; capture('ready');
      break;
    case 'null-play-finish':
    case 'null-pause-finish':
      if (mode === 'null-play-finish') a.play(); else a.pause();
      capture('pending');
      a.finish(); capture('finish');
      a.timeline = document.timeline; capture('attached');
      await a.ready; capture('ready');
      break;
    case 'play-pause-finish':
      a.play(); capture('play');
      a.pause(); capture('pause');
      a.finish(); capture('finish');
      await a.ready; capture('ready');
      break;
  }
  // This contract checks cancellation of the old operation and replacement
  // identity. Replacement ready settlement is covered separately against WPT.
  const watched = mode === 'play-pause-cancel' ? promises.slice(0,-1) : promises;
  const statuses = await Promise.all(watched.map(p => p.then(() => 'resolved', e => e.name)));
  a.cancel(); el.remove();
  return {records, statuses};
}
"""

_PROJECTION_SCRIPT = r"""
() => {
  const make = () => {
    const el = document.body.appendChild(document.createElement('div'));
    el.style.cssText = 'font-size:10px;width:2em;opacity:.2';
    return el;
  };
  const sample = (el, frames, time = 500) => {
    const a = el.animate(frames, {duration:1000, fill:'both'});
    a.pause(); a.currentTime = time;
    return a;
  };
  const fontOrders = [];
  const marginOrders = [];
  for (const reverse of [false, true]) {
    const el = make();
    const entries = [['width',['1em','3em']], ['font',['10px serif','30px serif']]];
    if (reverse) entries.reverse();
    const a = sample(el,Object.fromEntries(entries));
    const cs = getComputedStyle(el);
    fontOrders.push([parseFloat(cs.fontSize),parseFloat(cs.width)]);
    a.cancel(); el.remove();
    const margin = make();
    const margins = [['marginLeft',['20px','40px']], ['margin',['0px','100px']]];
    if (reverse) margins.reverse();
    const b = sample(margin,Object.fromEntries(margins));
    marginOrders.push(parseFloat(getComputedStyle(margin).marginLeft));
    b.cancel(); margin.remove();
  }
  const el = make();
  const lower = sample(el,{fontSize:['10px','30px'],width:['1em','3em']});
  const higher = sample(el,{fontSize:['100px','300px']});
  const before = parseFloat(getComputedStyle(el).width);
  lower.commitStyles();
  const committed = [parseFloat(el.style.fontSize),parseFloat(el.style.width)];
  higher.cancel(); lower.cancel();
  const stackCommit = [before,committed,parseFloat(getComputedStyle(el).width)];
  el.remove();
  const endpoints = [];
  for (const time of [999,1000,1001]) {
    const el = make();
    const a = el.animate({opacity:[0,1]},1000);
    a.pause(); a.currentTime = time; a.commitStyles();
    endpoints.push(parseFloat(el.style.opacity));
    a.cancel(); el.remove();
  }
  const style = document.head.appendChild(document.createElement('style'));
  style.textContent = '@keyframes contractFixed {from,to {opacity:1}} #contractOrigin::before {content:"X";font-size:inherit;color:inherit}';
  const admitted = make();
  admitted.style.cssText = 'opacity:0;transition:opacity 100s linear';
  getComputedStyle(admitted).opacity;
  admitted.style.animation = 'contractFixed 100s linear both paused';
  const transitionAdmission = [parseFloat(getComputedStyle(admitted).opacity),admitted.getAnimations().map(a=>a.constructor.name)];
  admitted.getAnimations().forEach(a=>a.cancel()); admitted.remove();
  const origin = make(); origin.id = 'contractOrigin'; origin.style.color = 'black';
  const a = sample(origin,{fontSize:['10px','30px'],color:['black','rgb(200,0,0)']});
  const primary = getComputedStyle(origin), pseudo = getComputedStyle(origin,'::before');
  const pseudoInheritance = [parseFloat(primary.fontSize),primary.color,parseFloat(pseudo.fontSize),pseudo.color];
  a.cancel(); origin.remove(); style.remove();
  return {fontOrders,marginOrders,stackCommit,endpoints,transitionAdmission,pseudoInheritance};
}
"""

_PROJECTION = {'fontOrders': [[20, 40], [20, 40]],
 'marginOrders': [30, 30],
 'stackCommit': [400, [20, 40], 40],
 'endpoints': [0.999, 1, 0.2],
 'transitionAdmission': [1, ['CSSAnimation']],
 'pseudoInheritance': [20, 'rgb(100, 0, 0)', 20, 'rgb(100, 0, 0)']}


_CANCEL_HANDLED_SCRIPT = r"""
async () => {
  const unhandled = [], ordinary = new Error('ordinary');
  const onUnhandled = event => {
    unhandled.push(event.reason === ordinary ? 'ordinary' : event.reason.name);
    event.preventDefault();
  };
  window.addEventListener('unhandledrejection', onUnhandled);
  const el = document.body.appendChild(document.createElement('div'));
  const a = el.animate({opacity:[0,1]}, 1000);
  const promises = [a.ready, a.finished];
  a.cancel();
  Promise.reject(ordinary);
  await new Promise(requestAnimationFrame);
  await new Promise(requestAnimationFrame);
  const rejected = await Promise.all(promises.map(p => p.then(() => 'resolved', e => e.name)));
  window.removeEventListener('unhandledrejection', onUnhandled);
  el.remove();
  return {unhandled, rejected};
}
"""


async def run_animation_command_contracts(page: Page) -> dict[str, Any]:
    observed = {}
    for name, expected in _COMMANDS.items():
        actual = await page.evaluate(_COMMAND_SCRIPT, name)
        assert_equal(actual, expected, f"Animation command sequence {name}")
        observed[name] = actual
    notifications = await page.evaluate(_CANCEL_HANDLED_SCRIPT)
    assert_equal(
        notifications,
        {"unhandled": ["ordinary"], "rejected": ["AbortError", "AbortError"]},
        "Cancellation preserves rejection results and marks its promises handled",
    )
    observed["cancellationNotifications"] = notifications
    return observed


async def run_animation_projection_contracts(page: Page) -> dict[str, Any]:
    observed = await page.evaluate(_PROJECTION_SCRIPT)
    assert_equal(observed, _PROJECTION, "Animation dependency and sampling contexts")
    return observed


_BOUNDARY_SCRIPT = r"""
async mode => {
  const el = document.body.appendChild(document.createElement('div'));
  el.style.cssText = 'opacity:.2;margin:0';
  const frameBarrier = () => new Promise(requestAnimationFrame);
  const observe = promise => {
    const result = {status:'pending'};
    promise.then(() => result.status='resolved', e => result.status=e.name);
    return result;
  };
  let result, a, frame, style;
  if (mode === 'cancel-replay-css-cancel') {
    style = document.head.appendChild(document.createElement('style'));
    style.textContent = '@keyframes contractCancel {from {opacity:0} to {opacity:1}}';
    el.style.animation = 'contractCancel 100s linear both paused';
    a = el.getAnimations()[0];
    await a.ready;
    let cancels = 0;
    a.addEventListener('cancel', () => cancels++);
    a.cancel(); a.play();
    const ready = observe(a.ready), finished = observe(a.finished);
    el.style.animation = 'none';
    getComputedStyle(el).opacity;
    await frameBarrier(); await frameBarrier();
    result = [ready.status, finished.status, a.playState, a.pending, cancels];
  } else if (mode === 'variable-shorthand') {
    el.style.setProperty('--m','10px 20px');
    a = el.animate({margin:['var(--m)','var(--m)']}, {duration:1000,fill:'both'});
    a.pause(); a.currentTime = 500;
    const margins = () => {
      const cs = getComputedStyle(el);
      return [cs.marginTop,cs.marginRight,cs.marginBottom,cs.marginLeft].map(parseFloat);
    };
    result = [margins()];
    for (const value of ['30px','30px 50px']) {
      el.style.setProperty('--m',value); result.push(margins());
    }
  } else if (mode === 'css-duplicate-offset') {
    style = document.head.appendChild(document.createElement('style'));
    style.textContent = '@keyframes contractDuplicate {from {opacity:0} to {opacity:.4} to {opacity:1}}';
    el.style.animation = 'contractDuplicate 1s linear both paused';
    a = el.getAnimations()[0]; a.currentTime = 500;
    const css = parseFloat(getComputedStyle(el).opacity);
    a.cancel(); el.style.animation = 'none';
    a = el.animate([{offset:0,opacity:0},{offset:1,opacity:.4},{offset:1,opacity:1}], {duration:1000,fill:'both'});
    a.pause(); a.currentTime = 500;
    result = [css,parseFloat(getComputedStyle(el).opacity)];
  } else {
    frame = document.body.appendChild(document.createElement('iframe'));
    await new Promise(resolve => { frame.onload = resolve; frame.srcdoc = '<!doctype html><body></body>'; });
    if (mode === 'creator-document-retirement') {
      const w = frame.contentWindow;
      a = new w.Animation(new w.KeyframeEffect(el, {opacity:[0,1]}, {duration:1000,fill:'both'}), document.timeline);
      a.pause(); a.currentTime = 500;
      frame.remove();
      await frameBarrier(); await frameBarrier();
      result = [a.playState,a.currentTime,parseFloat(getComputedStyle(el).opacity),el.getAnimations().length,a.effect.target===el,a.timeline===document.timeline];
    } else {
      a = new Animation(new KeyframeEffect(el,{opacity:[0,1]},1000),frame.contentDocument.timeline);
      a.pause(); const ready = observe(a.ready);
      frame.remove();
      const before = [a.currentTime,a.pending,a.playState,a.timeline.currentTime];
      a.playbackRate = 2;
      await Promise.resolve();
      result = [before,a.currentTime,a.pending,a.playState,ready.status];
    }
  }
  a.cancel(); el.remove(); frame?.remove(); style?.remove();
  return result;
}
"""

_BOUNDARIES = {
    'cancel-replay-css-cancel': ['AbortError','AbortError','idle',False,2],
    'variable-shorthand': [[10,20,10,20],[30,30,30,30],[30,50,30,50]],
    'css-duplicate-offset': [0.5,0.2],
    'creator-document-retirement': ['paused',500,0.5,1,True,True],
    'inactive-timeline-pause-rate': [[0,True,'paused',None],0,False,'paused','resolved'],
}


async def run_animation_boundary_contracts(page: Page) -> dict[str, Any]:
    observed = {}
    # Chrome 145 severs the CSS association on cancel()/play(), unlike 156.
    # That command sequence has a renderer regression and a separate 156
    # differential probe. These four contracts have the same independent
    # reference in both supported Chromium versions; neither engine skips them.
    for name in ('variable-shorthand', 'css-duplicate-offset',
                 'creator-document-retirement', 'inactive-timeline-pause-rate'):
        expected = _BOUNDARIES[name]
        actual = await page.evaluate(_BOUNDARY_SCRIPT, name)
        assert_equal(actual, expected, f'Animation boundary contract {name}')
        observed[name] = actual
    return observed
