globalThis.runDecoderStreamSourceContract = async function () {
  const checks = [];
  async function check(name, operation) {
    globalThis.__decoderStreamSourceProgress = name;
    try { checks.push({ name, passed: await operation() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  }
  async function decode(value, fatal) {
    const source = new ReadableStream({
      start(controller) { controller.enqueue(value); controller.close(); },
    });
    const reader = source.pipeThrough(new TextDecoderStream("utf-8", { fatal })).getReader();
    const result = { chunks: [], error: undefined };
    try {
      while (true) {
        const item = await reader.read();
        if (item.done) break;
        result.chunks.push(item.value);
      }
    } catch (error) { result.error = error; }
    return result;
  }
  const valid = [
    ["buffer", () => Uint8Array.of(65).buffer],
    ["view", () => Uint8Array.of(0, 65, 0).subarray(1, 2)],
    ["DataView", () => {
      const bytes = Uint8Array.of(0, 65, 0);
      return new DataView(bytes.buffer, 1, 1);
    }],
    ["shared buffer", () => {
      const bytes = new Uint8Array(new SharedArrayBuffer(1));
      bytes[0] = 65;
      return bytes.buffer;
    }],
    ["shared view", () => {
      const bytes = new Uint8Array(new SharedArrayBuffer(3));
      bytes[1] = 65;
      return bytes.subarray(1, 2);
    }],
    ["shared DataView", () => {
      const bytes = new Uint8Array(new SharedArrayBuffer(3));
      bytes[1] = 65;
      return new DataView(bytes.buffer, 1, 1);
    }],
  ];
  for (const kind of ["buffer", "view", "DataView"]) {
    valid.push([`detached ${kind}`, () => {
      const bytes = Uint8Array.of(65);
      const input = kind === "buffer" ? bytes.buffer : kind === "view" ? bytes : new DataView(bytes.buffer);
      bytes.buffer.transfer(0);
      return input;
    }]);
  }
  for (const fatal of [false, true]) {
    for (const [name, make] of valid) {
      await check(`valid ${name}, fatal=${fatal}`, async () => {
        const input = make();
        const result = await decode(input, fatal);
        const expected = name.startsWith("detached") ? [] : ["A"];
        return result.error === undefined && JSON.stringify(result.chunks) === JSON.stringify(expected);
      });
    }
  }
  const invalid = [
    ["Proxy", () => new Proxy(Uint8Array.of(65), {})],
    ["revoked Proxy", () => {
      const { proxy, revoke } = Proxy.revocable(Uint8Array.of(65), {});
      revoke();
      return proxy;
    }],
  ];
  for (const shared of [false, true]) {
    for (const kind of ["buffer", "view", "DataView"]) {
      invalid.push([`${shared ? "growable shared" : "resizable"} ${kind}`, () => {
        const buffer = shared ? new SharedArrayBuffer(1, { maxByteLength: 2 }) :
          new ArrayBuffer(1, { maxByteLength: 2 });
        new Uint8Array(buffer)[0] = 65;
        return kind === "buffer" ? buffer : kind === "view" ? new Uint8Array(buffer) : new DataView(buffer);
      }]);
    }
  }
  for (const fatal of [false, true]) {
    for (const [name, make] of invalid) {
      await check(`reject ${name}, fatal=${fatal}`, async () => {
        const input = make();
        const result = await decode(input, fatal);
        return result.error instanceof TypeError && result.chunks.length === 0;
      });
    }
  }
  await check("snapshot when queued transform runs", async () => {
    const input = Uint8Array.of(65);
    const stream = new TextDecoderStream();
    const writer = stream.writable.getWriter();
    const reader = stream.readable.getReader();
    const write = writer.write(input);
    input[0] = 66;
    const first = await reader.read();
    await write;
    const close = writer.close();
    const last = await reader.read();
    await close;
    return !first.done && first.value === "B" && last.done;
  });
  for (const [name, make] of valid.slice(0, 6)) {
    await check(`ignore author buffer getters: ${name}`, async () => {
      const input = make();
      let reads = 0;
      for (const key of ["buffer", "byteOffset", "byteLength"]) {
        Object.defineProperty(input, key, { get() { reads++; throw Error(key); } });
      }
      const result = await decode(input, false);
      return result.error === undefined && result.chunks.join("") === "A" && reads === 0;
    });
  }
  for (const fatal of [false, true]) {
    for (const kind of ["buffer", "view", "DataView"]) {
      await check(`reject detached resizable ${kind}, fatal=${fatal}`, async () => {
        const buffer = new ArrayBuffer(1, { maxByteLength: 2 });
        const input = kind === "buffer" ? buffer : kind === "view" ? new Uint8Array(buffer) : new DataView(buffer);
        buffer.transfer(0);
        const result = await decode(input, fatal);
        return result.error instanceof TypeError && result.chunks.length === 0;
      });
    }
  }
  const identityInputs = [
    ["null", () => null],
    ["object", () => ({})],
    ["author Proxy", trap => new Proxy(Uint8Array.of(65), {
      get() { trap(); }, has() { trap(); }, getPrototypeOf() { trap(); },
    })],
    invalid[1],
    ["resizable buffer", () => new ArrayBuffer(1, { maxByteLength: 2 })],
    ["growable shared view", () => new Uint8Array(new SharedArrayBuffer(1, { maxByteLength: 2 }))],
    ["detached resizable buffer", () => {
      const buffer = new ArrayBuffer(1, { maxByteLength: 2 });
      buffer.transfer(0);
      return buffer;
    }],
  ];
  const rejection = promise => promise.then(() => undefined, error => error);
  for (const [name, make] of identityInputs) {
    await check(`asynchronous error identity: ${name}`, async () => {
      let traps = 0;
      const input = make(() => { traps++; throw Error("Proxy trap"); });
      const stream = new TextDecoderStream();
      const writer = stream.writable.getWriter();
      const reader = stream.readable.getReader();
      const read = rejection(reader.read());
      const readerClosed = rejection(reader.closed);
      const writerClosed = rejection(writer.closed);
      let write;
      try { write = writer.write(input); }
      catch (error) { await reader.cancel().catch(() => {}); return false; }
      const ready = rejection(writer.ready);
      const error = await rejection(write);
      if (!(error instanceof TypeError)) {
        await reader.cancel("unexpectedly accepted input").catch(() => {});
        return false;
      }
      const subsequentWrite = rejection(writer.write(Uint8Array.of(65)));
      const errors = await Promise.all([read, readerClosed, writerClosed, ready, subsequentWrite]);
      return write instanceof Promise && traps === 0 && errors.every(value => value === error);
    });
  }
  for (const kind of ["buffer", "DataView", "shared buffer", "shared view",
    "detached buffer", "detached view", "detached DataView"]) {
    await check(`queued input mutation: ${kind}`, async () => {
      const shared = kind.startsWith("shared");
      const detach = kind.startsWith("detached");
      const buffer = shared ? new SharedArrayBuffer(3) : new ArrayBuffer(3);
      const bytes = new Uint8Array(buffer);
      bytes[1] = 65;
      const input = kind.endsWith("DataView") ? new DataView(buffer, 1, 1) :
        kind.endsWith("view") ? bytes.subarray(1, 2) : buffer;
      if (input === buffer) bytes.fill(65);
      const stream = new TextDecoderStream();
      const writer = stream.writable.getWriter();
      const reader = stream.readable.getReader();
      const write = writer.write(input);
      if (detach) buffer.transfer(0);
      else if (shared) for (let index = 0; index < bytes.length; index++) Atomics.store(bytes, index, 66);
      else bytes.fill(66);
      const read = reader.read();
      read.catch(() => {});
      await write;
      const close = writer.close();
      const first = await read;
      if (detach) { await close; return first.done; }
      const last = await reader.read();
      await close;
      return !first.done && first.value === (input === buffer ? "BBB" : "B") && last.done;
    });
  }
  for (const [name, make] of [invalid[0], identityInputs[4]]) {
    await check(`cancel before queued conversion: ${name}`, async () => {
      const input = make();
      const stream = new TextDecoderStream();
      const writer = stream.writable.getWriter();
      const reader = stream.readable.getReader();
      const closed = rejection(writer.closed);
      let settled = false;
      const write = writer.write(input).then(
        () => { settled = true; }, error => { settled = true; return error; });
      await Promise.resolve();
      await Promise.resolve();
      const wasPending = !settled;
      const reason = {};
      await reader.cancel(reason);
      return wasPending && await write === reason && await closed === reason;
    });
  }
  await check("shared UTF-8 prefix survives to fixed trailing chunk", async () => {
    const prefix = new SharedArrayBuffer(1);
    new Uint8Array(prefix)[0] = 0xe2;
    const source = new ReadableStream({ start(controller) {
      controller.enqueue(prefix);
      controller.enqueue(Uint8Array.of(0x82, 0xac));
      controller.close();
    } });
    const reader = source.pipeThrough(new TextDecoderStream("utf-8", { fatal: true })).getReader();
    const first = await reader.read();
    const last = await reader.read();
    return !first.done && first.value === "\u20ac" && last.done;
  });
  return { checks };
};
