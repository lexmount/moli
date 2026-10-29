(async () => {
  const checks = [];
  const observe = action => {try {return action();} catch (error) {return {exception: error.name, message: String(error)};}};
  const check = (name, actual, expected) => checks.push({name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected)});
  const capture = (action, sentinel, callee) => {
    try {action(); return null;} catch (error) {
      if (error === sentinel) return 'sentinel';
      if (callee && error.name === 'TypeError' && Object.getPrototypeOf(error) !== callee.TypeError.prototype) return 'wrong-realm-TypeError';
      return error.name;
    }
  };
  const htmlNS = 'http://www.w3.org/1999/xhtml';
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  let popup;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    if (globalThis.__elementOptionsIncludePopup !== false) {
      popup = window.open('', '_blank');
      if (!popup) throw new Error('popup unavailable');
      await new Promise(resolve => setTimeout(resolve, 0));
      realms.push(['popup', popup]);
    }
    for (const [ownerName, owner] of realms) {
      const docs = [
        ['live', owner.document],
        ['html', owner.document.implementation.createHTMLDocument('')],
        ['xml', owner.document.implementation.createDocument(null, 'root')],
        ['parsed-html', new owner.DOMParser().parseFromString('<!doctype html><body>', 'text/html')],
        ['parsed-xml', new owner.DOMParser().parseFromString('<root/>', 'application/xml')],
        ['new-document', new owner.Document()],
      ];
      const scoped = new owner.CustomElementRegistry();
      for (const [docName, doc] of docs) {
        for (const [calleeName, callee] of realms) {
          for (const ns of [false, true]) {
            const prefix = `${ownerName}/${docName}/${calleeName}/${ns ? 'createElementNS' : 'createElement'}`;
            const method = callee.Document.prototype[ns ? 'createElementNS' : 'createElement'];
            const create = (tag, options, namespace = htmlNS) => ns ? method.call(doc, namespace, tag, options) : method.call(doc, tag, options);
            for (const valid of [false, true]) {
              for (const stop of [-1, 0, 1, 2, 3, 4]) {
                const log = [], sentinel = {};
                const argument = (text, label, position) => ({toString() {log.push(label); if (stop === position) throw sentinel; return text;}});
                const options = {
                  get customElementRegistry() {log.push('registry'); if (stop === 2) throw sentinel; return undefined;},
                  get is() {log.push('is'); if (stop === 3) throw sentinel; return argument('x-order', 'is-string', 4);},
                };
                const tag = argument(valid ? 'div' : 'a b', 'name', 1);
                const namespace = argument(htmlNS, 'namespace', 0);
                const labels = ns ? ['namespace', 'name', 'registry', 'is', 'is-string'] : ['name', 'registry', 'is', 'is-string'];
                const stopLabel = ['namespace', 'name', 'registry', 'is', 'is-string'][stop];
                const stopIndex = labels.indexOf(stopLabel);
                const expectedLog = stopIndex < 0 ? labels : labels.slice(0, stopIndex + 1);
                check(prefix + `/order-${valid}-${stop}`, [capture(() => create(tag, options, namespace), sentinel, callee), log], [stopIndex < 0 ? (valid ? null : 'InvalidCharacterError') : 'sentinel', expectedLog]);
              }
            }
            const plain = observe(() => create('button', undefined).outerHTML);
            for (const value of [undefined, null, 'x-ignored', 0, 1, false, true, 1n]) {
              check(prefix + `/legacy-${typeof value}-${String(value)}`, observe(() => create('button', value).outerHTML), plain);
            }
            for (const valid of [false, true]) check(prefix + `/symbol-${valid}`, capture(() => create(valid ? 'div' : 'a b', Symbol()), undefined, callee), 'TypeError');
            for (const value of [{}, {is: undefined}, {customElementRegistry: undefined, is: undefined}]) {
              check(prefix + `/missing-is-${checks.length}`, observe(() => [create('button', value).outerHTML, create('button', value).getAttribute('is')]), [plain, null]);
            }
            for (const value of [null, '', 'x-built-in', 1, false]) {
              const log = [], options = {get customElementRegistry() {log.push('registry'); return undefined;}, get is() {log.push('is'); return value;}};
              check(prefix + `/is-${String(value)}`, observe(() => [create('button', options).getAttribute('is'), log]), [null, ['registry', 'is']]);
            }
            for (const make of [() => new owner.String('x-ignored'), () => function options() {}, () => Object.create({is: 'x-inherited'})]) {
              const options = make(), log = [];
              Object.defineProperty(options, 'customElementRegistry', {get() {log.push('registry'); return undefined;}});
              Object.defineProperty(options, 'is', {get() {log.push('is'); return 'x-boxed';}});
              Object.defineProperty(options, 'toString', {value() {throw new Error('dictionary toString must not run');}});
              check(prefix + `/dictionary-object-${checks.length}`, [capture(() => create('button', options), undefined, callee), log], [null, ['registry', 'is']]);
            }
            for (const registry of [null, scoped]) {
              for (const value of [null, '', 'x-built-in']) {
                for (const valid of [false, true]) {
                  const log = [], options = {get customElementRegistry() {log.push('registry'); return registry;}, get is() {log.push('is'); return value;}};
                  check(prefix + `/registry-is-${checks.length}`, [capture(() => create(valid ? 'div' : 'a b', options), undefined, callee), log], [valid ? 'NotSupportedError' : 'InvalidCharacterError', ['registry', 'is']]);
                }
              }
              check(prefix + `/registry-identity-${checks.length}`, observe(() => create('div', {customElementRegistry: registry, is: undefined}).customElementRegistry === registry), true);
            }
            const revoked = owner.Proxy.revocable(scoped, {}); revoked.revoke();
            let traps = 0;
            const invalidRegistries = [{}, Object.create(scoped), Object.create(owner.CustomElementRegistry.prototype), new owner.Proxy(scoped, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}}), revoked.proxy, false, 1, 'registry', Symbol()];
            for (const registry of invalidRegistries) {
              const log = [], options = {get customElementRegistry() {log.push('registry'); return registry;}, get is() {log.push('unexpected'); return 'x-built-in';}};
              check(prefix + `/invalid-registry-${checks.length}`, [capture(() => create('a b', options), undefined, callee), log, traps], ['TypeError', ['registry'], 0]);
            }
            const optionsProxy = new owner.Proxy({}, {get(target, key) {return key === 'customElementRegistry' ? undefined : key === 'is' ? 'x-proxy-options' : Reflect.get(target, key);}});
            check(prefix + '/options-author-proxy', capture(() => create('div', optionsProxy), undefined, callee), null);
            const revokedOptions = owner.Proxy.revocable({}, {}); revokedOptions.revoke();
            check(prefix + '/options-revoked-proxy', capture(() => create('a b', revokedOptions.proxy), undefined, callee), 'TypeError');
            for (let count = 0; count < (ns ? 2 : 1); count++) {
              let conversions = 0;
              const args = Array.from({length: count}, () => ({toString() {conversions++; throw 42;}}));
              check(prefix + `/arity-${count}`, [capture(() => method.call(doc, ...args), undefined, callee), conversions], ['TypeError', 0]);
            }
            const receivers = [{}, Object.create(doc), new owner.Proxy(doc, {})];
            const revokedDocument = owner.Proxy.revocable(doc, {}); revokedDocument.revoke(); receivers.push(revokedDocument.proxy);
            for (const receiver of receivers) {
              let conversions = 0;
              const value = {toString() {conversions++; throw 42;}};
              const args = ns ? [value, value, value] : [value, value];
              check(prefix + `/receiver-${checks.length}`, [capture(() => method.call(receiver, ...args), undefined, callee), conversions], ['TypeError', 0]);
            }
            if (ns) {
              const log = [], sentinel = {};
              check(prefix + '/namespace-before-flatten', [capture(() => create('p:div', {customElementRegistry: null, is: 'x-built-in'}, null), undefined, callee)], ['NamespaceError']);
              const options = {get customElementRegistry() {log.push('registry'); return undefined;}, get is() {log.push('is'); throw sentinel;}};
              check(prefix + '/options-before-namespace', [capture(() => create('p:div', options, null), sentinel, callee), log], ['sentinel', ['registry', 'is']]);
            }
          }
        }
      }
    }
    for (const [name, w] of realms) {
      for (const ns of [false, true]) {
        const label = `${name}/definition-reentrancy/${ns}`;
        const registry = new w.CustomElementRegistry();
        let constructed = 0;
        class Created extends w.HTMLElement {constructor() {super(); constructed++;}}
        const options = {customElementRegistry: registry, get is() {registry.define('x-options-' + ns, Created); return undefined;}};
        check(label, observe(() => {
          const element = ns ? w.document.createElementNS(htmlNS, 'x-options-' + ns, options) : w.document.createElement('x-options-' + ns, options);
          return [element instanceof Created, element.ownerDocument === w.document, element.customElementRegistry === registry, constructed];
        }), [true, true, true, 1]);
        const sentinel = {};
        const badOptions = {customElementRegistry: registry, get is() {throw sentinel;}};
        check(label + '/no-construction-after-throw', [capture(() => ns ? w.document.createElementNS(htmlNS, 'x-options-' + ns, badOptions) : w.document.createElement('x-options-' + ns, badOptions), sentinel), constructed], ['sentinel', 1]);
      }
    }
    // Genuine cross-realm registries within one Page share the native host.
    const foreign = new frame.contentWindow.CustomElementRegistry();
    for (const ns of [false, true]) {
      check(`cross-realm-registry/${ns}`, observe(() => {
        const element = ns ? document.createElementNS(null, 'foreign', {customElementRegistry: foreign}) : document.createElement('foreign', {customElementRegistry: foreign});
        return element.customElementRegistry === foreign;
      }), true);
    }
  } catch (error) {checks.push({name: 'uncaught', actual: String(error), expected: 'no exception', passed: false});}
  finally { frame.remove(); if (popup) popup.close(); }
  globalThis.__uiEventResults = {complete: true, includePopup: Boolean(popup), total: checks.length, passed: checks.filter(c => c.passed).length, checks};
  return checks.every(c => c.passed);
})()
