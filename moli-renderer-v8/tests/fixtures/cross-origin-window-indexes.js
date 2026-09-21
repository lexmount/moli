async function probeCrossOriginWindowIndexes({sameURL, crossURL, hostURL, nested = false, collectGarbage = () => {}}) {
  const create = async (url, name) => {
    const frame = document.createElement('iframe');
    frame.name = name;
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = url;
    document.body.appendChild(frame);
    await loaded;
    return frame;
  };
  if (nested) {
    const outer = await create(hostURL, 'outer');
    const run = outer.contentWindow.eval('(' + probeCrossOriginWindowIndexes.toString() + ')');
    const result = await run({sameURL, crossURL, collectGarbage});
    outer.remove();
    return result;
  }
  const failures = [];
  let checks = 0;
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const target = await create(sameURL, 'target');
  const observer = await create(crossURL, 'observer');
  const peer = await create(crossURL, 'peer');
  const ask = phase => new Promise(resolve => {
    const borrowed = [];
    const listener = event => {
      if (event.source !== observer.contentWindow) return;
      if (event.data.kind === 'borrowed') borrowed.push(event.data.phase);
      if (event.data.kind !== 'result') return;
      removeEventListener('message', listener);
      equal(borrowed, [phase], phase + ' borrowed postMessage delivery');
      resolve(event.data.result);
    };
    addEventListener('message', listener);
    observer.contentWindow.postMessage({kind:'probe', phase, selfIndex:1, peerIndex:2}, '*');
  });
  const check = async (phase, count, navigated = false) => {
    collectGarbage();
    const actual = await ask(phase);
    const expected = {
      length:count, prototype:true, extensible:true,
      preventExtensions:'SecurityError', samePrototype:'SecurityError', differentPrototype:'SecurityError',
      borrowMessage:true, keys:Array.from({length:count}, (_,i) => String(i)),
      ownIndex:true, parentIdentity:true,
      descriptor:['object',false,true,true], named:navigated ? 'SecurityError' : true,
      namedDescriptor:navigated ? 'SecurityError' : ['object',false,false,true],
      has:true, missing:'SecurityError', missingHas:'SecurityError', missingDescriptor:'SecurityError',
      nonIndex:'SecurityError', document:'SecurityError', deniedDescriptor:'SecurityError', missingNamedDescriptor:'SecurityError',
      set:'SecurityError', define:'SecurityError', delete:'SecurityError', focus:'ok', cachedIdentity:true,
      methodRealm:true, getterRealm:true, locationGetterRealm:true, locationSetterRealm:true,
      methodIdentity:true, getterIdentity:true,
      targetFrame:navigated ? true : 'SecurityError', targetDocument:navigated ? true : 'SecurityError',
      peerFrame:true, peerDocument:true
    };
    for (const key of Object.keys(expected)) equal(actual[key], expected[key], phase + ' ' + key);
  };
  await check('initial',3);
  const extra = await create(sameURL, 'extra');
  await check('appended',4);
  extra.remove();
  await check('removed',3);
  const loaded = new Promise(resolve => target.onload = resolve);
  target.src = crossURL;
  await loaded;
  await check('navigated',3,true);
  target.remove(); observer.remove(); peer.remove();
  return {checks, failures};
}
