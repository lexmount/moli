async function(fontBytes) {
  const rows = [], failures = [];
  const bytes = format => Uint8Array.from(fontBytes[format]);
  const check = (name, actual, expected) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  async function probe(name, make, expected) {
    let conversions = 0, descriptors = 0;
    const source = make();
    try {
      source[Symbol.toPrimitive] = () => { conversions++; return 'url(font.woff2)'; };
      const face = new FontFace(name, source, {get style() { descriptors++; return 'normal'; }});
      const loaded = face.loaded;
      const outcome = await face.load().then(value => value === face ? 'loaded' : 'wrong value', e => e.name);
      check(name, [outcome, face.status, loaded === face.loaded, loaded === face.load(), conversions, descriptors],
        [expected, expected === 'loaded' ? 'loaded' : 'error', true, true, 0, 1]);
    } catch (error) {
      check(name, [error.name, conversions, descriptors], [expected, 0, 0]);
    }
  }
  for (const format of ['ttf', 'woff', 'woff2']) {
    const data = bytes(format);
    for (const kind of ['buffer', 'view', 'offset-view', 'data-view']) {
      await probe(format + '-' + kind, () => {
        const padded = new Uint8Array(data.length + 20); padded.set(data, 8);
        return kind === 'buffer' ? data.slice().buffer : kind === 'view' ? data.slice() :
          kind === 'offset-view' ? padded.subarray(8, 8 + data.length) : new DataView(padded.buffer, 8, data.length);
      }, 'loaded');
    }
    for (const size of [0, 4, 8, 12, 44, 48, data.length - 10]) {
      await probe(format + '-truncated-' + size, () => data.slice(0, size), 'SyntaxError');
    }
  }
  for (const magic of ['OTTO', 'ttcf', 'true', 'typ1']) {
    await probe('magic-' + magic, () => new TextEncoder().encode(magic), 'SyntaxError');
  }
  for (const kind of ['buffer', 'view', 'data-view']) {
    await probe('detached-' + kind, () => {
      const buffer = bytes('ttf').buffer;
      const source = kind === 'buffer' ? buffer : kind === 'view' ? new Uint8Array(buffer) : new DataView(buffer);
      structuredClone(buffer, {transfer: [buffer]}); return source;
    }, 'SyntaxError');
    await probe('resizable-' + kind, () => {
      const data = bytes('ttf'), buffer = new ArrayBuffer(data.length, {maxByteLength: data.length * 2});
      new Uint8Array(buffer).set(data);
      return kind === 'buffer' ? buffer : kind === 'view' ? new Uint8Array(buffer) : new DataView(buffer);
    }, 'TypeError');
    await probe('shared-' + kind, () => {
      const data = bytes('ttf'), buffer = new WebAssembly.Memory({initial: 1, maximum: 1, shared: true}).buffer;
      new Uint8Array(buffer).set(data);
      return kind === 'buffer' ? buffer : kind === 'view' ? new Uint8Array(buffer, 0, data.length) : new DataView(buffer, 0, data.length);
    }, 'TypeError');
  }

  const mutable = bytes('ttf');
  const copied = new FontFace('Copied', mutable, {get style() { mutable.fill(0); return 'normal'; }});
  check('descriptor-mutation-before-copy', [await copied.loaded.catch(e => e.name), copied.status, mutable[1]], ['SyntaxError', 'error', 0]);
  const detached = bytes('ttf');
  const detachedFace = new FontFace('Detached', detached, {get style() {
    structuredClone(detached.buffer, {transfer: [detached.buffer]}); return 'normal';
  }});
  check('descriptor-detachment-before-copy', await detachedFace.loaded.catch(e => e.name), 'SyntaxError');
  const repaired = new Uint8Array(fontBytes.ttf.length);
  const repairedFace = new FontFace('Repaired', repaired, {get style() { repaired.set(bytes('ttf')); return 'normal'; }});
  check('descriptor-repair-before-copy', await repairedFace.loaded === repairedFace, true);
  const transferred = bytes('ttf');
  const retained = new FontFace('Retained', transferred);
  structuredClone(transferred.buffer, {transfer: [transferred.buffer]});
  check('retained-after-transfer', [await retained.load() === retained, retained.status], [true, 'loaded']);
  await probe('whole-padded-buffer', () => {
    const data = bytes('ttf'), padded = new Uint8Array(data.length + 20); padded.set(data, 8); return padded;
  }, 'SyntaxError');

  for (const format of ['ttf', 'woff', 'woff2']) {
    for (const trim of [0, 10]) {
      const data = bytes(format).slice(0, fontBytes[format].length - trim);
      const source = 'url("data:font/ttf;base64,' + btoa(String.fromCharCode(...data)) + '")';
      const face = new FontFace(format + trim, source);
      check('url-' + format + '-' + trim, await face.load().then(() => face.status, e => e.name), trim ? 'NetworkError' : 'loaded');
    }
  }

  const iframe = document.createElement('iframe');
  const ready = new Promise(resolve => iframe.onload = resolve);
  document.body.append(iframe); await ready;
  const other = iframe.contentWindow;
  const valid = new other.FontFace('Foreign', bytes('ttf'));
  const load = FontFace.prototype.load.call(valid);
  const invalid = new other.FontFace('Invalid', new Uint8Array([0, 1, 0, 0]));
  const rejection = await invalid.loaded.catch(e => [e.name, e instanceof other.DOMException]);
  let thrown;
  try { new other.FontFace('Resizable', new ArrayBuffer(8, {maxByteLength: 16})); }
  catch (e) { thrown = [e instanceof other.TypeError, e instanceof TypeError]; }
  check('cross-realm', [load instanceof other.Promise, await load === valid, rejection, thrown],
    [true, true, ['SyntaxError', true], [true, false]]);
  iframe.remove();
  return {rows, failures};
}
