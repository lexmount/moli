(async () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  for (const factory of ['createGain', 'createDelay', 'createStereoPanner']) {
    for (const automaticPull of [false, true]) {
      await check(factory + '/unsupported processing/' + automaticPull, async () => {
        const context = new OfflineAudioContext(1, 32, 8000);
        const source = context.createOscillator(), node = context[factory](), analyser = context.createAnalyser();
        source.connect(node); node.connect(analyser);
        if (!automaticPull) analyser.connect(context.destination);
        node.__moliAudioNodeNeedsProcessingBackend = false;
        source.start();
        const rendering = context.startRendering();
        assert(rendering instanceof Promise, 'failure is a rejected Promise');
        let failure; try { await rendering; } catch (error) { failure = error; }
        assert(failure instanceof DOMException && failure.name === 'NotSupportedError', 'unsupported processor cannot pass through the synthetic oscillator');
        assert(context.state === 'suspended', 'failure preserves context state');
        const bins = new Float32Array(analyser.frequencyBinCount); analyser.getFloatFrequencyData(bins);
        assert(bins.every(value => value === -Infinity), 'failure does not commit a synthetic render snapshot');
      });
    }
    await check(factory + '/inactive input is silence', async () => {
      const context = new OfflineAudioContext(1, 32, 8000), node = context[factory](), source = context.createOscillator();
      source.start(1); source.connect(node); node.connect(context.destination);
      const buffer = await context.startRendering();
      assert(buffer.getChannelData(0).every(value => value === 0), 'provable silence does not need DSP');
    });
    await check(factory + '/disconnected processor does not affect another graph', async () => {
      const context = new OfflineAudioContext(1, 32, 8000), node = context[factory](), source = context.createOscillator();
      source.connect(node); source.connect(context.destination); source.start();
      const buffer = await context.startRendering();
      assert(buffer.getChannelData(0).some(value => value !== 0), 'preserve the existing oscillator path outside the new node');
    });
  }
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
