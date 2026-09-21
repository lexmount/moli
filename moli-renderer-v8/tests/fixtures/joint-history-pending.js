async function jointHistoryPending(base, mode) {
  const frames = [], steps = [];
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  const loaded = frame => new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const path = frame => {
    const url = new URL(frame.contentWindow.location.href);
    return url.pathname + url.hash;
  };
  for (const name of ['a', 'b']) {
    const frame = document.createElement('iframe');
    const ready = loaded(frame);
    frame.src = base + '/' + mode + '/' + name + '0';
    document.body.appendChild(frame);
    await ready;
    await settle();
    frames.push(frame);
  }
  const initialLength = history.length;
  const record = label => steps.push({label, paths: frames.map(path), length: history.length - initialLength,
    entries: frames.map(frame => frame.contentWindow.navigation.entries().map(entry => {
      const url = new URL(entry.url);
      return url.pathname + url.hash;
    }))});
  const pushes = mode === 'race' ? [[0, 'a1'], [1, 'b1']] : [[0, 'a1'], [1, 'b1'], [0, 'a2']];
  for (const [index, name] of pushes) {
    const ready = loaded(frames[index]);
    frames[index].contentWindow.location.href = base + '/' + mode + '/' + name;
    await ready;
    await settle();
  }
  record('pushed');
  const complete = loaded(frames[1]);
  if (mode === 'race') {
    frames[0].addEventListener('load', () => {
      record('first-committed');
      frames[0].contentWindow.history.pushState(null, '', '#during');
      record('push-during');
      fetch(base + '/release');
    }, {once: true});
    history.go(-2);
  } else {
    frames[0].contentWindow.navigation.back();
    await fetch(base + '/wait');
    history.back();
    await fetch(base + '/release');
  }
  await complete;
  await settle();
  record('complete');
  for (const frame of frames) frame.remove();
  return steps;
}
