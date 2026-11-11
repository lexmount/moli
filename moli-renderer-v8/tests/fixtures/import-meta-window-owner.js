async function() {
  const result = {checks: 0, failures: []};
  const check = (name, value) => {
    result.checks++;
    if (!value) result.failures.push(name);
  };
  const url = path => new URL(path, location.href).href;
  const addMap = (doc, map) => {
    const script = doc.createElement('script');
    script.type = 'importmap';
    script.textContent = JSON.stringify(map);
    doc.head.appendChild(script);
  };
  const ready = id => new Promise(resolve => {
    const handler = event => {
      if (event.data !== id) return;
      removeEventListener('message', handler);
      resolve();
    };
    addEventListener('message', handler);
  });
  const html = (id, target) => '<!doctype html><base href="/scope/module.html">' +
    '<script type="importmap">' + JSON.stringify({
      imports: {key: target, blocked: null, 'pkg/': '/child-package/'},
      scopes: {'/scope/': {key: target + '?scoped'}}
    }) + '</script><script type="module">' +
    'window.metaResolve = import.meta.resolve; parent.postMessage(' + JSON.stringify(id) + ', "*");' +
    '</script><body>child';
  const probe = (resolve, input) => {
    try { return resolve(input); }
    catch (error) { return 'throws:' + error.name; }
  };
  addMap(document, {imports: {key: '/parent.mjs', blocked: '/parent-allowed.mjs'}});
  const parentResolve = (await import('data:text/javascript,export default import.meta.resolve;')).default;
  const frame = document.createElement('iframe');
  const firstReady = ready('first-import-meta-owner');
  frame.srcdoc = html('first-import-meta-owner', '/first.mjs');
  document.body.appendChild(frame);
  await firstReady;
  const first = frame.contentWindow;
  const resolve = first.metaResolve;
  const FirstTypeError = first.TypeError;
  check('parent map', parentResolve('key') === url('/parent.mjs'));
  check('child scoped map', probe(resolve, 'key') === url('/first.mjs?scoped'));
  check('child package map', probe(resolve, 'pkg/a.mjs') === url('/child-package/a.mjs'));
  check('child null entry', probe(resolve, 'blocked') === 'throws:TypeError');
  let nullError;
  try { resolve('blocked'); } catch (error) { nullError = error; }
  check('resolution TypeError belongs to child', Object.getPrototypeOf(nullError || {}) === FirstTypeError.prototype);
  check('receiver does not choose map', probe(resolve.bind({url: 'https://wrong.test/'}), 'key') === url('/first.mjs?scoped'));
  first.document.querySelector('base').href = '/different/';
  check('preparation base is retained', resolve('./relative.mjs') === url('/scope/relative.mjs'));
  check('scoped map uses preparation base', probe(resolve, 'key') === url('/first.mjs?scoped'));
  let conversions = 0;
  const converted = probe(resolve, {toString() {
    conversions++;
    addMap(first.document, {imports: {during: '/during-conversion.mjs'}});
    return 'during';
  }});
  check('map update during conversion', converted === url('/during-conversion.mjs'));
  check('convert once', conversions === 1);
  addMap(first.document, {imports: {key: '/wrong-late.mjs', fresh: '/fresh.mjs'}});
  check('resolved key is not remapped', probe(resolve, 'key') === url('/first.mjs?scoped'));
  check('unresolved key sees later map', probe(resolve, 'fresh') === url('/fresh.mjs'));
  const secondReady = ready('second-import-meta-owner');
  frame.srcdoc = html('second-import-meta-owner', '/second.mjs');
  await secondReady;
  const replacement = frame.contentWindow;
  const replacementResolve = replacement.metaResolve;
  check('replacement Window map', probe(replacementResolve, 'key') === url('/second.mjs?scoped'));
  check('old resolver keeps old Window map', probe(resolve, 'key') === url('/first.mjs?scoped'));
  replacement.foreignResolve = resolve;
  check('call from replacement realm', replacement.eval('foreignResolve("key")') === url('/first.mjs?scoped'));
  replacement.parentResolve = parentResolve;
  check('parent resolver keeps parent map', replacement.eval('parentResolve("key")') === url('/parent.mjs'));
  const marker = {};
  let thrown;
  try { resolve({toString() { throw marker; }}); } catch (error) { thrown = error; }
  check('conversion exception identity', thrown === marker);
  const afterRemoval = probe(replacementResolve, {toString() {
    frame.remove();
    return 'key';
  }});
  check('removal during conversion keeps map', afterRemoval === url('/second.mjs?scoped'));
  check('old resolver after removal', probe(resolve, 'key') === url('/first.mjs?scoped'));
  check('replacement resolver after removal', probe(replacementResolve, 'key') === url('/second.mjs?scoped'));
  check('main map remains isolated', parentResolve('key') === url('/parent.mjs'));
  result.completed = true;
  return result;
}
