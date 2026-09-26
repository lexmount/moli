window.writeErrorTrace = [];
window.writeErrorDetails = [];
window.writeErrorEscaped = [];
window.writeErrorRestored = [];
window.writeErrorToken = new Error('written runtime exception');

window.addEventListener('error', event => {
  const script = document.currentScript;
  const id = script && script.id;
  writeErrorTrace.push('error:' + id);
  writeErrorDetails.push({
    id,
    identity: id === 'runtime' ? event.error === writeErrorToken :
      id === 'syntax' ? event.error instanceof SyntaxError :
      id === 'external' && event.error === window.externalWriteError,
    filename: event.filename === (script && script.src || document.URL),
    target: event.target === window,
    trusted: event.isTrusted,
    cancelable: event.cancelable
  });
  event.preventDefault();
  document.write('<b class="reported">' + id + '</b>');
  queueMicrotask(() => writeErrorTrace.push('report-microtask:' + id));
});

function writeFailingScript(id, source, external) {
  const attributes = external ? ' src="data:text/javascript,' + encodeURIComponent(source) +
    '" onload="writeErrorTrace.push(\'load:external\')" onerror="writeErrorTrace.push(\'load-error:external\')"' : '';
  try {
    document.write('<script id="' + id + '"' + attributes + '>' +
      (external ? '' : source) + '<' + '/script>');
  } catch (error) {
    writeErrorEscaped.push(id);
  }
  writeErrorRestored.push(document.currentScript && document.currentScript.id);
  writeErrorTrace.push('returned:' + id);
}

writeFailingScript('runtime',
  'writeErrorTrace.push("body:runtime"); queueMicrotask(() => writeErrorTrace.push("body-microtask:runtime")); throw writeErrorToken;',
  false);
writeFailingScript('syntax', 'let = ;', false);
writeFailingScript('external',
  'writeErrorTrace.push("body:external"); queueMicrotask(() => writeErrorTrace.push("body-microtask:external")); window.externalWriteError = new TypeError("external writer"); throw externalWriteError;',
  true);
document.write('<script>writeErrorTrace.push("tail-script")<' + '/script><main id="tail">tail</main>');
writeErrorTrace.push('outer-end');

window.writeErrorResult = () => {
  let checks = 0;
  const failures = [];
  const check = (name, actual, expected) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected))
      failures.push({name, actual, expected});
  };
  const before = (first, second) => writeErrorTrace.includes(first) &&
    writeErrorTrace.indexOf(first) < writeErrorTrace.indexOf(second);
  check('exceptions stay inside written scripts', writeErrorEscaped, []);
  check('each exception reaches its Window', writeErrorDetails.map(error => error.id),
    ['runtime', 'syntax', 'external']);
  for (const field of ['identity', 'filename', 'target', 'trusted', 'cancelable'])
    check('error event ' + field, writeErrorDetails.map(error => error[field]), [true, true, true]);
  check('currentScript returns to the writer', writeErrorRestored, ['outer', 'outer', 'outer']);
  check('error handlers retain the insertion point',
    Array.from(document.querySelectorAll('.reported'), node => node.textContent),
    ['runtime', 'syntax', 'external']);
  check('inline runtime error precedes write return', before('error:runtime', 'returned:runtime'), true);
  check('inline syntax error precedes write return', before('error:syntax', 'returned:syntax'), true);
  check('external evaluation error still allows load', writeErrorTrace.filter(value => value.startsWith('load')), ['load:external']);
  check('external error precedes body microtasks', before('error:external', 'body-microtask:external'), true);
  check('inline error precedes its microtasks', before('error:runtime', 'body-microtask:runtime'), true);
  check('microtasks retain enqueue order', before('body-microtask:external', 'report-microtask:external'), true);
  check('load precedes later parser scripts', before('load:external', 'tail-script'), true);
  check('outer writer continues', writeErrorTrace.includes('outer-end'), true);
  check('parser reaches the written tail', document.getElementById('tail')?.textContent, 'tail');
  check('currentScript clears after parsing', document.currentScript, null);
  return {checks, failures};
};
