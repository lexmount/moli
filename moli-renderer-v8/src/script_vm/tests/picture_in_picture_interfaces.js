(() => {
  const checks = [];
  function check(name, action) {
    try { checks.push({name, passed: action() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  const caught = action => { try { action(); return null; } catch (error) { return error; } };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  const owners = [window, frame.contentWindow, popup];
  try {
    for (const [realm, owner] of owners.entries()) {
      const Window = owner.PictureInPictureWindow;
      const Event = owner.PictureInPictureEvent;
      const typeError = error => error !== null && Object.getPrototypeOf(error) === owner.TypeError.prototype;
      check(`${realm}/Window constructor and inheritance`, () => Window.name === 'PictureInPictureWindow' && Window.length === 0 &&
        Object.getPrototypeOf(Window.prototype) === owner.EventTarget.prototype &&
        Object.getOwnPropertyDescriptor(Window.prototype, Symbol.toStringTag).value === 'PictureInPictureWindow');
      check(`${realm}/event constructor and inheritance`, () => Event.name === 'PictureInPictureEvent' && Event.length === 2 &&
        Object.getPrototypeOf(Event.prototype) === owner.Event.prototype &&
        Object.getOwnPropertyDescriptor(Event.prototype, Symbol.toStringTag).value === 'PictureInPictureEvent');
      let traps = 0;
      const proxy = new Proxy({}, {get() { traps++; throw Error('get trap'); }, getPrototypeOf() { traps++; throw Error('prototype trap'); }});
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      const invalidReceivers = [undefined, null, {}, new owner.EventTarget(), new owner.Event('x'), Window.prototype,
        Object.create(Window.prototype), Object.create(Event.prototype), proxy, revoked.proxy];
      for (const [name, prototype, setter] of [['width', Window.prototype, false], ['height', Window.prototype, false],
          ['onresize', Window.prototype, true], ['pictureInPictureWindow', Event.prototype, false]]) {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
        check(`${realm}/${name}/own enumerable configurable accessor`, () => !!descriptor && descriptor.enumerable && descriptor.configurable &&
          typeof descriptor.get === 'function' && descriptor.get.length === 0 &&
          (setter ? typeof descriptor.set === 'function' && descriptor.set.length === 1 : descriptor.set === undefined));
        for (const [index, receiver] of invalidReceivers.entries()) {
          check(`${realm}/${name}/invalid getter receiver ${index}`, () => typeError(caught(() => descriptor.get.call(receiver))));
          if (setter) {
            check(`${realm}/${name}/invalid setter receiver ${index}`, () => {
              let conversions = 0;
              const value = {toString() { conversions++; throw Error('unexpected conversion'); }};
              return typeError(caught(() => descriptor.set.call(receiver, value))) && conversions === 0;
            });
          }
        }
      }
      for (const construct of [false, true]) {
        check(`${realm}/Window illegal constructor/${construct}`, () => {
          let conversions = 0;
          const value = {toString() { conversions++; throw Error('unexpected conversion'); }};
          const error = caught(() => construct ? new Window(value) : Window(value));
          return typeError(error) && conversions === 0;
        });
      }
      for (const count of [0, 1]) {
        check(`${realm}/event requires two arguments before type conversion/${count}`, () => {
          let conversions = 0;
          const value = {toString() { conversions++; throw Error('unexpected conversion'); }};
          return typeError(caught(() => Reflect.construct(Event, count ? [value] : []))) && conversions === 0;
        });
      }
      check(`${realm}/event requires new before argument conversion`, () => {
        let conversions = 0;
        return typeError(caught(() => Event({toString() { conversions++; throw Error('unexpected conversion'); }}, {}))) && conversions === 0;
      });
      for (const [index, dictionary] of [undefined, null, {}, false, 1, 'x', 1n, Symbol('dictionary')].entries()) {
        check(`${realm}/invalid or incomplete dictionary/${index}`, () => typeError(caught(() => new Event('x', dictionary))));
      }
      for (const [index, value] of [undefined, null, false, 1, 'x', 1n, Symbol('window'), {}, Window.prototype,
          Object.create(Window.prototype), Object.create(new owner.EventTarget()), proxy, revoked.proxy].entries()) {
        check(`${realm}/required native window brand/${index}`, () => typeError(caught(() => new Event('x', {pictureInPictureWindow: value}))));
      }
      for (const throwingMember of ['type', 'bubbles', 'cancelable', 'composed', 'pictureInPictureWindow', 'none']) {
        check(`${realm}/dictionary conversion order and exception identity/${throwingMember}`, () => {
          const log = [], marker = {};
          const read = name => {
            log.push(name);
            if (name === throwingMember) throw marker;
            return name === 'type' ? 'resize' : name === 'pictureInPictureWindow' ? {} : false;
          };
          const type = {toString() { return read('type'); }};
          const dictionary = {};
          for (const name of ['bubbles', 'cancelable', 'composed', 'pictureInPictureWindow']) {
            Object.defineProperty(dictionary, name, {get() { return read(name); }});
          }
          const error = caught(() => new Event(type, dictionary));
          const names = ['type', 'bubbles', 'cancelable', 'composed', 'pictureInPictureWindow'];
          const expected = throwingMember === 'none' ? names : names.slice(0, names.indexOf(throwingMember) + 1);
          return JSON.stringify(log) === JSON.stringify(expected) && (throwingMember === 'none' ? typeError(error) : error === marker);
        });
      }
      check(`${realm}/author and revoked Proxy brand checks do not invoke traps`, () => traps === 0);
    }
  } finally {
    popup.close(); frame.remove();
  }
  const facts = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  globalThis.__uiEventResults = facts;
  return facts.passed === facts.total;
})()
