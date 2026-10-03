(() => {
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: !!passed});
  const child = document.querySelector('#child').contentWindow;
  const realms = [window, child];
  const properties = ['type', 'bubbles', 'cancelable', 'composed', 'isTrusted',
    'defaultPrevented', 'returnValue', 'cancelBubble', 'target', 'currentTarget',
    'eventPhase', 'timeStamp', 'view', 'detail', 'screenX', 'screenY', 'clientX',
    'clientY', 'button', 'buttons', 'relatedTarget', 'ctrlKey', 'altKey',
    'shiftKey', 'metaKey', 'movementX', 'movementY'];
  const snapshot = event => properties.map(name => event[name]);
  const unchanged = (event, before) => snapshot(event).every((value, index) =>
    Object.is(value, before[index]));
  const caught = fn => { try { fn(); } catch (error) { return error; } };
  const makers = ['MouseEvent', 'WheelEvent', 'PointerEvent', 'DragEvent', 'createEvent'];
  const seed = realm => ({bubbles: false, cancelable: true, composed: true,
    view: realm, detail: 7, screenX: 11, screenY: 12, clientX: 13, clientY: 14,
    button: 2, buttons: 9, movementX: 21, movementY: 22, ctrlKey: true,
    relatedTarget: realm.document.body});
  for (let methodRealm = 0; methodRealm < realms.length; methodRealm++) {
    const callee = realms[methodRealm];
    const method = callee.MouseEvent.prototype.initMouseEvent;
    for (let receiverRealm = 0; receiverRealm < realms.length; receiverRealm++) {
      const realm = realms[receiverRealm];
      const detached = realm.document.implementation.createHTMLDocument('');
      const nativeProxy = detached.createElement('select');
      for (const kind of makers) {
        const prefix = `${methodRealm}/${receiverRealm}/${kind}: `;
        const make = () => {
          if (kind !== 'createEvent') return new realm[kind]('before', seed(realm));
          const event = realm.document.createEvent('MouseEvents');
          method.call(event, 'before', false, true, realm, 7, 11, 12, 13, 14,
            true, false, false, false, 2, realm.document.body);
          return event;
        };
        const args = (view = null, related = null, button = 0) =>
          ['after', true, false, view, 3, 1, 2, 3, 4, false, true, true, false, button, related];
        check(prefix + 'method metadata', method.length === 1 && method.name === 'initMouseEvent');
        for (const [label, values] of [['omitted', ['after']],
          ['undefined', ['after', undefined, undefined, undefined]],
          ['null', ['after', false, false, null]]]) {
          const event = make();
          const stamp = event.timeStamp, composed = event.composed;
          check(prefix + label + ' return', method.apply(event, values) === undefined);
          check(prefix + label + ' nullable defaults', event.view === null &&
            event.relatedTarget === null && event.detail === 0 && event.button === 0 &&
            event.screenX === 0 && event.screenY === 0 && event.clientX === 0 && event.clientY === 0);
          check(prefix + label + ' creation state', event.timeStamp === stamp && event.composed === composed);
        }
        for (let index = 0; index < realms.length; index++) {
          const view = realms[index];
          const event = make();
          method.apply(event, args(view));
          check(prefix + 'genuine Window ' + index, event.view === view && event.detail === 3);
        }
        const validTargets = [null, undefined, new realm.EventTarget(), realm,
          realm.document, realm.document.body, realm.document.createTextNode('text'),
          nativeProxy, child.document.body];
        validTargets.forEach((target, index) => {
          const event = make();
          const values = args(); values[14] = target;
          method.apply(event, values);
          check(prefix + 'genuine relatedTarget ' + index,
            event.relatedTarget === (target == null ? null : target));
        });
        for (const omitted of [undefined, null]) {
          const event = make(), values = args(); values[14] = omitted;
          method.apply(event, values);
          check(prefix + 'nullable relatedTarget ' + String(omitted), event.relatedTarget === null);
        }
        let traps = 0;
        const trap = {get() { traps++; throw new Error('author get trap'); },
          getPrototypeOf() { traps++; throw new Error('author prototype trap'); }};
        const revokedWindow = Proxy.revocable(realm, {}); revokedWindow.revoke();
        const invalidViews = [{}, 1, 'window', true, Symbol('window'), 1n,
          Object.create(realm.Window.prototype), Object.create(realm),
          new Proxy(realm, trap), revokedWindow.proxy, realm.document];
        invalidViews.forEach((view, index) => {
          const event = make(); event.preventDefault(); event.stopPropagation();
          const before = snapshot(event), order = [];
          const values = args(view); values[0] = {toString() { order.push('type'); return 'after'; }};
          values[4] = {valueOf() { order.push('detail'); throw new Error('late detail'); }};
          const error = caught(() => method.apply(event, values));
          check(prefix + 'invalid Window ' + index + ' callee error', error instanceof callee.TypeError);
          check(prefix + 'invalid Window ' + index + ' conversion order', order.join() === 'type');
          check(prefix + 'invalid Window ' + index + ' atomic state', unchanged(event, before));
        });
        const revokedTarget = Proxy.revocable(nativeProxy, {}); revokedTarget.revoke();
        const invalidTargets = [{}, 1, 'target', true, Symbol('target'), 1n,
          Object.create(realm.EventTarget.prototype), Object.create(nativeProxy),
          new Proxy(nativeProxy, trap), revokedTarget.proxy, new realm.Event('event')];
        invalidTargets.forEach((related, index) => {
          const event = make(); event.preventDefault(); event.stopPropagation();
          const before = snapshot(event), order = [], values = args(null, related);
          values[0] = {toString() { order.push('type'); return 'after'; }};
          values[13] = {valueOf() { order.push('button'); return 65535; }};
          const error = caught(() => method.apply(event, values));
          check(prefix + 'invalid EventTarget ' + index + ' callee error', error instanceof callee.TypeError);
          check(prefix + 'invalid EventTarget ' + index + ' conversion order', order.join() === 'type,button');
          check(prefix + 'invalid EventTarget ' + index + ' atomic state', unchanged(event, before));
        });
        check(prefix + 'interface conversion skips author Proxy traps', traps === 0);
        for (const [caseIndex, [input, expected]] of [[32767, 32767], [32768, -32768],
          [65535, -1], [65536, 0], [-32769, 32767], [-65537, -1],
          [65535.9, -1], [-65535.9, 1], ['32768', -32768], [NaN, 0],
          [Infinity, 0], [-Infinity, 0], [true, 1], [null, 0], [undefined, 0]].entries()) {
          const event = make(), values = args(); values[13] = input;
          method.apply(event, values);
          check(prefix + 'short button ' + caseIndex + '/' + String(input), event.button === expected);
        }
        for (const [input, expected] of [[2147483648, -2147483648],
          [4294967295, -1], [4294967296, 0], [-2147483649, 2147483647],
          [1.9, 1], [-1.9, -1], [NaN, 0], [Infinity, 0]]) {
          const event = make(), values = args();
          for (let index = 4; index <= 8; index++) values[index] = input;
          method.apply(event, values);
          check(prefix + 'long coordinates ' + String(input),
            [event.detail, event.screenX, event.screenY, event.clientX, event.clientY]
              .every(value => value === expected));
        }
        {
          const event = make(), order = [], values = args(child, nativeProxy);
          values[0] = {toString() { order.push(0); return 'new\ud800\0\udc00'; }};
          for (const index of [4, 5, 6, 7, 8, 13]) values[index] = {
            valueOf() { order.push(index); return index === 13 ? 65535 : index; }};
          for (const index of [1, 2, 9, 10, 11, 12]) values[index] = {
            valueOf() { throw new Error('booleans must not coerce'); }};
          method.apply(event, values);
          check(prefix + 'left to right conversion', order.join() === '0,4,5,6,7,8,13');
          check(prefix + 'UTF-16 type and truthy flags', event.type === 'new\ud800\0\udc00' &&
            event.bubbles && event.cancelable && event.ctrlKey && event.altKey && event.shiftKey && event.metaKey);
          check(prefix + 'converted values', event.view === child && event.detail === 4 &&
            event.screenX === 5 && event.screenY === 6 && event.clientX === 7 &&
            event.clientY === 8 && event.button === -1 && event.relatedTarget === nativeProxy);
        }
        for (const index of [0, 4, 5, 6, 7, 8, 13]) {
          for (const input of [Symbol('conversion'), 1n, 'sentinel']) {
            if (index === 0 && input === 1n) continue;
            const event = make(); event.preventDefault();
            const before = snapshot(event), values = args();
            const sentinel = new realm.RangeError('conversion');
            values[index] = input === 'sentinel' ? {[Symbol.toPrimitive]() { throw sentinel; }} : input;
            const error = caught(() => method.apply(event, values));
            check(prefix + 'conversion failure ' + index + '/' + String(input),
              (input === 'sentinel' ? error === sentinel : error instanceof callee.TypeError) &&
              unchanged(event, before));
          }
        }
        {
          const event = make(), before = snapshot(event);
          const revoked = Proxy.revocable(event, {}); revoked.revoke();
          let conversions = 0;
          const values = args(); values[0] = {toString() { conversions++; return 'after'; }};
          for (const receiver of [null, undefined, {}, realm.MouseEvent.prototype,
            Object.create(event), new Proxy(event, trap), revoked.proxy]) {
            check(prefix + 'invalid receiver ' + rows.length,
              caught(() => method.apply(receiver, values)) instanceof callee.TypeError);
          }
          check(prefix + 'receiver validation precedes conversion', conversions === 0 && traps === 0 &&
            unchanged(event, before));
        }
        {
          const event = make(), target = new realm.EventTarget(); let called = 0;
          target.addEventListener('before', current => {
            called++; current.preventDefault(); const before = snapshot(current), order = [];
            const values = args(null, nativeProxy);
            values[0] = {toString() { order.push('type'); return 'after'; }};
            values[13] = {valueOf() { order.push('button'); return 65535; }};
            method.apply(current, values);
            check(prefix + 'dispatch converts before guard', order.join() === 'type,button');
            check(prefix + 'dispatch valid arguments preserve state', unchanged(current, before));
            const badView = args({});
            check(prefix + 'dispatch invalid Window still rejects',
              caught(() => method.apply(current, badView)) instanceof callee.TypeError);
            const badRelated = args(null, {});
            check(prefix + 'dispatch invalid EventTarget still rejects',
              caught(() => method.apply(current, badRelated)) instanceof callee.TypeError);
            check(prefix + 'dispatch failures preserve state', unchanged(current, before));
          });
          check(prefix + 'dispatch remains canceled', target.dispatchEvent(event) === false && called === 1);
          const stamp = event.timeStamp, composed = event.composed;
          method.apply(event, args(null, nativeProxy, 32768));
          check(prefix + 'reusable after dispatch', event.type === 'after' && event.view === null &&
            event.relatedTarget === nativeProxy && event.button === -32768 && !event.defaultPrevented &&
            event.target === null && event.currentTarget === null && event.timeStamp === stamp && event.composed === composed);
        }
      }
    }
  }
  globalThis.__uiEventResults = {passed: rows.filter(row => row.passed).length,
    total: rows.length, rows};
  return rows.every(row => row.passed);
})()
