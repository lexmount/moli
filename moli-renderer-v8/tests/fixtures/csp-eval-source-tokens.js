(() => {
  const attempt = callback => {
    try {
      callback();
      return 'allowed';
    } catch (error) {
      return error.name;
    }
  };
  return [
    attempt(() => eval('21 * 2')),
    attempt(() => new Function('return 42')()),
    attempt(() => new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))),
  ];
})()
