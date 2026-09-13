// NavigatorLanguage.languages is a FrozenArray<DOMString> in its own realm.
(() => {
  const languages = navigator.languages;
  const check = (value, message) => { if (!value) throw new Error(message); };
  check(Array.isArray(languages), 'languages must be an Array');
  check(Object.getPrototypeOf(languages) === Array.prototype, 'languages array realm');
  check(languages.every(value => typeof value === 'string'), 'language values');
  check(Object.isFrozen(languages), 'languages must be frozen');
  check(!Object.isExtensible(languages), 'languages must not be extensible');
  const length = Object.getOwnPropertyDescriptor(languages, 'length');
  check(!length.writable && !length.configurable && !length.enumerable, 'length descriptor');
  for (let index = 0; index < languages.length; index++) {
    const item = Object.getOwnPropertyDescriptor(languages, index);
    check(!item.writable && !item.configurable && item.enumerable, 'item descriptor');
  }
  const before = JSON.stringify(languages);
  check(!Reflect.set(languages, '0', 'changed'), 'Reflect.set must fail');
  check(!Reflect.defineProperty(languages, 'extra', {value: true}), 'extra property must fail');
  if (languages.length) check(!Reflect.deleteProperty(languages, '0'), 'delete must fail');
  for (const mutate of [
    () => { 'use strict'; languages[0] = 'changed'; },
    () => languages.push('changed'),
    () => { 'use strict'; languages.length = 0; }
  ]) {
    let threw = false;
    try { mutate(); } catch (error) { threw = error instanceof TypeError; }
    check(threw, 'strict mutation must throw TypeError');
  }
  check(JSON.stringify(languages) === before, 'cached language values must not mutate');
  check(JSON.stringify(navigator.languages) === before, 'later reads must remain unchanged');
  return 'ok';
})()
