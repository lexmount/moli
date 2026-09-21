async function noCorsFillProbe(base) {
  const checks = [];
  const check = (label, actual, wanted) => checks.push({label, actual, wanted, pass: actual === wanted});
  const cases = [];
  for (const name of ['Accept', 'Accept-Language', 'Content-Language']) {
    for (const [label, values, wanted, mergedAllowed] of [
      ['overflow', ['a'.repeat(126), 'b'], 'a'.repeat(126), false],
      ['at-limit', ['a'.repeat(125), 'b'], 'a'.repeat(125) + ', b', true],
      ['after-limit', ['a'.repeat(125), 'b', 'c'], 'a'.repeat(125) + ', b', false],
      ['empty-first', ['', 'b'.repeat(127)], '', false],
    ]) cases.push([name + '/' + label, name, values, wanted, mergedAllowed]);
  }
  const longType = 'text/plain;x=' + 'a'.repeat(114);
  cases.push(
    ['type/duplicate', 'Content-Type', ['text/plain', 'text/plain'], 'text/plain', false],
    ['type/unsafe', 'Content-Type', ['text/plain;charset=utf8', '"'], 'text/plain;charset=utf8', false],
    ['type/invalid-first', 'Content-Type', ['application/json', 'text/plain'], 'text/plain', false],
    ['type/overflow', 'Content-Type', [longType, 'ok'], longType, false],
  );
  for (const [label, name, values, wanted, mergedAllowed] of cases) {
    const names = [name, name.toLowerCase(), name.toUpperCase()];
    const pairs = values.map((value, index) => [names[index], value]);
    const original = JSON.stringify(pairs);
    const url = base + '/echo?case=' + encodeURIComponent(label);
    const request = new Request(url, {mode: 'no-cors', headers: pairs});
    const joined = values.join(', ');
    const mergedWanted = mergedAllowed ? joined : null;
    check(label + '/sequence', request.headers.get(name), wanted);
    check(label + '/record', new Request(url, {mode: 'no-cors', headers: Object.fromEntries(pairs)}).headers.get(name), wanted);
    const appended = new Request(url, {mode: 'no-cors'});
    for (const [key, value] of pairs) appended.headers.append(key, value);
    check(label + '/append', appended.headers.get(name), wanted);
    check(label + '/clone', request.clone().headers.get(name), wanted);
    const inherited = new Request(url, {mode: 'no-cors'});
    check(label + '/inherited-mode', new Request(inherited, {headers: pairs}).headers.get(name), wanted);
    const cors = new Request(url, {headers: pairs});
    check(label + '/override', new Request(cors, {mode: 'no-cors', headers: pairs}).headers.get(name), wanted);
    Object.defineProperty(cors.headers, Symbol.iterator, {
      get() { throw new Error('Inherited headers must not use the JS iterator'); }
    });
    // Without init.headers, Request copies the underlying header list and
    // appends each entry under the new guard (Fetch Request constructor).
    check(label + '/inherited-headers', new Request(cors, {mode: 'no-cors'}).headers.get(name), wanted);
    check(label + '/inherited-clone', new Request(cors.clone(), {mode: 'no-cors'}).headers.get(name), wanted);
    const appendedCors = new Request(url);
    for (const [key, value] of pairs) appendedCors.headers.append(key, value);
    check(label + '/inherited-appended', new Request(appendedCors, {mode: 'no-cors'}).headers.get(name), wanted);
    // Explicit init.headers is a HeadersInit union (sequence or record).
    // WebIDL consumes Headers' JS iterator, whose values are combined already.
    check(label + '/headers-init', new Request(url, {mode: 'no-cors', headers: new Headers(pairs)}).headers.get(name), mergedWanted);
    check(label + '/cors', cors.headers.get(name), joined);
    check(label + '/response', new Response(null, {headers: pairs}).headers.get(name), joined);
    check(label + '/input-unchanged', JSON.stringify(pairs), original);
    for (const [kind, input, init] of [
      ['direct', url, {mode: 'no-cors', headers: pairs}],
      ['captured', request, undefined],
      ['override', cors, {mode: 'no-cors', headers: pairs}],
      ['inherited', cors, {mode: 'no-cors'}],
    ]) {
      let actual;
      try {
        const response = await fetch(input, init);
        actual = new Headers((await response.json()).headers).get(name);
      } catch (error) { actual = String(error); }
      check(label + '/fetch-' + kind, actual, wanted);
    }
    const reset = name === 'Content-Type' ? 'text/plain' : 'reset';
    request.headers.set(name, reset);
    check(label + '/set', request.headers.get(name), reset);
    request.headers.set(name, 'a'.repeat(129));
    check(label + '/set-rejected', request.headers.get(name), reset);
    request.headers.delete(name);
    check(label + '/delete', request.headers.get(name), null);
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
