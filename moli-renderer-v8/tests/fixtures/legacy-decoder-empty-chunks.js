globalThis.runLegacyDecoderEmptyChunkContract = async function () {
  const checks = [];
  async function check(name, operation) {
    globalThis.__legacyDecoderEmptyChunkProgress = name;
    try { checks.push({ name, passed: await operation() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  }
  const vectors = [
    ["big5", [0xfe, 0x40], "\u9442"],
    ["shift_jis", [0x81, 0x87], "\u221e"],
    ["euc-kr", [0x81, 0x41], "\uac02"],
  ];
  const emptyChunks = count => Array.from({ length: count }, () => new Uint8Array());
  function chunks(bytes, emptyCount) {
    return [new Uint8Array(), Uint8Array.of(bytes[0]), ...emptyChunks(emptyCount),
      Uint8Array.of(bytes[1]), new Uint8Array()];
  }
  async function streamResult(encoding, fatal, input) {
    const source = new ReadableStream({ start(controller) {
      for (const chunk of input) controller.enqueue(chunk);
      controller.close();
    } });
    const reader = source.pipeThrough(new TextDecoderStream(encoding, { fatal })).getReader();
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
  for (const [encoding, bytes, expected] of vectors) {
    for (const fatal of [false, true]) {
      await check(`${encoding}, fatal=${fatal}: whole input`, () =>
        new TextDecoder(encoding, { fatal }).decode(Uint8Array.from(bytes)) === expected);
      for (const count of [0, 1, 4]) {
        await check(`${encoding}, fatal=${fatal}: decode across ${count} empty chunks`, () => {
          const decoder = new TextDecoder(encoding, { fatal });
          let output = "";
          for (const chunk of chunks(bytes, count)) output += decoder.decode(chunk, { stream: true });
          output += decoder.decode();
          return output === expected;
        });
        await check(`${encoding}, fatal=${fatal}: stream across ${count} empty chunks`, async () => {
          const result = await streamResult(encoding, fatal, chunks(bytes, count));
          return result.error === undefined && result.chunks.every(chunk => chunk.length > 0) &&
            result.chunks.join("") === expected;
        });
      }
      await check(`${encoding}, fatal=${fatal}: pending byte at decode EOF`, () => {
        const decoder = new TextDecoder(encoding, { fatal });
        if (decoder.decode(Uint8Array.of(bytes[0]), { stream: true }) !== "") return false;
        for (const chunk of emptyChunks(4)) {
          if (decoder.decode(chunk, { stream: true }) !== "") return false;
        }
        let replaced = false, failed = false;
        try { replaced = decoder.decode() === "\ufffd"; }
        catch (error) { failed = error instanceof TypeError; }
        return (fatal ? failed : replaced) && decoder.decode(Uint8Array.of(65)) === "A";
      });
      await check(`${encoding}, fatal=${fatal}: pending byte at stream EOF`, async () => {
        const result = await streamResult(encoding, fatal,
          [Uint8Array.of(bytes[0]), ...emptyChunks(4)]);
        return fatal ? result.error instanceof TypeError && result.chunks.length === 0 :
          result.error === undefined && JSON.stringify(result.chunks) === JSON.stringify(["\ufffd"]);
      });
    }
  }
  return { checks };
};
