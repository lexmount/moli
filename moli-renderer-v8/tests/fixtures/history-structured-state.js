async function historyStructuredState(base, mode) {
  const frame = document.createElement('iframe');
  const firstURL = base + '/common/blank.html?state';
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  async function load(action) {
    const loaded = new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
    action();
    await loaded;
    await settle();
  }
  await load(() => {
    frame.src = firstURL;
    document.body.appendChild(frame);
  });
  let getterCalls = 0, jsonCalls = 0;
  const w = frame.contentWindow;
  w.recordStateGetter = () => getterCalls++;
  w.recordJSONHook = () => jsonCalls++;
  w.eval(`
    Object.defineProperty(Object.prototype, 'toJSON', {
      configurable: true,
      value() { recordJSONHook(); throw new Error('history must not use JSON'); }
    });
    const shared = {value: 42};
    const buffer = new ArrayBuffer(8);
    new Uint8Array(buffer).set([10, 20, 30, 40, 50, 60, 70, 80]);
    globalThis.sourceState = {
      shared, alias: shared,
      date: new Date(123456), regexp: /state/gi,
      map: new Map([[shared, new Set([shared])]]),
      bytes: new Uint8Array(buffer, 1, 3), view: new DataView(buffer, 2, 2),
      blob: new Blob(['history'], {type: 'text/plain'}),
      file: new File(['entry'], 'state.txt', {type: 'text/plain', lastModified: 123}),
      exception: new DOMException('state message', 'DataError'),
      point: new DOMPoint(1, 2, 3, 4),
      bigint: 12345678901234567890n,
      undefinedValue: undefined, nan: NaN, infinity: Infinity, negativeZero: -0,
      sparse: [, undefined, 3],
      get observed() { recordStateGetter(); return 17; }
    };
    sourceState.self = sourceState;
  `);
  if (mode === 'traverse-builtins') w.eval('delete sourceState.blob; delete sourceState.file;');
  const primitive = mode.startsWith('null') || mode.startsWith('undefined');
  const value = primitive ? (mode.startsWith('null') ? 'null' : 'undefined') : 'sourceState';
  w.eval(`history.replaceState(${value}, '');`);
  if (value !== 'undefined') w.eval(`navigation.updateCurrentEntry({state: ${value}});`);

  async function graphChecks(state, realm) {
    if (primitive) return {primitive: state === (mode.startsWith('null') ? null : undefined)};
    const checks = {
      cycle: state.self === state,
      alias: state.shared === state.alias && state.shared.value === 42,
      date: state.date instanceof realm.Date && state.date.getTime() === 123456,
      regexp: state.regexp instanceof realm.RegExp && state.regexp.source === 'state' && state.regexp.flags === 'gi',
      map: state.map instanceof realm.Map && state.map.get(state.shared) instanceof realm.Set && state.map.get(state.shared).has(state.shared),
      bytes: state.bytes instanceof realm.Uint8Array && state.bytes.byteOffset === 1 && Array.from(state.bytes).join() === '20,30,40',
      view: state.view instanceof realm.DataView && state.view.byteOffset === 2 && state.view.byteLength === 2 && state.view.getUint8(0) === 30,
      buffer: state.bytes.buffer === state.view.buffer,
      exception: state.exception instanceof realm.DOMException && state.exception.name === 'DataError' && state.exception.message === 'state message',
      point: state.point instanceof realm.DOMPoint && state.point.x === 1 && state.point.y === 2 && state.point.z === 3 && state.point.w === 4,
      bigint: state.bigint === 12345678901234567890n,
      undefinedValue: Object.hasOwn(state, 'undefinedValue') && state.undefinedValue === undefined,
      numbers: Number.isNaN(state.nan) && state.infinity === Infinity && Object.is(state.negativeZero, -0),
      sparse: state.sparse.length === 3 && !(0 in state.sparse) && 1 in state.sparse && state.sparse[1] === undefined && state.sparse[2] === 3,
      getter: state.observed === 17 && !Object.getOwnPropertyDescriptor(state, 'observed').get,
    };
    if (mode !== 'traverse-builtins') {
      checks.blob = state.blob instanceof realm.Blob && state.blob.type === 'text/plain' && await state.blob.text() === 'history';
      checks.file = state.file instanceof realm.File && state.file.name === 'state.txt' && state.file.lastModified === 123 && await state.file.text() === 'entry';
    }
    return checks;
  }
  async function record(expectNullHistory = false) {
    const realm = frame.contentWindow;
    return {
      history: expectNullHistory ? {cleared: realm.history.state === null} : await graphChecks(realm.history.state, realm),
      navigation: await graphChecks(realm.navigation.currentEntry.getState(), realm),
    };
  }
  const before = await record();
  if (!primitive) {
    // Neither caller mutations nor mutations to exposed clones may change
    // the snapshot that a later Document or entry receives.
    w.sourceState.shared.value = 99;
    w.history.state.shared.value = 100;
    w.navigation.currentEntry.getState().shared.value = 101;
  }
  if (mode.startsWith('traverse')) {
    await load(() => w.location.href = base + '/common/blank.html?away');
    await load(() => frame.contentWindow.history.back());
  } else if (mode === 'fragment') {
    const changed = new Promise(resolve => w.addEventListener('hashchange', resolve, {once: true}));
    w.location.hash = 'next';
    await changed;
  } else {
    await load(() => w.location.reload());
  }
  const after = await record(mode === 'fragment');
  const result = {before, after, getterCalls, jsonCalls};
  frame.remove();
  return result;
}

async function historyStateStoragePolicy() {
  const module = new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0]));
  history.replaceState('safe', '');
  navigation.updateCurrentEntry({state: 'safe'});
  const entry = navigation.currentEntry;
  let events = 0;
  navigation.addEventListener('navigate', event => { events++; event.preventDefault(); });
  const errors = [];
  try { history.replaceState(module, ''); errors.push('accepted'); }
  catch (error) { errors.push(error.name); }
  try { navigation.updateCurrentEntry({state: module}); errors.push('accepted'); }
  catch (error) { errors.push(error.name); }
  for (const operation of [() => navigation.navigate('#bad', {state: module}), () => navigation.reload({state: module})]) {
    const result = operation();
    const [committed, finished] = await Promise.all([result.committed.catch(e => e), result.finished.catch(e => e)]);
    errors.push(committed.name, finished.name);
  }
  return {errors, events, entryUnchanged: entry === navigation.currentEntry, history: history.state,
          navigation: navigation.currentEntry.getState() === 'safe', runtimeCloneAllowed: structuredClone(module) instanceof WebAssembly.Module};
}
