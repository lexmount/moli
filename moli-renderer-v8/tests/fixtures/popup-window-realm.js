(() => {
  const rows = [];
  const popup = open();
  function check(name, expected, run) {
    let actual;
    try { actual = run(); } catch (error) { actual = {name: error.name, message: error.message}; }
    rows.push({name, expected, actual, pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  globalThis.openerOnlySentinel = 73;
  Object.prototype.openerPrototypeSentinel = 79;
  try {
    popup.document.open();
    popup.document.write('<script>window.realmReport = [globalThis === window, window === self, document.defaultView === window, typeof openerOnlySentinel, Object.prototype.hasOwnProperty("openerPrototypeSentinel"), Object !== opener.Object];<\/script>');
    popup.document.close();
    check('script-global-and-opener-isolation', [true,true,true,'undefined',false,true], () => popup.realmReport);
    check('Window-is-realm-global', true, () => popup.Function('return globalThis')() === popup);
    check('eval-uses-popup-global', true, () => popup.eval('globalThis') === popup);
    check('document-creation-realm', true, () => Object.getPrototypeOf(popup.document) === popup.HTMLDocument.prototype);
    check('element-creation-realm', true, () => Object.getPrototypeOf(popup.document.createElement('div')) === popup.HTMLDivElement.prototype);
    delete popup.onwheel;
    check('deleted-native-property-does-not-fall-back-to-opener', 'undefined', () => popup.Function('return typeof onwheel')());
    check('global-assignment-stays-in-popup', [83,'undefined'], () => {
      popup.Function('globalThis.popupOnlySentinel = 83')();
      return [popup.popupOnlySentinel, typeof popupOnlySentinel];
    });
    check('popup-intrinsics-have-own-realm', [true,true,true], () => ['Array','TypeError','Event'].map(name => typeof popup[name] === 'function' && popup[name] !== window[name]));
  } finally {
    popup?.close(); delete globalThis.openerOnlySentinel; delete Object.prototype.openerPrototypeSentinel;
  }
  const failures=rows.filter(row=>!row.pass);
  globalThis.__nodeReplacementResults = {total:rows.length,passed:rows.length-failures.length,failures,rows};
  return failures.length === 0;
})()
