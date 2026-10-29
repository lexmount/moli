globalThis.runTextDecoderInputSnapshotContract = async function () {
  const checks = [];
  function check(name, operation) {
    try { checks.push({ name, passed: operation() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  }
  for (const [name, make] of [
    ["buffer", () => {
      const bytes = Uint8Array.of(65);
      return { input: bytes.buffer, mutate: () => bytes[0] = 66 };
    }],
    ["view offset", () => {
      const bytes = Uint8Array.of(0, 65, 0);
      return { input: bytes.subarray(1, 2), mutate: () => bytes[1] = 66 };
    }],
    ["DataView offset", () => {
      const bytes = Uint8Array.of(0, 65, 0);
      return { input: new DataView(bytes.buffer, 1, 1), mutate: () => bytes[1] = 66 };
    }],
    ["shared buffer", () => {
      const bytes = new Uint8Array(new SharedArrayBuffer(1));
      bytes[0] = 65;
      return { input: bytes.buffer, mutate: () => Atomics.store(bytes, 0, 66) };
    }],
    ["shared view offset", () => {
      const bytes = new Uint8Array(new SharedArrayBuffer(3));
      bytes[1] = 65;
      return { input: bytes.subarray(1, 2), mutate: () => Atomics.store(bytes, 1, 66) };
    }],
  ]) {
    check(`snapshot after options: ${name}`, () => {
      const { input, mutate } = make();
      let reads = 0;
      const output = new TextDecoder().decode(input, {
        get stream() { reads++; mutate(); return false; },
      });
      return reads === 1 && output === "B";
    });
  }
  for (const length of [1, 10000]) {
    for (const kind of ["buffer", "view", "DataView"]) {
      check(`detach during options: ${kind} length=${length}`, () => {
        const bytes = new Uint8Array(length).fill(42);
        const input = kind === "buffer" ? bytes.buffer : kind === "view" ? bytes : new DataView(bytes.buffer);
        let reads = 0;
        const output = new TextDecoder().decode(input, {
          get stream() { reads++; bytes.buffer.transfer(0); return false; },
        });
        return reads === 1 && output === "";
      });
    }
  }
  const invalidSources = [
    ["null", () => null],
    ["object", () => ({})],
    ["array", () => []],
    ["forged view", () => Object.create(Uint8Array.prototype)],
    ["Proxy", () => new Proxy(Uint8Array.of(65), {})],
    ["revoked Proxy", () => {
      const proxy = Proxy.revocable(Uint8Array.of(65), {});
      proxy.revoke();
      return proxy.proxy;
    }],
  ];
  for (const shared of [false, true]) {
    for (const kind of ["buffer", "view", "DataView"]) {
      invalidSources.push([`${shared ? "growable shared" : "resizable"} ${kind}`, () => {
        const buffer = shared ? new SharedArrayBuffer(1, { maxByteLength: 2 }) :
          new ArrayBuffer(1, { maxByteLength: 2 });
        return kind === "buffer" ? buffer : kind === "view" ? new Uint8Array(buffer) : new DataView(buffer);
      }]);
    }
  }
  for (const kind of ["buffer", "view"]) {
    invalidSources.push([`detached resizable ${kind}`, () => {
      const buffer = new ArrayBuffer(1, { maxByteLength: 2 });
      const view = new Uint8Array(buffer);
      buffer.transfer(0);
      return kind === "buffer" ? buffer : view;
    }]);
  }
  for (const [name, make] of invalidSources) {
    check(`reject before options: ${name}`, () => {
      const input = make();
      let reads = 0;
      try {
        new TextDecoder().decode(input, { get stream() { reads++; return false; } });
        return false;
      } catch (error) { return error instanceof TypeError && reads === 0; }
    });
  }
  check("options exception identity", () => {
    const sentinel = {};
    try {
      new TextDecoder().decode(Uint8Array.of(65), { get stream() { throw sentinel; } });
      return false;
    } catch (error) { return error === sentinel; }
  });
  check("undefined converts options and flushes", () => {
    const decoder = new TextDecoder();
    decoder.decode(Uint8Array.of(0xc9), { stream: true });
    let reads = 0;
    const output = decoder.decode(undefined, { get stream() { reads++; return false; } });
    return reads === 1 && output === "\ufffd";
  });
  for (const kind of ["buffer", "view", "DataView", "shared buffer"]) {
    check(`ignore author buffer getters: ${kind}`, () => {
      const buffer = kind === "shared buffer" ? new SharedArrayBuffer(1) : new ArrayBuffer(1);
      new Uint8Array(buffer)[0] = 65;
      const input = kind === "view" ? new Uint8Array(buffer) : kind === "DataView" ? new DataView(buffer) : buffer;
      let reads = 0;
      for (const name of ["byteLength", "buffer", "byteOffset"]) {
        Object.defineProperty(input, name, { get() { reads++; throw Error(name); } });
      }
      return new TextDecoder().decode(input) === "A" && reads === 0;
    });
  }
  check("receiver check before options", () => {
    let reads = 0;
    try {
      TextDecoder.prototype.decode.call({}, Uint8Array.of(65), { get stream() { reads++; return false; } });
      return false;
    } catch (error) { return error instanceof TypeError && reads === 0; }
  });
  check("reentrant options decode", () => {
    const decoder = new TextDecoder();
    let reads = 0;
    const output = decoder.decode(Uint8Array.of(0xac), {
      get stream() {
        reads++;
        decoder.decode(Uint8Array.of(0xe2, 0x82), { stream: true });
        return false;
      },
    });
    return reads === 1 && output === "\u20ac";
  });
  return { checks };
};
