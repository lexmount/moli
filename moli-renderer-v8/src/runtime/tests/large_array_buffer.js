function largeArrayBufferProbe() {
  const length = 2 ** 31 + 17;
  const buffer = new ArrayBuffer(length);
  const bytes = new Uint8Array(buffer);
  const offsets = [0, 4095, 2 ** 30, length - 1];
  const zeroed = offsets.every(offset => bytes[offset] === 0);
  bytes[0] = 63;
  const view = new DataView(buffer);
  view.setUint32(length - 4, 0x12345678);
  const transferred = buffer.transfer();
  return {
    length: transferred.byteLength,
    zeroed,
    first: new Uint8Array(transferred)[0],
    lastWord: new DataView(transferred).getUint32(length - 4),
    sourceDetached: buffer.byteLength === 0,
  };
}
