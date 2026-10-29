(async () => {
  async function decodeStream(stream, chunks) {
    const writer = stream.writable.getWriter();
    const reader = stream.readable.getReader();
    const writing = (async () => {
      for (const chunk of chunks) await writer.write(new Uint8Array(chunk));
      await writer.close();
    })();
    let text = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      text += value;
    }
    await writing;
    return text;
  }

  const decoder = new TextDecoder();
  const split = decoder.decode(new Uint8Array([0xE2]), { stream: true })
    + decoder.decode(new Uint8Array([0x82]), { stream: true })
    + decoder.decode(new Uint8Array([0xAC]));
  const utf8 = await decodeStream(new TextDecoderStream(), [
    [0xEF], [0xBB, 0xBF, 0x68, 0xC3], [0xA9]
  ]);
  const legacy = await decodeStream(new TextDecoderStream("gbk"), [
    [0xCC], [0xAB, 0xC6, 0xBD, 0xD1, 0xF3]
  ]);
  const bom = await decodeStream(new TextDecoderStream("utf-8", { ignoreBOM: true }), [
    [0xEF], [0xBB, 0xBF, 0x41]
  ]);

  const fatalStream = new TextDecoderStream("utf-8", { fatal: true });
  const writer = fatalStream.writable.getWriter();
  const reader = fatalStream.readable.getReader();
  writer.closed.catch(() => {});
  reader.closed.catch(() => {});
  const fatal = await Promise.all([
    writer.write(new Uint8Array([0xFF])).then(() => "unexpected", error => error.name),
    reader.read().then(() => "unexpected", error => error.name)
  ]);
  return { split, utf8, legacy, bomKept: bom === "\uFEFFA", fatal };
})()
