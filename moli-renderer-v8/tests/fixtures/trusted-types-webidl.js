(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const rejects = (callback, message) => {
    let error;
    try { callback(); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, message);
  };
  const isWindow = typeof window !== "undefined";
  const owner = isWindow ? globalThis : WorkerGlobalScope.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(owner, "trustedTypes");
  assert(typeof descriptor?.get === "function", "trustedTypes is an accessor");
  assert(descriptor.set === undefined && !Object.hasOwn(descriptor, "value"),
    "trustedTypes has no setter or data value");
  assert(descriptor.enumerable && descriptor.configurable, "trustedTypes flags");
  const getter = descriptor.get;
  assert(getter.name === "get trustedTypes" && getter.length === 0, "getter metadata");
  assert(!Object.hasOwn(getter, "prototype"), "getter is not a constructor");
  rejects(() => Reflect.construct(getter, []), "new getter");
  if (!isWindow) assert(!Object.hasOwn(globalThis, "trustedTypes"), "worker placement");

  const factory = trustedTypes;
  assert(factory instanceof TrustedTypePolicyFactory, "factory interface");
  for (const receiver of [globalThis, null, undefined]) {
    assert(getter.call(receiver) === factory, "getter global and nullish receivers");
  }
  for (const receiver of [{}, Object.create(globalThis), new Proxy(globalThis, {}), 1]) {
    rejects(() => getter.call(receiver), "getter receiver brand");
  }
  assert(!Reflect.set(globalThis, "trustedTypes", {}), "readonly write fails");
  rejects(() => { "use strict"; globalThis.trustedTypes = {}; }, "strict readonly write");
  assert(trustedTypes === factory, "readonly write preserves factory");
  assert(Object.getOwnPropertyDescriptor(owner, "trustedTypes").get === getter,
    "getter remains an accessor after materialization");
  assert(factory.createPolicy.length === 1, "optional policy options do not count in length");
  const policy = factory.createPolicy("webidl-surface", {
    createHTML: value => value,
    createScript: value => value,
    createScriptURL: value => value,
  });
  const entries = [
    [TrustedHTML, policy.createHTML("<b>html</b>"), "<b>html</b>", "isHTML"],
    [TrustedScript, policy.createScript("1 + 1"), "1 + 1", "isScript"],
    [TrustedScriptURL, policy.createScriptURL("/app.js"), "/app.js", "isScriptURL"],
  ];
  for (const [Constructor, value, text, predicate] of entries) {
    const proto = Constructor.prototype;
    const prototypeDescriptor = Object.getOwnPropertyDescriptor(Constructor, "prototype");
    assert(!prototypeDescriptor.writable && !prototypeDescriptor.enumerable &&
      !prototypeDescriptor.configurable, `${Constructor.name}.prototype flags`);
    const constructorDescriptor = Object.getOwnPropertyDescriptor(proto, "constructor");
    assert(constructorDescriptor.value === Constructor && constructorDescriptor.writable &&
      !constructorDescriptor.enumerable && constructorDescriptor.configurable,
      `${Constructor.name} constructor backlink`);
    rejects(() => Constructor(), `${Constructor.name} call`);
    rejects(() => new Constructor(), `${Constructor.name} construction`);
    const tag = Object.getOwnPropertyDescriptor(proto, Symbol.toStringTag);
    assert(tag.value === Constructor.name && !tag.writable && !tag.enumerable && tag.configurable,
      `${Constructor.name} tag flags`);
    assert(!Object.hasOwn(proto, "valueOf") && value.valueOf() === value,
      `${Constructor.name} inherits Object valueOf`);
    const fake = Object.create(proto);
    const revoked = Proxy.revocable(value, {});
    revoked.revoke();
    const fakeValues = [fake, Object.create(value), new Proxy(value, {}), revoked.proxy,
      {}, null, undefined, text];
    for (const name of ["toString", "toJSON"]) {
      const method = Object.getOwnPropertyDescriptor(proto, name);
      assert(method.enumerable && method.configurable && method.writable,
        `${Constructor.name}.${name} flags`);
      assert(method.value.name === name && method.value.length === 0 &&
        !Object.hasOwn(method.value, "prototype"), `${name} metadata`);
      rejects(() => Reflect.construct(method.value, []), `new ${name}`);
      assert(method.value.call(value) === text, `${name} data`);
      for (const other of [...fakeValues, ...entries.filter(entry => entry[0] !== Constructor)
        .map(entry => entry[1])]) {
        rejects(() => method.value.call(other), `${Constructor.name}.${name} receiver brand`);
      }
    }
    rejects(() => factory[predicate](), `${predicate} missing argument`);
    for (const other of fakeValues) {
      assert(factory[predicate](other) === false, `${predicate} non-instance`);
    }
    for (const [Other, other] of entries) {
      assert(factory[predicate](other) === (Other === Constructor), `${predicate} kind`);
    }
    const trap = new Proxy({}, { get() { throw new Error("predicate must not coerce"); } });
    assert(factory[predicate](trap) === false, `${predicate} does not coerce input`);
    rejects(() => factory[predicate].call({}, value), `${predicate} factory brand`);
    assert(String(value) === text && JSON.stringify(value) === JSON.stringify(text),
      `${Constructor.name} string and JSON conversion`);
    value.toString = () => "7";
    assert(Number(value) === 7, `${Constructor.name} default primitive conversion`);
    delete value.toString;
  }

  const Policy = TrustedTypePolicy;
  globalThis.TrustedTypePolicy = function ReplacementPolicy() {};
  globalThis.TrustedTypePolicyFactory = function ReplacementFactory() {};
  const afterOverride = factory.createPolicy("after-public-overrides", {createHTML: value => value});
  assert(afterOverride instanceof Policy && String(afterOverride.createHTML("safe")) === "safe",
    "policy creation uses intrinsic prototypes after public constructor replacement");

  Object.defineProperty(globalThis, "trustedTypes", {value: "shadowed", configurable: true});
  assert(getter.call(globalThis) === factory, "captured getter ignores public replacement");
  return "ok";
})()
