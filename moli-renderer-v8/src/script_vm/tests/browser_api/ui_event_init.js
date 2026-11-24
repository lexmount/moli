(() => {
  const rows = [];
  const check = (name, run) => {
    try { rows.push({name, passed: run() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const child = document.getElementById('child').contentWindow;
  const texts = ['', '\ud800', '\udc00', '\ud83d\ude00', 'a\ud800b\udc00\u0000'];
  check('initUIEvent converts detail and updates its view', () => {
    const event = new UIEvent('before');
    event.initUIEvent('after', true, true, child, 4294967297);
    return event.type === 'after' && event.bubbles && event.cancelable &&
      event.view === child && event.detail === 1;
  });
  for (const text of texts) {
    check('initCompositionEvent preserves UTF-16 code units', () => {
      const event = new CompositionEvent('before');
      event.initCompositionEvent('after', true, false, null, text);
      if (event.type !== 'after' || event.data !== text) return false;
      UIEvent.prototype.initUIEvent.call(event, 'changed', false, true, child, 3.9);
      return event.type === 'changed' && event.view === child && event.detail === 3 &&
        event.data === text;
    });
  }
  check('failed legacy argument conversion leaves native state intact', () => {
    const sentinel = {};
    const event = new CompositionEvent('before');
    event.initCompositionEvent('retained', false, false, null, 'before');
    try {
      event.initCompositionEvent('after', true, true, null, {
        toString() { throw sentinel; }
      });
    } catch (error) {
      return error === sentinel && event.type === 'retained' && event.data === 'before' &&
        !event.bubbles && !event.cancelable;
    }
    return false;
  });
  check('initCompositionEvent validates the receiver before conversion', () => {
    let converted = false;
    try {
      CompositionEvent.prototype.initCompositionEvent.call(
        new UIEvent('x'), {toString() { converted = true; return 'after'; }}
      );
    } catch (error) {
      return error instanceof TypeError && !converted;
    }
    return false;
  });
  globalThis.__uiEventResults = {rows};
  return rows.every(row => row.passed);
})()
