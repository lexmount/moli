(() => {
  const passed = [], failures = [];
  const check = (name, action) => {
    try {
      if (!action()) throw new Error('assertion returned false');
      passed.push(name);
    } catch (error) {
      failures.push({name, message: String(error && error.stack || error)});
    }
  };
  const iframe = document.createElement('iframe');
  document.body.appendChild(iframe);
  const foreign = iframe.contentWindow;
  for (const [label, realm] of [['top', window], ['child', foreign]]) {
    const owner = realm.document;
    const cue = new realm.VTTCue(10, 11, '');
    const method = realm.VTTCue.prototype.getCueAsHTML;
    const parse = text => { cue.text = text; return method.call(cue); };
    const one = text => parse(text).firstChild;
    const test = (name, action) => check(label + '/' + name, action);
    test('empty fragment has no children', () => parse('').childNodes.length === 0);
    test('raw UTF-16 and NUL', () => one('\uD800x\uDC00\0\r\n').data === '\uD800x\uDC00\0\r\n');
    test('HTML named and longest references', () => one('&copy;&AMP;&notit;&nsubE;').data === '©&¬it;⫅\u0338');
    test('HTML numeric references', () => one('&#128;&#x1f600;&#0;&#xD800;&#1114112;').data === '€😀\uFFFD\uFFFD\uFFFD');
    test('unknown references', () => one('&unknown;&#x;&;').data === '&unknown;&#x;&;');
    test('entity less-than is text', () => one('&lt;b&gt;literal').data === '<b>literal');
    test('class UTF-16 and NUL', () => one('<c.\uD800.\uDC00\0>').getAttribute('class') === '\uD800 \uDC00\0');
    test('empty classes excluded', () => one('<c..a...b.>').getAttribute('class') === 'a b');
    test('class references remain literal', () => one('<c.&amp;>').getAttribute('class') === '&amp;');
    test('empty voice attribute exists', () => one('<v>').getAttribute('title') === '');
    test('voice annotation UTF-16', () => one('<v \uD800\0\uDC00>').getAttribute('title') === '\uD800\0\uDC00');
    test('annotation references and whitespace', () => one('<v \tFoo&amp;\r\n&#32;Bar\t>').getAttribute('title') === 'Foo& Bar');
    test('annotation preserves NBSP and VT', () => one('<v \u00A0\u000B >').getAttribute('title') === '\u00A0\u000B');
    test('annotation is not HTML attribute mode', () => one('<v &notit;>').getAttribute('title') === '¬it;');
    test('tag CR is not a delimiter', () => parse('<v\rname>text').textContent === 'text' && parse('<v\rname>text').firstChild.nodeType === 3);
    test('partial start tag at EOF', () => one('<b').localName === 'b');
    test('partial classes at EOF', () => one('<c.a').getAttribute('class') === 'a');
    test('partial annotation at EOF', () => one('<v speaker').getAttribute('title') === 'speaker');
    test('end tag names exact', () => one('<b>x</b >y</b.z>z').textContent === 'xyz');
    test('ruby close unwinds current rt', () => parse('<ruby>a<rt>b</ruby>c').childNodes.length === 2);
    test('rt requires immediate ruby', () => one('<ruby><b><rt>x').firstChild.firstChild.nodeType === 3);
    test('lang span annotation', () => one('<lang en-US>x').getAttribute('lang') === 'en-US');
    test('empty lang attribute exists', () => one('<lang>').getAttribute('lang') === '');
    test('nested lang closes one level', () => one('<lang en><lang fr>x</lang>y').childNodes.length === 2);
    test('lang does not add attributes to descendants', () => one('<lang en><b>x').firstChild.attributes.length === 0);
    test('timestamp is native PI', () => {const pi = one('<1:00:00.500>'); return pi instanceof realm.ProcessingInstruction && pi instanceof realm.CharacterData && pi.target === 'timestamp' && pi.data === '01:00:00.500';});
    test('timestamp does not round hours', () => one('<9007199254740993:00:00.999>').data === '9007199254740993:00:00.999');
    test('timestamp partial token at EOF', () => one('<00:00.001').data === '00:00:00.001');
    test('timestamp grammar rejects trailing data', () => parse('<00:00.000x>').childNodes.length === 0);
    test('timestamps need no cue interval or monotonic order', () => parse('<00:01.000><00:00.000><00:01.000>').childNodes.length === 3);
    test('fresh fragment and native ownership', () => {
      const first = parse('<v.a Voice><b>text</b><00:00.001>');
      const second = method.call(cue);
      const nodes = [first, first.firstChild, first.firstChild.firstChild, first.firstChild.firstChild.firstChild, first.firstChild.lastChild];
      return first !== second && nodes.every(node => node.ownerDocument === owner) && first instanceof realm.DocumentFragment && first.firstChild instanceof realm.HTMLSpanElement && first.firstChild.firstChild instanceof realm.HTMLElement && first.firstChild.firstChild.firstChild instanceof realm.Text;
    });
    test('native text ignores author getter and DOM factories', () => {
      cue.text = '<b>private</b>';
      Object.defineProperty(cue, 'text', {get() {throw new Error('author cue text getter');}, configurable:true});
      const descriptor = Object.getOwnPropertyDescriptor(realm.Document.prototype, 'createElement');
      Object.defineProperty(realm.Document.prototype, 'createElement', {value() {throw new Error('author DOM factory');}, configurable:true});
      try {return method.call(cue).firstChild.textContent === 'private';}
      finally {delete cue.text; Object.defineProperty(realm.Document.prototype, 'createElement', descriptor);}
    });
  }
  iframe.remove();
  return JSON.stringify({passed, failures});
})()
