function xhrInvalidReceiverProbe(callee, owner) {
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const xhr = new owner.XMLHttpRequest();
  const upload = xhr.upload;
  let argumentReads = 0, receiverTraps = 0;
  const marker = {};
  const argument = new Proxy({}, {get() { argumentReads++; throw marker; }});
  const cases = [];
  const attributes = [
    'onreadystatechange', 'readyState', 'timeout', 'withCredentials', 'upload',
    'responseURL', 'status', 'statusText', 'responseType', 'response', 'responseText'
  ];
  if (Object.hasOwn(callee.XMLHttpRequest.prototype, 'responseXML')) attributes.push('responseXML');
  for (const name of attributes) {
    const descriptor = Object.getOwnPropertyDescriptor(callee.XMLHttpRequest.prototype, name);
    cases.push([name + ' getter', descriptor.get, xhr, upload, []]);
    if (descriptor.set) cases.push([name + ' setter', descriptor.set, xhr, upload, [argument]]);
  }
  for (const [name, args] of [
    ['open', [argument, argument]], ['send', [argument]], ['abort', []],
    ['setRequestHeader', [argument, argument]], ['getResponseHeader', [argument]],
    ['getAllResponseHeaders', []], ['overrideMimeType', [argument]]
  ]) cases.push([name, callee.XMLHttpRequest.prototype[name], xhr, upload, args]);
  for (const target of [xhr, upload]) {
    for (const name of ['onloadstart', 'onprogress', 'onabort', 'onerror', 'onload', 'ontimeout', 'onloadend']) {
      const descriptor = Object.getOwnPropertyDescriptor(callee.XMLHttpRequestEventTarget.prototype, name);
      cases.push([name + ' getter', descriptor.get, target, new owner.EventTarget(), []]);
      cases.push([name + ' setter', descriptor.set, target, new owner.EventTarget(), [argument]]);
    }
  }
  for (const [name, callback, real, wrong, args] of cases) {
    const revoked = Proxy.revocable(real, {});
    revoked.revoke();
    const proxy = new Proxy(real, {
      get() { receiverTraps++; throw marker; },
      getPrototypeOf() { receiverTraps++; throw marker; }
    });
    for (const receiver of [
      null, undefined, 0, '', true, 1n, Symbol('receiver'), {}, wrong,
      Object.getPrototypeOf(real), Object.create(Object.getPrototypeOf(real)),
      Object.create(real), proxy, revoked.proxy
    ]) {
      let caught;
      try { Reflect.apply(callback, receiver, args); } catch (error) { caught = error; }
      assert(caught && Object.getPrototypeOf(caught) === callee.TypeError.prototype,
        name + ': expected a callee-realm TypeError');
    }
  }
  assert(argumentReads === 0 && receiverTraps === 0, 'receiver checks must not execute author code');
  assert(xhr.readyState === 0 && xhr.timeout === 0 && xhr.withCredentials === false,
    'invalid receivers must not mutate the genuine XHR');
  return 'ok';
}

function xhrGenuineReceiverProbe(callee, owner) {
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const xhr = new owner.XMLHttpRequest(), upload = xhr.upload;
  const prototype = callee.XMLHttpRequest.prototype;
  const descriptor = name => Object.getOwnPropertyDescriptor(prototype, name);
  const get = name => descriptor(name).get.call(xhr);
  Object.setPrototypeOf(xhr, null);
  Object.setPrototypeOf(upload, null);
  for (const [name, expected] of [
    ['readyState', 0], ['status', 0], ['statusText', ''], ['responseURL', ''],
    ['responseText', ''], ['response', ''], ['responseType', ''], ['timeout', 0],
    ['withCredentials', false], ['upload', upload]
  ]) assert(get(name) === expected, name + ': native state survives prototype replacement');
  if (Object.hasOwn(prototype, 'responseXML')) assert(get('responseXML') === null, 'initial responseXML');
  descriptor('timeout').set.call(xhr, 12345);
  descriptor('withCredentials').set.call(xhr, true);
  descriptor('responseType').set.call(xhr, 'text');
  assert(get('timeout') === 12345 && get('withCredentials') && get('responseType') === 'text', 'native setters');
  descriptor('timeout').set.call(xhr, 0);
  descriptor('responseType').set.call(xhr, '');
  for (const target of [xhr, upload]) {
    for (const name of ['onloadstart', 'onprogress', 'onabort', 'onerror', 'onload', 'ontimeout', 'onloadend']) {
      const property = Object.getOwnPropertyDescriptor(callee.XMLHttpRequestEventTarget.prototype, name);
      let calls = 0;
      const callback = function(event) {
        assert(this === target && event.target === target, 'native handler target');
        calls++;
      };
      property.set.call(target, callback);
      assert(property.get.call(target) === callback, name + ': handler identity');
      callee.EventTarget.prototype.dispatchEvent.call(target, new owner.Event(name.slice(2)));
      assert(calls === 1, name + ': native listener registry');
      property.set.call(target, null);
      assert(property.get.call(target) === null, name + ': clear handler');
    }
  }
  let readyStates = 0;
  descriptor('onreadystatechange').set.call(xhr, () => readyStates++);
  prototype.open.call(xhr, 'GET', 'data:,receiver');
  assert(get('readyState') === 1 && readyStates === 1, 'borrowed open uses the genuine owner');
  prototype.setRequestHeader.call(xhr, 'X-Receiver', 'genuine');
  prototype.overrideMimeType.call(xhr, 'text/plain');
  assert(prototype.getResponseHeader.call(xhr, 'x-receiver') === null, 'request header is not a response');
  assert(prototype.getAllResponseHeaders.call(xhr) === '', 'no response headers before send');
  prototype.abort.call(xhr);
  assert(get('readyState') === 1 && readyStates === 1, 'abort before send preserves OPENED without events');
  return 'ok';
}
