async function probeFrameContainerNames({childURL, ownerWindow}) {
  const window = ownerWindow;
  const document = window.document;
  const result = {checks: 0, failures: []};
  const equal = (label, actual, expected) => {
    result.checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) result.failures.push({label, actual, expected});
  };
  const observe = callback => {
    try { return callback(); } catch (error) { return error.name; }
  };
  const create = async (name, remote) => {
    const frame = document.createElement('iframe');
    frame.name = name;
    if (remote) frame.src = childURL;
    else frame.srcdoc = '<!doctype html><body><p id="marker">local document</p>';
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    document.body.append(frame);
    await loaded;
    return frame;
  };
  const tell = (frame, message) => new Promise(resolve => {
    const channel = new MessageChannel();
    channel.port1.onmessage = event => {
      channel.port1.close();
      resolve(event.data);
    };
    frame.contentWindow.postMessage(message, '*', [channel.port2]);
  });
  const caller = await create('remote-caller', true);
  const peer = await create('published-peer', true);
  const peerWindow = peer.contentWindow;
  let duplicate;
  const namedDescriptor = name => {
    for (let object = window; object; object = Object.getPrototypeOf(object)) {
      const descriptor = Object.getOwnPropertyDescriptor(object, name);
      if (descriptor) return descriptor;
    }
  };
  const visible = (name, expected, label) => {
    equal(label + ': get', observe(() => window[name] === expected), true);
    equal(label + ': descriptor', observe(() => {
      const descriptor = namedDescriptor(name);
      return descriptor === undefined ? false : descriptor.value === expected;
    }), true);
  };
  const hidden = (name, label) => {
    equal(label + ': get', observe(() => typeof window[name]), 'undefined');
    equal(label + ': descriptor', observe(() => namedDescriptor(name) === undefined), true);
  };
  const lookup = name => tell(caller, {action:'lookup', name, index:1});
  try {
    visible('published-peer', peerWindow, 'embedder publishes cross-origin child');
    equal('parent cannot read cross-origin DOM', observe(() => window['published-peer'].document), 'SecurityError');
    equal('same-origin sibling can read peer DOM', await lookup('published-peer'), {same:true, text:'peer document'});
    equal('unknown name remains denied to sibling', await lookup('missing-frame'), {stage:'name', error:'SecurityError'});

    equal('cross-origin child can rename itself', await tell(peer, {action:'rename', name:'author-only'}), 'author-only');
    hidden('author-only', 'child cannot publish a new name');
    hidden('published-peer', 'old target name disappears');
    equal('sibling cannot discover unpublished name', await lookup('author-only'), {stage:'name', error:'SecurityError'});

    peer.name = 'author-only';
    visible('author-only', peerWindow, 'embedder authorizes the new name');
    equal('authorization preserves DOM boundary', observe(() => window['author-only'].document), 'SecurityError');
    equal('sibling sees authorized name', await lookup('author-only'), {same:true, text:'peer document'});
    hidden('AUTHOR-ONLY', 'container name matching is case-sensitive');

    peer.name = 'private-container';
    await tell(peer, {action:'rename', name:'author-only'});
    hidden('author-only', 'container no longer authorizes target name');
    duplicate = await create('author-only', false);
    hidden('author-only', 'hidden first match does not expose a later duplicate');
    equal('duplicate stays indexed', window[2] === duplicate.contentWindow, true);
    equal('sibling cannot bypass the first match', await lookup('author-only'), {stage:'name', error:'SecurityError'});

    peer.name = 'author-only';
    visible('author-only', peerWindow, 'matching attribute exposes the first duplicate');
    equal('sibling still selects first duplicate', await lookup('author-only'), {same:true, text:'peer document'});
    peer.remove();
    visible('author-only', duplicate.contentWindow, 'removal reveals the next duplicate');
    equal('local duplicate remains readable', window['author-only'].document.querySelector('#marker').textContent, 'local document');
    equal('cross-origin sibling cannot read local duplicate', await lookup('author-only'), {stage:'document', error:'SecurityError'});
    duplicate.remove();
    hidden('author-only', 'removal clears the published name');
    return result;
  } finally {
    caller.remove();
    peer.remove();
    if (duplicate) duplicate.remove();
  }
}
