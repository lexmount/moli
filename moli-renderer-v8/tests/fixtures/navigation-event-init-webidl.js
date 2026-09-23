async function navigationEventInitWebIdl(base, targetKind) {
  let checks = 0;
  const failures = [];
  const check = (ok, label) => { checks++; if (!ok) failures.push(label); };
  const frame = document.createElement('iframe');
  frame.src = base + '/common/blank.html';
  await new Promise(resolve => { frame.onload = resolve; document.body.appendChild(frame); });
  const win = targetKind === 'child' ? frame.contentWindow : window;
  const other = targetKind === 'child' ? window : frame.contentWindow;
  const destination = owner => {
    let result;
    owner.navigation.addEventListener('navigate', event => { result = event.destination; }, {once: true});
    owner.history.pushState(null, '', '#capture');
    return result;
  };
  const dest = destination(win);
  const foreignDest = destination(other);
  const signal = new win.AbortController().signal;
  const formData = new win.FormData();
  const sourceElement = win.document.createElement('a');
  const from = win.navigation.currentEntry;
  const baseNames = ['bubbles', 'cancelable', 'composed'];
  const navValues = {
    bubbles: false, cancelable: false, composed: false,
    canIntercept: false, destination: dest, downloadRequest: null,
    formData: null, hasUAVisualTransition: false, hashChange: false,
    info: undefined, navigationType: 'push', signal,
    sourceElement: null, userInitiated: false
  };
  const changeValues = {bubbles: false, cancelable: false, composed: false, from, navigationType: null};
  const cases = [
    ['NavigateEvent', win.NavigateEvent, navValues, {destination: dest, signal}],
    ['NavigationCurrentEntryChangeEvent', win.NavigationCurrentEntryChangeEvent, changeValues, {from}]
  ];
  const typeError = (fn, label) => {
    let error;
    try { fn(); } catch (value) { error = value; }
    check(error instanceof win.TypeError, label);
  };
  for (const [label, C, values, required] of cases) {
    check(C.length === 2, label + ' constructor length');
    let conversions = 0;
    typeError(() => new C({toString() { conversions++; return 'x'; }}), label + ' required dictionary arity');
    check(conversions === 0, label + ' arity checked before type conversion');
    for (const value of [undefined, null, 1, 'x', true, Symbol('init')])
      typeError(() => new C('x', value), label + ' rejects missing or invalid dictionary ' + String(value));
    const names = [...baseNames, ...Object.keys(values).filter(name => !baseNames.includes(name)).sort()];
    const read = (throwAt) => {
      const log = [];
      const sentinel = {};
      const init = Object.create(null);
      for (const name of names) Object.defineProperty(init, name, {get() {
        log.push(name);
        if (name === throwAt) throw sentinel;
        return values[name];
      }});
      let result, error;
      try { result = new C({toString() { log.push('type'); return 'x'; }}, init); }
      catch (value) { error = value; }
      return {result, error, sentinel, log};
    };
    const complete = read();
    check(complete.result instanceof C && complete.result.type === 'x', label + ' dictionary accepted');
    check(JSON.stringify(complete.log) === JSON.stringify(['type', ...names]), label + ' inherited and lexical member order');
    for (const name of names) {
      const failure = read(name);
      check(failure.error === failure.sentinel, label + ' preserves getter exception ' + name);
      check(JSON.stringify(failure.log) === JSON.stringify(['type', ...names.slice(0, names.indexOf(name) + 1)]), label + ' stops conversion after ' + name);
    }
    for (const token of ['', 'Push', 'auto', 'invalid', 7, Symbol('type')])
      typeError(() => new C('x', {...required, navigationType: token}), label + ' rejects enum ' + String(token));
    for (const token of ['push', 'replace', 'reload', 'traverse']) {
      conversions = 0;
      const event = new C('x', {...required, navigationType: {toString() { conversions++; return token; }}});
      check(event.navigationType === token && conversions === 1, label + ' converts enum ' + token);
    }
    const inherited = new C('x', Object.create({...required, navigationType: 'reload'}));
    check(inherited.navigationType === 'reload', label + ' inherits dictionary values');
    let laterReads = 0;
    const bad = {...required};
    const first = label === 'NavigateEvent' ? 'destination' : 'from';
    bad[first] = {};
    Object.defineProperty(bad, 'navigationType', {get() { laterReads++; return 'push'; }});
    typeError(() => new C('x', bad), label + ' rejects invalid interface before later members');
    check(laterReads === 0, label + ' does not read members after interface failure');
  }
  const N = win.NavigateEvent;
  const E = win.NavigationCurrentEntryChangeEvent;
  typeError(() => new N('x', {destination: dest, signal, navigationType: null}), 'NavigateEvent null enum is invalid');
  check(new E('x', {from, navigationType: null}).navigationType === null, 'nullable enum preserves null');
  check(new N('x', {destination: dest, signal, navigationType: undefined}).navigationType === 'push', 'undefined enum gets push default');
  check(new E('x', {from, navigationType: undefined}).navigationType === null, 'undefined nullable enum gets null default');
  for (const value of [null, undefined]) {
    const event = new N('x', {destination: dest, signal, formData: value, sourceElement: value, downloadRequest: value});
    check(event.formData === null && event.sourceElement === null && event.downloadRequest === null, 'nullable member defaults ' + String(value));
  }
  const input = {toString() { return 'download\ud800'; }};
  const downloaded = new N('x', {destination: dest, signal, downloadRequest: input});
  check(downloaded.downloadRequest === 'download\ud800', 'downloadRequest DOMString conversion preserves UTF-16');
  for (const value of [false, 23, 1n])
    check(new N('x', {destination: dest, signal, downloadRequest: value}).downloadRequest === String(value), 'downloadRequest converts ' + String(value));
  typeError(() => new N('x', {destination: dest, signal, downloadRequest: Symbol('download')}), 'downloadRequest rejects Symbol');
  const marker = {};
  let error;
  try { new N('x', {destination: dest, signal, downloadRequest: {toString() { throw marker; }}}); } catch (value) { error = value; }
  check(error === marker, 'downloadRequest conversion preserves thrown exception');
  const boolean = {valueOf() { throw marker; }, toString() { throw marker; }};
  const info = new Proxy({}, {get() { throw marker; }, getPrototypeOf() { throw marker; }});
  const truthy = new N('x', {destination: dest, signal, info, bubbles: boolean, cancelable: boolean, composed: boolean, canIntercept: boolean, hashChange: boolean, hasUAVisualTransition: boolean, userInitiated: boolean});
  check(truthy.bubbles && truthy.cancelable && truthy.composed && truthy.canIntercept && truthy.hashChange && truthy.hasUAVisualTransition && truthy.userInitiated, 'boolean members use ToBoolean');
  check(truthy.info === info, 'any info preserves Proxy identity');
  const foreign = new N('x', {destination: foreignDest, signal: new other.AbortController().signal, formData: new other.FormData(), sourceElement: other.document.createElement('a')});
  check(foreign.destination === foreignDest && foreign.formData instanceof other.FormData && foreign.sourceElement.ownerDocument === other.document, 'cross-realm genuine interface values');
  check(new E('x', {from: other.navigation.currentEntry}).from === other.navigation.currentEntry, 'cross-realm history entry');
  const nativeElement = win.document.implementation.createHTMLDocument('').createElement('select');
  check(new N('x', {destination: dest, signal, sourceElement: nativeElement}).sourceElement === nativeElement, 'registered native element Proxy accepted');
  let traps = 0;
  for (const [name, real, C, required, nullable] of [
    ['destination', dest, N, {destination: dest, signal}, false],
    ['signal', signal, N, {destination: dest, signal}, false],
    ['formData', formData, N, {destination: dest, signal}, true],
    ['sourceElement', nativeElement, N, {destination: dest, signal}, true],
    ['from', from, E, {from}, false]
  ]) {
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    const proxy = new Proxy(real, {get() { traps++; throw marker; }, getPrototypeOf() { traps++; throw marker; }});
    const invalid = [{}, Object.create(real), Object.create(Object.getPrototypeOf(real)), proxy, revoked.proxy, 1, 'x'];
    if (!nullable) invalid.push(null, undefined);
    for (const [index, value] of invalid.entries()) typeError(() => new C('x', {...required, [name]: value}), name + ' rejects forged value ' + index);
  }
  check(traps === 0, 'interface validation avoids author Proxy traps');
  const saved = Object.getPrototypeOf(dest);
  Object.setPrototypeOf(dest, null);
  try { check(new N('x', {destination: dest, signal}).destination === dest, 'native identity survives prototype change'); }
  finally { Object.setPrototypeOf(dest, saved); }
  frame.remove();
  return {checks, failures};
}
