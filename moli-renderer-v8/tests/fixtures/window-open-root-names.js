(() => {
  const rootName = Object.getOwnPropertyDescriptor(window, 'name');
  const rootOpener = Object.getOwnPropertyDescriptor(window, 'opener');
  window.name = 'named-root';
  const frame = (doc, name) => {
    const element = doc.createElement('iframe');
    element.name = name;
    doc.body.append(element);
    return element.contentWindow;
  };
  const sibling = frame(document, 'named-root');
  const source = frame(document, 'source');
  const nested = frame(source.document, 'named-root');
  const deep = frame(source.document, 'deep');
  const caseFrame = frame(document, 'Named-root');
  const popup = open('', 'named-popup');
  const popupSibling = frame(popup.document, 'named-popup');
  const popupSource = frame(popup.document, 'popup-source');
  const windows = {root: window, sibling, source, nested, deep, caseFrame,
                   popup, popupSibling, popupSource};
  let nameReads = 0;
  for (const value of Object.values(windows)) {
    Object.defineProperty(value, 'name', {configurable: true, get() {
      ++nameReads;
      throw new Error('target lookup read Window.name');
    }});
  }
  const cases = [
    ['root', 'named-root', 'root'],
    ['source', 'named-root', 'nested'],
    ['deep', 'named-root', 'root'],
    ['sibling', 'named-root', 'sibling'],
    ['popup', 'named-root', 'root'],
    ['popupSource', 'named-root', 'root'],
    ['root', 'named-popup', 'popup'],
    ['source', 'named-popup', 'popup'],
    ['popup', 'named-popup', 'popup'],
    ['popupSource', 'named-popup', 'popup'],
    ['popupSibling', 'named-popup', 'popupSibling'],
    ['root', 'Named-root', 'caseFrame']
  ];
  const rows = [];
  for (const callee of ['root', 'source', 'popup']) {
    for (const [receiver, target, expected] of cases) {
      try {
        const opened = windows[callee].open.call(windows[receiver], '', target);
        rows.push({callee, receiver, target, selected: opened === windows[expected],
                   opener: opened.opener === windows[receiver]});
      } catch (error) {
        rows.push({callee, receiver, target, error: error.name});
      }
    }
  }
  rootName.set.call(window, 'renamed-root');
  const renamed = deep.open('', 'renamed-root') === window;
  const oldName = deep.open('', 'named-root') === sibling;
  rootOpener.set.call(window, null);
  const disowned = rootOpener.get.call(window) === null;
  const authorOpener = {};
  window.opener = authorOpener;
  const selectedAfterShadowing = deep.open('', 'renamed-root') === window;
  const shadowPreserved = window.opener === authorOpener;
  const nativeOpenerUpdated = rootOpener.get.call(window) === deep;
  return {rows, nameReads, renamed, oldName, disowned, selectedAfterShadowing,
          shadowPreserved, nativeOpenerUpdated};
})()
