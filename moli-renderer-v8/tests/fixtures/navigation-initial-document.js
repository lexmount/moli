globalThis.initialNavigationProbe = async function(mode) {
  const frame = document.createElement('iframe');
  frame.name = 'initial-navigation-target';
  const loaded = Promise.withResolvers();
  frame.onload = loaded.resolve;
  if(mode === 'srcdoc') frame.srcdoc = '<p>loaded</p>';
  else if(mode.startsWith('javascript')) frame.src = "javascript:'<p>loaded</p>'";
  else if(mode.startsWith('loaded-')) frame.src = new URL('/child', location.href).href;
  (document.body || document.documentElement || document).appendChild(frame);
  await loaded.promise;
  if(mode.startsWith('loaded-')) {
    const blankLoaded = new Promise(resolve => frame.onload = resolve);
    frame.src = 'about:blank';
    await blankLoaded;
  }
  // Run after load has returned, so history behavior is not affected by an
  // in-progress load event.
  await new Promise(resolve => setTimeout(resolve, 0));
  const child = frame.contentWindow;
  const nav = child.navigation;
  const events = [];
  for(const type of ['navigate','currententrychange','navigatesuccess','navigateerror']) {
    nav.addEventListener(type, () => events.push(type));
  }
  const snapshot = () => ({
    url:child.location.href.replaceAll(location.origin, '<origin>'),
    entries:nav.entries().map(entry => entry.url.replaceAll(location.origin, '<origin>')),
    current:nav.currentEntry?.url.replaceAll(location.origin, '<origin>') ?? null,
    activation:nav.activation === null,
    transition:nav.transition === null,
    back:nav.canGoBack,
    forward:nav.canGoForward
  });
  const before = snapshot();
  const promises = [];
  if(mode.includes('-navigate-') || mode === 'initial-anchor') {
    const crossDocument = mode.endsWith('-relative') || mode.endsWith('-cross') || mode === 'initial-anchor';
    const nextLoad = crossDocument ? new Promise(resolve => frame.onload = resolve) : null;
    if(mode === 'initial-anchor') {
      frame.name = 'initial-navigation-target';
      const link = document.createElement('a');
      link.href = new URL('/child', location.href).href;
      link.target = frame.name;
      (document.body || document.documentElement || document).appendChild(link);
      link.click();
    } else {
      const target = mode.endsWith('-relative') ? '#relative' : mode.endsWith('-cross') || mode.endsWith('-push') ? new URL('/child', location.href).href : 'about:blank#fragment';
      const result = nav.navigate(target, mode.endsWith('-push') ? {history:'push'} : {});
      for(const key of ['committed', 'finished']) result[key].then(() => promises.push(key + ':fulfilled'), error => promises.push(key + ':' + error.name));
    }
    if(nextLoad) await nextLoad;
  } else if(mode.endsWith('-fragment')) child.location.href = 'about:blank#fragment';
  else if(mode === 'initial-history') {
    child.history.pushState({value:1}, '', 'about:blank#first');
    child.history.pushState({value:2}, '', 'about:blank#second');
  } else if(mode === 'initial-open') {
    child.document.open();
    child.document.write('<p>written</p>');
    child.document.close();
  }
  await new Promise(resolve => setTimeout(resolve, 20));
  const after = snapshot();
  let update;
  try {
    nav.updateCurrentEntry({state:{value:3}});
    update = nav.currentEntry?.getState().value;
  } catch(error) { update = error.name; }
  const result = {before,after,update,events,promises,sameNavigation:nav === child.navigation,newCurrent:child.navigation.currentEntry?.url.replaceAll(location.origin, "<origin>") ?? null};
  frame.remove();
  return result;
};
