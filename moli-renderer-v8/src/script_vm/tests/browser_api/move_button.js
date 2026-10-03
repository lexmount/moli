function __installMoveButtonProbe(capture) {
  document.body.style.cssText = 'margin:0;user-select:none';
  document.body.innerHTML = '<div id="first" style="position:absolute;left:40px;top:40px;width:120px;height:120px"></div><div id="second" style="position:absolute;left:220px;top:40px;width:120px;height:120px"></div>';
  const first = document.getElementById('first');
  const rows = [];
  const checks = [];
  const check = (name, actual, expected) => checks.push({name, actual, expected, passed: actual === expected});
  const mouseBoundary = ['mouseover', 'mouseout', 'mouseenter', 'mouseleave'];
  const types = ['pointerdown', 'pointerup', 'pointermove', 'pointerrawupdate', 'pointerover', 'pointerout', 'pointerenter', 'pointerleave', 'gotpointercapture', 'lostpointercapture', 'mousedown', 'mouseup', 'mousemove', ...mouseBoundary];
  for (const type of types) document.addEventListener(type, event => {
    if (!['first', 'second'].includes(event.target.id)) return;
    const phase = globalThis.__movePhase;
    if (!phase) return;
    rows.push({phase: phase.name, type: event.type, target: event.target.id, button: event.button, buttons: event.buttons, pointerType: event.pointerType || '', trusted: event.isTrusted, bubbles: event.bubbles, composed: event.composed, cancelable: event.cancelable});
    if (event.type === 'mousedown' || event.type === 'mouseup') event.preventDefault();
  }, true);
  if (capture) first.addEventListener('pointerdown', event => first.setPointerCapture(event.pointerId));
  for (const type of ['contextmenu', 'auxclick']) document.addEventListener(type, event => event.preventDefault());
  return {finish() {
    for (const phase of ['hover', 'held', 'chord-held', 'cross', 'after']) {
      const movement = rows.filter(row => row.phase === phase);
      const mouse = movement.filter(row => row.type === 'mousemove');
      const pointer = movement.filter(row => row.type === 'pointermove');
      check(`${phase}:mousemove count`, mouse.length, 1);
      check(`${phase}:pointermove count`, pointer.length, 1);
      for (const row of movement) {
        const expectedButton = row.type.startsWith('pointer') || row.type.endsWith('pointercapture') ? -1 : 0;
        check(`${phase}:${row.type}@${row.target}:button`, row.button, expectedButton);
        check(`${phase}:${row.type}@${row.target}:trusted`, row.trusted, true);

      }
      if (phase === 'cross') {
        check('cross:mouse recipient', mouse[0] && mouse[0].target, capture ? 'first' : 'second');
        check('cross:pointer recipient', pointer[0] && pointer[0].target, capture ? 'first' : 'second');
      }
    }
    for (const [phase, pointerType, mouseType] of [['down', 'pointerdown', 'mousedown'], ['up', 'pointerup', 'mouseup'], ['chord-down', 'pointermove', 'mousedown'], ['chord-up', 'pointermove', 'mouseup']]) {
      for (const type of [pointerType, mouseType]) {
        const selected = rows.filter(row => row.phase === phase && row.type === type);
        check(`${phase}:${type} count`, selected.length, 1);
      }
    }
    for (const row of rows.filter(row => ['down', 'up', 'chord-down', 'chord-up'].includes(row.phase) && ['pointerdown', 'pointerup', 'pointermove', 'mousedown', 'mouseup'].includes(row.type))) {
      const expected = globalThis.__moveExpected[row.phase];
      check(`${row.phase}:${row.type}:changed button`, row.button, expected.button);
      check(`${row.phase}:${row.type}:buttons`, row.buttons, expected.buttons);
    }
    for (const row of rows.filter(row => ['hover', 'held', 'chord-held', 'cross', 'after'].includes(row.phase))) {
      check(`${row.phase}:${row.type}@${row.target}:buttons`, row.buttons, globalThis.__moveExpected[row.phase].buttons);
    }
    for (const row of rows) {
      const boundary = ['mouseenter', 'mouseleave', 'pointerenter', 'pointerleave'].includes(row.type);
      const nonCancelable = boundary || ['pointerrawupdate', 'gotpointercapture', 'lostpointercapture'].includes(row.type);
      check(`${row.phase}:${row.type}@${row.target}:bubbles`, row.bubbles, !boundary);
      check(`${row.phase}:${row.type}@${row.target}:composed`, row.composed, !boundary);
      check(`${row.phase}:${row.type}@${row.target}:cancelable`, row.cancelable, !nonCancelable);
    }
    if (!capture) {
      for (const type of mouseBoundary) check(`cross:has ${type}`, rows.some(row => row.phase === 'cross' && row.type === type), true);
    } else {
      check('capture acquired', rows.some(row => row.type === 'gotpointercapture'), true);
      check('capture released', rows.some(row => row.type === 'lostpointercapture'), true);
      check('capture held until final up', rows.filter(row => row.type === 'lostpointercapture' && row.phase !== 'up').length, 0);
    }
    const authoredConstructorValues = [MouseEvent, PointerEvent].flatMap(constructor => ['mousemove', 'mouseover', 'pointermove'].map(type => ({constructor: constructor.name, type, supplied: 2, button: new constructor(type, {button: 2}).button})));
    return {checks, rows, authoredConstructorValues, passed: checks.filter(row => row.passed).length, total: checks.length, complete: checks.every(row => row.passed)};
  }};
}
