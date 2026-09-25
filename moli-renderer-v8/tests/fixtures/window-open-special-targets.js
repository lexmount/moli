globalThis.specialTargetCase = (() => {
  const frame = document.createElement('iframe');
  document.body.append(frame);
  const middle = frame.contentWindow;
  const nested = middle.document.createElement('iframe');
  middle.document.body.append(nested);
  const deep = nested.contentWindow;
  const popup = open('', 'special-target-popup');
  const popupFrame = popup.document.createElement('iframe');
  popup.document.body.append(popupFrame);
  const popupChild = popupFrame.contentWindow;
  const windows = {root: window, middle, deep, popup, popupChild};
  const targets = {
    root: ['root', 'root', 'root'],
    deep: ['deep', 'middle', 'root'],
    popup: ['popup', 'popup', 'popup'],
    popupChild: ['popupChild', 'popup', 'popup']
  };
  const names = ['_self', '_parent', '_top'];
  const identify = value => Object.keys(windows).find(key => windows[key] === value) || String(value);
  return {
    ready() {
      return Object.values(windows).every(w => w.document.readyState === 'complete');
    },
    run() {
      const failures = [];
      let count = 0, parentReads = 0;
      for (const shadowParent of [false, true]) {
        const descriptors = Object.values(windows).map(w => Object.getOwnPropertyDescriptor(w, 'parent'));
        try {
          if (shadowParent) {
            for (const w of Object.values(windows)) {
              Object.defineProperty(w, 'parent', {configurable: true, get() {
                ++parentReads;
                throw new Error('Window.parent must not participate in target selection');
              }});
            }
          }
          for (const callee of ['root', 'deep', 'popup']) {
            for (const [receiver, destinations] of Object.entries(targets)) {
              for (let i = 0; i < names.length; ++i) {
                for (const name of [names[i], names[i].toUpperCase()]) {
                  for (const features of ['', 'noopener', 'noreferrer']) {
                    const expected = windows[destinations[i]];
                    const before = [expected.document, expected.location.href,
                                    expected.history.length, expected.opener];
                    const label = [callee, receiver, name, features, shadowParent].join('/');
                    ++count;
                    try {
                      const actual = windows[callee].open.call(windows[receiver], '', name, features);
                      if (actual !== expected) failures.push(label + ': returned ' + identify(actual));
                      const after = [expected.document, expected.location.href,
                                     expected.history.length, expected.opener];
                      if (!before.every((value, j) => value === after[j])) failures.push(label + ': navigated');
                    } catch (error) {
                      failures.push(label + ': ' + error);
                    }
                  }
                }
              }
            }
          }
        } finally {
          Object.values(windows).forEach((w, i) => Object.defineProperty(w, 'parent', descriptors[i]));
        }
      }
      return {count, parentReads, failures};
    },
    close() {
      popup.close();
      frame.remove();
    }
  };
})();
