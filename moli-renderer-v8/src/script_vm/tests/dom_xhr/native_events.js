async function checkNativeXhrEvents(config) {
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  frame.srcdoc = '<!doctype html><body>XHR event realm</body>';
  document.body.append(frame);
  await loaded;
  const realms = [self, frame.contentWindow];
  const receiver = realms[config.receiver];
  const methods = realms[config.methods].XMLHttpRequest.prototype;
  const add = realms[config.registration].EventTarget.prototype.addEventListener;
  const dispatch = receiver.EventTarget.prototype.dispatchEvent;
  const E = receiver.Event, P = receiver.ProgressEvent, D = receiver.DOMException;
  const xhr = new receiver.XMLHttpRequest(), upload = xhr.upload;
  const saved = ['Event', 'ProgressEvent'].map(name => [name, Object.getOwnPropertyDescriptor(receiver, name)]);
  const events = [], failures = [];
  let firstReady = null, synthetic = false, syntheticObserved = false;
  function check(value, message) { if (!value) failures.push(message); }
  const done = new Promise(resolve => xhr.onloadend = resolve);
  for (const target of [xhr, upload]) {
    for (const type of ['readystatechange', 'loadstart', 'progress', 'load', 'error', 'abort', 'loadend']) {
      if (target === upload && type === 'readystatechange') continue;
      add.call(target, type, event => {
        try {
          check(event instanceof E, type + ': Event brand in receiver realm');
          check(Object.getPrototypeOf(event) === (type === 'readystatechange' ? E.prototype : P.prototype), type + ': exact prototype');
          check(event.type === type, type + ': type');
          check(event.isTrusted === !synthetic, type + ': trust');
          check(event.target === target && event.currentTarget === target, type + ': dispatch target');
          check(event.eventPhase === 2, type + ': AT_TARGET');
          check(!event.bubbles && !event.cancelable && !event.composed, type + ': event flags');
          check(Number.isFinite(event.timeStamp), type + ': timestamp');
          const path = E.prototype.composedPath.call(event);
          check(path.length === 1 && path[0] === target, type + ': path');
          E.prototype.preventDefault.call(event);
          check(!event.defaultPrevented, type + ': noncancelable');
          if (synthetic) { syntheticObserved = true; return; }
          events.push({type, event, target});
          if (type === 'readystatechange' && !firstReady) {
            firstReady = event;
            let error;
            try { dispatch.call(xhr, event); } catch (caught) { error = caught; }
            check(error instanceof D && error.name === 'InvalidStateError', 'recursive dispatch rejection');
            check(event.isTrusted, 'failed recursive dispatch preserves trust');
          }
        } catch (error) { failures.push(type + ': ' + error); }
      });
    }
  }
  try {
    if (config.tamper !== 'none') {
      for (const name of ['Event', 'ProgressEvent']) {
        const trap = () => { throw new Error('author ' + name + ' must not run'); };
        Object.defineProperty(receiver, name, config.tamper === 'getter'
          ? {get:trap, configurable:true}
          : {value:function() { return trap(); }, writable:true, configurable:true});
      }
    }
    methods.open.call(xhr, config.flow === 'post' ? 'POST' : 'GET', config.url);
    methods.send.call(xhr, config.flow === 'post' ? 'payload' : null);
    if (config.flow === 'abort') xhr.abort();
    await done;
    const terminal = config.flow === 'error' ? 'error' : config.flow === 'abort' ? 'abort' : 'load';
    check(firstReady !== null, 'readystatechange delivered');
    check(events.some(record => record.target === xhr && record.type === 'loadstart'), 'download loadstart delivered');
    check(events.some(record => record.target === xhr && record.type === terminal), 'terminal event delivered');
    for (const {event, type, target} of events) {
      check(event.currentTarget === null && event.eventPhase === 0, type + ': dispatch cleanup');
      check(event.target === target && event.isTrusted, type + ': retained event');
      check(E.prototype.composedPath.call(event).length === 0, type + ': retained path cleanup');
      if (type !== 'readystatechange') {
        check(event instanceof P, type + ': ProgressEvent brand');
        check(typeof event.loaded === 'number' && typeof event.total === 'number', type + ': progress fields');
      }
    }
    if (config.flow === 'post') {
      const uploaded = events.find(record => record.target === upload && record.type === 'load');
      check(uploaded && uploaded.event.loaded === 7 && uploaded.event.total === 7 && uploaded.event.lengthComputable, 'native upload payload');
    }
    synthetic = true;
    dispatch.call(xhr, firstReady);
    check(syntheticObserved && !firstReady.isTrusted, 'script redispatch clears trust');
    check(new E('script').isTrusted === false && new P('script').isTrusted === false, 'script constructors remain untrusted');
    if (failures.length) throw new Error(failures.join('; '));
  } finally {
    for (const [name, descriptor] of saved) Object.defineProperty(receiver, name, descriptor);
    frame.remove();
  }
}
