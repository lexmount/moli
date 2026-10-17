globalThis.runTextDecoderStateContract = async function () {
  const checks = [];
  const readable = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, "readable").get;
  const writable = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, "writable").get;
  async function check(name, operation) {
    try {
      checks.push({ name, passed: await operation() === true });
    } catch (error) {
      checks.push({ name, passed: false, error: String(error) });
    }
  }
  async function decode(label, options, chunks, Decoder = TextDecoderStream) {
    const decoder = new Decoder(label, options);
    const endpoints = { readable: readable.call(decoder), writable: writable.call(decoder) };
    const source = new ReadableStream({ start(controller) {
      for (const bytes of chunks) controller.enqueue(Uint8Array.from(bytes));
      controller.close();
    } });
    const reader = source.pipeThrough(endpoints).getReader();
    const output = [];
    while (true) {
      const item = await reader.read();
      if (item.done) return output;
      output.push(item.value);
    }
  }
  for (const [name, label, options, chunks, expected] of [
    ["ASCII chunks", "utf-8", {}, [[97], [], [98, 99]], "abc"],
    ["split UTF-8", "utf-8", {}, [[0xe2], [0x82, 0xac]], "\u20ac"],
    ["split UTF-16BE", "utf-16be", {}, [[0], [65]], "A"],
    ["split UTF-16LE", "utf-16le", {}, [[65], [0]], "A"],
    ["UTF-8 BOM removal", "utf-8", {}, [[0xef], [0xbb], [0xbf, 65]], "A"],
    ["UTF-8 BOM preservation", "utf-8", { ignoreBOM: true }, [[0xef], [0xbb], [0xbf, 65]], "\ufeffA"],
    ["UTF-16LE BOM removal", "utf-16le", {}, [[0xff], [0xfe, 65, 0]], "A"],
    ["UTF-16LE BOM preservation", "utf-16le", { ignoreBOM: true }, [[0xff], [0xfe, 65, 0]], "\ufeffA"],
    ["legacy GBK", "gbk", {}, [[0xcc], [0xab, 0xc6], [0xbd, 0xd1], [0xf3]], "\u592a\u5e73\u6d0b"],
    ["ISO-2022-JP state", "iso-2022-jp", {}, [[0x1b, 0x28], [0x4a, 0x7e]], "\u203e"],
    ["incomplete UTF-8 flush", "utf-8", {}, [[0xe2], [0x82]], "\ufffd"],
    ["incomplete UTF-16 flush", "utf-16be", {}, [[0]], "\ufffd"],
  ]) {
    await check(name, async () => (await decode(label, options, chunks)).join("") === expected);
  }
  for (const [label, chunks] of [["utf-8", [[0xff]]], ["utf-8", [[0xe2]]], ["utf-16be", [[0]]]]) {
    await check(`fatal ${label} ${chunks[0][0]}`, async () => {
      try { await decode(label, { fatal: true }, chunks); return false; }
      catch (error) { return error instanceof TypeError; }
    });
  }
  await check("interleaved decoder states", async () => {
    const results = await Promise.all([
      decode("utf-8", {}, [[0xe2], [0x82, 0xac]]),
      decode("utf-16be", {}, [[0], [65]]),
      decode("windows-1252", {}, [[0x80]]),
    ]);
    return JSON.stringify(results.map(parts => parts.join(""))) === JSON.stringify(["\u20ac", "A", "\u20ac"]);
  });
  for (const throws of [true, false]) {
    await check(`subclass writable getter ${throws ? "throws" : "redirects"}`, async () => {
      let calls = 0;
      class Decoder extends TextDecoderStream {
        get writable() {
          calls++;
          if (throws) throw Error("author writable getter");
          return new WritableStream();
        }
      }
      const result = await decode("utf-8", {}, [[97, 98, 99]], Decoder);
      return calls === 0 && result.join("") === "abc";
    });
  }
  await check("public prototype writable getter", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, "writable");
    let calls = 0;
    Object.defineProperty(TextDecoderStream.prototype, "writable", { configurable: true, get() {
      calls++;
      throw Error("replaced writable getter");
    } });
    try {
      return (await decode("utf-8", {}, [[65]])).join("") === "A" && calls === 0;
    } finally { Object.defineProperty(TextDecoderStream.prototype, "writable", descriptor); }
  });
  await check("stream state ignores the public TextDecoder constructor", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, "TextDecoder");
    let calls = 0;
    Object.defineProperty(globalThis, "TextDecoder", { configurable: true, get() {
      calls++;
      throw Error("public TextDecoder getter");
    } });
    try {
      return (await decode("utf-8", {}, [[65]])).join("") === "A" && calls === 0;
    } finally { Object.defineProperty(globalThis, "TextDecoder", descriptor); }
  });
  return { checks };
};
