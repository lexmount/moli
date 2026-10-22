(realm) => {
  const rows = [];
  const check = (name, callback) => {
    try { rows.push({name, passed: callback() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  for (const name of ['count', 'countReset', 'time', 'timeLog', 'timeEnd']) {
    const method = realm.console[name];
    check(`${name}: string conversion exactly once with arbitrary receiver`, () => {
      let conversions = 0;
      const result = method.call({}, {toString() { conversions++; return 'coerced-label'; }});
      return result === undefined && conversions === 1;
    });
    check(`${name}: string primitive hint`, () => {
      const hints = [];
      method({[Symbol.toPrimitive](hint) { hints.push(hint); return 'hint-label'; }});
      return JSON.stringify(hints) === '["string"]';
    });
    check(`${name}: original thrown value`, () => {
      const marker = {};
      try { method({toString() { throw marker; }}); }
      catch (error) { return error === marker; }
      return false;
    });
    check(`${name}: primitive Symbol rejects in callee realm`, () => {
      try { method(realm.Symbol('label')); }
      catch (error) { return error instanceof realm.TypeError; }
      return false;
    });
    check(`${name}: nonprimitive conversion rejects in callee realm`, () => {
      try { method({toString() { return {}; }, valueOf() { return {}; }}); }
      catch (error) { return error instanceof realm.TypeError; }
      return false;
    });
    check(`${name}: revoked Proxy rejects in callee realm`, () => {
      const {proxy, revoke} = realm.Proxy.revocable({}, {});
      revoke();
      try { method(proxy); }
      catch (error) { return error instanceof realm.TypeError; }
      return false;
    });
    for (const value of [undefined, null, false, 0, 1n, '\ud800', '\ud801', '\ufffd']) {
      check(`${name}: primitive ${String(value)}`, () => method(value) === undefined);
    }
  }
  return rows;
}
