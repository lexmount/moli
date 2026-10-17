(() => {
  const checks = [];
  globalThis.__blobReadingProgress = { stage: "URL intrinsic query objects", checks };
  const NativeURL = URL;
  const publicDescriptor = Object.getOwnPropertyDescriptor(globalThis, "URLSearchParams");
  let NativeParams;
  const restore = () => Object.defineProperty(globalThis, "URLSearchParams", publicDescriptor);
  const factories = [
    ["constructor", input => new NativeURL(input)],
    ["parse", input => NativeURL.parse(input)]
  ];
  for (const [factory, create] of factories) {
    for (const scenario of ["throwing getter", "throwing constructor", "shared author object", "deleted global", "conversion replacement"]) {
      let calls = 0, conversions = 0;
      const marker = new Error("author URLSearchParams must not run");
      const shared = {};
      let detail;
      const poisonGetter = () => Object.defineProperty(globalThis, "URLSearchParams", {
        configurable: true, get() { calls++; throw marker; }
      });
      try {
        let input = "https://query-intrinsic.test/?a=1";
        if (scenario === "throwing getter") poisonGetter();
        else if (scenario === "throwing constructor") Object.defineProperty(globalThis, "URLSearchParams", {
          configurable: true, writable: true, value: function () { calls++; throw marker; }
        });
        else if (scenario === "shared author object") Object.defineProperty(globalThis, "URLSearchParams", {
          configurable: true, writable: true, value: function () { calls++; return shared; }
        });
        else if (scenario === "deleted global") delete globalThis.URLSearchParams;
        else input = { toString() { conversions++; poisonGetter(); return "https://query-intrinsic.test/?a=1"; } };
        const url = create(input), params = url.searchParams;
        const second = scenario === "shared author object" ? create("https://query-intrinsic.test/?a=2") : null;
        restore(); NativeParams ??= globalThis.URLSearchParams;
        let sharedRejected = false;
        try { NativeParams.prototype.get.call(shared, "a"); }
        catch (error) { sharedRejected = error instanceof TypeError; }
        const initial = NativeParams.prototype.get.call(params, "a");
        params.append("b", "2");
        const appended = url.search === "?a=1&b=2";
        url.search = "?c=3";
        detail = { calls, conversions, intrinsicPrototype: Object.getPrototypeOf(params) === NativeParams.prototype,
          initial, appended, sameObject: params === url.searchParams, updated: params.get("c"), sharedRejected,
          independent: second === null || second.searchParams !== params && second.searchParams.get("a") === "2" };
        checks.push({ name: `${factory}: ${scenario}`, passed: calls === 0 && detail.intrinsicPrototype && initial === "1"
          && appended && detail.sameObject && detail.updated === "3" && sharedRejected && detail.independent
          && conversions === (scenario === "conversion replacement" ? 1 : 0), detail });
      } catch (error) {
        checks.push({ name: `${factory}: ${scenario}`, passed: false,
          detail: { calls, conversions, error: error === marker ? "author marker" : String(error.stack || error) } });
      } finally { restore(); NativeParams ??= globalThis.URLSearchParams; }
    }
  }
  globalThis.__uiEventResults = { checks };
})();
