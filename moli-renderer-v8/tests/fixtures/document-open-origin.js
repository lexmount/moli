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
    catch (error) {
      equal([error.name, error instanceof Constructor], [name, true], label);
      if (name === 'SecurityError') equal(error.code, 18, label + ' code');
    }
  };
  document.domain = location.hostname;
  const create = async url => {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = url; document.body.append(frame);
    await loaded;
    return frame;
  };
  const same = await create(sameURL);
  const remote = await create(crossURL);
  const w = remote.contentWindow, d = remote.contentDocument;
  equal(d !== null, true, 'domain relaxation permits DOM access');
  const childException = w.DOMException;
  const originDocuments = [
    ['live', d],
    ['constructed', d.implementation.createHTMLDocument('child')],
    ['parsed', new w.DOMParser().parseFromString('<p>parsed</p>', 'text/html')],
    ['borrowed parser', DOMParser.prototype.parseFromString.call(new w.DOMParser(), '<p>parsed</p>', 'text/html')],
    ['cloned', d.cloneNode(true)]
  ];
  for (const [kind, doc] of originDocuments) {
    const original = doc.documentElement, url = doc.URL;
    for (const method of ['open', 'write', 'writeln']) {
      throws(() => doc[method]('<p>replacement</p>'),
        doc.open === Document.prototype.open ? DOMException : childException,
        'SecurityError', kind + '/' + method);
      throws(() => Document.prototype[method].call(doc, '<p>replacement</p>'),
        DOMException, 'SecurityError', kind + '/borrowed ' + method);
      equal(doc.documentElement === original, true, kind + '/' + method + ' keeps tree');
      equal(doc.URL, url, kind + '/' + method + ' keeps URL');
    }
  }
  // The function's realm is not the entry realm of this synchronous call.
  throws(() => w.Function('doc', 'return doc.open()')(d), childException,
    'SecurityError', 'foreign JS function does not change entry document');
  const xml = new w.DOMParser().parseFromString('<root/>', 'application/xml');
  for (const method of ['open', 'write', 'writeln'])
    throws(() => xml[method]('text'), childException, 'InvalidStateError', 'XML precedes origin/' + method);
  let converted = 0;
  throws(() => d.write({toString() {converted++; return 'text'}}), childException,
    'SecurityError', 'write converts before origin check');
  equal(converted, 1, 'write conversion count');
  const conversionError = {};
  try { d.write({toString() {throw conversionError}}); equal('returned', 'threw', 'conversion error'); }
  catch (error) { equal(error === conversionError, true, 'conversion exception precedes origin'); }

  // Same-origin borrowed methods and inherited about:blank origins remain valid.
  for (const method of ['open', 'write', 'writeln']) {
    const doc = same.contentDocument.implementation.createHTMLDocument('allowed');
    try { w.Document.prototype[method].call(doc, 'allowed'); equal(true, true, 'same-origin/' + method); }
    catch (error) { equal(error.name, 'allowed', 'same-origin/' + method); }
    doc.close();
  }
  const blank = document.body.appendChild(document.createElement('iframe'));
  equal(blank.contentDocument.open() === blank.contentDocument, true, 'inherited about:blank open');
  blank.contentDocument.close();
  blank.remove();

  const root = document.documentElement;
  const response = new Promise(resolve => {
    const listener = event => {
      if (event.source !== w || event.data?.token !== 'document-open-origin') return;
      removeEventListener('message', listener); resolve(event.data);
    };
    addEventListener('message', listener);
  });
  w.postMessage('document-open-origin', '*');
  const observed = await response;
  equal(observed.attempts, [['SecurityError',18,true],['SecurityError',18,true]], 'child entry rejects parent document');
  equal(observed.own, true, 'child entry can open its own document');
  equal(document.documentElement === root, true, 'rejected parent open keeps root');
  const stream = d.getElementById('stream');
  try { d.write('parent'); d.writeln(' tail'); }
  catch (error) { equal(error.name, 'allowed', 'existing insertion point does not run open steps'); }
  equal(d.getElementById('stream') === stream, true, 'existing stream is not replaced');
  equal(stream.textContent, 'childparent tail\n', 'cross-origin write into existing stream');
  throws(() => d.open(), childException, 'SecurityError', 'explicit open still rejects existing stream');
  d.close();
  throws(() => d.write('closed'), childException, 'SecurityError', 'closed stream requires new origin check');
  same.remove(); remote.remove();
  for (const [kind, doc] of originDocuments)
    throws(() => Document.prototype.open.call(doc), DOMException, 'SecurityError', 'retained origin/' + kind);
  return {checks, failures};
}
