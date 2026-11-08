// Feishin's startup registers media controls before connecting two playback
// gains through a preamp, equalizer and compressor. Both missing APIs used to
// throw during React effects and leave the application root empty.
// Also run against Chrome 157.0.8103.38 as the compatibility oracle.
(() => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const session = navigator.mediaSession;
  const actions = ['nexttrack', 'pause', 'play', 'previoustrack', 'seekto', 'stop', 'seekbackward', 'seekforward'];
  for (const action of actions) session.setActionHandler(action, () => {});
  const context = new AudioContext({latencyHint: 'playback'});
  const gains = [context.createGain(), context.createGain()];
  const preamp = context.createGain();
  preamp.gain.value = 1;
  const filters = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map(frequency => {
    const filter = context.createBiquadFilter();
    filter.type = 'peaking';
    filter.frequency.value = frequency;
    filter.Q.value = 1.41;
    filter.gain.value = 0;
    return filter;
  });
  const compressor = context.createDynamicsCompressor();
  compressor.threshold.value = 0;
  compressor.ratio.value = 1;
  compressor.attack.value = 0;
  compressor.release.value = 0.25;
  compressor.knee.value = 0;
  for (const gain of gains) assert(gain.connect(preamp) === preamp, 'playback gains');
  preamp.connect(filters[0]);
  for (let i = 0; i < filters.length - 1; i++) filters[i].connect(filters[i + 1]);
  filters.at(-1).connect(compressor);
  compressor.connect(context.destination);
  const sources = gains.map(gain => {
    const element = document.createElement('audio');
    const source = context.createMediaElementSource(element);
    assert(source.mediaElement === element, 'media element ownership');
    source.connect(gain);
    return source;
  });
  document.body.innerHTML = '<main id="root">Player initialized</main>';
  assert(document.getElementById('root').textContent === 'Player initialized', 'startup completed');
  for (const action of actions) session.setActionHandler(action, null);
  for (const node of [...sources, ...gains, preamp, ...filters, compressor]) node.disconnect();
  context.close();
  return 'ok';
})()
