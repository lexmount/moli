(async () => {
  const checks = [], watches = [];
  async function check(name, action) {
    try { checks.push({name, passed: (await action()) === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  const caught = action => {try {action(); return null;} catch (error) {return error;}};
  const rejected = async action => {try {await action(); return null;} catch (error) {return error;}};
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  try {
    for (const [realm, owner] of [window, frame.contentWindow, popup].entries()) {
      const C = owner.RemotePlayback, P = C.prototype, M = owner.HTMLMediaElement.prototype;
      const typeError = error => error !== null && Object.getPrototypeOf(error) === owner.TypeError.prototype;
      const domError = (error, name) => error?.name === name && Object.getPrototypeOf(error) === owner.DOMException.prototype;
      const media = owner.document.createElement('video');
      const remoteDescriptor = Object.getOwnPropertyDescriptor(M, 'remote');
      let remote;
      await check(`${realm}/associated native object`, () => {
        remote = remoteDescriptor.get.call(media);
        return remote instanceof C && remote instanceof owner.EventTarget && Object.getPrototypeOf(remote) === P &&
          remote === remoteDescriptor.get.call(media) && remote !== owner.document.createElement('audio').remote;
      });
      await check(`${realm}/constructor inheritance and descriptors`, () => C.name === 'RemotePlayback' && C.length === 0 &&
        Object.getPrototypeOf(C) === owner.EventTarget && Object.getPrototypeOf(P) === owner.EventTarget.prototype &&
        Object.getOwnPropertyDescriptor(P, Symbol.toStringTag).value === 'RemotePlayback');
      for (const construct of [false, true]) {
        await check(`${realm}/illegal constructor/${construct}`, () => typeError(caught(() => construct ? new C() : C())));
      }
      let traps = 0;
      const author = new Proxy(remote ?? {}, {get() {traps++; throw Error('get');}, getPrototypeOf() {traps++; throw Error('prototype');}});
      const revoked = Proxy.revocable(remote ?? {}, {}); revoked.revoke();
      const invalid = [undefined, null, {}, P, Object.create(P), Object.create(remote ?? {}), author, revoked.proxy,
        new owner.EventTarget(), media];
      for (const name of ['state', 'onconnecting', 'onconnect', 'ondisconnect']) {
        const d = Object.getOwnPropertyDescriptor(P, name), setter = name !== 'state';
        await check(`${realm}/${name}/descriptor`, () => d?.enumerable === true && d.configurable && typeof d.get === 'function' && d.get.length === 0 &&
          (setter ? typeof d.set === 'function' && d.set.length === 1 : d.set === undefined));
        for (const [index, value] of invalid.entries()) {
          await check(`${realm}/${name}/getter receiver ${index}`, () => typeError(caught(() => d.get.call(value))));
          if (setter) await check(`${realm}/${name}/setter receiver ${index}`, () => typeError(caught(() => d.set.call(value, () => {}))));
        }
      }
      for (const [name, length] of [['watchAvailability', 1], ['cancelWatchAvailability', 0], ['prompt', 0]]) {
        const d = Object.getOwnPropertyDescriptor(P, name);
        await check(`${realm}/${name}/descriptor`, () => d?.writable === true && d.enumerable && d.configurable &&
          typeof d.value === 'function' && d.value.length === length && d.value.name === name);
        for (const [index, value] of invalid.entries()) {
          await check(`${realm}/${name}/Promise receiver ${index}`, async () => {
            let conversions = 0, promise;
            const arg = name === 'watchAvailability' ? () => {} : {valueOf() {conversions++; throw Error('conversion');}};
            const synchronous = caught(() => {promise = d.value.call(value, arg);});
            return synchronous === null && promise instanceof owner.Promise && typeError(await rejected(() => promise)) && conversions === 0;
          });
        }
      }
      for (const [name, setter] of [['remote', false], ['disableRemotePlayback', true]]) {
        const d = Object.getOwnPropertyDescriptor(M, name);
        await check(`${realm}/media/${name}/descriptor`, () => d?.enumerable === true && d.configurable && d.get.length === 0 &&
          (setter ? typeof d.set === 'function' && d.set.length === 1 : d.set === undefined));
        const badMedia = [undefined, null, {}, M, Object.create(M), Object.create(media),
          new Proxy(media, {}), revoked.proxy, owner.document.createElement('div')];
        for (const [index, value] of badMedia.entries()) {
          await check(`${realm}/media/${name}/getter receiver ${index}`, () => typeError(caught(() => d.get.call(value))));
          if (setter) await check(`${realm}/media/${name}/setter receiver ${index}`, () => typeError(caught(() => d.set.call(value, {}))));
        }
      }
      await check(`${realm}/native disabled state ignores public expandos`, async () => {
        media.setAttribute('disableremoteplayback', '');
        Object.defineProperty(media, 'disableRemotePlayback', {value: false, configurable: true});
        const error = await rejected(() => P.prompt.call(remote));
        delete media.disableRemotePlayback; media.removeAttribute('disableremoteplayback');
        return domError(error, 'InvalidStateError');
      });
      const disabled = Object.getOwnPropertyDescriptor(M, 'disableRemotePlayback');
      for (const [index, value] of [undefined, null, false, true, 0, 1, '', 'false', {}, Symbol('flag'), 0n, 1n].entries()) {
        await check(`${realm}/boolean reflection/${index}`, () => {
          disabled.set.call(media, value);
          return disabled.get.call(media) === Boolean(value) && media.hasAttribute('disableremoteplayback') === Boolean(value);
        });
      }
      await check(`${realm}/boolean reflection does not coerce objects`, () => {
        let conversions = 0;
        disabled.set.call(media, {valueOf() {conversions++; throw Error('valueOf');}, toString() {conversions++; throw Error('toString');}});
        return disabled.get.call(media) && conversions === 0;
      });
      for (const [index, callback] of [undefined, null, false, 1, 'x', 1n, Symbol('callback'), {}, {handleEvent() {}}].entries()) {
        await check(`${realm}/callback conversion before disabled check/${index}`, async () =>
          typeError(await rejected(() => P.watchAvailability.call(remote, callback))));
      }
      await check(`${realm}/required callback is a rejected Promise`, async () => typeError(await rejected(() => P.watchAvailability.call(remote))));
      await check(`${realm}/disabled watch`, async () => domError(await rejected(() => P.watchAvailability.call(remote, () => {})), 'InvalidStateError'));
      for (const id of [undefined, 0, 1]) {
        await check(`${realm}/disabled cancel/${id}`, async () => domError(await rejected(() => P.cancelWatchAvailability.call(remote, id)), 'InvalidStateError'));
      }
      await check(`${realm}/conversion exception identity before disabled cancel`, async () => {
        const marker = {}, log = [];
        const error = await rejected(() => P.cancelWatchAvailability.call(remote, {valueOf() {log.push('valueOf'); throw marker;}}));
        return error === marker && log.join() === 'valueOf';
      });
      for (const id of [Symbol('id'), 1n]) {
        await check(`${realm}/invalid long before disabled cancel/${typeof id}`, async () => typeError(await rejected(() => P.cancelWatchAvailability.call(remote, id))));
      }
      await check(`${realm}/disabled prompt`, async () => domError(await rejected(() => P.prompt.call(remote)), 'InvalidStateError'));
      if (disabled) disabled.set.call(media, false);
      for (const id of [null, 0, -1, 1, 2**32 + 1, NaN, Infinity, -Infinity, '1']) {
        await check(`${realm}/unknown callback id/${String(id)}`, async () => domError(await rejected(() => P.cancelWatchAvailability.call(remote, id)), 'NotFoundError'));
      }
      await check(`${realm}/cancel omitted and undefined`, async () => await P.cancelWatchAvailability.call(remote) === undefined &&
        await P.cancelWatchAvailability.call(remote, undefined) === undefined);
      await check(`${realm}/long conversion occurs once`, async () => {
        let count = 0;
        const error = await rejected(() => P.cancelWatchAvailability.call(remote, {valueOf() {count++; return 1;}}));
        return count === 1 && domError(error, 'NotFoundError');
      });
      await check(`${realm}/initial disconnected state`, () => remote.state === 'disconnected');
      for (const eventType of ['connecting', 'connect', 'disconnect']) {
        await check(`${realm}/ordered EventHandler/${eventType}`, () => {
          const target = remote, log = [], before = () => log.push('before'), after = () => log.push('after');
          const name = 'on' + eventType;
          target.addEventListener(eventType, before);
          target[name] = () => log.push('old');
          target.addEventListener(eventType, after);
          target[name] = function(event) {log.push(this === target && event.currentTarget === target ? 'handler' : 'wrong'); return false;};
          const event = new owner.Event(eventType, {cancelable:true});
          const result = !target.dispatchEvent(event) && event.defaultPrevented && log.join() === 'before,handler,after';
          target[name] = null; target.removeEventListener(eventType, before); target.removeEventListener(eventType, after);
          return result && target.state === 'disconnected';
        });
        for (const [index, value] of [undefined, null, false, 1, 'x', 1n, Symbol('handler')].entries()) {
          await check(`${realm}/handler primitive/${eventType}/${index}`, () => {remote['on'+eventType] = value; return remote['on'+eventType] === null;});
        }
      }
      await check(`${realm}/availability resolves before one boolean callback`, async () => {
        const log = [];
        let count = 0, thisValue, resolveCallback;
        const callback = new Promise(resolve => {resolveCallback = resolve;});
        const pending = P.watchAvailability.call(remote, function(available) {'use strict'; count++; thisValue = this; log.push('callback'); resolveCallback(available);});
        const value = await pending.then(value => {log.push('resolved'); return value;});
        const available = await callback;
        watches.push({realm, resolvedType:typeof value, resolvedValue:value ?? null, available, count, order:log,
          undefinedThis:thisValue === undefined, remoteThis:thisValue === remote});
        // The lifetime-unavailable branch has no id; monitoring-capable UAs
        // instead return a positive id. Original WPT id checks stay separate.
        return (value === undefined || Number.isInteger(value) && value > 0) && available === false && count === 1 &&
          thisValue === undefined && log.join() === 'resolved,callback';
      });
      await check(`${realm}/borrowed media getter keeps owner realm`, () => {
        const foreign = window.document.createElement('audio');
        const result = remoteDescriptor.get.call(foreign);
        return result instanceof window.RemotePlayback && result === foreign.remote && (owner === window || !(result instanceof C));
      });
      await check(`${realm}/author and revoked Proxy checks execute no traps`, () => traps === 0);
      await check(`${realm}/public constructor replacement preserves native source`, () => {
        const original = owner.RemotePlayback;
        try {owner.RemotePlayback = function() {throw Error('author constructor');}; return owner.document.createElement('audio').remote instanceof original;}
        finally {owner.RemotePlayback = original;}
      });
    }
  } finally {popup.close(); frame.remove();}
  const facts = {complete:true, total:checks.length, passed:checks.filter(row => row.passed).length, checks, watches};
  globalThis.__uiEventResults = facts;
  return facts.passed === facts.total;
})()
