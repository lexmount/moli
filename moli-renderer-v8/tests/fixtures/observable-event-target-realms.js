(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  let observableRealm;
  const check = (ok, label) => { checks++; if (!ok) failures.push(label); };
  const frame = document.createElement('iframe');
  const loaded = () => new Promise(resolve => { frame.onload = resolve; });
  const firstLoad = loaded();
  // Start from a committed document so the next navigation replaces the
  // Window; initial about:blank reuse is a separate navigation policy.
  frame.srcdoc = '<!doctype html><title>first</title>';
  document.body.appendChild(frame);
  await firstLoad;
  try {
    const child = frame.contentWindow, events = [];
    const originalEvent = child.Event;
    check(child.document.title === 'first', 'first document committed');
    const when = EventTarget.prototype.when;
    const active = when.call(child, 'test');
    const pending = when.call(child, 'test');
    // The Observable draft does not yet specify the allocation realm for
    // when(). Record the engine choice separately from lifecycle assertions.
    observableRealm = {method: active instanceof Observable, target: active instanceof child.Observable};
    check(Object.prototype.toString.call(active) === '[object Observable]', 'native Observable brand');
    active.subscribe(event => events.push('active:' + event.type));
    dispatchEvent(new Event('test'));
    check(events.length === 0, 'borrowed method targets child Window');
    child.dispatchEvent(new child.Event('test'));
    check(events.join() === 'active:test', 'live child Window delivery');
    const replacement = loaded();
    frame.srcdoc = '<!doctype html><title>replacement</title>';
    await replacement;
    check(child.document.title === 'replacement' && child.Event !== originalEvent, 'replacement document and realm committed');
    check(frame.contentWindow === child, 'WindowProxy identity survives navigation');
    pending.subscribe(event => events.push('retired:' + event.type));
    child.dispatchEvent(new child.Event('test'));
    check(events.join() === 'active:test', 'old sources never register on replacement Window');
    const controller = new AbortController();
    when.call(child, 'test').subscribe(event => events.push('new:' + event.type), {signal: controller.signal});
    child.dispatchEvent(new child.Event('test'));
    check(events.join() === 'active:test,new:test', 'replacement Window has its own source');
    controller.abort();
    child.dispatchEvent(new child.Event('test'));
    check(events.length === 2, 'cross-realm subscription abort removes child listener');
    const detached = when.call(child, 'test');
    frame.remove();
    detached.subscribe(() => events.push('detached'));
    child.dispatchEvent(new child.Event('test'));
    check(events.length === 2, 'removed child cannot acquire an event stream listener');
  } catch (error) { failures.push('unexpected: ' + error.stack); }
  finally { frame.remove(); }
  return {checks, failures, observableRealm};
})()
