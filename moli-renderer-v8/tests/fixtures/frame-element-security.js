async ({sameURL, crossURL}) => {
  const failures = [];
  let checks = 0;
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected))
      failures.push({label, actual, expected});
  };
  const throws = (callback, Constructor, name, label) => {
    try { callback(); equal('returned', name, label); }
    catch (error) { equal([error.name, error instanceof Constructor], [name, true], label); }
  };
  const get = Object.getOwnPropertyDescriptor(window, 'frameElement').get;
  equal(get.call(window), null, 'top frameElement');
  const frames = [];
  const create = async (url, sandbox) => {
    const frame = document.createElement('iframe');
    if (sandbox) frame.sandbox = 'allow-scripts';
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = url;
    document.body.appendChild(frame);
    frames.push(frame);
    await loaded;
    return frame;
  };
  let same;
  for (const mode of ['same', 'cross', 'sandbox']) {
    const frame = await create(mode === 'cross' ? crossURL : sameURL, mode === 'sandbox');
    const child = frame.contentWindow;
    if (mode === 'same') {
      same = frame;
      equal(child.frameElement === frame, true, 'same-origin direct identity');
      equal(get.call(child) === frame, true, 'same-origin borrowed identity');
    } else {
      throws(() => child.frameElement, DOMException, 'SecurityError', mode + ' direct');
      throws(() => get.call(child), DOMException, 'SecurityError', mode + ' borrowed');
    }
    const response = new Promise(resolve => {
      const listener = event => {
        if (event.source !== child) return;
        removeEventListener('message', listener);
        resolve(event.data);
      };
      addEventListener('message', listener);
    });
    child.postMessage('frame-element-security', '*');
    equal(await response, mode === 'same'
      ? {own:'IFRAME', borrowed:'IFRAME', parent:'null', nested:'IFRAME', nestedBorrowed:'IFRAME'}
      : {own:'null', borrowed:'null', parent:'SecurityError:true', nested:mode === 'cross' ? 'IFRAME' : 'SecurityError:true', nestedBorrowed:mode === 'cross' ? 'IFRAME' : 'SecurityError:true'}, mode + ' child observations');
    if (mode !== 'same') {
      frame.remove();
      throws(() => get.call(child), DOMException, 'SecurityError', mode + ' removed borrowed');
    }
  }
  const child = same.contentWindow;
  const childGet = Object.getOwnPropertyDescriptor(child, 'frameElement').get;
  const childRead = child.Function('return frameElement');
  const childDOMException = child.DOMException;
  let traps = 0;
  const revoked = Proxy.revocable(child, {}); revoked.revoke();
  for (const invalid of [{}, Object.create(child), new Proxy(child, {
    get() { traps++; throw new Error('author trap'); },
    getPrototypeOf() { traps++; throw new Error('author trap'); }
  }), revoked.proxy]) throws(() => get.call(invalid), TypeError, 'TypeError', 'invalid Window receiver');
  equal(traps, 0, 'brand check does not run author traps');
  child.document.domain = child.document.domain;
  throws(() => child.frameElement, DOMException, 'SecurityError', 'one-sided domain direct');
  throws(() => get.call(child), DOMException, 'SecurityError', 'one-sided domain parent getter');
  equal(childGet.call(child), null, 'cached child getter uses its realm origin');
  equal(childRead(), null, 'child execution cannot expose cross-origin container');
  throws(() => childGet.call(window), childDOMException, 'SecurityError', 'child getter error realm');
  document.domain = document.domain;
  equal(child.frameElement === same, true, 'relaxed domain direct identity');
  equal(get.call(child) === same, true, 'relaxed domain parent getter');
  equal(childGet.call(child) === same, true, 'relaxed domain child getter');
  same.remove();
  equal(get.call(child), null, 'removed same-origin receiver');
  equal(childGet.call(child), null, 'removed cached child getter');
  equal(childRead(), null, 'removed child execution');
  for (const frame of frames) frame.remove();
  return {checks, failures};
}
