async function workerFontsProbe(fontBytes) {
  const rows = [], failures = [];
  const check = (name, actual, expected) => {
    actual = JSON.parse(JSON.stringify(actual));
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  check('exposure', [typeof FontFace, typeof FontFaceSet, typeof FontFaceSetLoadEvent, typeof fonts], ['function', 'function', 'function', 'object']);
  const descriptor = Object.getOwnPropertyDescriptor(WorkerGlobalScope.prototype, 'fonts');
  check('descriptor', [typeof descriptor.get, descriptor.set, descriptor.enumerable, descriptor.configurable], ['function', undefined, true, true]);
  const set = fonts;
  check('same-object', [set === fonts, set instanceof FontFaceSet, await set.ready === set, set.status, set.size], [true, true, true, 'loaded', 0]);
  const revoked = Proxy.revocable(self, {}); revoked.revoke();
  for (const [name, value] of [['plain', {}], ['inherited', Object.create(self)], ['proxy', new Proxy(self, {})], ['revoked', revoked.proxy]]) {
    let result = 'accepted'; try { descriptor.get.call(value); } catch (error) { result = error.name; }
    check('receiver-' + name, result, 'TypeError');
  }
  for (const prefix of ['', 'medium ']) {
    for (const keyword of ['initial', 'inherit', 'unset', 'default', 'revert', 'revert-layer']) {
      check('css-wide-' + prefix + keyword, await set.load(prefix + keyword).then(() => 'accepted', error => error.name), 'SyntaxError');
    }
  }
  for (const keyword of ['initial', 'inherit', 'unset', 'default', 'revert', 'revert-layer']) {
    check('quoted-' + keyword, (await set.load('12px "' + keyword + '"')).length, 0);
  }
  const bytes = Uint8Array.from(fontBytes);
  const valid = new FontFace('Binary', bytes);
  check('binary', [await valid.loaded === valid, set.add(valid) === set, set.has(valid), set.check('12px Binary'), (await set.load('12px Binary'))[0] === valid], [true, true, true, true, true]);
  const invalid = new FontFace('Invalid', new Uint8Array([0, 1, 0, 0]));
  check('invalid-binary', await invalid.loaded.catch(error => [error.name, error instanceof DOMException]), ['SyntaxError', true]);
  const source = 'url("data:font/ttf;base64,' + btoa(String.fromCharCode(...bytes)) + '")';
  const remote = new FontFace('Async', source);
  set.add(remote);
  const events = [];
  for (const name of ['loading', 'loadingdone', 'loadingerror']) set.addEventListener(name, event => events.push([event.type, event.fontfaces.length]));
  const done = new Promise(resolve => set.addEventListener('loadingdone', resolve, {once: true}));
  const ready = set.ready;
  const loaded = remote.load();
  check('loading-state', [remote.status, set.status, set.ready === ready, loaded === remote.loaded], ['loading', 'loading', false, true]);
  for (let i = 0; i < 20; i++) clearTimeout(i);
  check('completion', [await loaded === remote, await set.ready === set, remote.status, set.status], [true, true, 'loaded', 'loaded']);
  await done;
  check('events', events, [['loading', 0], ['loadingdone', 1]]);
  self.fetch = () => { throw new Error('author fetch must not be used'); };
  Object.defineProperty(ArrayBuffer.prototype, 'then', {configurable: true, get() { throw new Error('private font bytes'); }});
  const fallback = new FontFace('Fallback', 'url(data:font/ttf;base64,AAEAAA==), ' + source);
  check('fallback', await fallback.load().then(value => value === fallback), true);
  delete ArrayBuffer.prototype.then;
  check('collection', [set.size, [...set].length, set.delete(valid), set.has(valid), set.size], [2, 2, true, false, 1]);
  set.clear();
  check('clear', [set.size, [...set].length], [0, 0]);
  return {rows, failures};
}
