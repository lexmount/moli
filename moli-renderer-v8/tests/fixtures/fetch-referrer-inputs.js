async function fetchReferrerInputs(base) {
  const cases = [
    ['url', {referrer: base.replace('://', '://user:password@') + '/selected?q=1#secret'}],
    ['request', {referrer: base + '/request#secret'}],
    ['clone', {referrer: base + '/clone#secret'}],
    ['override', {referrer: base + '/override#secret'}],
    ['override', {referrer: ''}],
    ['request', {referrer: ''}],
    ['url', {referrer: 'about:client'}],
    ['url', {}],
    ['url', {referrer: './relative?q=1#secret'}],
    ['url', {referrer: base + '/' + 'x'.repeat(4100)}],
    ['request', {referrer: base + '/origin', referrerPolicy: 'origin'}],
    ['request', {referrer: base + '/omitted', referrerPolicy: 'no-referrer'}],
    ['override', {referrer: 'about:client'}],
    ['url', {referrer: 'https://unrelated.invalid/private'}]
  ];
  for (const [index, [form, init]] of cases.entries()) {
    let input = base + '/echo?case=' + index, options = init;
    let original;
    if (form !== 'url') {
      input = new Request(input, form === 'override' ? {referrer: base + '/old'} : init);
      if (form === 'clone') input = input.clone();
      if (form !== 'override') options = undefined;
      original = input.referrer;
    }
    const response = await fetch(input, options);
    if (response.status !== 200 || await response.text() !== 'ok') throw new Error('response ' + index);
    if (original !== undefined && input.referrer !== original) throw new Error('mutated input ' + index);
  }
  const sentinel = {};
  try {
    await fetch(base + '/must-not-fetch', {get referrer() {throw sentinel;}});
    throw new Error('missing conversion rejection');
  } catch (error) { if (error !== sentinel) throw error; }
  return 'pass';
}
