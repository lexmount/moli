async function textModuleProbe() {
  const result = {checks: 0, failures: []};
  const check = (name, actual, expected = true) => {
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const text = url => import(url, {with: {type: 'text'}});
  const cases = [
    ['empty', 'data:,', ''],
    ['unicode', 'data:text/plain;charset=windows-1250,%C5%9B%C4%87%C4%85%C5%BC%C5%BA%F0%9F%98%80', 'śćążź😀'],
    ['utf8-bom', 'data:application/octet-stream,%EF%BB%BFhello', 'hello'],
    ['utf16-bom', 'data:text/plain;charset=utf-16,%FF%FEa%00', '\uFFFD\uFFFDa\0'],
    ['replacement', 'data:text/plain,%ED%A0%80%FF', '\uFFFD\uFFFD\uFFFD\uFFFD'],
    ['nul-and-lines', 'data:text/plain,a%00b%0D%0Ac%0Dd', 'a\0b\r\nc\rd'],
    ['wasm-mime', 'data:application/wasm;base64,AGFzbQEAAAA=', '\0asm\u0001\0\0\0'],
    ['json-mime', 'data:application/json,%7B%22answer%22%3A42%7D', '{"answer":42}'],
    ['source-is-inert', 'data:text/javascript,globalThis.textModuleExecuted%3Dtrue%3B', 'globalThis.textModuleExecuted=true;'],
  ];
  for (const [name, url, expected] of cases) {
    try {
      const [first, second] = await Promise.all([text(url), text(url)]);
      check(name + ':value', first.default, expected);
      check(name + ':identity', first === second && first === await text(url));
      check(name + ':namespace', Object.getPrototypeOf(first) === null && !Object.isExtensible(first));
      check(name + ':exports', Object.keys(first).join(','), 'default');
    } catch (error) { check(name + ':load', error.name, 'fulfilled'); }
  }
  check('inert', globalThis.textModuleExecuted === undefined);
  for (const firstType of ['json', 'text']) {
    const url = 'data:application/json,%7B%22answer%22%3A42%7D#' + firstType;
    try {
      const secondType = firstType === 'json' ? 'text' : 'json';
      const first = await import(url, {with: {type: firstType}});
      const second = await import(url, {with: {type: secondType}});
      const json = firstType === 'json' ? first : second;
      const string = firstType === 'text' ? first : second;
      check(firstType + ':json', json.default.answer, 42);
      check(firstType + ':text', string.default, '{"answer":42}');
      check(firstType + ':separate-records', json !== string);
    } catch (error) { check(firstType + ':distinct-types', error.name, 'fulfilled'); }
  }
  for (const textFirst of [false, true]) {
    const url = 'data:application/octet-stream,opaque#' + textFirst;
    const wrong = () => import(url).then(() => 'fulfilled', error => error.name);
    try {
      if (textFirst) check('text-before-wrong', (await text(url)).default, 'opaque');
      check('untyped-rejected:' + textFirst, await wrong(), 'TypeError');
      check('text-after-wrong:' + textFirst, (await text(url)).default, 'opaque');
    } catch (error) { check('untyped-isolation:' + textFirst, error.name, 'fulfilled'); }
  }
  try {
    const url = 'data:text/plain,leaf';
    const graph = 'data:text/javascript,' + encodeURIComponent(
      'export {default} from ' + JSON.stringify(url) + ' with {type:"text"};');
    check('static-reexport', (await import(graph)).default, 'leaf');
    const missing = 'data:text/javascript,' + encodeURIComponent(
      'import {missing} from ' + JSON.stringify(url) + ' with {type:"text"};');
    check('no-named-export', await import(missing).then(() => 'fulfilled', e => e.name), 'SyntaxError');
  } catch (error) { check('static-graph', error.name, 'fulfilled'); }
  for (const type of ['Text', '', 'javascript-or-wasm']) {
    check('invalid-type:' + type,
      await import('data:text/plain,blocked', {with: {type}}).then(() => 'fulfilled', e => e.name), 'TypeError');
  }
  return result;
}
