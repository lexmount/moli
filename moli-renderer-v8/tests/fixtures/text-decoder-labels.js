globalThis.__textDecoderLabelResults = (() => {
  const checks = [];
  function check(name, callback) {
    try {
      checks.push({ name, passed: callback() === true });
    } catch (error) {
      checks.push({ name, passed: false, error: String(error) });
    }
  }
  const aliases = [
    ["UTF-8", "utf-8"],
    ["LaTiN1", "windows-1252"],
    ["UNICODEFFFE", "utf-16be"],
  ];
  const asciiWhitespace = [0x09, 0x0a, 0x0c, 0x0d, 0x20];
  const invalidWhitespace = [
    0x00, 0x0b, 0x85, 0xa0, 0x1680,
    0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005,
    0x2006, 0x2007, 0x2008, 0x2009, 0x200a,
    0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff,
  ];
  const positions = [
    ["leading", (space, label) => space + label],
    ["trailing", (space, label) => label + space],
    ["both", (space, label) => space + label + space],
  ];
  function rejectsLabel(Decoder, label) {
    try {
      new Decoder(label);
      return false;
    } catch (error) {
      return error instanceof RangeError;
    }
  }
  for (const Decoder of [TextDecoder, TextDecoderStream]) {
    for (const [label, encoding] of aliases) {
      check(`${Decoder.name}: ASCII case ${label}`,
        () => new Decoder(label).encoding === encoding);
      for (const codePoint of asciiWhitespace) {
        for (const [position, wrap] of positions) {
          check(`${Decoder.name}: ASCII whitespace ${codePoint} ${position} ${label}`,
            () => new Decoder(wrap(String.fromCodePoint(codePoint), label)).encoding === encoding);
        }
      }
      for (const codePoint of invalidWhitespace) {
        for (const [position, wrap] of positions) {
          check(`${Decoder.name}: invalid whitespace ${codePoint} ${position} ${label}`,
            () => rejectsLabel(Decoder, wrap(String.fromCodePoint(codePoint), label)));
        }
      }
    }
    for (const label of [
      "replacement", "iso-2022-cn", "iso-2022-cn-ext",
      "iso-2022-kr", "hz-gb-2312", "csiso2022kr",
    ]) {
      check(`${Decoder.name}: replacement label ${label}`, () => rejectsLabel(Decoder, label));
    }
    check(`${Decoder.name}: default label`, () => new Decoder().encoding === "utf-8");
    check(`${Decoder.name}: converts the label once`, () => {
      let conversions = 0;
      const decoder = new Decoder({ toString() { conversions++; return "\tUTF-8\n"; } });
      return conversions === 1 && decoder.encoding === "utf-8";
    });
    check(`${Decoder.name}: label conversion exception`, () => {
      const sentinel = {};
      try {
        new Decoder({ toString() { throw sentinel; } });
        return false;
      } catch (error) {
        return error === sentinel;
      }
    });
    check(`${Decoder.name}: options convert before label validation`, () => {
      const log = [];
      const sentinel = {};
      try {
        new Decoder({ toString() { log.push("label"); return "\u00a0UTF-8"; } }, {
          get fatal() { log.push("fatal"); throw sentinel; },
        });
        return false;
      } catch (error) {
        return error === sentinel && log.join(",") === "label,fatal";
      }
    });
  }
  return { checks };
})();
