(async () => {
  const checks = [];
  const check = (name, operation) => {
    try { operation(); checks.push({name, passed:true}); }
    catch (error) { checks.push({name, passed:false, error:String(error)}); }
  };
  const popup = window.open('about:blank', '_blank');
  if (!popup) throw Error('native popup required');
  try {
    for (let attempt = 0; attempt < 100 && (!popup.HTMLMediaElement || !popup.document.body); attempt++) {
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    const targets = [['root', document.createElement('audio')], ['popup', popup.document.createElement('audio')]];
    for (const [calleeName, realm] of [['root', window], ['popup', popup]]) {
      for (const [ownerName, media] of targets) {
        for (const name of ['onencrypted', 'onwaitingforkey', 'onclick']) {
          check(calleeName + '/' + ownerName + '/' + name + '/native-owner dispatch', () => {
            const prototype = name === 'onclick' ? realm.HTMLElement.prototype : realm.HTMLMediaElement.prototype;
            const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
            let calls = 0, sameThis = false, sameTarget = false;
            const callback = function(event) { calls++; sameThis = this === media; sameTarget = event.currentTarget === media; };
            descriptor.set.call(media, callback);
            if (descriptor.get.call(media) !== callback || media[name] !== callback) throw Error('handler identity');
            media.dispatchEvent(new Event(name.slice(2)));
            if (calls !== 1 || !sameThis || !sameTarget) throw Error(JSON.stringify({calls,sameThis,sameTarget}));
            descriptor.set.call(media, null);
            if (descriptor.get.call(media) !== null || media[name] !== null) throw Error('handler removal');
          });
        }
      }
    }
    globalThis.__uiEventResults = {complete:true, total:checks.length,
      passed:checks.filter(row => row.passed).length, checks,
      separateFunctionRealm: popup.Function !== Function,
      separateMediaInterface: popup.HTMLMediaElement !== HTMLMediaElement};
    return checks.every(row => row.passed);
  } finally { popup.close(); }
})()
