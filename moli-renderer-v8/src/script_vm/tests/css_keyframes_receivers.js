((bindings, values, label) => {
  const checks = [];
  const check = (name, fn) => {
    try {
      const result = fn();
      checks.push({name: label + ': ' + name, passed: result === true, detail: result === true ? null : String(result)});
    } catch (error) {
      checks.push({name: label + ': ' + name, passed: false, detail: String(error)});
    }
  };
  const sheet = new values.CSSStyleSheet();
  sheet.replaceSync('div {color:red} @keyframes original {from {left:0px} to {left:100px}}');
  const frames = sheet.cssRules[1];
  const frame = frames.cssRules[0];
  const styleRule = sheet.cssRules[0];
  const members = [
    ['CSSKeyframesRule', 'name', 'get', frames],
    ['CSSKeyframesRule', 'name', 'set', frames],
    ['CSSKeyframesRule', 'cssRules', 'get', frames],
    ['CSSKeyframesRule', 'length', 'get', frames],
    ['CSSKeyframesRule', 'appendRule', 'value', frames],
    ['CSSKeyframesRule', 'deleteRule', 'value', frames],
    ['CSSKeyframesRule', 'findRule', 'value', frames],
    ['CSSKeyframeRule', 'keyText', 'get', frame],
    ['CSSKeyframeRule', 'keyText', 'set', frame],
    ['CSSKeyframeRule', 'style', 'get', frame],
    ['CSSKeyframeRule', 'style', 'set', frame],
  ];
  for (const [interfaceName, name, kind, real] of members) {
    const prototype = bindings[interfaceName].prototype;
    const descriptor = bindings.Object.getOwnPropertyDescriptor(prototype, name);
    const fn = descriptor?.[kind];
    const prefix = interfaceName + '.' + name + ' ' + kind;
    const converts = kind === 'set' || kind === 'value';
    check(prefix + ' descriptor', () => descriptor.enumerable && descriptor.configurable && typeof fn === 'function' && fn.length === (converts ? 1 : 0));
    let traps = 0;
    const revoked = values.Proxy.revocable(real, {}); revoked.revoke();
    const invalid = [undefined, null, false, 0, '', 1n, Symbol('receiver'), {}, prototype,
      values.Object.create(prototype), values.Object.create(real), styleRule,
      interfaceName === 'CSSKeyframeRule' ? frames : frame,
      new values.Proxy(real, {get() {traps++; throw Error('get trap');}, getPrototypeOf() {traps++; throw Error('prototype trap');}}), revoked.proxy];
    for (const [index, receiver] of invalid.entries()) {
      check(prefix + ' rejects receiver ' + index + ' before conversion', () => {
        let conversions = 0;
        const argument = {[Symbol.toPrimitive]() {conversions++; throw Error('converted invalid receiver');}};
        let error;
        try {fn.call(receiver, argument);} catch (caught) {error = caught;}
        return error instanceof bindings.TypeError && conversions === 0;
      });
    }
    check(prefix + ' no author proxy traps', () => traps === 0);
    if (converts) {
      check(prefix + ' preserves conversion exception identity', () => {
        const sentinel = new values.Error('conversion');
        let calls = 0, error;
        try {fn.call(real, {[Symbol.toPrimitive]() {calls++; throw sentinel;}});} catch (caught) {error = caught;}
        return error === sentinel && calls === 1;
      });
    }
    if (kind === 'value') {
      check(prefix + ' missing required argument', () => {
        try {fn.call(real);} catch (error) {return error instanceof bindings.TypeError;}
        return false;
      });
    }
  }
  const framesPrototype = bindings.CSSKeyframesRule.prototype;
  const framePrototype = bindings.CSSKeyframeRule.prototype;
  const get = (prototype, name, receiver) => bindings.Object.getOwnPropertyDescriptor(prototype, name).get.call(receiver);
  const set = (prototype, name, receiver, value) => bindings.Object.getOwnPropertyDescriptor(prototype, name).set.call(receiver, value);
  check('native cross realm getter', () => get(framesPrototype, 'name', frames) === 'original' && get(framePrototype, 'keyText', frame) === '0%');
  check('cssRules SameObject', () => get(framesPrototype, 'cssRules', frames) === frames.cssRules);
  check('style SameObject', () => get(framePrototype, 'style', frame) === frame.style);
  check('name setter reaches native sheet', () => {set(framesPrototype, 'name', frames, 'renamed'); return frames.name === 'renamed' && sheet.cssRules[1] === frames;});
  check('appendRule and numeric identity', () => {
    framesPrototype.appendRule.call(frames, '75% {left:75px}');
    return frames.length === 3 && frames[2] === frames.cssRules[2] && framesPrototype.findRule.call(frames, '75%') === frames[2];
  });
  check('keyText setter reaches native selector', () => {set(framePrototype, 'keyText', frame, '25%'); return frame.keyText === '25%' && framesPrototype.findRule.call(frames, '25%') === frame;});
  check('style PutForwards preserves object and native declarations', () => {
    const style = frame.style; set(framePrototype, 'style', frame, 'left:12px');
    return frame.style === style && style.getPropertyValue('left') === '12px' && frame.cssText.includes('12px');
  });
  check('deleteRule reaches native sheet', () => {framesPrototype.deleteRule.call(frames, '75%'); return frames.length === 2 && framesPrototype.findRule.call(frames, '75%') === null;});
  for (const [interfaceName, real, name, expected] of [['CSSKeyframesRule', frames, 'name', 'renamed'], ['CSSKeyframeRule', frame, 'keyText', '25%']]) {
    const original = values.Object.getPrototypeOf(real);
    check(interfaceName + ' identity survives prototype changes', () => {
      try {values.Object.setPrototypeOf(real, null); return get(bindings[interfaceName].prototype, name, real) === expected;}
      finally {values.Object.setPrototypeOf(real, original);}
    });
  }
  const fresh = new values.CSSStyleSheet();
  fresh.replaceSync('@keyframes lazy {from {left:1px} to {left:2px}}');
  const lazy = fresh.cssRules[0];
  const list = get(framesPrototype, 'cssRules', lazy);
  check('lazy cssRules belongs to receiver realm', () => list instanceof values.CSSRuleList && (bindings === values || !(list instanceof bindings.CSSRuleList)));
  const lazyFrame = framesPrototype.findRule.call(lazy, 'from');
  check('lazy findRule belongs to receiver realm', () => lazyFrame instanceof values.CSSKeyframeRule && lazyFrame === list[0]);
  const declaration = get(framePrototype, 'style', lazyFrame);
  check('lazy style belongs to receiver realm', () => declaration instanceof values.CSSStyleDeclaration && (bindings === values || !(declaration instanceof bindings.CSSStyleDeclaration)));
  check('lazy cross realm objects stay cached', () => get(framesPrototype, 'cssRules', lazy) === list && get(framePrototype, 'style', lazyFrame) === declaration);
  fresh.deleteRule(0);
  check('detached keyframes retain native receiver identity', () => get(framesPrototype, 'name', lazy) === 'lazy' && get(framesPrototype, 'cssRules', lazy) === list);
  check('detached keyframe remains mutable', () => {set(framePrototype, 'keyText', lazyFrame, '20%'); set(framePrototype, 'style', lazyFrame, 'left:9px'); return lazyFrame.keyText === '20%' && declaration.getPropertyValue('left') === '9px';});
  return checks;
})
