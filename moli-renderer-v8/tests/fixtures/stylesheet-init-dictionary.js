(() => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const members = ['alternate','disabled','media'];
  const trace = [], options = {}, sentinel = {};
  for (const key of members) Object.defineProperty(options, key, {
    configurable: true, get() { trace.push(key); }
  });
  new CSSStyleSheet(options);
  assert(trace.join() === members.join(), 'CSSStyleSheetInit supported member order: ' + trace);
  for (const stop of members) {
    trace.length = 0;
    const bad = Object.create(options);
    Object.defineProperty(bad, stop, {get() { trace.push(stop); throw sentinel; }});
    let error;
    try { new CSSStyleSheet(bad); } catch (caught) { error = caught; }
    assert(error === sentinel && trace.join() === members.slice(0, members.indexOf(stop) + 1).join(), stop + ' original exception and short circuit');
  }
  for (const options of [1, true, 'options', 1n, Symbol('options')]) {
    let error;
    try { new CSSStyleSheet(options); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, 'primitive dictionary rejected');
  }
  const disabled = new CSSStyleSheet({alternate: true, disabled: true, media: 'screen'});
  assert(disabled.disabled && disabled.media.mediaText === 'screen', 'native options projection');
  trace.length = 0;
  new CSSStyleSheet({get disabled() { trace.push('disabled'); }, get media() {
    trace.push('media'); return {toString() { trace.push('string'); return 'print'; }};
  }});
  assert(trace.join() === 'disabled,media,string', 'DOMString union conversion');
  const sheet = new CSSStyleSheet({media: 'screen'});
  sheet.replaceSync('@media print { body { color: red; } }');
  const detachedRule = sheet.cssRules[0];
  const ruleMedia = detachedRule.media;
  sheet.deleteRule(0);
  const style = document.createElement('style');
  style.textContent = '@import url("data:text/css,") print;';
  document.head.appendChild(style);
  try {
    const lists = [sheet.media, ruleMedia, style.sheet.cssRules[0].media];
    const readText = Object.getOwnPropertyDescriptor(MediaList.prototype, 'mediaText').get;
    for (const list of lists) {
      const expected = readText.call(list);
      let authorReads = 0;
      Object.defineProperty(list, 'toString', {value() { authorReads++; throw sentinel; }});
      Object.defineProperty(list, 'mediaText', {get() { authorReads++; throw sentinel; }});
      Object.setPrototypeOf(list, null);
      const copied = new CSSStyleSheet({media: list});
      assert(copied.media.mediaText === expected && authorReads === 0, 'native MediaList union preserves identity and bypasses author projection');
    }
  } finally {
    style.remove();
  }
  return true;
})()
