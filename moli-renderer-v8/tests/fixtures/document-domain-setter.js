async ({kind, sourceURL}) => {
  const result = {checks: 0, failures: []};
  const check = (name, actual, expected) => {
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const frames = [];
  let popup;
  const createFrame = async (owner, mode) => {
    const frame = owner.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    if (mode === 'srcdoc') frame.srcdoc = '<!doctype html><body>inherited';
    else if (mode === 'network') frame.src = sourceURL;
    owner.body.append(frame);
    frames.push(frame);
    await loaded;
    return frame.contentWindow;
  };
  try {
    let target = window;
    let originOwner = document;
    if (kind === 'iframe' || kind.startsWith('nested-')) {
      target = await createFrame(document, 'network');
      originOwner = target.document;
      if (kind.startsWith('nested-')) {
        target = await createFrame(originOwner, kind.slice('nested-'.length));
      }
    } else if (kind === 'blank' || kind === 'srcdoc') {
      target = await createFrame(document, kind);
    } else if (kind === 'popup' || kind === 'blank-popup') {
      popup = open('about:blank');
      if (!popup) throw new Error('popup did not open');
      if (kind === 'popup') {
        const loaded = new Promise(resolve => {
          const listener = event => {
            if (event.source !== popup || event.data !== 'domain-setter-ready') return;
            removeEventListener('message', listener);
            resolve();
          };
          addEventListener('message', listener);
        });
        popup.location = sourceURL;
        await loaded;
        originOwner = popup.document;
      }
      target = popup;
    }

    const doc = target.document;
    const Exception = target.DOMException;
    const original = location.hostname;
    const parentDomain = original.split('.').slice(1).join('.');
    check('initial domain', doc.domain, original);
    const assign = (name, value, expected, throws) => {
      let outcome = 'accepted';
      try { doc.domain = value; }
      catch (error) {
        outcome = `${error.name}:${error.code}:${error instanceof Exception}`;
      }
      check(name + ': outcome', outcome, throws ? 'SecurityError:18:true' : 'accepted');
      check(name + ': target domain', doc.domain, expected);
      check(name + ': origin owner domain', originOwner.domain, expected);
      if (originOwner !== document) check(name + ': independent parent', document.domain, original);
    };
    assign('self assignment', original, original, false);
    assign('reject trailing dot', original + '.', original, true);
    assign('reject multiple trailing dots', original + '..', original, true);
    assign('reject percent-encoded trailing dot', original + '%2e', original, true);
    assign('reject IDNA trailing dot', original + '\u3002', original, true);
    assign('relax to parent', parentDomain, parentDomain, false);
    assign('reject trailing dot after relaxation', parentDomain + '.', parentDomain, true);
    assign('normalized self assignment', parentDomain.toUpperCase(), parentDomain, false);
    return result;
  } finally {
    for (const frame of frames.reverse()) frame.remove();
    if (popup) popup.close();
  }
}
