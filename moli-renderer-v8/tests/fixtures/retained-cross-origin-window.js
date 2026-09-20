async function ({childURL, shadow = false, mode = 'remove'}) {
  const observations = [], failures = [];
  let checks = 0;
  const check = (name, fn, expected) => {
    ++checks;
    let actual;
    try { actual = fn(); } catch (error) { actual = error.name; }
    if (actual !== expected) failures.push({name, actual, expected});
  };
  const root = document.createElement('div');
  document.body.append(root);
  const container = shadow ? root.attachShadow({mode: 'closed'}) : root;
  const frame = document.createElement('iframe');
  frame.src = childURL;
  const load = action => new Promise(resolve => {frame.onload = resolve; action();});
  await load(() => container.append(frame));
  const old = frame.contentWindow, getters = {};
  const props = ['closed', 'parent', 'top', 'length', 'opener', 'location'];
  for (const name of props) getters[name] = Object.getOwnPropertyDescriptor(old, name).get;
  const setLocation = Object.getOwnPropertyDescriptor(old, 'location').set;
  const oldLocation = old.location;
  const postMessage = old.postMessage;
  for (const name of ['window', 'self', 'frames']) check('live:' + name, () => old[name] === old, true);
  const snapshot = (phase, closed) => {
    const expected = {closed, parent: closed ? null : window, top: closed ? null : window,
      length: closed ? 0 : 1, opener: null, location: oldLocation};
    for (const name of props) {
      check(phase + ':direct:' + name, () => old[name] === expected[name], true);
      check(phase + ':cached:' + name, () => getters[name].call(old) === expected[name], true);
      check(phase + ':descriptor:' + name, () => typeof Object.getOwnPropertyDescriptor(old, name).get, 'function');
    }
    for (const name of ['close', 'focus', 'blur', 'postMessage']) {
      check(phase + ':method:' + name, () => typeof old[name], 'function');
    }
    for (const name of ['document', 'name', 'frameElement', 'unknown']) {
      check(phase + ':forbidden:' + name, () => {void old[name];}, 'SecurityError');
    }
    check(phase + ':href', () => oldLocation.href, 'SecurityError');
    check(phase + ':index', () => typeof old[0], closed ? 'SecurityError' : 'object');
    if (closed) {
      for (const name of ['window', 'self', 'frames']) {
        // HTML returns the realm's GlobalThisValue; Chromium instead clears
        // these aliases on detach. Both must remain cross-origin readable.
        check(phase + ':readable:' + name, () => old[name] === old || old[name] === null, true);
      }
    }
    observations.push(phase);
  };
  snapshot('live', false);
  if (mode === 'ancestor') root.remove();
  else if (mode === 'replace') container.replaceChildren();
  else frame.remove();
  snapshot('removed', true);
  await load(() => {
    if (!root.isConnected) document.body.append(root);
    container.append(frame);
  });
  const fresh = frame.contentWindow;
  check('replacement:identity', () => fresh !== old, true);
  check('replacement:closed', () => fresh.closed, false);
  snapshot('reinserted', true);
  check('old:postMessage', () => {old.postMessage('stale-direct', '*'); return true;}, true);
  check('old:bound-postMessage', () => {postMessage.call(old, 'stale-cached', '*'); return true;}, true);
  check('old:location-setter', () => {setLocation.call(old, childURL + '#stale-setter'); return true;}, true);
  check('old:location-assign', () => {old.location = childURL + '#stale-direct'; return true;}, true);
  check('old:location-href', () => {oldLocation.href = childURL + '#stale-location'; return true;}, true);
  check('old:location-replace', () => {oldLocation.replace(childURL + '#stale-replace'); return true;}, true);
  // A reply from the replacement is a task barrier: any misrouted messages
  // accepted above would be observed before this request.
  const token = 'retained-window-barrier';
  const reply = await new Promise(resolve => {
    const onmessage = event => {
      if (event.source !== fresh || !event.data || event.data.token !== token) return;
      removeEventListener('message', onmessage);
      resolve(event.data);
    };
    addEventListener('message', onmessage);
    fresh.postMessage({token}, '*');
  });
  check('replacement:messages', () => reply.messages.join(','), '');
  check('replacement:location', () => reply.hash, '');
  frame.remove();root.remove();
  return {checks, failures, observations};
}
