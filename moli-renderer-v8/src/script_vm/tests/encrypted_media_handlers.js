(() => {
  const checks = [];
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const check = (name, operation) => {
    try { operation(); checks.push({name, passed:true}); }
    catch (error) { checks.push({name, passed:false, error:String(error)}); }
  };
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const frame = document.querySelector('iframe') || document.body.appendChild(document.createElement('iframe'));
  const child = frame.contentWindow;
  const detached = document.implementation.createHTMLDocument('');
  const realms = [['main', window], ['child', child]];
  const documents = [['main', document], ['child', child.document], ['windowless', detached]];
  for (const [realmName, realm] of realms) {
    for (const type of ['encrypted', 'waitingforkey']) {
      const name = 'on' + type;
      const descriptor = Object.getOwnPropertyDescriptor(realm.HTMLMediaElement.prototype, name);
      const prefix = realmName + '/' + name;
      check(prefix + '/own descriptor', () => {
        assert(descriptor?.enumerable && descriptor.configurable, 'own enumerable configurable accessor');
        assert(typeof descriptor.get === 'function' && typeof descriptor.set === 'function', 'getter and setter');
        assert(descriptor.get.length === 0 && descriptor.set.length === 1, 'arity');
        assert(descriptor.get.name === 'get ' + name && descriptor.set.name === 'set ' + name, 'names');
        assert(Object.getPrototypeOf(descriptor.get) === realm.Function.prototype, 'getter realm');
      });
      check(prefix + '/interface placement', () => {
        for (const owner of [realm.HTMLElement.prototype, realm.Element.prototype, realm.Document.prototype, realm]) {
          assert(!Object.hasOwn(owner, name), 'not installed on unrelated interface');
        }
        for (const tag of ['audio', 'video']) {
          const media = realm.document.createElement(tag);
          assert(name in media && !Object.hasOwn(media, name), 'inherited by ' + tag);
        }
        assert(!(name in realm.document.createElement('div')), 'not on arbitrary HTMLElement');
      });
      for (const [documentName, owner] of documents) {
        for (const tag of ['audio', 'video']) {
          const targetName = prefix + '/' + documentName + '/' + tag;
          check(targetName + '/ordered dispatch and replacement', () => {
            const media = owner.createElement(tag);
            assert(descriptor.get.call(media) === null, 'initial null');
            const order = [];
            media.addEventListener(type, () => order.push('before'));
            descriptor.set.call(media, () => order.push('old'));
            media.addEventListener(type, () => order.push('after'));
            const handler = function(event) {
              assert(this === media && event.currentTarget === media && event.target === media, 'receiver identity');
              assert(event.type === type && event.isTrusted === false, 'authored event');
              order.push('replacement');
              return false;
            };
            descriptor.set.call(media, handler);
            assert(descriptor.get.call(media) === handler && media[name] === handler, 'same handler');
            assert(!Object.hasOwn(media, name) && !media.hasAttribute(name), 'no expando or content mutation');
            const first = new Event(type, {cancelable:true});
            assert(media.dispatchEvent(first) === false && first.defaultPrevented, 'return false cancels');
            assert(order.join() === 'before,replacement,after', 'replacement preserves position');
            descriptor.set.call(media, null);
            assert(descriptor.get.call(media) === null, 'removal');
            order.length = 0;
            const next = function() { order.push('reactivated'); return 0; };
            descriptor.set.call(media, next);
            assert(media.dispatchEvent(new Event(type, {cancelable:true})), 'only false cancels');
            assert(order.join() === 'before,after,reactivated', 'reactivation appends');
          });
          check(targetName + '/conversion preserves object and clears primitives', () => {
            const media = owner.createElement(tag);
            let reads = 0;
            const object = {get handleEvent() { reads++; throw Error('unexpected callback interface'); }};
            descriptor.set.call(media, object);
            assert(descriptor.get.call(media) === object, 'object identity');
            assert(media.dispatchEvent(new Event(type)) && reads === 0, 'noncallable object stays silent');
            for (const value of [undefined, null, false, 0, '', Symbol('handler'), 1n]) {
              descriptor.set.call(media, () => {});
              descriptor.set.call(media, value);
              assert(descriptor.get.call(media) === null, 'primitive converts to null');
            }
          });
        }
      }
      const real = realm.document.createElement('audio');
      let traps = 0;
      const proxy = new Proxy(real, {
        get() {traps++; throw Error('get trap');},
        getPrototypeOf() {traps++; throw Error('prototype trap');}
      });
      const revoked = Proxy.revocable(real, {}); revoked.revoke();
      for (const [label, receiver] of [
        ['plain', {}], ['forged', Object.create(realm.HTMLMediaElement.prototype)],
        ['inherited', Object.create(real)], ['author proxy', proxy], ['revoked proxy', revoked.proxy],
        ['element', realm.document.createElement('div')], ['document', realm.document],
        ['window', realm], ['null', null], ['undefined', undefined]
      ]) {
        for (const operation of ['get', 'set']) {
          check(prefix + '/receiver/' + label + '/' + operation, () => {
            assert(descriptor && typeof descriptor[operation] === 'function', 'accessor must exist before receiver validation');
            let error;
            try { descriptor[operation].call(receiver, function() {}); } catch (caught) { error = caught; }
            assert(error && Object.getPrototypeOf(error) === realm.TypeError.prototype, 'callee realm TypeError');
            assert(traps === 0, 'brand check must not run author traps');
          });
        }
      }
      for (const tag of ['audio', 'video']) {
        check(prefix + '/' + tag + '/content attribute and borrowed getter realm', () => {
          const media = realm.document.createElement(tag);
          media.setAttribute(name, 'this.setAttribute("data-fired", event.type); return false');
          const borrowed = Object.getOwnPropertyDescriptor((realm === window ? child : window).HTMLMediaElement.prototype, name).get;
          const handler = borrowed.call(media);
          assert(typeof handler === 'function' && Object.getPrototypeOf(handler) === realm.Function.prototype, 'target realm compilation');
          assert(borrowed.call(media) === handler && media[name] === handler, 'compiled function identity');
          assert(!media.dispatchEvent(new Event(type, {cancelable:true})) && media.getAttribute('data-fired') === type, 'content dispatch');
          media.removeAttribute(name);
          assert(media[name] === null && media.dispatchEvent(new Event(type, {cancelable:true})), 'content removal');
        });
        check(prefix + '/' + tag + '/empty content attribute is callable', () => {
          const media = realm.document.createElement(tag);
          media.setAttribute(name, '');
          const handler = media[name];
          assert(typeof handler === 'function' && handler === media[name], 'empty body creates stable handler');
          assert(media.dispatchEvent(new Event(type, {cancelable:true})), 'empty body does not cancel');
        });
        check(prefix + '/' + tag + '/IDL replacement and attribute removal', () => {
          const media = realm.document.createElement(tag);
          media.setAttribute(name, 'throw Error("replaced content handler")');
          let count = 0;
          const handler = () => {count++;};
          descriptor.set.call(media, handler);
          assert(media[name] === handler && media.getAttribute(name).includes('replaced'), 'IDL value overrides content without changing attribute');
          media.dispatchEvent(new Event(type));
          assert(count === 1, 'replacement invokes once');
          media.removeAttribute(name);
          assert(media[name] === null, 'attribute removal clears current handler');
        });
        check(prefix + '/' + tag + '/windowless content is disabled, authored callback works', () => {
          const media = detached.createElement(tag);
          media.setAttribute(name, 'throw Error("windowless inline callback")');
          assert(descriptor.get.call(media) === null && media.dispatchEvent(new Event(type)), 'no compilation without scripting');
          let count = 0;
          descriptor.set.call(media, () => {count++;});
          media.dispatchEvent(new Event(type));
          assert(count === 1, 'authored callback dispatch');
        });
      }
    }
  }
  globalThis.__uiEventResults = {complete:true, total:checks.length,
    passed:checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
