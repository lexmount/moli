(() => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const realms = [window, document.querySelector('iframe').contentWindow];
  const aliases = [
    ['BeforeUnloadEvent', 'BeforeUnloadEvent'], ['CompositionEvent', 'CompositionEvent'],
    ['CustomEvent', 'CustomEvent'], ['DeviceMotionEvent', 'DeviceMotionEvent'],
    ['DeviceOrientationEvent', 'DeviceOrientationEvent'], ['DragEvent', 'DragEvent'],
    ['Event', 'Event'], ['Events', 'Event'], ['FocusEvent', 'FocusEvent'],
    ['HashChangeEvent', 'HashChangeEvent'], ['HTMLEvents', 'Event'],
    ['KeyboardEvent', 'KeyboardEvent'], ['MessageEvent', 'MessageEvent'],
    ['MouseEvent', 'MouseEvent'], ['MouseEvents', 'MouseEvent'],
    ['StorageEvent', 'StorageEvent'], ['SVGEvents', 'Event'], ['TextEvent', 'TextEvent'],
    ['TouchEvent', 'TouchEvent'], ['UIEvent', 'UIEvent'], ['UIEvents', 'UIEvent']
  ];
  const rejected = (realm, operation, name) => {
    try { operation(); }
    catch (error) {
      return Object.getPrototypeOf(error) === realm.DOMException.prototype &&
        error.name === name;
    }
    return false;
  };
  const typeError = (realm, operation) => {
    try { operation(); }
    catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
    return false;
  };
  const defaults = event => event.type === '' && event.target === null &&
    event.currentTarget === null && event.eventPhase === 0 && !event.bubbles &&
    !event.cancelable && !event.composed && !event.defaultPrevented &&
    !event.isTrusted && Number.isFinite(event.timeStamp);
  for (const [ownerIndex, owner] of realms.entries()) {
    const docs = [owner.document, owner.document.implementation.createHTMLDocument(''),
      owner.document.implementation.createDocument('urn:legacy-event', 'root'),
      new owner.DOMParser().parseFromString('<root/>', 'application/xml')];
    for (const [docIndex, doc] of docs.entries()) {
      for (const [calleeIndex, callee] of realms.entries()) {
        const prefix = `${ownerIndex}/${docIndex}/${calleeIndex}/`;
        const factory = callee.Document.prototype.createEvent;
        const create = name => factory.call(doc, name);
        const exposed = name => typeof owner[name] === 'function' &&
          (name !== 'TouchEvent' || 'ontouchstart' in doc);
        check(prefix + 'factory metadata', () => {
          const d = Object.getOwnPropertyDescriptor(callee.Document.prototype, 'createEvent');
          return d.enumerable && d.configurable && d.writable &&
            factory.name === 'createEvent' && factory.length === 1;
        });
        for (const [alias, name] of aliases) {
          for (const variant of [alias, alias.toLowerCase(), alias.toUpperCase()]) {
            check(prefix + 'alias/defaults/' + variant, () => {
              if (!exposed(name)) return rejected(callee, () => create(variant), 'NotSupportedError');
              const event = create(variant);
              return Object.getPrototypeOf(event) === owner[name].prototype && defaults(event);
            });
          }
        }
        for (const name of ['DragEvent', 'HashChangeEvent', 'TouchEvent']) {
          const unavailable = () => rejected(callee, () => create(name), 'NotSupportedError');
          check(prefix + name + '/subclass state', () => {
            if (!exposed(name)) return unavailable();
            const event = create(name);
            if (name === 'HashChangeEvent') return event.oldURL === '' && event.newURL === '';
            if (name === 'DragEvent') return event.dataTransfer === null && event.view === null &&
              event.detail === 0 && event.clientX === 0 && event.clientY === 0 && !event.ctrlKey;
            return event.touches.length === 0 && event.targetTouches.length === 0 &&
              event.changedTouches.length === 0 && event.view === null && event.detail === 0;
          });
          check(prefix + name + '/native brand', () => {
            if (!exposed(name)) return unavailable();
            const event = create(name), key = name === 'HashChangeEvent' ? 'oldURL' :
              name === 'DragEvent' ? 'dataTransfer' : 'touches';
            const getter = Object.getOwnPropertyDescriptor(callee[name].prototype, key).get;
            const value = getter.call(event);
            return name === 'HashChangeEvent' ? value === '' :
              name === 'DragEvent' ? value === null : value.length === 0;
          });
          check(prefix + name + '/uninitialized dispatch rejected', () => {
            if (!exposed(name)) return unavailable();
            return rejected(owner, () => new owner.EventTarget().dispatchEvent(create(name)), 'InvalidStateError');
          });
          check(prefix + name + '/initialize and dispatch', () => {
            if (!exposed(name)) return unavailable();
            const event = create(name), target = new owner.EventTarget();
            let seen = 0, state = false;
            target.addEventListener('legacy', current => {
              seen++;
              state = current === event && current.target === target &&
                current.currentTarget === target && current.eventPhase === 2;
              current.preventDefault();
            });
            const initialized = callee.Event.prototype.initEvent.call(event, 'legacy', false, true);
            return initialized === undefined && target.dispatchEvent(event) === false &&
              seen === 1 && state && event.defaultPrevented && event.currentTarget === null &&
              Object.getPrototypeOf(event) === owner[name].prototype;
          });
          check(prefix + name + '/reinitialize retains subclass', () => {
            if (!exposed(name)) return unavailable();
            const event = create(name), target = new owner.EventTarget();
            event.initEvent('first', true, true);
            target.addEventListener('first', current => current.initEvent('during', false, false));
            target.dispatchEvent(event);
            if (event.type !== 'first' || !event.bubbles || !event.cancelable) return false;
            event.initEvent('second', false, false);
            return event.type === 'second' && event.target === null && !event.bubbles &&
              !event.cancelable && !event.defaultPrevented && Object.getPrototypeOf(event) === owner[name].prototype;
          });
        }
        for (const name of ['', 'TimeEvent', 'SubmitEvent', 'PointerEvent', 'WheelEvent',
          'SVGEvent', 'DragEvents', 'HashChangeEvents', 'TouchEvents', 'UIEvent ', ' UIEvent',
          'U\u0130Event', 'U\u0131Event', 'ErrorEvent', 'FormDataEvent', 'AnimationEvent']) {
          check(prefix + 'unsupported/' + name, () => rejected(callee, () => create(name), 'NotSupportedError'));
        }
        check(prefix + 'missing argument', () => typeError(callee, () => factory.call(doc)));
        check(prefix + 'conversion once', () => {
          let calls = 0;
          const event = create({toString() { calls++; return 'HashChangeEvent'; }});
          return calls === 1 && Object.getPrototypeOf(event) === owner.HashChangeEvent.prototype;
        });
        let traps = 0;
        const proxy = new owner.Proxy(doc, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const revoked = owner.Proxy.revocable(doc, {}); revoked.revoke();
        for (const [index, receiver] of [{}, Object.create(doc), Object.create(callee.Document.prototype),
          proxy, revoked.proxy, doc.createElement('div')].entries()) {
          check(prefix + 'receiver/' + index, () => {
            let conversions = 0;
            return typeError(callee, () => factory.call(receiver, {toString() { conversions++; throw 42; }})) &&
              conversions === 0 && traps === 0;
          });
        }
      }
    }
  }
  globalThis.__documentCreateEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  globalThis.__uiEventResults = globalThis.__documentCreateEventResults;
  return true;
})()
