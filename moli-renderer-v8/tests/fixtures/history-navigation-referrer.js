async function historyNavigationReferrer(base, mode) {
  const source = document.createElement('iframe');
  source.id = 'referrer-source';
  const load = frame => new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const initialLoad = load(source);
  source.src = base + '/initial?case=' + mode;
  (document.body || document.documentElement || document).appendChild(source);
  await initialLoad;

  let target = source;
  if (mode === 'sibling') {
    target = document.createElement('iframe');
    target.id = 'referrer-target';
    const targetLoad = load(target);
    target.src = base + '/target-initial';
    source.after(target);
    await targetLoad;
  }

  const changed = base + '/changed/' + mode + '?q=1#fragment';
  const destination = base + '/destination?case=' + mode;
  const navigated = load(target);
  const observed = source.contentWindow.eval(`(() => {
    if (${JSON.stringify(mode)} === 'base') {
      const base = document.createElement('base');
      base.href = '/assets/';
      document.head.appendChild(base);
    }
    let observed;
    const navigate = () => {
      observed = [document.URL, document.baseURI];
      const target = ${JSON.stringify(mode)} === 'sibling'
        ? parent.document.getElementById('referrer-target').contentWindow : window;
      target.location.href = ${JSON.stringify(destination)};
    };
    if (${JSON.stringify(mode)} === 'currententrychange') {
      navigation.addEventListener('currententrychange', navigate, {once: true});
    }
    history[${JSON.stringify(mode)} === 'replace' ? 'replaceState' : 'pushState'](
      null, '', ${JSON.stringify(changed)});
    if (${JSON.stringify(mode)} !== 'currententrychange') navigate();
    if (${JSON.stringify(mode)} === 'pending') {
      history.replaceState(null, '', '/changed/after-navigation');
    }
    return observed;
  })()`);
  await navigated;
  const result = {
    observed,
    referrer: target.contentDocument.referrer,
    destination: target.contentDocument.URL,
  };
  source.remove();
  if (target !== source) target.remove();
  return result;
}
