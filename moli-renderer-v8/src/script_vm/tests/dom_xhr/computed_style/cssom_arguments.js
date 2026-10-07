(() => {
  const checks = [];
  const record = (name, verify) => {
    try {
      checks.push({name, passed: verify() === true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error)});
    }
  };
  const capture = callback => {
    try { return {threw: false, value: callback()}; }
    catch (error) { return {threw: true, error}; }
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  try {
    for (const [index, owner] of [window, frame.contentWindow, popup].entries()) {
      const doc = owner.document;
      const element = doc.createElement('div');
      element.id = 'computed-style-arguments-target';
      element.style.color = 'rgb(1, 2, 3)';
      const sheet = doc.createElement('style');
      sheet.textContent = '#computed-style-arguments-target::before { content: "x"; color: rgb(4, 5, 6); }';
      doc.head.appendChild(sheet);
      doc.body.appendChild(element);
      const call = (...args) => Reflect.apply(owner.getComputedStyle, owner, args);
      const label = name => `owner ${index}/${name}`;
      const shadowHost = doc.createElement('div');
      const revoked = Proxy.revocable(element, {});
      revoked.revoke();
      let traps = 0;
      const authorProxy = new Proxy(element, {get() { traps++; return undefined; }});
      const invalid = [
        ['undefined', undefined], ['null', null], ['boolean', false], ['number', 0],
        ['NaN', NaN], ['bigint', 1n], ['string', 'div'], ['symbol', Symbol('element')],
        ['object', {}], ['forged prototype', Object.create(owner.Element.prototype)],
        ['inherited native', Object.create(element)], ['author Proxy', authorProxy],
        ['revoked Proxy', revoked.proxy], ['Document', doc], ['Text', doc.createTextNode('x')],
        ['Comment', doc.createComment('x')], ['DocumentFragment', doc.createDocumentFragment()],
        ['ShadowRoot', shadowHost.attachShadow({mode: 'open'})], ['Window', owner],
        ['CSSStyleProperties', element.style], ['Attr', doc.createAttribute('x')],
      ];
      for (const [name, value] of invalid) {
        let conversions = 0;
        const sentinel = {};
        const pseudo = {get [Symbol.toPrimitive]() { conversions++; throw sentinel; }};
        const failure = capture(() => call(value, pseudo));
        record(label(name + '/callee TypeError'), () => failure.threw && failure.error instanceof owner.TypeError);
        record(label(name + '/pseudo not converted'), () => conversions === 0);
        record(label(name + '/no author traps'), () => traps === 0);
      }
      record(label('required element'), () => {
        const failure = capture(() => call());
        return failure.threw && failure.error instanceof owner.TypeError;
      });
      for (const [name, pseudo] of [['undefined', undefined], ['null', null], ['empty', ''],
          ['no colon', 'before'], ['boolean', true], ['number', 7], ['bigint', 2n]]) {
        record(label('nullable pseudo/' + name), () => call(element, pseudo).color === 'rgb(1, 2, 3)');
      }
      record(label('missing pseudo'), () => call(element).color === 'rgb(1, 2, 3)');
      const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.style.color = 'rgb(1, 2, 3)';
      doc.body.appendChild(svg);
      record(label('SVG Element'), () => call(svg).color === 'rgb(1, 2, 3)');
      const detached = doc.createElement('div');
      record(label('detached Element'), () => call(detached).length === 0);
      for (const [name, sentinel] of [['object', {}], ['undefined', undefined], ['null', null],
          ['number', 19], ['symbol', Symbol('thrown')]]) {
        let conversions = 0;
        const pseudo = {toString() { conversions++; throw sentinel; }};
        const failure = capture(() => call(element, pseudo));
        record(label('thrown pseudo/' + name), () => failure.threw && failure.error === sentinel);
        record(label('thrown pseudo/' + name + '/once'), () => conversions === 1);
        record(label('thrown pseudo/' + name + '/next call usable'), () => call(element).color === 'rgb(1, 2, 3)');
      }
      for (const [name, pseudo] of [['symbol', Symbol('pseudo')], ['symbol result', {[Symbol.toPrimitive]() { return Symbol(); }}],
          ['object result', {[Symbol.toPrimitive]() { return {}; }}]]) {
        const failure = capture(() => call(element, pseudo));
        record(label('pseudo TypeError/' + name), () => failure.threw && failure.error instanceof owner.TypeError);
      }
      for (const phase of ['primitive getter', 'primitive call', 'toString getter', 'valueOf call']) {
        let steps = [];
        const sentinel = {};
        const pseudo = phase === 'primitive getter' ? {get [Symbol.toPrimitive]() { steps.push('get'); throw sentinel; }} :
          phase === 'primitive call' ? {[Symbol.toPrimitive](hint) { steps.push(hint); throw sentinel; }} :
          phase === 'toString getter' ? {get toString() { steps.push('get'); throw sentinel; }} :
          {toString() { steps.push('toString'); return {}; }, valueOf() { steps.push('valueOf'); throw sentinel; }};
        const failure = capture(() => call(element, pseudo));
        record(label(phase + '/exception'), () => failure.threw && failure.error === sentinel);
        record(label(phase + '/order'), () => steps.join(',') ===
          (phase === 'primitive call' ? 'string' : phase === 'valueOf call' ? 'toString,valueOf' : 'get'));
      }
      let steps = [];
      const pseudo = {
        get [Symbol.toPrimitive]() {
          steps.push('get');
          return hint => { steps.push(hint); return '::before'; };
        },
        toString() { steps.push('unexpected'); return ''; },
      };
      const before = call(element, pseudo);
      record(label('pseudo string conversion order'), () => steps.join(',') === 'get,string');
      record(label('converted pseudo target'), () => before.color === 'rgb(4, 5, 6)');
      record(label('originating style after pseudo'), () => call(element).color === 'rgb(1, 2, 3)');
      svg.remove(); element.remove(); sheet.remove();
    }
    const detachedDocument = document.implementation.createHTMLDocument('');
    const nativeProxy = detachedDocument.createElement('select');
    record('native registered Proxy accepted', () => window.getComputedStyle(nativeProxy).length === 0);
    let conversions = 0;
    const pseudo = {toString() { conversions++; return ''; }};
    const failure = capture(() => window.getComputedStyle(new Proxy(nativeProxy, {}), pseudo));
    record('author Proxy around native Proxy rejected', () => failure.threw && failure.error instanceof TypeError);
    record('author Proxy around native Proxy conversion order', () => conversions === 0);
    globalThis.__computedStyleArgumentsResults = {
      complete: true, total: checks.length,
      passed: checks.filter(row => row.passed).length, checks,
    };
    globalThis.__uiEventResults = globalThis.__computedStyleArgumentsResults;
    return true;
  } finally {
    frame.remove();
    if (popup && !popup.closed) popup.close();
  }
})();
