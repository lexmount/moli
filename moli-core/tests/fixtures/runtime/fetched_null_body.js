async function runFetchedNullBodyProbe(url) {
  const errors = [];
  const check = (value, label) => { if (!value) errors.push(label); };
  const cases = [['head', 200]];
  for (const method of ['GET', 'POST', 'OPTIONS']) {
    for (const status of [204, 205, 304]) cases.push([method, status]);
  }
  for (const [method, status] of cases) {
    const label = method + '/' + status;
    const controller = new AbortController();
    const response = await fetch(url + '?code=' + status, {method, signal: controller.signal});
    check(response.status === status, label + ': status');
    check(response.body === null, label + ': null body');
    if (response.body !== null) continue;
    const clone = response.clone();
    check(!response.bodyUsed && clone.body === null, label + ': unused clone');
    check(await response.text() === '', label + ': empty text before abort');
    controller.abort({reason: 'after response headers'});
    for (const body of [response, clone]) {
      for (let repeat = 0; repeat < 2; ++repeat) {
        check(await body.text() === '', label + ': repeated text after abort');
        check((await body.arrayBuffer()).byteLength === 0, label + ': empty arrayBuffer');
        check((await body.bytes()).length === 0, label + ': empty bytes');
        const blob = await body.blob();
        check(blob.size === 0 && blob.type === 'application/x-www-form-urlencoded', label + ': empty blob');
        check(Array.from((await body.formData()).entries()).length === 0, label + ': empty formData');
        try { await body.json(); errors.push(label + ': empty JSON accepted'); }
        catch (error) { check(error instanceof SyntaxError, label + ': JSON error'); }
        check(!body.bodyUsed, label + ': null body remains unused');
        check(body.clone().body === null, label + ': clone after consumption');
      }
    }
  }
  for (const empty of [false, true]) {
    const response = await fetch(url + '?code=200&empty=' + Number(empty));
    check(response.body instanceof ReadableStream, 'GET/200: non-null stream, empty=' + empty);
    const clone = response.clone();
    const expected = empty ? '' : 'name=value';
    check(await response.text() === expected && await clone.text() === expected, 'GET/200: body bytes');
    check(response.bodyUsed && clone.bodyUsed, 'GET/200: consumed streams become used');
  }
  return {errors};
}
