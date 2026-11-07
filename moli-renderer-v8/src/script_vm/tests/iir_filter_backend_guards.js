(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  for (const automaticPull of [false, true]) await check('IIR/active processing/' + automaticPull, async () => {
    const context = new OfflineAudioContext(1, 16, 44100), node = new IIRFilterNode(context, {feedforward: [1], feedback: [1, -.5]}), source = context.createOscillator(), analyser = context.createAnalyser();
    source.connect(node).connect(analyser); source.start();
    if (!automaticPull) analyser.connect(context.destination);
    node.__moliAudioNodeNeedsProcessingBackend = false;
    const rendering = context.startRendering(); assert(rendering instanceof Promise, 'failure is a rejected Promise');
    let failure; try { await rendering; } catch (error) { failure = error; }
    assert(failure instanceof DOMException && failure.name === 'NotSupportedError' && context.state === 'suspended', 'unsupported IIR filtering rejects before state is committed');
    const bins = new Float32Array(analyser.frequencyBinCount); analyser.getFloatFrequencyData(bins);
    assert(bins.every(value => value === -Infinity), 'failed processing leaves analyser snapshots untouched');
    node.disconnect(); source.disconnect(); source.connect(context.destination);
    assert((await context.startRendering()).getChannelData(0).some(value => value !== 0), 'disconnect permits retry using existing oscillator path');
  });
  await check('IIR/disconnected filter does not block silence', async () => {
    const context = new OfflineAudioContext(1, 16, 44100), node = new IIRFilterNode(context, {feedforward: [1], feedback: [1, -.5]});
    assert((await context.startRendering()).getChannelData(0).every(value => value === 0), 'unreachable filter does not require IIR processing');
  });
  await check('IIR/inactive connected graph is silent', async () => {
    const context = new OfflineAudioContext(1, 16, 44100), source = context.createOscillator(), node = new IIRFilterNode(context, {feedforward: [1], feedback: [1, -.5]});
    source.start(1); source.connect(node).connect(context.destination);
    assert((await context.startRendering()).getChannelData(0).every(value => value === 0), 'future source is provably silent');
  });
  globalThis.__nodeReplacementResults = {rows, passed:rows.filter(row => row.pass).length, total:rows.length, failures:rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
