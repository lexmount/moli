async function runAbortSignalEventProbe(scenario) {
  if (typeof document !== 'undefined' && document.readyState !== 'complete') {
    await new Promise(resolve => addEventListener('load', resolve, {once: true}));
  }
  // Let document lifecycle events finish before replacing the public constructor.
  await new Promise(resolve => setTimeout(resolve, 0));
  const errors = [];
  const events = [];
  let reads = 0;
  const check = (condition, label) => { if (!condition) errors.push(label); };
  const during = (event, signal, trusted, label) => {
    check(event.target === signal && event.srcElement === signal, label + ': target');
    check(event.currentTarget === signal && event.eventPhase === 2, label + ': dispatch fields');
    check(event.isTrusted === trusted, label + ': trust');
    const path = event.composedPath();
    check(path.length === 1 && path[0] === signal, label + ': path');
  };
  const after = (event, signal, label) => {
    check(event.target === signal, label + ': target retained');
    check(event.currentTarget === null && event.eventPhase === 0, label + ': dispatch fields cleared');
    check(event.composedPath().length === 0 && !event.cancelBubble, label + ': propagation cleared');
  };
  const throws = (callback, name, label) => {
    let caught = false;
    try { callback(); }
    catch (error) { caught = true; check(error.name === name, label + ': exception type'); }
    check(caught, label + ': throws');
  };

  try {
    if (scenario.startsWith('native-')) {
      const original = Object.getOwnPropertyDescriptor(globalThis, 'Event');
      const observed = [];
      const onerror = event => { errors.push('uncaught: ' + event.message); event.preventDefault(); };
      addEventListener('error', onerror);
      let deadline;
      try {
        if (scenario === 'native-getter') {
          Object.defineProperty(globalThis, 'Event', {configurable: true, get() {
            reads++; throw new Error('public Event getter');
          }});
        } else if (scenario === 'native-replacement') {
          Object.defineProperty(globalThis, 'Event', {configurable: true, value: function () {
            reads++; throw new Error('public Event constructor');
          }});
        } else if (scenario === 'native-missing') {
          delete globalThis.Event;
        }
        const record = (signal, label) => signal.addEventListener('abort', function (event) {
          events.push(label);
          observed.push([event, signal, label]);
          during(event, signal, true, label);
          check(this === signal && event.type === 'abort', label + ': callback');
          check(!event.bubbles && !event.cancelable && !event.composed, label + ': defaults');
          event.preventDefault();
          check(!event.defaultPrevented, label + ': uncancelable');
        });
        const controller = new AbortController();
        const composite = AbortSignal.any([controller.signal]);
        record(controller.signal, 'source');
        record(composite, 'composite');
        const reason = {cancel: true};
        controller.abort(reason);
        check(events.join(',') === 'source,composite', 'controller events are synchronous');
        check(composite.reason === reason, 'composite reason identity');
        const writable = new WritableStream({start(controller) { record(controller.signal, 'stream'); }});
        await writable.abort(reason);
        const timeout = AbortSignal.timeout(0);
        record(timeout, 'timeout');
        await new Promise(resolve => {
          deadline = setTimeout(() => { errors.push('timeout abort event missing'); resolve(); }, 500);
          timeout.onabort = () => { clearTimeout(deadline); resolve(); };
        });
        check(timeout.reason.name === 'TimeoutError', 'timeout reason');
        // Promise reactions can run at callback cleanup before native dispatch ends.
        await new Promise(resolve => setTimeout(resolve, 0));
      } finally {
        clearTimeout(deadline);
        Object.defineProperty(globalThis, 'Event', original);
        removeEventListener('error', onerror);
      }
      check(reads === 0, 'native event construction does not access public Event');
      check(new Set(observed.map(([event]) => event)).size === 4, 'each signal gets a distinct event');
      for (const [event, signal, label] of observed) {
        check(event instanceof Event, label + ': intrinsic prototype');
        check(event.isTrusted, label + ': stays trusted after native dispatch');
        after(event, signal, label);
      }
    } else if (scenario === 'script') {
      const source = new AbortController();
      const recipient = new AbortController();
      let nativeEvent;
      source.signal.addEventListener('abort', event => {
        nativeEvent = event;
        events.push('native');
        during(event, source.signal, true, 'native');
        throws(() => recipient.signal.dispatchEvent(event), 'InvalidStateError', 'active redispatch');
        during(event, source.signal, true, 'rejected redispatch preserves active event');
        event.initEvent('changed', true, true);
        check(event.type === 'abort' && !event.bubbles && !event.cancelable, 'active initEvent is ignored');
        event.stopImmediatePropagation();
      });
      source.signal.addEventListener('abort', () => errors.push('stopped listener fired'));
      source.abort('reason');
      after(nativeEvent, source.signal, 'native');
      recipient.signal.onabort = event => {
        events.push('replayed');
        during(event, recipient.signal, false, 'replayed');
        check(!recipient.signal.aborted && recipient.signal.reason === undefined, 'script event leaves signal pending');
        event.stopPropagation();
      };
      check(recipient.signal.dispatchEvent(nativeEvent), 'replayed abort is uncancelable');
      check(!nativeEvent.isTrusted, 'redispatch clears trust');
      after(nativeEvent, recipient.signal, 'replayed');
      const cancelable = new Event('probe', {cancelable: true});
      recipient.signal.addEventListener('probe', event => {
        events.push('passive');
        during(event, recipient.signal, false, 'synthetic');
        const before = event.defaultPrevented;
        event.preventDefault();
        check(event.defaultPrevented === before, 'passive cancellation is ignored');
      }, {passive: true});
      recipient.signal.addEventListener('probe', event => {
        events.push('active');
        event.preventDefault();
        event.stopImmediatePropagation();
      }, {once: true});
      recipient.signal.addEventListener('probe', () => events.push('tail'));
      check(!recipient.signal.dispatchEvent(cancelable), 'canceling a synthetic event returns false');
      after(cancelable, recipient.signal, 'synthetic');
      check(!recipient.signal.dispatchEvent(cancelable), 'redispatch preserves cancellation');
      after(cancelable, recipient.signal, 'synthetic again');
      const stopped = new Event('pre-stopped');
      recipient.signal.addEventListener('pre-stopped', () => events.push('pre-stopped'));
      stopped.stopPropagation();
      recipient.signal.dispatchEvent(stopped);
      check(!events.includes('pre-stopped'), 'pre-stopped event skips listeners');
      after(stopped, recipient.signal, 'pre-stopped');
      recipient.signal.dispatchEvent(stopped);
      check(events.includes('pre-stopped'), 'pre-stopped event can be dispatched again');
      for (const value of [null, undefined, {}, {type: 'abort'}, Object.create(Event.prototype)]) {
        throws(() => recipient.signal.dispatchEvent(value), 'TypeError', 'non-Event argument');
      }
      if (typeof document !== 'undefined') {
        throws(() => recipient.signal.dispatchEvent(document.createEvent('Event')), 'InvalidStateError', 'uninitialized Event');
      }
    } else {
      errors.push('unknown scenario ' + scenario);
    }
  } catch (error) {
    errors.push(String(error));
  }
  return {errors, events, reads};
}
