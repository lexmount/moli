globalThis.runTextEncoderBoundaryContract = async function () {
  const checks = [];
  async function check(name, operation) {
    try { checks.push({ name, passed: await operation() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  }
  function collect(readable) {
    const reader = readable.getReader();
    const result = { chunks: [], error: undefined };
    result.done = (async () => {
      try {
        while (true) {
          const item = await reader.read();
          if (item.done) return;
          result.chunks.push(Array.from(item.value));
        }
      } catch (error) { result.error = error; }
    })();
    return result;
  }
  const inputs = [
    "", "abc", "\u0000\u20ac\u4e2d", "A\ud83d\udc99B",
    "\ud800", "\udfff", "\ud800\udc00", "\udbff\udfff",
    "\ud800A\udc00", "\ud800\udbff\udc00", "\udbff\udfff\ud800",
    "\ud800\ud800\udc00\udfff",
  ];
  for (const input of ["abc", "\ud83d\udc99"]) {
    await check(`whole input ${input}`, async () => {
      const encoder = new TextEncoderStream();
      const result = collect(encoder.readable);
      const writer = encoder.writable.getWriter();
      await writer.write(input);
      await writer.close();
      await result.done;
      return result.error === undefined && JSON.stringify(result.chunks) ===
        JSON.stringify([Array.from(new TextEncoder().encode(input))]);
    });
  }
  for (const [index, input] of inputs.entries()) {
    const expected = Array.from(new TextEncoder().encode(input));
    for (let split = 0; split <= input.length; split++) {
      await check(`partition ${index} at ${split}`, async () => {
        const parts = [input.slice(0, split), "", input.slice(split), ""];
        const source = new ReadableStream({ start(controller) {
          for (const part of parts) controller.enqueue(part);
          controller.close();
        } });
        const result = collect(source.pipeThrough(new TextEncoderStream()));
        await result.done;
        return result.error === undefined && result.chunks.every(chunk => chunk.length > 0) &&
          JSON.stringify(result.chunks.flat()) === JSON.stringify(expected);
      });
    }
  }
  for (const end of ["close", "pair", "abort", "throw"]) {
    await check(`pending leading surrogate: ${end}`, async () => {
      const encoder = new TextEncoderStream();
      const result = collect(encoder.readable);
      const writer = encoder.writable.getWriter();
      writer.closed.catch(() => {});
      await writer.write("\ud83d");
      await writer.write("");
      const held = result.chunks.length === 0;
      const sentinel = {};
      if (end === "pair") {
        await writer.write("\udc99");
        await writer.close();
      } else if (end === "close") {
        await writer.close();
      } else if (end === "abort") {
        await writer.abort(sentinel);
      } else {
        try {
          await writer.write({ toString() { throw sentinel; } });
          return false;
        } catch (error) { if (error !== sentinel) return false; }
      }
      await result.done;
      const expected = end === "close" ? [[0xef, 0xbf, 0xbd]] :
        end === "pair" ? [[0xf0, 0x9f, 0x92, 0x99]] : [];
      return held && JSON.stringify(result.chunks) === JSON.stringify(expected) &&
        result.error === (end === "abort" || end === "throw" ? sentinel : undefined);
    });
  }
  await check("DOMString conversion preserves a surrogate pair split across objects", async () => {
    const encoder = new TextEncoderStream();
    const result = collect(encoder.readable);
    const writer = encoder.writable.getWriter();
    let calls = 0;
    for (const value of ["\ud83d", "", "\udc99"]) {
      await writer.write({ toString() { calls++; return value; } });
    }
    await writer.close();
    await result.done;
    return calls === 3 && result.error === undefined &&
      JSON.stringify(result.chunks) === JSON.stringify([[0xf0, 0x9f, 0x92, 0x99]]);
  });
  await check("interleaved encoders have separate pending surrogates", async () => {
    const left = new TextEncoderStream(), right = new TextEncoderStream();
    const a = collect(left.readable), b = collect(right.readable);
    const aw = left.writable.getWriter(), bw = right.writable.getWriter();
    await aw.write("\ud83d");
    await bw.write("A");
    await aw.write("\udc99");
    await Promise.all([aw.close(), bw.close(), a.done, b.done]);
    return a.error === undefined && b.error === undefined &&
      JSON.stringify(a.chunks) === JSON.stringify([[0xf0, 0x9f, 0x92, 0x99]]) &&
      JSON.stringify(b.chunks) === JSON.stringify([[65]]);
  });
  return { checks };
};
