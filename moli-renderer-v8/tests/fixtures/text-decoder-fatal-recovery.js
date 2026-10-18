globalThis.runFatalDecoderRecoveryContract = async function () {
  const checks = [];
  function check(name, operation) {
    try { checks.push({ name, passed: operation() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  }
  function fails(decoder, bytes, stream) {
    try { decoder.decode(Uint8Array.from(bytes), { stream }); return false; }
    catch (error) { return error instanceof TypeError; }
  }
  for (const [encoding, bom, invalid, unit] of [
    ["utf-8", [0xef, 0xbb, 0xbf], [0xff], value => [value]],
    ["utf-16le", [0xff, 0xfe], [0, 0xdc], value => [value, 0]],
    ["utf-16be", [0xfe, 0xff], [0xdc, 0], value => [0, value]],
  ]) {
    for (const ignoreBOM of [false, true]) {
      check(`${encoding} first fatal call, ignoreBOM=${ignoreBOM}`, () => {
        const decoder = new TextDecoder(encoding, { fatal: true, ignoreBOM });
        if (!fails(decoder, [...bom, ...invalid], true)) return false;
        const next = decoder.decode(Uint8Array.from([...bom, ...unit(65)]));
        return next === (ignoreBOM ? "\ufeffA" : "A");
      });
      for (const prefix of ["empty", "ASCII", "BOM"]) {
        check(`${encoding} successful ${prefix} before error, ignoreBOM=${ignoreBOM}`, () => {
          const decoder = new TextDecoder(encoding, { fatal: true, ignoreBOM });
          const bytes = prefix === "BOM" ? bom : prefix === "ASCII" ? unit(65) : [];
          const expected = prefix === "ASCII" ? "A" : prefix === "BOM" && ignoreBOM ? "\ufeff" : "";
          if (decoder.decode(Uint8Array.from(bytes), { stream: true }) !== expected) return false;
          if (!fails(decoder, invalid, true)) return false;
          const next = decoder.decode(Uint8Array.from([...bom, ...unit(66)]));
          const expectedNext = ignoreBOM || prefix !== "empty" ? "\ufeffB" : "B";
          const fresh = decoder.decode(Uint8Array.from([...bom, ...unit(67)]));
          return next === expectedNext && fresh === (ignoreBOM ? "\ufeffC" : "C");
        });
      }
    }
  }
  for (const [name, escape, byte, expected] of [
    ["Roman", 0x4a, 0x7e, "\u203e"],
    ["Katakana", 0x49, 0x42, "\uff82"],
  ]) {
    const switchBytes = [0x1b, 0x28, escape];
    check(`${name} recovery after streaming error`, () => {
      const decoder = new TextDecoder("iso-2022-jp", { fatal: true });
      return decoder.decode(Uint8Array.of(byte)) === String.fromCharCode(byte) &&
        fails(decoder, [...switchBytes, 0xff], true) &&
        decoder.decode(Uint8Array.of(byte)) === expected;
    });
    check(`${name} reset after successful non-stream decode`, () => {
      const decoder = new TextDecoder("iso-2022-jp", { fatal: true });
      return decoder.decode(Uint8Array.from([...switchBytes, byte])) === expected &&
        decoder.decode(Uint8Array.of(byte)) === String.fromCharCode(byte);
    });
    check(`${name} reset after non-stream error`, () => {
      const decoder = new TextDecoder("iso-2022-jp", { fatal: true });
      return fails(decoder, [...switchBytes, 0xff], false) &&
        decoder.decode(Uint8Array.of(byte)) === String.fromCharCode(byte);
    });
  }
  return { checks };
};
