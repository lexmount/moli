(() => {
  const EventConstructor = Event;
  const mainBody = document.body;
  const parser = new DOMParser();
  const parsed = () => parser.parseFromString('<title>old</title><p>old</p>', 'text/html');
  const unsafe = () => Document.parseHTMLUnsafe('<p>old</p>');
  const sources = [
    ['implementation', document.implementation.createHTMLDocument('old'), false],
    ['DOMParser', parsed(), false],
    ['parseHTMLUnsafe', unsafe(), true],
    ['live shallow clone', document.cloneNode(), true],
    ['live deep clone', document.cloneNode(true), true],
    ['DOMParser shallow clone', parsed().cloneNode(), false],
    ['DOMParser deep clone', parsed().cloneNode(true), false],
    ['unsafe shallow clone', unsafe().cloneNode(), true],
    ['unsafe deep clone', unsafe().cloneNode(true), true],
  ];
  const failures = [];
  let checks = 0;
  function equal(label, actual, expected) {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  }
  for (const [name, doc, allowShadow] of sources) {
    try {
      const originalURL = doc.URL;
      let erasedListenerCalls = 0;
      doc.addEventListener('probe', () => erasedListenerCalls++);
      equal(name + ' open receiver', doc.open() === doc, true);
      equal(name + ' open clears all children', doc.childNodes.length, 0);
      equal(name + ' open readyState', doc.readyState, 'loading');
      equal(name + ' open resets mode', doc.compatMode, 'CSS1Compat');
      equal(name + ' open keeps URL', doc.URL, originalURL);
      doc.dispatchEvent(new EventConstructor('probe'));
      equal(name + ' open erases listeners', erasedListenerCalls, 0);
      const events = [];
      for (const type of ['readystatechange', 'DOMContentLoaded', 'load']) {
        doc.addEventListener(type, event => events.push([
          event.type, doc.readyState, event.bubbles, event.isTrusted,
          event.target === doc, event.currentTarget === doc,
        ]));
      }
      doc.write('<!doctype html><html><head><title>written</title></head><body><section id="host"><template shadowrootmode="open"><span>shadow</span></template><b id="keep">light</b>');
      equal(name + ' document title', doc.title, 'written');
      equal(name + ' document doctype', doc.doctype?.name, 'html');
      equal(name + ' write retains loading', doc.readyState, 'loading');
      equal(name + ' write has no lifecycle events yet', events.length, 0);
      const host = doc.getElementById('host');
      const keep = doc.getElementById('keep');
      equal(name + ' declarative shadow root', host?.shadowRoot?.textContent ?? null, allowShadow ? 'shadow' : null);
      equal(name + ' ordinary template when disabled', !!host?.querySelector('template'), !allowShadow);
      let listenerCalls = 0;
      keep.addEventListener('probe', () => listenerCalls++);
      const observer = new MutationObserver(() => {});
      observer.observe(host, {childList: true});
      doc.write('<i id="ta');
      doc.write('il">tail</i></section>');
      const tail = doc.getElementById('tail');
      equal(name + ' streamed token', tail?.textContent, 'tail');
      equal(name + ' tree builder insertion point', tail?.parentNode === host, true);
      equal(name + ' existing node identity', doc.getElementById('keep') === keep, true);
      keep.dispatchEvent(new EventConstructor('probe'));
      equal(name + ' existing listener identity', listenerCalls, 1);
      equal(name + ' parser insert is observable', observer.takeRecords().some(record => Array.from(record.addedNodes).includes(tail)), true);
      observer.disconnect();
      const priorEvent = globalThis.Event;
      globalThis.Event = function() { throw new Error('author Event constructor called'); };
      try { doc.close(); } finally { globalThis.Event = priorEvent; }
      equal(name + ' close readiness', doc.readyState, 'complete');
      equal(name + ' close lifecycle', JSON.stringify(events), JSON.stringify([
        ['readystatechange', 'interactive', false, true, true, true],
        ['DOMContentLoaded', 'interactive', true, true, true, true],
        ['readystatechange', 'complete', false, true, true, true],
      ]));
      const closedRoot = doc.documentElement;
      doc.close();
      equal(name + ' repeated close keeps tree', doc.documentElement === closedRoot, true);
      equal(name + ' repeated close has no events', events.length, 3);
      doc.writeln('<p id="replacement">new</p>');
      equal(name + ' implicit open replaces previous tree', doc.getElementById('keep'), null);
      equal(name + ' implicit open creates body', doc.body?.textContent, 'new\n');
      equal(name + ' no doctype enters quirks mode', doc.compatMode, 'BackCompat');
      doc.close();
      equal(name + ' implicit open clears previous listeners', events.length, 3);
      doc.open();
      doc.write('<script>globalThis.__windowlessStreamScriptRan = true;</scr' + 'ipt>');
      doc.close();
      equal(name + ' scripts remain inert', globalThis.__windowlessStreamScriptRan, undefined);
      equal(name + ' main Document unchanged', document.body === mainBody, true);
      equal(name + ' no browsing context', doc.defaultView, null);
    } catch (error) {
      equal(name + ' unexpected exception', error.name + ': ' + error.message, null);
    }
  }
  try {
    const a = document.implementation.createHTMLDocument('');
    const b = document.implementation.createHTMLDocument('');
    a.write('<div id="a">');
    b.write('<div id="b">');
    const rootA = a.getElementById('a');
    const rootB = b.getElementById('b');
    a.write('<b>A</b>');
    b.write('<i>B</i>');
    a.close();
    b.close();
    equal('independent parser A', rootA?.firstChild?.textContent, 'A');
    equal('independent parser B', rootB?.firstChild?.textContent, 'B');
    equal('independent node identities', a.getElementById('a') === rootA && b.getElementById('b') === rootB, true);

    const doc = document.implementation.createHTMLDocument('');
    doc.open();
    let oldDOMContentLoaded = 0;
    doc.addEventListener('DOMContentLoaded', () => oldDOMContentLoaded++);
    doc.addEventListener('readystatechange', () => {
      if (doc.readyState === 'interactive') {
        doc.open();
        doc.write('<p id="new-stream">replacement</p>');
      }
    });
    doc.write('<p>old stream</p>');
    doc.close();
    equal('reentrant open retains replacement parser', doc.readyState, 'loading');
    equal('reentrant open retains replacement content', doc.getElementById('new-stream')?.textContent, 'replacement');
    equal('old close does not dispatch after replacement', oldDOMContentLoaded, 0);
    doc.close();
    equal('replacement can close independently', doc.readyState, 'complete');
    doc.open();
    doc.close();
    equal('empty stream produces document structure', doc.documentElement?.localName + '/' + doc.head?.localName + '/' + doc.body?.localName, 'html/head/body');
  } catch (error) {
    equal('stream lifecycle unexpected exception', error.name + ': ' + error.message, null);
  }
  try {
    const frame = document.body.appendChild(document.createElement('iframe'));
    try {
      const other = frame.contentWindow;
      const doc = other.document.implementation.createHTMLDocument('');
      Document.prototype.open.call(doc);
      const events = [];
      for (const type of ['readystatechange', 'DOMContentLoaded']) {
        doc.addEventListener(type, event => events.push([
          event.type, event instanceof other.Event, event instanceof Event,
        ]));
      }
      Document.prototype.write.call(doc, '<p>content</p>');
      Document.prototype.close.call(doc);
      equal('lifecycle events use the Document realm', JSON.stringify(events), JSON.stringify([
        ['readystatechange', true, false],
        ['DOMContentLoaded', true, false],
        ['readystatechange', true, false],
      ]));
    } finally {
      frame.remove();
    }
  } catch (error) {
    equal('cross-realm lifecycle unexpected exception', error.name + ': ' + error.message, null);
  }
  return {checks, failures};
})()
