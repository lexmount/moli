(() => {
  const rows = [], H = 'http://www.w3.org/1999/xhtml';
  const realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const originals = realms.map(([name, win]) => ({name, ctor: win.RegExp, exec: win.RegExp.prototype.exec}));
  const statics = ['input', 'lastMatch', 'lastParen', 'leftContext', 'rightContext', '$1', '$2', '$3', '$4', '$5', '$6', '$7', '$8', '$9'];
  const seed = () => {
    for (const {name, ctor, exec} of originals) exec.call(new ctor('(sentinel)-('+name+')'), 'before-sentinel-'+name+'-after');
    return originals.map(({ctor}) => statics.map(key => ctor[key]));
  };
  const unchanged = before => originals.every(({ctor}, index) => statics.every((key, field) => ctor[key] === before[index][field]));
  const check = (name, body) => {
    try {rows.push({name, passed: body() === true});}
    catch (error) {rows.push({name, passed: false, error: String(error)});}
  };
  const cases = [
    ['digits match', '\\d+', '123', false],
    ['full value', '\\d+', 'x123', true],
    ['nested rejection', '(\\d+)*$', '12345678901234567890123456789123456789z', true],
    ['nested match', '(\\d+)*$', '12345678901234567890123456789123456789', false],
    ['backreference match', '([a-z])\\1', 'aa', false],
    ['backreference rejection', '([a-z])\\1', 'ab', true],
    ['lookahead match', '(?=.*[0-9]).+', 'abc1', false],
    ['lookahead rejection', '(?=.*[0-9]).+', 'abc', true],
    ['unicode intersection', '[\\p{ASCII}&&\\p{Letter}]+', 'abc', false],
    ['unicode intersection rejection', '[\\p{ASCII}&&\\p{Letter}]+', '\u00e9', true],
    ['unicode string set', '[\\q{ab|cd}]', 'ab', false],
    ['unicode string set rejection', '[\\q{ab|cd}]', 'a', true],
    ['invalid class', '[', 'x', false],
    ['invalid standalone source balanced by wrapper', ')abc(', 'x', false],
    ['empty pattern', '', 'x', true],
    ['empty value', '\\d+', '', false],
    ['supplementary character', '\\p{Emoji}', '\ud83d\ude00', false],
    ['escaped punctuation', 'a\\.b', 'a.b', false],
    ['invalid unicode-set punctuation', '[()]', 'x', false],
    ['absent pattern', null, 'anything', false],
  ];
  const methods = ['validity', 'checkValidity', 'reportValidity', 'validationMessage', 'formValidity'];
  for (const [ownerName, owner] of realms) for (const flavor of ['live', 'html', 'parsed', 'xml']) {
    const doc = flavor === 'live' ? owner.document : flavor === 'html' ? owner.document.implementation.createHTMLDocument('patterns') :
      flavor === 'parsed' ? new owner.DOMParser().parseFromString('<!doctype html><body>', 'text/html') :
      owner.document.implementation.createDocument(H, 'html');
    const holder = (doc.body || doc.documentElement).appendChild(doc.createElementNS(H, 'div'));
    const form = holder.appendChild(doc.createElementNS(H, 'form'));
    const input = form.appendChild(doc.createElementNS(H, 'input'));
    let invalidEvents = 0;
    input.addEventListener('invalid', event => {invalidEvents++;event.preventDefault();});
    try {
      for (const [calleeName, callee] of realms) {
        for (const [caseName, pattern, value, expected] of cases) {
          input.type = 'text';input.multiple = false;
          if (pattern === null) input.removeAttribute('pattern'); else input.pattern = pattern;
          input.value = value;
          for (const method of methods) {
            check(`${ownerName}/${flavor}/${calleeName}/${caseName}/${method}`, () => {
              invalidEvents = 0;
              const before = seed();
              let result;
              if (method === 'validity') {
                const state = Object.getOwnPropertyDescriptor(callee.HTMLInputElement.prototype, 'validity').get.call(input);
                result = state.patternMismatch === expected && state.valid !== expected;
              } else if (method === 'checkValidity' || method === 'reportValidity') result = callee.HTMLInputElement.prototype[method].call(input) !== expected;
              else if (method === 'validationMessage') result = (Object.getOwnPropertyDescriptor(callee.HTMLInputElement.prototype, 'validationMessage').get.call(input) !== '') === expected;
              else result = callee.HTMLFormElement.prototype.checkValidity.call(form) !== expected;
              const fires = expected && ['checkValidity', 'reportValidity', 'formValidity'].includes(method);
              return result && invalidEvents === Number(fires) && unchanged(before);
            });
          }
        }
        const typed = [
          ...['text', 'search', 'tel', 'password'].flatMap(type => [[type, '(a)\\1', 'aa', false], [type, '(a)\\1', 'ab', true]]),
          ['url', 'https://example\\.test/', 'https://example.test/', false],
          ['url', 'https://example\\.test/', 'https://other.test/', true],
          ['email', '[a-z]+@example\\.test', 'a@example.test', false],
          ['email', '[a-z]+@example\\.test', 'a@other.test', true],
          ...[['number', '123'], ['date', '2026-10-22'], ['color', '#ff0000'], ['range', '50'], ['checkbox', 'x'], ['hidden', 'x']].map(([type, value]) => [type, 'never-matches', value, false]),
        ];
        for (const [type, pattern, value, expected] of typed) {
          input.type = type;input.multiple = false;input.pattern = pattern;input.value = value;
          check(`${ownerName}/${flavor}/${calleeName}/type/${type}/${value}`, () => {
            const before = seed();
            const state = Object.getOwnPropertyDescriptor(callee.HTMLInputElement.prototype, 'validity').get.call(input);
            return state.patternMismatch === expected && state.valid !== expected &&
              callee.HTMLInputElement.prototype.checkValidity.call(input) !== expected && unchanged(before);
          });
        }
        input.type = 'email';input.multiple = true;input.pattern = '[a-z]+@example\\.test';
        for (const [value, expected] of [['a@example.test,b@example.test', false], ['a@example.test, b@other.test', true], ['', false]]) {
          input.value = value;
          check(`${ownerName}/${flavor}/${calleeName}/multiple/${value}`, () => {
            const before = seed();
            const state = Object.getOwnPropertyDescriptor(callee.HTMLInputElement.prototype, 'validity').get.call(input);
            return state.patternMismatch === expected && state.valid !== expected &&
              callee.HTMLInputElement.prototype.checkValidity.call(input) !== expected && unchanged(before);
          });
        }
      }
    } finally {
      // Orphan nodes can remain in a native arena; leave a cheap pattern for
      // later selector scans, instead of carrying an adversarial expression.
      input.type = 'text';input.multiple = false;input.pattern = '.*';input.value = 'cleanup';holder.remove();
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
