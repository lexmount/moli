((bindings, values, label) => {
  const checks = [];
  const check = (name, fn) => {
    try { const result = fn(); checks.push({name: label + ': ' + name, passed: result === true, detail: result === true ? null : String(result)}); }
    catch (error) { checks.push({name: label + ': ' + name, passed: false, detail: String(error)}); }
  };
  const throwsTypeError = fn => { try { fn(); } catch (error) { return error instanceof bindings.TypeError; } return false; };
  const doc = values.document;
  const style = doc.createElement('style');
  style.textContent = '@keyframes cssInterface { to { left: 100px } }';
  doc.head.appendChild(style);
  const element = doc.createElement('div'); doc.body.appendChild(element);
  element.style.animation = 'cssInterface 100s';
  const animation = element.getAnimations()[0];
  try {
    check('native CSSAnimation instance and inheritance', () => animation instanceof values.CSSAnimation && animation instanceof values.Animation && !(animation instanceof values.CSSTransition));
    check('animation name', () => animation.animationName === 'cssInterface');
    check('getAnimations identity', () => element.getAnimations()[0] === animation);
    check('callee Array, target animation realm', () => {
      const result = bindings.Element.prototype.getAnimations.call(element);
      return result instanceof bindings.Array && result[0] === animation && result[0] instanceof values.CSSAnimation;
    });
    for (const [interfaceName, member] of [['CSSAnimation', 'animationName'], ['CSSTransition', 'transitionProperty'], ['AnimationEvent', 'animation'], ['TransitionEvent', 'animation']]) {
      const prototype = bindings[interfaceName].prototype;
      const descriptor = bindings.Object.getOwnPropertyDescriptor(prototype, member);
      check(interfaceName + '.' + member + ' descriptor', () => descriptor?.enumerable === true && descriptor.configurable === true && typeof descriptor.get === 'function' && descriptor.get.length === 0 && descriptor.set === undefined);
      let traps = 0;
      const revoked = values.Proxy.revocable({}, {}); revoked.revoke();
      for (const [index, receiver] of [undefined, null, 0, {}, values.Object.create(prototype), values.Object.create(animation || {}), new values.Proxy(animation || {}, {get() {traps++; throw Error('trap');}, getPrototypeOf() {traps++; throw Error('trap');}}), revoked.proxy].entries()) {
        check(interfaceName + '.' + member + ' rejects receiver ' + index, () => throwsTypeError(() => descriptor.get.call(receiver)));
      }
      check(interfaceName + '.' + member + ' no proxy traps', () => traps === 0);
    }
    for (const [name, stringMember, cssName, supplied] of [['AnimationEvent', 'animationName', 'CSSAnimation', animation], ['TransitionEvent', 'propertyName', 'CSSTransition', undefined]]) {
      const C = bindings[name];
      for (const [index, init] of [undefined, null, {}, {animation: undefined}, {animation: null}].entries()) {
        check(name + ' nullable default ' + index, () => new C('test', init).animation === null);
      }
      check(name + ' inherited nullable member', () => new C('test', values.Object.create({animation: null})).animation === null);
      check(name + ' preserves payload UTF-16', () => new C('test', {[stringMember]: '\ud800X\udfff', pseudoElement: '\udfff'} )[stringMember] === '\ud800X\udfff');
      const order = [];
      const init = new values.Proxy({}, {get(target, key) {order.push(key); return undefined;}});
      check(name + ' dictionary order', () => {
        new C('test', init);
        const expected = name === 'AnimationEvent' ? ['bubbles','cancelable','composed','animation','animationName','elapsedTime','pseudoElement'] : ['bubbles','cancelable','composed','animation','elapsedTime','propertyName','pseudoElement'];
        return JSON.stringify(order) === JSON.stringify(expected);
      });
      const sentinel = new values.Error('animation getter');
      check(name + ' getter exception identity', () => {try { new C('test', {get animation() {throw sentinel;}}); } catch(error) {return error === sentinel;} return false;});
      const revoked = values.Proxy.revocable(animation || {}, {}); revoked.revoke();
      const invalid = [false, 0, '', Symbol('animation'), 1n, {}, values.Object.create(values[cssName].prototype), new values.Animation(), new values.Proxy(animation || {}, {get() {throw Error('trap');}, getPrototypeOf() {throw Error('trap');}}), revoked.proxy];
      if (name === 'TransitionEvent') invalid.push(animation);
      for (const [index, value] of invalid.entries()) {
        let laterReads = 0;
        check(name + ' rejects animation ' + index + ' before remaining members', () => {
          const options = {animation: value, get [stringMember]() {laterReads++; return 'changed';}, get elapsedTime() {laterReads++; return 1;}};
          return throwsTypeError(() => new C('test', options)) && laterReads === 0;
        });
      }
      if (name === 'AnimationEvent') {
        check(name + ' stores native animation identity', () => new C('test', {animation: supplied}).animation === animation && animation !== undefined);
        check(name + ' accepts inherited native animation', () => new C('test', values.Object.create({animation})).animation === animation && animation !== undefined);
        const event = new C('test', {animation});
        check(name + ' association survives dispatch', () => {element.dispatchEvent(event); return event.animation === animation && animation !== undefined;});
        check(name + ' read only association', () => !bindings.Reflect.set(event, 'animation', null) && event.animation === animation);
      }
    }
  } finally { element.remove(); style.remove(); }
  return checks;
})
