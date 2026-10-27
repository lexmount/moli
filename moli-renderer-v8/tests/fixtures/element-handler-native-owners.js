(async () => {
  const checks = [];
  function checkHandler(calleeName, realm, ownerName, media) {
    const name = `${calleeName}/${ownerName}/onclick/native-owner dispatch`;
    try {
      const descriptor = Object.getOwnPropertyDescriptor(realm.HTMLElement.prototype, 'onclick');
      let calls = 0, sameThis = false, sameTarget = false;
      const callback = function(event) {
        calls++;
        sameThis = this === media;
        sameTarget = event.currentTarget === media;
      };
      descriptor.set.call(media, callback);
      if (descriptor.get.call(media) !== callback || media.onclick !== callback)
        throw Error('handler identity');
      media.dispatchEvent(new Event('click'));
      if (calls !== 1 || !sameThis || !sameTarget)
        throw Error(JSON.stringify({calls, sameThis, sameTarget}));
      descriptor.set.call(media, null);
      if (descriptor.get.call(media) !== null || media.onclick !== null)
        throw Error('handler removal');
      checks.push({name, passed: true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error)});
    }
  }
  const popup = window.open('about:blank', '_blank');
  if (!popup) throw Error('native popup required');
  try {
    for (let attempt = 0; attempt < 100 && (!popup.HTMLMediaElement || !popup.document.body); attempt++)
      await new Promise(resolve => setTimeout(resolve, 10));
    const targets = [['root', document.createElement('audio')], ['popup', popup.document.createElement('audio')]];
    for (const [calleeName, realm] of [['root', window], ['popup', popup]])
      for (const [ownerName, media] of targets)
        checkHandler(calleeName, realm, ownerName, media);
    // Foreign Page access to windowless wrappers needs separate owner lookup
    // work. Keep this regression scoped to receiver resolution supported by main.
    const detached = document.implementation.createHTMLDocument('');
    checkHandler('root', window, 'windowless', detached.createElement('audio'));
    globalThis.__uiEventResults = {total: checks.length, checks,
      separateFunctionRealm: popup.Function !== Function,
      separateMediaInterface: popup.HTMLMediaElement !== HTMLMediaElement};
    return checks.every(row => row.passed);
  } finally { popup.close(); }
})()
