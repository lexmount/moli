async function jointHistoryTraversal(base, mode) {
  const crossDocument = mode.startsWith('cross-document') || mode.startsWith('navigation-cross-document');
  const frames = [];
  const loads = [];
  const steps = [];
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  const path = frame => {
    const url = new URL(frame.contentWindow.location.href);
    return url.pathname + url.hash;
  };
  for (const name of ['a', 'b']) {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
    frame.src = base + '/' + name + '0';
    document.body.appendChild(frame);
    await loaded;
    await settle();
    frames.push(frame);
    const index = loads.length;
    loads.push(0);
    frame.addEventListener('load', () => loads[index]++);
  }
  const initialLength = history.length;
  const record = label => steps.push({
    label,
    paths: frames.map(path),
    length: history.length - initialLength,
    childLengths: frames.map(frame => frame.contentWindow.history.length - initialLength),
    entries: frames.map(frame => frame.contentWindow.navigation.entries().map(entry => {
      const url = new URL(entry.url);
      return url.pathname + url.hash;
    })),
  });
  const waitForPaths = async expected => {
    const requiredLoads = loads.map((count, index) => count +
      (crossDocument && path(frames[index]) !== expected[index] ? 1 : 0));
    for (let attempt = 0; attempt < 100; attempt++) {
      if (frames.every((frame, index) => path(frame) === expected[index] && loads[index] >= requiredLoads[index])) {
        await settle();
        return;
      }
      await new Promise(resolve => setTimeout(resolve, 5));
    }
    throw new Error('history traversal did not reach ' + expected + '; got ' + frames.map(path));
  };
  for (const [index, destination] of [[0, 'a1'], [1, 'b1'], [0, 'a2']]) {
    const frame = frames[index];
    if (crossDocument) {
      const loaded = new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
      frame.contentWindow.location.href = base + '/' + destination;
      await loaded;
    } else {
      frame.contentWindow.history.pushState({destination}, '', '#' + destination);
    }
    await settle();
  }
  record('pushed');
  if (mode === 'navigation') {
    await frames[0].contentWindow.navigation.back().committed;
    await waitForPaths(['/a0#a1', '/b0#b1']);
    record('child-a-back');
    await frames[1].contentWindow.navigation.back().committed;
    await waitForPaths(['/a0#a1', '/b0']);
    record('child-b-back');
    await frames[0].contentWindow.navigation.back().committed;
    await waitForPaths(['/a0', '/b0']);
    record('first-step');
    await frames[1].contentWindow.navigation.forward().committed;
    await waitForPaths(['/a0#a1', '/b0#b1']);
    record('both-forward');
    await frames[0].contentWindow.navigation.forward().committed;
    await waitForPaths(['/a0#a2', '/b0#b1']);
    record('last-step');
  } else if (mode === 'navigation-cross-document') {
    const identities = () => JSON.stringify(frames.map(frame =>
      frame.contentWindow.navigation.entries().map(entry => [entry.key, entry.id])));
    const before = identities();
    frames[0].contentWindow.navigation.traverseTo(frames[0].contentWindow.navigation.entries()[0].key);
    await waitForPaths(['/a0', '/b0']);
    record('first-step');
    frames[0].contentWindow.navigation.traverseTo(frames[0].contentWindow.navigation.entries()[2].key);
    await waitForPaths(['/a2', '/b1']);
    record('last-step');
    if (identities() !== before) throw new Error('cross-document traversal changed entry keys or IDs');
  } else if (mode === 'cross-document-queued' || mode === 'navigation-cross-document-queued') {
    if (mode === 'navigation-cross-document-queued') {
      frames[0].contentWindow.navigation.back();
    } else {
      history.back();
    }
    history.back();
    await waitForPaths(['/a1', '/b0']);
    record('back-twice');
    history.go(2);
    await waitForPaths(['/a2', '/b1']);
    record('forward-two');
  } else if (crossDocument) {
    history.back();
    await waitForPaths(['/a1', '/b1']);
    record('back');
    history.go(-2);
    await waitForPaths(['/a0', '/b0']);
    record('back-two');
    history.go(3);
    await waitForPaths(['/a2', '/b1']);
    record('forward-three');
  } else if (mode === 'back') {
    history.back();
    await waitForPaths(['/a0#a1', '/b0#b1']);
    record('back');
  } else {
    if (mode === 'queued') {
      history.back();
      frames[0].contentWindow.history.back();
    } else if (mode === 'child') {
      frames[1].contentWindow.history.go(-2);
    } else {
      history.go(-2);
    }
    await waitForPaths(['/a0#a1', '/b0']);
    record('back-two');
    if (mode === 'fork') {
      frames[0].contentWindow.history.pushState({destination: 'fork'}, '', '#fork');
      await settle();
      record('fork');
    } else {
      history.go(2);
      await waitForPaths(['/a0#a2', '/b0#b1']);
      record('forward-two');
    }
  }
  for (const frame of frames) frame.remove();
  return steps;
}
