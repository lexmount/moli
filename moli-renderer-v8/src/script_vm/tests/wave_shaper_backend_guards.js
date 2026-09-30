(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const rejected = async context => {
    const rendering = context.startRendering();
    assert(rendering instanceof Promise, 'startRendering returns a Promise');
    let failure; try { await rendering; } catch (error) { failure = error; }
    assert(failure instanceof DOMException && failure.name === 'NotSupportedError', 'unsupported processing or DC generation cannot pretend to render');
    assert(context.state === 'suspended', 'failure does not commit rendering state');
  };
  for (const curve of [[1, 1], [-1, 1, 1], [-1, 2], [NaN, NaN]]) {
    for (const automaticPull of [false, true]) await check('WaveShaper/DC/' + String(curve) + '/' + automaticPull, async () => {
      const context = new OfflineAudioContext(1, 16, 44100), node = context.createWaveShaper(), analyser = context.createAnalyser();
      node.curve = new Float32Array(curve); node.connect(analyser);
      if (!automaticPull) analyser.connect(context.destination);
      node.__moliAudioNodeNeedsGenerationBackend = false;
      await rejected(context);
      const bins = new Float32Array(analyser.frequencyBinCount); analyser.getFloatFrequencyData(bins);
      assert(bins.every(value => value === -Infinity), 'failure leaves analyser snapshots untouched');
      node.curve = null;
      const buffer = await context.startRendering();
      assert(buffer.length === 16 && buffer.getChannelData(0).every(value => value === 0), 'clearing curve removes generation requirement and permits silent retry');
    });
  }
  await check('WaveShaper/zero-centered inactive node', async () => {
    const context = new OfflineAudioContext(1, 16, 44100), node = new WaveShaperNode(context, {curve: [-1, 0, 1]});
    node.connect(context.destination);
    assert((await context.startRendering()).getChannelData(0).every(value => value === 0), 'provable silence needs no DSP');
  });
  await check('WaveShaper/active processing and retry', async () => {
    const context = new OfflineAudioContext(1, 16, 44100), source = context.createOscillator(), node = new WaveShaperNode(context, {curve: [-1, 0, 1]});
    source.connect(node).connect(context.destination); source.start(); node.__moliAudioNodeNeedsProcessingBackend = false;
    await rejected(context);
    node.disconnect(); source.disconnect(); source.connect(context.destination);
    assert((await context.startRendering()).getChannelData(0).some(value => value !== 0), 'failed processing does not commit state or alter existing oscillator path');
  });
  await check('WaveShaper/disconnected DC generator', async () => {
    const context = new OfflineAudioContext(1, 16, 44100), node = new WaveShaperNode(context, {curve: [1, 1]});
    node.connect(context.destination); node.disconnect();
    assert((await context.startRendering()).getChannelData(0).every(value => value === 0), 'unreachable generator does not block silence');
  });
  globalThis.__nodeReplacementResults = {rows, passed:rows.filter(row => row.pass).length, total:rows.length, failures:rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
