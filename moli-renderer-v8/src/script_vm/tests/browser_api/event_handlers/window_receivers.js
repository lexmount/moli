(() => {
  const child = document.getElementById('child').contentWindow;
  const popup = window.open('', '_blank');
  const childPopup = child.open('', '_blank');
  const owners = [window, child, popup, childPopup];
  const names = ['onload', 'onresize', 'onerror', 'onmessageerror',
    'onunhandledrejection', 'onrejectionhandled', 'onmouseenter', 'onmouseleave'];
  const rows = [];
  function check(name, expected, run) {
    let actual;
    try { actual = run(); }
    catch (error) { actual = {error: error.name, message: error.message}; }
    rows.push({name, expected, actual,
      pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  const saved = owners.map(owner => names.map(name => owner[name]));
  try {
    for (let s = 0; s < owners.length; ++s) {
      const source = owners[s];
      for (const name of names) {
        const descriptor = Object.getOwnPropertyDescriptor(source, name);
        const CalleeTypeError = descriptor.get.constructor('return TypeError')();
        for (let t = 0; t < owners.length; ++t) {
          const target = owners[t];
          const prefix = `${s}->${t}:${name}`;
          const callbacks = owners.map(() => function () {});
          owners.forEach((owner, i) => { owner[name] = callbacks[i]; });
          check(prefix + ':get', true, () => descriptor.get.call(target) === callbacks[t]);
          check(prefix + ':set', true, () => {
            const replacement = function () {};
            descriptor.set.call(target, replacement);
            return owners.every((owner, i) => owner[name] === (i === t ? replacement : callbacks[i]));
          });
          check(prefix + ':object-value', true, () => {
            const value = {handleEvent() { throw new Error('not a listener object'); }};
            descriptor.set.call(target, value);
            return descriptor.get.call(target) === value && target[name] === value;
          });
          check(prefix + ':primitive-clears', true, () => {
            descriptor.set.call(target, 42);
            return target[name] === null && descriptor.get.call(target) === null;
          });
          owners.forEach(owner => { owner[name] = null; });
        }

        let traps = 0;
        const trap = () => { ++traps; throw new Error('author Proxy trap'); };
        const revoked = Proxy.revocable(source, {});
        revoked.revoke();
        const invalid = [{}, Object.create(source), new Proxy(source, {}),
          new Proxy(source, {get: trap, getPrototypeOf: trap, has: trap}), revoked.proxy];
        const lenient = name === 'onmouseenter' || name === 'onmouseleave';
        for (let i = 0; i < invalid.length; ++i) {
          for (const operation of ['get', 'set']) {
            check(`${s}:${name}:${operation}:invalid-${i}`, [true, 0], () => {
              traps = 0;
              try {
                const value = descriptor[operation].call(invalid[i], () => {});
                return [lenient && value === undefined, traps];
              } catch (error) {
                return [!lenient && error instanceof CalleeTypeError, traps];
              }
            });
          }
        }
      }
    }

    for (let t = 0; t < owners.length; ++t) {
      const target = owners[t];
      target.__handlerOwnerMarker = t;
      for (let s = 0; s < owners.length; ++s) {
        for (const name of ['onresize', 'onerror']) {
          check(`${s}->${t}:${name}:lazy-content`, true, () => {
            owners.forEach(owner => { owner[name] = null; });
            target.document.body.setAttribute(name, 'return __handlerOwnerMarker;');
            try {
              const accessor = Object.getOwnPropertyDescriptor(owners[s], name).get;
              const handler = accessor.call(target);
              const TargetFunction = Object.getOwnPropertyDescriptor(target, name).get.constructor;
              return typeof handler === 'function' && handler() === t && handler.call(target) === t &&
                handler.constructor === TargetFunction &&
                handler === target.document.body[name] &&
                owners.every((owner, i) => i === t || owner[name] === null);
            } finally {
              target.document.body.removeAttribute(name);
            }
          });
        }
        check(`${s}->${t}:dispatch-order`, ['before', 'replacement', 'after',
          'before', 'after', 'before', 'after', 'readded'], () => {
          const trace = [];
          const before = () => trace.push('before');
          const after = () => trace.push('after');
          const descriptor = Object.getOwnPropertyDescriptor(owners[s], 'onresize');
          const callback = label => function (event) {
            trace.push(this === target && event.currentTarget === target ? label : 'wrong receiver');
            return false;
          };
          const dispatch = () => target.dispatchEvent(new Event('resize', {cancelable: true}));
          try {
            target.addEventListener('resize', before);
            descriptor.set.call(target, callback('original'));
            target.addEventListener('resize', after);
            descriptor.set.call(target, callback('replacement'));
            if (dispatch() !== false) trace.push('not canceled');
            descriptor.set.call(target, null);
            if (dispatch() !== true) trace.push('still canceled');
            descriptor.set.call(target, callback('readded'));
            if (dispatch() !== false) trace.push('not canceled');
            return trace;
          } finally {
            target.removeEventListener('resize', before);
            target.removeEventListener('resize', after);
            target.onresize = null;
          }
        });
        check(`${s}->${t}:compilation-error-owner`, [[t, true]], () => {
          const trace = [];
          const TargetSyntaxError = Object.getOwnPropertyDescriptor(target, 'onresize')
            .get.constructor('return SyntaxError')();
          const listeners = owners.map((owner, i) => event => {
            trace.push([i, event.error instanceof TargetSyntaxError]);
            event.preventDefault();
          });
          owners.forEach((owner, i) => owner.addEventListener('error', listeners[i]));
          try {
            target.document.body.setAttribute('onresize', '}');
            const getter = Object.getOwnPropertyDescriptor(owners[s], 'onresize').get;
            if (getter.call(target) !== null) trace.push('not null');
            if (getter.call(target) !== null) trace.push('not null on second read');
            return trace;
          } finally {
            target.document.body.removeAttribute('onresize');
            owners.forEach((owner, i) => owner.removeEventListener('error', listeners[i]));
          }
        });
      }
      for (let s = 0; s < 2; ++s) {
        check(`${s}->${t}:borrowed-body-getter`, true, () => {
          target.document.body.setAttribute('onresize', 'return this.__handlerOwnerMarker;');
          try {
            const getter = Object.getOwnPropertyDescriptor(owners[s].HTMLBodyElement.prototype,
              'onresize').get;
            const handler = getter.call(target.document.body);
            return handler === target.onresize && handler.call(target) === t &&
              handler.constructor === Object.getOwnPropertyDescriptor(target, 'onresize').get.constructor;
          } finally { target.document.body.removeAttribute('onresize'); }
        });
      }
    }
  } finally {
    owners.forEach((owner, i) => {
      names.forEach((name, j) => { owner[name] = saved[i][j]; });
      delete owner.__handlerOwnerMarker;
    });
    popup.close();
    childPopup.close();
  }
  const failures = rows.filter(row => !row.pass);
  globalThis.__nodeReplacementResults = {total: rows.length,
    passed: rows.length - failures.length, failures, rows};
  return failures.length === 0;
})()
