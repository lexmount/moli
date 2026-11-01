async function fetchHeaderNameInputs(base) {
  function check(condition, message) {
    if (!condition) throw new Error(message);
  }
  async function send(label, input, init, name, value) {
    const response = await fetch(input, init);
    const raw = await response.text();
    const fields = raw.split('\r\n').filter(line => {
      const colon = line.indexOf(':');
      return colon > 0 && line.slice(0, colon).toLowerCase() === 'x-first';
    });
    check(response.status === 200 && fields.length === 1 &&
          fields[0] === `${name}: ${value}`, `${label}: ${JSON.stringify(fields)}`);
  }
  function publicView(headers, value) {
    check(headers.get('X-fIRst') === value && headers.has('x-FIRST'), 'case-insensitive lookup');
    check(JSON.stringify([...headers.entries()]) === JSON.stringify([['x-first', value]]),
          'public iteration must have lowercase names');
    headers.forEach((entry, name) => {
      check(name === 'x-first' && entry === value, 'forEach projection');
    });
  }

  const url = `${base}/echo`;
  const pairs = [['X-First', 'one'], ['x-FIRST', 'two']];
  await send('array', url, {headers: pairs}, 'X-First', 'one, two');
  await send('reverse array', url, {headers: [['x-FIRST', 'one'], ['X-First', 'two']]},
             'x-FIRST', 'one, two');
  const request = new Request(url, {headers: pairs});
  publicView(request.headers, 'one, two');
  await send('Request', request, undefined, 'X-First', 'one, two');
  await send('clone', request.clone(), undefined, 'X-First', 'one, two');
  await send('copy', new Request(request), undefined, 'X-First', 'one, two');

  request.headers.append('x-fIRsT', 'three');
  publicView(request.headers, 'one, two, three');
  await send('append', request, undefined, 'X-First', 'one, two, three');
  request.headers.set('x-FIRST', 'set');
  publicView(request.headers, 'set');
  await send('set', request, undefined, 'X-First', 'set');
  request.headers.delete('X-firST');
  check(!request.headers.has('x-first') && [...request.headers].length === 0, 'delete all duplicates');
  request.headers.append('x-FiRsT', 'after');
  publicView(request.headers, 'after');
  await send('delete and append', request, undefined, 'x-FiRsT', 'after');

  await send('Headers initializer', url, {headers: new Headers(pairs)}, 'x-first', 'one, two');
  await send('explicit Request headers', new Request(request, {headers: request.headers}),
             undefined, 'x-first', 'after');
  let iteratorCalls = 0;
  const iterator = request.headers[Symbol.iterator];
  request.headers[Symbol.iterator] = function* () {
    iteratorCalls++;
    yield* iterator.call(this);
  };
  await send('internal Request copy', new Request(request), undefined, 'x-FiRsT', 'after');
  check(iteratorCalls === 0, 'internal copy must not invoke the public iterator');
  await send('explicit iterable', url, {headers: request.headers}, 'x-first', 'after');
  check(iteratorCalls === 1, 'Headers initializer must invoke the public iterator');

  const cookies = new Headers([['SET-Cookie', 'a=1'], ['set-cookie', 'b=2']]);
  check(JSON.stringify(cookies.getSetCookie()) === '["a=1","b=2"]', 'getSetCookie retains values');
  check(JSON.stringify([...cookies]) === '[["set-cookie","a=1"],["set-cookie","b=2"]]',
        'Set-Cookie public projection');
  cookies.set('sEt-CooKie', 'c=3');
  check(JSON.stringify(cookies.getSetCookie()) === '["c=3"]', 'set removes later cookies');
  cookies.delete('Set-Cookie');
  check(cookies.getSetCookie().length === 0, 'delete mixed-case cookies');
  return 'pass';
}
