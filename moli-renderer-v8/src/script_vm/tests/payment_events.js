(async function paymentEventChecks(realms) {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: Boolean(passed)});
  const throws = (name, action, Constructor, expectedName) => {
    let error;
    try { action(); } catch (caught) { error = caught; }
    check(name, error instanceof Constructor && (!expectedName || error.name === expectedName));
  };
  for (let ownerIndex = 0; ownerIndex < realms.length; ownerIndex++) {
    const owner = realms[ownerIndex];
    for (const name of ['PaymentRequestUpdateEvent', 'PaymentMethodChangeEvent']) {
      const C = owner[name], p = C.prototype;
      const prefix = ownerIndex + ':' + name + ':';
      check(prefix + 'constructor metadata', C.length === 1 && C.name === name);
      const empty = new C('test');
      check(prefix + 'Event inheritance', empty instanceof owner.Event && empty instanceof owner.PaymentRequestUpdateEvent);
      check(prefix + 'default event header', empty.type === 'test' && !empty.bubbles && !empty.cancelable && !empty.composed && !empty.isTrusted && empty.target === null);
      for (const init of [undefined, null, {}]) {
        const event = new C('test', init);
        check(prefix + 'empty dictionary ' + String(init), !event.bubbles && !event.cancelable && !event.composed);
        if (name === 'PaymentMethodChangeEvent') check(prefix + 'payload defaults ' + String(init), event.methodName === '' && event.methodDetails === null);
      }
      throws(prefix + 'requires new', () => C('test'), owner.TypeError);
      throws(prefix + 'requires type', () => new C(), owner.TypeError);
      const order = [];
      const details = new Proxy({}, {get() {throw Error('payload object must not be read');}});
      const init = {get bubbles() {order.push('bubbles');return true;}, get cancelable() {order.push('cancelable');return true;}, get composed() {order.push('composed');return true;}, get methodDetails() {order.push('methodDetails');return details;}, get methodName() {order.push('methodName');return {toString() {order.push('name-string');return 'pay\ud800';}};}};
      const event = new C({toString() {order.push('type');return 'change\udc00';}}, init);
      const expectedOrder = ['type', 'bubbles', 'cancelable', 'composed'];
      if (name === 'PaymentMethodChangeEvent') expectedOrder.push('methodDetails', 'methodName', 'name-string');
      check(prefix + 'dictionary conversion order', order.join() === expectedOrder.join());
      check(prefix + 'UTF16 event type', event.type === 'change\udc00');
      check(prefix + 'event flags', event.bubbles && event.cancelable && event.composed && !event.isTrusted);
      if (name === 'PaymentMethodChangeEvent') {
        check(prefix + 'payload identity and UTF16', event.methodDetails === details && event.methodName === 'pay\ud800');
        for (const value of [undefined, null, {}, [], () => {}, new Proxy({}, {})]) {
          const result = new C('x', {methodDetails: value});
          check(prefix + 'nullable object ' + String(typeof value), result.methodDetails === (value == null ? null : value));
        }
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        check(prefix + 'revoked object payload retained', new C('x', {methodDetails: revoked.proxy}).methodDetails === revoked.proxy);
        for (const value of [true, 1, 'x', Symbol('s'), 2n]) throws(prefix + 'reject primitive payload ' + typeof value, () => new C('x', {methodDetails: value}), owner.TypeError);
        const token = {};
        let nameRead = false, caught;
        try {new C('x', {get methodDetails() {throw token;}, get methodName() {nameRead = true;return 'x';}});} catch(error) {caught = error;}
        check(prefix + 'dictionary exception and stop order', caught === token && !nameRead);
      }
      const target = new owner.EventTarget();
      const seen = [];
      target.addEventListener(event.type, received => {seen.push(received === event && received.target === target && received.currentTarget === target);});
      target.dispatchEvent(event);
      check(prefix + 'native EventTarget dispatch', seen.length === 1 && seen[0] && event.target === target && event.currentTarget === null && !event.isTrusted);
      for (let calleeIndex = 0; calleeIndex < realms.length; calleeIndex++) {
        const callee = realms[calleeIndex], q = callee.PaymentRequestUpdateEvent.prototype;
        const label = prefix + 'callee-' + calleeIndex + ':';
        const update = q.updateWith;
        const descriptor = Object.getOwnPropertyDescriptor(q, 'updateWith');
        check(label + 'operation descriptor', descriptor.enumerable && descriptor.configurable && descriptor.writable && update.length === 1 && update.name === 'updateWith');
        const invalid = [null, undefined, {}, Object.create(p), Object.create(event), new Proxy(event, {})];
        const revoked = Proxy.revocable(event, {}); revoked.revoke(); invalid.push(revoked.proxy);
        for (let i = 0; i < invalid.length; i++) {
          let reads = 0;
          throws(label + 'reject receiver before conversion ' + i, () => update.call(invalid[i], {get then() {reads++;throw Error('conversion');}}), callee.TypeError);
          check(label + 'no receiver conversion ' + i, reads === 0);
        }
        throws(label + 'missing argument', () => update.call(event), callee.TypeError);
        for (const value of [undefined, null, {}, 1]) throws(label + 'untrusted value ' + typeof value, () => update.call(event, value), callee.DOMException, 'InvalidStateError');
        const calls = [];
        const value = {get then() {calls.push('get');return resolve => {calls.push('call');resolve({});};}};
        throws(label + 'thenable update error', () => update.call(event, value), callee.DOMException, 'InvalidStateError');
        check(label + 'then getter before state check', calls.join() === 'get');
        await Promise.resolve(); await Promise.resolve();
        check(label + 'then invoked asynchronously', calls.join() === 'get,call');
        const nativeCalls = [];
        const promise = owner.Promise.resolve({});
        Object.defineProperty(promise, 'then', {get() {nativeCalls.push('get');return resolve => {nativeCalls.push('call');resolve({});};}});
        Object.defineProperty(promise, 'constructor', {get() {throw Error('Promise constructor must not be read');}});
        throws(label + 'native promise update error', () => update.call(event, promise), callee.DOMException, 'InvalidStateError');
        check(label + 'WebIDL native Promise wrapping', nativeCalls.join() === 'get');
        await Promise.resolve(); await Promise.resolve();
        check(label + 'native Promise then invoked asynchronously', nativeCalls.join() === 'get,call');
        const delivered = new C('during');
        const dispatchOrder = [];
        target.addEventListener('during', () => {throws(label + 'untrusted during dispatch', () => update.call(delivered, {}), callee.DOMException, 'InvalidStateError');dispatchOrder.push(1);}, {once:true});
        target.addEventListener('during', () => dispatchOrder.push(2), {once:true});
        target.dispatchEvent(delivered);
        check(label + 'failed update leaves propagation intact', dispatchOrder.join() === '1,2');
        if (name === 'PaymentMethodChangeEvent') {
          for (const key of ['methodName', 'methodDetails']) {
            const desc = Object.getOwnPropertyDescriptor(callee.PaymentMethodChangeEvent.prototype, key);
            check(label + key + ':accessor descriptor', desc.enumerable && desc.configurable && desc.set === undefined);
            check(label + key + ':cross-realm getter', desc.get.call(event) === event[key]);
            for (let i = 0; i < invalid.length; i++) throws(label + key + ':brand ' + i, () => desc.get.call(invalid[i]), callee.TypeError);
          }
        }
      }
    }
  }
  return checks;
})
