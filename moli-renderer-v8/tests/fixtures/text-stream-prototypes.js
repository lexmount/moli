globalThis.runTextStreamPrototypeContract = async function () {
  const checks = [];
  function check(name, operation) {
    try { checks.push({ name, passed: operation() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  }
  for (const [Constructor, make, attributes, other, codec] of [
    [TextEncoderStream, () => new TextEncoderStream(),
      ["encoding", "readable", "writable"], () => new TextDecoderStream(), () => new TextEncoder()],
    [TextDecoderStream, () => new TextDecoderStream("utf-16le", { fatal: true, ignoreBOM: true }),
      ["encoding", "fatal", "ignoreBOM", "readable", "writable"],
      () => new TextEncoderStream(), () => new TextDecoder()],
  ]) {
    const prototype = Constructor.prototype;
    const matches = (name, value) => name === "readable" ? value instanceof ReadableStream :
      name === "writable" ? value instanceof WritableStream :
      name === "encoding" ? value === (Constructor === TextEncoderStream ? "utf-8" : "utf-16le") :
      value === true;
    for (const name of attributes) {
      const prefix = `${Constructor.name}.${name}`;
      check(`${prefix}: prototype accessor descriptor`, () => {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
        return descriptor !== undefined && typeof descriptor.get === "function" &&
          descriptor.get.name === `get ${name}` && descriptor.get.length === 0 &&
          descriptor.set === undefined && descriptor.enumerable && descriptor.configurable &&
          !Object.hasOwn(descriptor, "value") && !Object.hasOwn(descriptor, "writable");
      });
      check(`${prefix}: no own property`, () => !Object.hasOwn(make(), name));
      check(`${prefix}: value and identity`, () => {
        const receiver = make();
        const getter = Object.getOwnPropertyDescriptor(prototype, name).get;
        return matches(name, receiver[name]) && getter.call(receiver) === receiver[name] &&
          receiver[name] === receiver[name];
      });
      check(`${prefix}: readonly assignment`, () => {
        const receiver = make();
        const value = receiver[name];
        if (Reflect.set(receiver, name, null)) return false;
        try {
          (() => { "use strict"; receiver[name] = null; })();
          return false;
        } catch (error) {
          return error instanceof TypeError && !Object.hasOwn(receiver, name) && receiver[name] === value;
        }
      });
      for (const [kind, create] of [
        ["null", () => null], ["undefined", () => undefined], ["number", () => 1],
        ["object", () => ({})], ["prototype", () => prototype],
        ["inherited instance", () => Object.create(make())],
        ["author Proxy", trap => new Proxy(make(), {
          get() { trap(); }, has() { trap(); }, getPrototypeOf() { trap(); },
        })],
        ["revoked Proxy", () => {
          const { proxy, revoke } = Proxy.revocable(make(), {});
          revoke();
          return proxy;
        }],
        ["other stream", other], ["non-stream codec", codec],
      ]) {
        check(`${prefix}: reject ${kind}`, () => {
          let traps = 0;
          const receiver = create(() => { traps++; throw Error("Proxy trap"); });
          const getter = Object.getOwnPropertyDescriptor(prototype, name).get;
          try { getter.call(receiver); return false; }
          catch (error) { return error instanceof TypeError && traps === 0; }
        });
      }
      check(`${prefix}: changed receiver prototype`, () => {
        const receiver = make();
        const value = receiver[name];
        Object.setPrototypeOf(receiver, null);
        return matches(name, value) && Object.getOwnPropertyDescriptor(prototype, name).get.call(receiver) === value;
      });
      check(`${prefix}: subclass receiver`, () => {
        class Derived extends Constructor {}
        const receiver = Constructor === TextEncoderStream ? new Derived() :
          new Derived("utf-16le", { fatal: true, ignoreBOM: true });
        return matches(name, Object.getOwnPropertyDescriptor(prototype, name).get.call(receiver));
      });
      check(`${prefix}: own property cannot replace native state`, () => {
        const receiver = make();
        const value = receiver[name];
        const sentinel = {};
        Object.defineProperty(receiver, name, { value: sentinel });
        return matches(name, value) && receiver[name] === sentinel &&
          Object.getOwnPropertyDescriptor(prototype, name).get.call(receiver) === value;
      });
      check(`${prefix}: frozen receiver`, () => {
        const receiver = make();
        const value = receiver[name];
        Object.freeze(receiver);
        return matches(name, value) && Object.getOwnPropertyDescriptor(prototype, name).get.call(receiver) === value;
      });
    }
    check(`${Constructor.name}: ignores inherited setters during construction`, () => {
      let calls = 0;
      let receiver;
      try {
        for (const name of attributes) {
          Object.defineProperty(Object.prototype, name, {
            configurable: true,
            set() { calls++; throw Error(name); },
          });
        }
        receiver = make();
      } finally {
        for (const name of attributes) delete Object.prototype[name];
      }
      return calls === 0 && attributes.every(name => matches(name, receiver[name]));
    });
  }
  return { checks };
};
