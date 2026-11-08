(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const failure = (operation, name, label) => {
    let caught;
    try { operation(); } catch (error) { caught = error; }
    assert(caught?.name === name, label + ': ' + caught);
  };
  const source = (start, first, end, last) => new StaticRange({
    startContainer: start, startOffset: first, endContainer: end, endOffset: last,
  });
  const convert = range => new InputEvent('beforeinput', {targetRanges: [range]}).getTargetRanges()[0];
  const host = document.createElement('div'); document.body.append(host);
  const text = document.createTextNode('abc'), comment = document.createComment('abc');
  host.append(text, comment);
  for (const node of [text, comment, host]) {
    const length = node === host ? 2 : 3;
    for (const offset of [length + 1, 0xffffffff]) {
      failure(() => convert(source(node, offset, node, 0)), 'IndexSizeError', 'invalid start');
      failure(() => convert(source(node, 0, node, offset)), 'IndexSizeError', 'invalid end');
    }
  }
  const staleText = source(text, 2, text, 3);
  text.deleteData(0, 2);
  failure(() => convert(staleText), 'IndexSizeError', 'offset invalidated by character edit');
  const staleChildren = source(host, 0, host, 2);
  comment.remove();
  failure(() => convert(staleChildren), 'IndexSizeError', 'offset invalidated by child removal');
  text.data = 'abc';
  const reversed = convert(source(text, 2, text, 1));
  assert(reversed.startContainer === text && reversed.endContainer === text && reversed.startOffset === 1 && reversed.endOffset === 1, 'reversed offsets collapse at end');
  const first = document.createTextNode('first'), last = document.createTextNode('last'); host.append(first, last);
  const reversedNodes = convert(source(last, 2, first, 1));
  assert(reversedNodes.startContainer === first && reversedNodes.endContainer === first && reversedNodes.startOffset === 1 && reversedNodes.endOffset === 1, 'reversed nodes collapse at end');
  const detached = document.createTextNode('detached');
  const roots = convert(source(text, 1, detached, 2));
  assert(roots.startContainer === detached && roots.endContainer === detached && roots.startOffset === 2 && roots.endOffset === 2, 'different roots collapse at end');
  const live = new Range(); live.setStart(text, 2); live.setEnd(text, 1);
  assert(live.collapsed && live.startOffset === 1 && live.endOffset === 1, 'public setters share collapse algorithm');
  live.setEnd(detached, 2);
  assert(live.collapsed && live.startContainer === detached && live.startOffset === 2, 'public setters share root handling');
  failure(() => live.setStart(text, 99), 'IndexSizeError', 'public setter length validation');
  failure(() => live.setEnd(document.implementation.createDocumentType('html', '', ''), 0), 'InvalidNodeTypeError', 'doctype rejected before offset');
  const invalid = source(text, 99, text, 99);
  failure(() => new InputEvent('beforeinput', {targetRanges: [invalid, {}]}), 'TypeError', 'complete IDL conversion precedes live range algorithm');
  const protectedSource = source(text, 0, text, 2);
  const NativeRange = Range, NativeStaticRange = StaticRange;
  const setStart = Range.prototype.setStart, setEnd = Range.prototype.setEnd;
  const protectedLive = new Range();
  for (const key of ['nodeType', 'length', 'childNodes', 'parentNode', 'previousSibling'])
    Object.defineProperty(text, key, {configurable: true, get() {throw Error('author node property ' + key);}});
  for (const key of ['startContainer', 'startOffset', 'endContainer', 'endOffset'])
    Object.defineProperty(protectedSource, key, {get() {throw Error('author range boundary');}});
  try {
    globalThis.Range = function() {throw Error('author Range');};
    globalThis.StaticRange = function() {throw Error('author StaticRange');};
    NativeRange.prototype.setStart = NativeRange.prototype.setEnd = function() {throw Error('author setter');};
    const snapshot = convert(protectedSource);
    assert(snapshot.startContainer === text && snapshot.startOffset === 0 && snapshot.endOffset === 2, 'intrinsic native conversion');
    setStart.call(protectedLive, text, 2); setEnd.call(protectedLive, text, 1);
    assert(protectedLive.collapsed && protectedLive.startOffset === 1, 'public algorithm ignores author node properties');
  } finally {
    globalThis.Range = NativeRange; globalThis.StaticRange = NativeStaticRange;
    NativeRange.prototype.setStart = setStart; NativeRange.prototype.setEnd = setEnd;
    for (const key of ['nodeType', 'length', 'childNodes', 'parentNode', 'previousSibling']) delete text[key];
    host.remove();
  }
  return true;
})()
