async function() {
  const result = {checks: 0, failures: []};
  const check = (name, value) => {
    result.checks++;
    if (!value) result.failures.push(name);
  };
  const url = path => new URL(path, location.href).href;
  const ready = id => new Promise(resolve => {
    const handler = event => {
      if (event.data !== id) return;
      removeEventListener('message', handler);
      resolve();
    };
    addEventListener('message', handler);
  });
  const html = (id, imports) => '<!doctype html><base href="/open/base.html">' +
    '<script type="importmap">' + JSON.stringify({imports}) + '</script>' +
    '<script type="module">window.metaResolve = import.meta.resolve; parent.postMessage(' +
    JSON.stringify(id) + ', "*");</script><body>child';
  const probe = (resolve, input) => {
    try { return resolve(input); }
    catch (error) { return 'throws:' + error.name; }
  };
  const frame = document.createElement('iframe');
  const initialReady = ready('import-meta-before-open');
  frame.srcdoc = html('import-meta-before-open', {key: '/before-open.mjs'});
  document.body.appendChild(frame);
  await initialReady;
  const child = frame.contentWindow;
  const oldResolve = child.metaResolve;
  check('initial map', probe(oldResolve, 'key') === url('/before-open.mjs'));
  const opened = ready('import-meta-after-open');
  child.document.open();
  child.document.write(html('import-meta-after-open', {key: '/wrong-remap.mjs', late: '/after-open.mjs'}));
  child.document.close();
  await opened;
  const newResolve = child.metaResolve;
  check('same Window after document.open', frame.contentWindow === child);
  check('new resolver preserves old map', probe(newResolve, 'key') === url('/before-open.mjs'));
  check('old resolver preserves old map', probe(oldResolve, 'key') === url('/before-open.mjs'));
  check('new resolver sees late map', probe(newResolve, 'late') === url('/after-open.mjs'));
  check('old resolver sees same late map', probe(oldResolve, 'late') === url('/after-open.mjs'));
  frame.remove();
  check('old map survives removal', probe(oldResolve, 'key') === url('/before-open.mjs'));
  check('late map survives removal', probe(newResolve, 'late') === url('/after-open.mjs'));
  result.completed = true;
  return result;
}
