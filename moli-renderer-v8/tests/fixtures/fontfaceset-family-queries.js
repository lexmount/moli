async function fontFaceSetFamilyQueriesProbe(realm, set, fontBytes) {
  const rows = [], failures = [];
  const check = (name, actual, expected) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  const bytes = new realm.Uint8Array(fontBytes);
  for (const name of ['serif', 'sans-serif', 'monospace', 'cursive', 'fantasy', 'system-ui', 'math', 'SANS-SERIF']) {
    const face = new realm.FontFace(name, 'local("MoliAbsentFontQuery_4912")');
    face.loaded.catch(() => {});
    set.add(face);
    check(name + ':family', face.family, name === name.toLowerCase() ? JSON.stringify(name) : name);
    check(name + ':generic-check', set.check('16px ' + name), true);
    const generic = await set.load('16px ' + name).then(values => values.length, error => error.name);
    check(name + ':generic-load', [generic, face.status], [0, 'unloaded']);
    const escaped = '\\' + name.charCodeAt(0).toString(16) + ' ' + name.slice(1);
    check(name + ':escaped-generic', [await set.load('italic 16px ' + escaped).then(values => values.length, error => error.name), face.status], [0, 'unloaded']);
    const literal = await set.load('16px ' + JSON.stringify(name)).then(values => values.length, error => error.name);
    check(name + ':literal-load', [literal, face.status], ['NetworkError', 'error']);
    set.delete(face);
    const loaded = new realm.FontFace(name, bytes);
    set.add(loaded);
    const selected = await set.load('16px ' + JSON.stringify(name) + ', ' + name);
    check(name + ':mixed-list', [selected.length, selected[0] === loaded], [1, true]);
    const literalEscape = await set.load('16px "' + escaped + '"');
    check(name + ':escaped-literal', [literalEscape.length, literalEscape[0] === loaded], [1, true]);
    set.delete(loaded);
  }
  for (const name of ['ui-serif', 'ui-sans-serif', 'ui-monospace', 'ui-rounded', 'fangsong', 'Literal Family']) {
    const loaded = new realm.FontFace(name, bytes);
    set.add(loaded);
    for (const query of [name, JSON.stringify(name)]) {
      const selected = await set.load('16px ' + query);
      check(name + ':' + query, [selected.length, selected[0] === loaded], [1, true]);
    }
    set.delete(loaded);
  }
  const first = new realm.FontFace('FirstLiteral', bytes);
  const second = new realm.FontFace('SecondLiteral', bytes);
  set.add(first); set.add(second);
  const ordered = await set.load('italic bold 16px/1.2 SecondLiteral, sans-serif, FirstLiteral');
  check('literal-order-with-generic', [ordered.length, ordered[0] === second, ordered[1] === first], [2, true, true]);
  set.delete(first); set.delete(second);
  return {rows, failures};
}
