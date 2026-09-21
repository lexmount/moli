async function headersForEachProbe() {
  const checks = [];
  const check = (label, actual, wanted) => {
    checks.push({label, actual, wanted, pass: JSON.stringify(actual) === JSON.stringify(wanted)});
  };
  const initial = [['a', '1'], ['b', '2'], ['c', '3']];
  const cases = [
    ['delete-next', initial, (headers, key) => {
      if (key === 'a') headers.delete('b');
    }, [['a', '1'], ['c', '3']]],
    ['delete-current', initial, (headers, key) => {
      if (key === 'a') headers.delete('a');
    }, [['a', '1'], ['c', '3']]],
    ['delete-previous', [...initial, ['d', '4']], (headers, key) => {
      if (key === 'b') headers.delete('a');
    }, [['a', '1'], ['b', '2'], ['d', '4']]],
    ['clear', initial, headers => {
      for (const key of ['a', 'b', 'c']) headers.delete(key);
    }, [['a', '1']]],
    ['update-next', initial, (headers, key) => {
      if (key === 'a') headers.set('b', '20');
    }, [['a', '1'], ['b', '20'], ['c', '3']]],
    ['append-next', initial, (headers, key) => {
      if (key === 'a') headers.append('d', '4');
    }, [...initial, ['d', '4']]],
    ['append-existing', initial, (headers, key) => {
      if (key === 'a') headers.append('b', 'tail');
    }, [['a', '1'], ['b', '2, tail'], ['c', '3']]],
    ['append-empty-duplicate', [['b', ''], ['a', '1'], ['B', '2']], (headers, key) => {
      if (key === 'a') headers.append('b', '');
    }, [['a', '1'], ['b', ', 2, ']]],
    ['insert-before', [['b', '2'], ['c', '3']], (headers, key) => {
      if (key === 'b' && !headers.has('a')) headers.append('a', '1');
    }, [['b', '2'], ['b', '2'], ['c', '3']]],
    ['reinsert-next', initial, (headers, key) => {
      if (key === 'a') { headers.delete('b'); headers.append('b', 'new'); }
    }, [['a', '1'], ['b', 'new'], ['c', '3']]],
    ['replace-list', initial, (headers, key) => {
      if (key === 'a') {
        for (const name of ['a', 'b', 'c']) headers.delete(name);
        headers.append('d', '4');
        headers.append('e', '5');
      }
    }, [['a', '1'], ['e', '5']]],
    ['append-cookie', [['Set-Cookie', 'a=1'], ['set-cookie', 'b=2'], ['x-tail', 'end']], (headers, key, value) => {
      if (key === 'set-cookie' && value === 'a=1') headers.append('Set-Cookie', 'c=3');
    }, [['set-cookie', 'a=1'], ['set-cookie', 'b=2'], ['set-cookie', 'c=3'], ['x-tail', 'end']]],
    ['delete-cookies', [['Set-Cookie', 'a=1'], ['set-cookie', 'b=2'], ['x-tail', 'end']], (headers, key) => {
      if (key === 'set-cookie') headers.delete(key);
    }, [['set-cookie', 'a=1']]],
  ];
  for (const [label, pairs, mutate, wanted] of cases) {
    const headers = new Headers(pairs);
    const receiver = {};
    const seen = [];
    let argumentsMatch = true;
    const result = headers.forEach(function(value, key, owner) {
      'use strict';
      argumentsMatch &&= this === receiver && owner === headers && arguments.length === 3;
      seen.push([key, value]);
      if (seen.length > 20) throw new Error(label + ' did not finish');
      mutate(owner, key, value);
    }, receiver);
    check(label, seen, wanted);
    check(label + '/callback-contract', [argumentsMatch, result === undefined], [true, true]);
  }

  // Every active cursor shares the refreshed view and keeps its own index.
  {
    const headers = new Headers(initial);
    const entries = headers.entries();
    const keys = headers.keys();
    const values = headers.values();
    const iterable = headers[Symbol.iterator]();
    check('shared-view/first', [entries.next().value, keys.next().value, values.next().value, iterable.next().value],
      [['a', '1'], 'a', '1', ['a', '1']]);
    const seen = [];
    headers.forEach((value, key) => {
      seen.push([key, value]);
      if (key === 'a') { headers.delete('b'); headers.append('d', '4'); }
      if (key === 'c') headers.set('d', '40');
    });
    check('shared-view/forEach', seen, [['a', '1'], ['c', '3'], ['d', '40']]);
    check('shared-view/entries', Array.from(entries), [['c', '3'], ['d', '40']]);
    check('shared-view/keys', Array.from(keys), ['c', 'd']);
    check('shared-view/values', Array.from(values), ['3', '40']);
    check('shared-view/iterable', Array.from(iterable), [['c', '3'], ['d', '40']]);
  }

  // Returned pairs must not expose mutable cache storage.
  {
    const headers = new Headers(initial);
    const first = headers.entries().next().value;
    first[0] = 'changed';
    first[1] = 'changed';
    first.length = 0;
    check('shared-view/fresh-pairs', Array.from(headers), initial);
    const seen = [];
    headers.forEach((value, key) => seen.push([key, value]));
    check('shared-view/unchanged-callback-values', seen, initial);
    check('shared-view/private', Reflect.ownKeys(headers), []);
  }

  // Reentrant calls have independent positions and observe the same live list.
  {
    const headers = new Headers(initial);
    const outer = [];
    const inner = [];
    headers.forEach((value, key) => {
      outer.push([key, value]);
      if (key === 'a') headers.forEach((innerValue, innerKey) => {
        inner.push([innerKey, innerValue]);
        if (innerKey === 'a') { headers.delete('b'); headers.append('d', '4'); }
      });
    });
    check('reentrant/inner', inner, [['a', '1'], ['c', '3'], ['d', '4']]);
    check('reentrant/outer', outer, [['a', '1'], ['c', '3'], ['d', '4']]);
  }

  // forEach reads internal pairs, independently of author-provided iterators.
  {
    const headers = new Headers([['a', '1'], ['b', '2']]);
    let getterCalls = 0;
    for (const key of ['entries', 'keys', 'values', Symbol.iterator]) {
      Object.defineProperty(headers, key, {get() { getterCalls++; throw new Error('public iterator'); }});
    }
    const seen = [];
    headers.forEach((value, key) => {
      seen.push([key, value]);
      if (key === 'a') { headers.delete('b'); headers.append('c', '3'); }
    });
    check('internal-pairs', seen, [['a', '1'], ['c', '3']]);
    check('public-iterators-not-read', getterCalls, 0);
  }

  // Calling an author Proxy and propagating abrupt completion must still work.
  {
    const headers = new Headers(initial);
    const marker = {};
    const receiver = {};
    const seen = [];
    let applyCalls = 0;
    let callGetterCalls = 0;
    let shapeMatches = true;
    const target = function(value, key, owner) {
      'use strict';
      shapeMatches &&= this === receiver && owner === headers && arguments.length === 3;
      seen.push([key, value]);
      if (key === 'a') headers.set('b', 'new');
      else { headers.append('d', '4'); throw marker; }
    };
    Object.defineProperty(target, 'call', {get() { callGetterCalls++; throw new Error('callback.call'); }});
    const callback = new Proxy(target, {apply(fn, receiver, args) {
      applyCalls++;
      return Reflect.apply(fn, receiver, args);
    }});
    let caught;
    try { headers.forEach(callback, receiver); } catch (error) { caught = error; }
    check('abrupt/seen', seen, [['a', '1'], ['b', 'new']]);
    check('abrupt/callback-contract', [caught === marker, shapeMatches, applyCalls, callGetterCalls], [true, true, 2, 0]);
    check('abrupt/mutation-retained', headers.get('d'), '4');
  }

  {
    const headers = new Headers(initial);
    const callback = Proxy.revocable(() => { headers.set('b', 'new'); callback.revoke(); }, {});
    let caught;
    try { headers.forEach(callback.proxy); } catch (error) { caught = error; }
    check('revoked-callback', [caught instanceof TypeError, headers.get('b')], [true, 'new']);
  }

  {
    const headers = new Headers();
    let called = false;
    const result = headers.forEach(() => { called = true; });
    check('empty-list', [called, result === undefined], [false, true]);
    let caught;
    try { headers.forEach({}); } catch (error) { caught = error; }
    check('empty-list-validates-callback', caught instanceof TypeError, true);
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
