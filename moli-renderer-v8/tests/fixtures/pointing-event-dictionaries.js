(() => {
  const assert = (ok, label) => {if (!ok) throw Error(label);};
  const base = ['bubbles','cancelable','composed','detail','sourceCapabilities','view'];
  // Existing Moli modifier extensions remain supported; compare the common
  // Chromium 145 members and preserve all unfiltered observations separately.
  const extensions = new Set(['modifierFnLock','modifierHyper','modifierSuper','modifierSymbolLock']);
  const modifiers = ['altKey','ctrlKey','metaKey','modifierAltGraph','modifierCapsLock',
    'modifierFn','modifierNumLock','modifierScrollLock','modifierSymbol','shiftKey'];
  const mouse = [...base, ...modifiers, 'button','buttons','clientX','clientY',
    'movementX','movementY','region','relatedTarget','screenX','screenY'];
  const pointer = ['altitudeAngle','azimuthAngle','coalescedEvents','height','isPrimary',
    'persistentDeviceId','pointerId','pointerType','predictedEvents','pressure',
    'tangentialPressure','tiltX','tiltY','twist','width'];
  const cases = [[MouseEvent,mouse], [WheelEvent,[...mouse,'deltaMode','deltaX','deltaY','deltaZ','wheelDeltaX','wheelDeltaY']],
    [DragEvent,[...mouse,'dataTransfer']], [PointerEvent,[...mouse,...pointer]]];
  for (const [Constructor, members] of cases) {
    const trace = [];
    new Constructor('audit', new Proxy({}, {get(_target,key) {if (!extensions.has(key)) trace.push(String(key));}}));
    assert(trace.join() === members.join(), Constructor.name + ' order: ' + trace);
    for (const stop of members) {
      const sentinel = {};
      trace.length = 0;
      let error;
      try {new Constructor('audit', new Proxy({}, {get(_target,key) {
        if (!extensions.has(key)) trace.push(String(key)); if (key === stop) throw sentinel;
      }}));} catch (caught) {error = caught;}
      assert(error === sentinel, Constructor.name + '/' + stop + ' exception identity');
      assert(trace.join() === members.slice(0,members.indexOf(stop)+1).join(),
        Constructor.name + '/' + stop + ' stops conversion');
    }
    for (const relatedTarget of [{}, new Proxy(document.body,{})]) {
      let error;
      try {new Constructor('audit',{relatedTarget});} catch (caught) {error=caught;}
      assert(error instanceof TypeError, Constructor.name + ' relatedTarget native brand');
    }
    const event = new Constructor('audit', {relatedTarget:document.body, detail:2147483648,
      clientX:12, button:1, buttons:-1, modifierAltGraph:true});
    assert(event.relatedTarget === document.body && event.detail === -2147483648 &&
      event.clientX === 12 && event.button === 1 && event.buttons === 65535 &&
      event.getModifierState('AltGraph'), Constructor.name + ' converted state');
    for (const value of [1,true,Symbol('init')]) {
      let error;
      try {new Constructor('audit',value);} catch (caught) {error=caught;}
      assert(error instanceof TypeError, Constructor.name + ' primitive init');
    }
  }
  for (const member of ['coalescedEvents','predictedEvents']) {
    let next = 0, error;
    const sentinel = {};
    const values = {[Symbol.iterator]() {return {next() {
      if (next++) throw sentinel;
      return {done:false,value:{}};
    }};}};
    try {new PointerEvent('audit',{[member]:values});} catch (caught) {error=caught;}
    assert(error instanceof TypeError && next === 1, member + ' validates each item before advancing');
    const item = new PointerEvent('move');
    const event = new PointerEvent('audit',{[member]:new Set([item])});
    const method = member === 'coalescedEvents' ? 'getCoalescedEvents' : 'getPredictedEvents';
    const first=event[method](); first.length=0;
    assert(event[method]()[0] === item, member + ' identity and independent arrays');
  }
  assert(new WheelEvent('audit',{deltaMode:-1}).deltaMode === 4294967295, 'WheelEvent unsigned long');
  const transfer=new DataTransfer();
  assert(new DragEvent('audit',{dataTransfer:transfer}).dataTransfer===transfer, 'DataTransfer identity');
  for (const [init, expected] of [[{deltaX:2.5,deltaY:-4.5},[2,-4,-4]],
    [{wheelDeltaX:13,wheelDeltaY:27},[13,27,27]], [{deltaY:1e300},[0,2147483647,2147483647]],
    [{wheelDeltaX:2147483648,wheelDeltaY:-1.5},[-2147483648,-1,-1]]]) {
    const e=new WheelEvent('audit',init);
    assert([e.wheelDeltaX,e.wheelDeltaY,e.wheelDelta].join()===expected.join(), 'WheelEvent legacy delta state');
  }
  const event=new PointerEvent('audit',{pressure:0.1,pointerId:2147483648,pointerType:'\ud800',tiltX:-45});
  assert(event.pressure===Math.fround(0.1) && event.pointerId===-2147483648 &&
    event.pointerType==='\ud800' && event.azimuthAngle===Math.PI &&
    Math.abs(event.altitudeAngle-Math.PI/4)<1e-14, 'PointerEvent numeric and UTF-16 state');
  return true;
})()
