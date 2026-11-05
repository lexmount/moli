importScripts(initiatorBase + '/library.js?case=import-library');

globalThis.collectInitiators = () => Object.fromEntries(
  performance.getEntriesByType('resource')
    .map(entry => [new URL(entry.name).searchParams.get('case'), entry.initiatorUrl])
    .filter(([label]) => label !== null));

// The public origin property is replaceable; request attribution must use the
// immutable native worker origin instead.
self.origin = 'https://replaced-origin.invalid';

const xhrProbe = (label, asynchronous) => new Promise((resolve, reject) => {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', initiatorBase + '/probe?case=' + label, asynchronous);
  xhr.onload = () => fromLibrary(label + '-handler').then(resolve, reject);
  xhr.onerror = () => reject(new Error('XHR failed: ' + label));
  xhr.send();
});

if (initiatorScenario !== 'unknown') {
  onmessage = async () => {
    try {
      if (initiatorScenario === 'privacy') {
        performance.clearResourceTimings();
        await fromLibrary('same');
        await fromLibrary('cors', initiatorOther + '/cors?case=cors', {mode: 'cors'});
        await fromLibrary('opaque', initiatorOther + '/probe?case=opaque', {mode: 'no-cors'});
        await fromLibrary('tao', initiatorOther + '/tao?case=tao', {mode: 'no-cors'});
        let rejected = false;
        try {
          await fromLibrary('blocked', initiatorOther + '/probe?case=blocked', {mode: 'cors'});
        } catch {
          rejected = true;
        }
        if (!rejected) throw new Error('cross-origin fetch unexpectedly succeeded');
      } else {
        await Promise.all([
          fromLibrary('direct'),
          Promise.resolve().then(() => fromLibrary('promise')),
          new Promise((resolve, reject) => queueMicrotask(() => fromLibrary('microtask').then(resolve, reject))),
          new Promise((resolve, reject) => setTimeout(() => fromLibrary('timer').then(resolve, reject), 0)),
          new Promise((resolve, reject) => {
            const id = setInterval(() => {
              clearInterval(id);
              fromLibrary('interval').then(resolve, reject);
            }, 0);
          }),
          new Promise((resolve, reject) => {
            globalThis.finishString = resolve;
            globalThis.rejectString = reject;
            setTimeout("fromLibrary('string').then(finishString, rejectString)", 0);
          }),
          xhrProbe('xhr', true),
          xhrProbe('xhr-sync', false),
        ]);
      }
      postMessage(collectInitiators());
    } catch (error) {
      postMessage({error: String(error.stack || error)});
    }
    close();
  };
}
postMessage('ready');
