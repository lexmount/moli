(() => {
  const failures = [];
  let cases = 0;
  const observers = [
    ['length', owner => owner.frames.length, 1],
    ['numeric-value', owner => owner[1] === undefined, true],
    ['numeric-query', owner => 1 in owner, false],
    ['numeric-descriptor', owner => Object.getOwnPropertyDescriptor(owner, '1') === undefined, true],
    ['numeric-enumeration', owner => Object.getOwnPropertyNames(owner).includes('1'), false],
    ['contentWindow', (_owner, frame) => frame.contentWindow === null, true],
    ['contentDocument', (_owner, frame) => frame.contentDocument === null, true],
    ['getSVGDocument', (_owner, frame) => frame.getSVGDocument() === null, true],
    ['named-window', owner => owner.pendingInsertionFrame === undefined, true],
    ['named-document', (owner, frame) => owner.document.pendingInsertionFrame === frame, true],
    ['named-document-collection', (owner, frame) => {
      const peer = owner.document.createElement('form');
      peer.name = frame.name;
      owner.document.body.append(peer);
      try {
        const items = owner.document[frame.name];
        const matched = items.length === 2 && items[0] === frame && items[1] === peer;
        peer.remove();
        return matched && items.length === 1 && items[0] === frame;
      } finally {
        peer.remove();
      }
    }, true],
    ['name-mutation', (_owner, frame) => {
      frame.name = 'renamedPendingFrame';
      return frame.contentWindow === null;
    }, true],
    ['id-mutation', (_owner, frame) => {
      frame.id = 'pendingFrameId';
      return frame.contentWindow === null;
    }, true],
    ['src-mutation', (_owner, frame) => {
      frame.src = 'about:blank';
      return frame.contentWindow === null;
    }, true],
  ];
  for (const realm of ['main', 'child']) {
    const parent = realm === 'child' ? document.createElement('iframe') : null;
    if (parent) {
      // Blank grandchildren must not repeat their ancestor's URL.
      parent.src = 'about:blank?insertion-owner';
      document.body.append(parent);
    }
    const owner = parent ? parent.contentWindow : window;
    const doc = owner.document;
    for (const [name, observe, expected] of observers) {
      const fragment = doc.createDocumentFragment();
      const first = fragment.appendChild(doc.createElement('iframe'));
      const second = fragment.appendChild(doc.createElement('iframe'));
      second.name = 'pendingInsertionFrame';
      const events = [];
      let secondLoaded = false;
      let observation;
      first.onload = () => {
        events.push('first');
        observation = {
          firstConnected: first.isConnected,
          secondConnected: second.isConnected,
          secondLoadedBeforeRead: secondLoaded,
        };
        try {
          observation.value = observe(owner, second);
        } catch (error) {
          observation.error = String(error.stack || error);
        }
        observation.secondLoadedAfterRead = secondLoaded;
      };
      second.onload = () => {
        events.push('second');
        secondLoaded = true;
      };
      try {
        doc.body.append(fragment);
        cases++;
        if (!observation || !observation.firstConnected || !observation.secondConnected ||
            observation.secondLoadedBeforeRead || observation.secondLoadedAfterRead ||
            observation.error || observation.value !== expected || !secondLoaded ||
            events.join(',') !== 'first,second' || owner.frames.length !== 2 ||
            owner[0] !== first.contentWindow || owner[1] !== second.contentWindow ||
            second.contentWindow.name !== second.name) {
          failures.push({realm, name, expected, observation, secondLoaded, events});
        }
      } finally {
        first.remove();
        second.remove();
      }
    }
    if (parent) parent.remove();
  }
  return {cases, failures};
})()
