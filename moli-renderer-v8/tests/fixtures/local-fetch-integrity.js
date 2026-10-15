async function localFetchIntegrityProbe() {
  const text = '0123456789';
  const integrity = 'sha256-hNiYd/DUBB77a/kaFvAkjy/Vc+avBcGflr7bn4gveII=';
  const wrong = 'sha256-OAg8fukSHhdAGINWahSKpcLi1V3FO8SpSgJlF9v/PGs=';
  const empty = 'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=';
  let checks = 0;
  async function check(label, input, init, expected) {
    let actual;
    try {
      const response = await fetch(input, init);
      actual = [response.status, await response.text()];
    } catch (error) {
      if (!(error instanceof TypeError)) throw error;
      actual = 'TypeError';
    }
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(label + ': ' + JSON.stringify({actual, expected}));
    }
    checks++;
  }
  const blob = URL.createObjectURL(new Blob([text]));
  const emptyBlob = URL.createObjectURL(new Blob([]));
  try {
    await check('blob/no-metadata', blob, {}, [200, text]);
    await check('blob/matching', blob, {integrity}, [200, text]);
    await check('blob/mismatch', blob, {integrity: wrong}, 'TypeError');
    await check('blob/unsupported', blob, {integrity: 'unknown-hash'}, [200, text]);
    await check('blob/non-ascii-leading', blob, {integrity: '\u00a0' + wrong}, [200, text]);
    await check('blob/non-ascii-trailing', blob, {integrity: wrong + '\u00a0'}, [200, text]);
    await check('blob/no-cors-matching', blob, {mode: 'no-cors', integrity}, [200, text]);
    await check('data/matching', 'data:,' + text, {integrity}, [200, text]);
    await check('data/mismatch', 'data:,' + text, {integrity: wrong}, 'TypeError');
    await check('data/null-body', 'data:,' + text, {method: 'HEAD', integrity}, 'TypeError');
    await check('data/null-body-unsupported', 'data:,', {method: 'HEAD', integrity: 'unknown-hash'}, 'TypeError');
    await check('blob/empty-body', emptyBlob, {integrity: empty}, [200, '']);
    await check('data/empty-body-unsupported', 'data:,', {integrity: 'unknown-hash'}, [200, '']);
    const captured = new Request(blob, {integrity});
    const capturedWrong = new Request(blob, {integrity: wrong});
    URL.revokeObjectURL(blob);
    await check('blob/captured-matching', captured, {}, [200, text]);
    await check('blob/captured-mismatch', capturedWrong, {}, 'TypeError');
    await check('blob/revoked-url', blob, {integrity}, 'TypeError');
  } finally {
    URL.revokeObjectURL(blob);
    URL.revokeObjectURL(emptyBlob);
  }
  return {state: 'pass', checks};
}
