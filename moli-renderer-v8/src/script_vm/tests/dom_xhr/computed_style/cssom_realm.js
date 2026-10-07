(() => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  const owners = [window, frame.contentWindow, popup];
  const lifecycleObservations = [];
  const resources = [];
  try {
    const methods = owners.map(owner => owner.getComputedStyle);
    for (const [targetIndex, owner] of owners.entries()) {
      const doc = owner.document;
      const style = doc.createElement('style');
      const color = `rgb(${10 + targetIndex}, ${20 + targetIndex}, ${30 + targetIndex})`;
      const before = `rgb(${40 + targetIndex}, ${50 + targetIndex}, ${60 + targetIndex})`;
      style.textContent = `.realm-target { color: ${color}; } .realm-target::before { content: "x"; color: ${before}; }`;
      doc.head.appendChild(style);
      resources.push(style);
      for (const [kind, target] of [
        ['HTML', doc.createElement('div')],
        ['SVG', doc.createElementNS('http://www.w3.org/2000/svg', 'svg')],
        ['detached', doc.createElement('div')],
      ]) {
        target.setAttribute('class', 'realm-target');
        if (kind !== 'detached') doc.body.appendChild(target);
        resources.push(target);
        for (const [functionIndex, method] of methods.entries()) {
          for (const [receiverIndex, receiver] of owners.entries()) {
            for (const [pseudoName, pseudo, expected] of [
              ['originating', undefined, color], ['before', '::before', before],
              ['invalid', '::unknown-pseudo', ''],
            ]) {
              const label = `${targetIndex}/${kind}/${functionIndex}/${receiverIndex}/${pseudoName}`;
              const get = () => Reflect.apply(method, receiver, [target, pseudo]);
              let first, second;
              check(label + '/object', () => { first = get(); second = get(); return first !== null && second !== null; });
              const prototype = (receiver.CSSStyleProperties ?? receiver.CSSStyleDeclaration).prototype;
              const value = kind === 'detached' ? '' : expected;
              check(label + '/receiver prototype', () => Object.getPrototypeOf(first) === prototype);
              check(label + '/receiver brand', () => first instanceof receiver.CSSStyleDeclaration);
              check(label + '/target document value', () => first.color === value);
              check(label + '/borrowed property method', () =>
                Reflect.apply(receiver.CSSStyleDeclaration.prototype.getPropertyValue, first, ['color']) === value);
              check(label + '/new object', () => first !== second);
              check(label + '/independent expando', () => {
                first.__realmMarker = label;
                return first.__realmMarker === label && !Object.hasOwn(second, '__realmMarker');
              });
              check(label + '/empty declaration', () => (first.length === 0) === (value === ''));
            }
            const label = `${targetIndex}/${kind}/${functionIndex}/${receiverIndex}/live`;
            let held;
            check(label + '/create', () => { held = Reflect.apply(method, receiver, [target]); return held !== null; });
            target.style.color = 'rgb(70, 80, 90)';
            check(label + '/mutation', () => held.color === (kind === 'detached' ? '' : 'rgb(70, 80, 90)'));
            target.remove();
            check(label + '/disconnect', () => held.color === '' && held.length === 0);
            if (kind !== 'detached') doc.body.appendChild(target);
            check(label + '/reconnect', () => held.color === (kind === 'detached' ? '' : 'rgb(70, 80, 90)'));
            target.style.color = '';
          }
        }
      }
    }
    for (const [functionIndex, method] of methods.entries()) {
      const callee = owners[functionIndex];
      for (const [receiverIndex, receiver] of owners.entries()) {
        for (const [kind, value] of [['object', {}], ['null', null], ['author Proxy', new Proxy(document.body, {})]]) {
          let conversions = 0;
          check(`${functionIndex}/${receiverIndex}/${kind}/callee TypeError before pseudo conversion`, () => {
            try { Reflect.apply(method, receiver, [value, {toString() { conversions++; return ''; }}]); }
            catch (error) { return Object.getPrototypeOf(error) === callee.TypeError.prototype && conversions === 0; }
            return false;
          });
        }
        const exception = {functionIndex, receiverIndex};
        check(`${functionIndex}/${receiverIndex}/pseudo preserves exception identity`, () => {
          try { Reflect.apply(method, receiver, [document.body, {toString() { throw exception; }}]); }
          catch (error) { return error === exception; }
          return false;
        });
        let conversions = 0;
        check(`${functionIndex}/${receiverIndex}/conversion precedes native dispatch`, () => {
          const result = Reflect.apply(method, receiver, [document.body, {toString() { conversions++; return ''; }}]);
          return conversions === 1 && result !== null;
        });
      }
      let conversions = 0;
      check(`${functionIndex}/invalid receiver before conversions`, () => {
        try { Reflect.apply(method, {}, [document.body, {toString() { conversions++; return ''; }}]); }
        catch (error) { return Object.getPrototypeOf(error) === callee.TypeError.prototype && conversions === 0; }
        return false;
      });
    }
    for (const mode of ['remove-before-call', 'remove-during-conversion']) {
      const retiringFrame = document.createElement('iframe');
      document.body.appendChild(retiringFrame);
      const retired = retiringFrame.contentWindow;
      const method = retired.getComputedStyle;
      if (mode === 'remove-before-call') retiringFrame.remove();
      for (const [functionName, fn] of [['main', methods[0]], ['retired', method]]) {
        try {
          const result = Reflect.apply(fn, retired, [document.body, {toString() {
            if (mode === 'remove-during-conversion') retiringFrame.remove();
            return '';
          }}]);
          lifecycleObservations.push({mode, functionName, null: result === null,
            color: result?.color, length: result?.length});
        } catch (error) { lifecycleObservations.push({mode, functionName, error: String(error)}); }
      }
      retiringFrame.remove();
    }
    globalThis.__computedStyleRealmResults = {complete: true, total: checks.length,
      passed: checks.filter(row => row.passed).length, checks, lifecycleObservations};
    globalThis.__uiEventResults = globalThis.__computedStyleRealmResults;
    return true;
  } finally {
    for (const resource of resources) resource.remove();
    frame.remove();
    if (popup && !popup.closed) popup.close();
  }
})();
