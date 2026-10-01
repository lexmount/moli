globalThis.__probeEventListenerTypes = function eventListenerTypeRows(targets, EventCtor, OtherEventCtor = EventCtor) {
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: !!passed});
  const pairs = [['\ud800', '\ufffd'], ['\udc00', '\ufffd'], ['\ud800', '\ud801'],
    ['prefix\ud800\0suffix', 'prefix\ufffd\0suffix'], ['\ud800\ud800', '\ufffd\ufffd'],
    ['\udc00\ud800', '\ufffd\ufffd'], ['load', 'LOAD'], ['é', 'e\u0301'],
    ['type\0', 'type'], ['\ud83d\ude80', '\ufffd\ufffd']];
  for (const [name, target] of targets) {
    for (const [raw, other] of pairs) {
      const prefix = name + ' ' + JSON.stringify([raw, other]);
      const calls = [];
      const first = event => calls.push('first:' + (event.type === raw));
      const second = () => calls.push('second');
      target.addEventListener(raw, first);
      target.addEventListener(other, second);
      const event = new EventCtor(raw);
      target.dispatchEvent(event);
      check(prefix + ' distinct dispatch', calls.join(',') === 'first:true');
      calls.length = 0;
      target.removeEventListener(other, first);
      target.dispatchEvent(new EventCtor(raw));
      check(prefix + ' removal preserves other key', calls.join(',') === 'first:true');
      target.removeEventListener(raw, first);
      target.removeEventListener(other, second);
      calls.length = 0;
      const both = () => calls.push('both');
      target.addEventListener(raw, both, {once:true});
      target.addEventListener(other, both, {once:true});
      target.addEventListener(raw, both, {once:false});
      target.dispatchEvent(new EventCtor(raw));
      target.dispatchEvent(new EventCtor(raw));
      target.dispatchEvent(new EventCtor(other));
      target.dispatchEvent(new EventCtor(other));
      check(prefix + ' duplicate and once identity', calls.join(',') === 'both,both');
      target.removeEventListener(raw, both);
      target.removeEventListener(other, both);
      calls.length = 0;
      const controller = new AbortController();
      target.addEventListener(raw, both, {signal:controller.signal});
      target.addEventListener(other, both);
      controller.abort();
      target.dispatchEvent(new EventCtor(raw));
      target.dispatchEvent(new EventCtor(other));
      check(prefix + ' abort removes exact key', calls.join(',') === 'both');
      target.removeEventListener(raw, both);
      target.removeEventListener(other, both);
      calls.length = 0;
      let conversions = 0;
      target.addEventListener({[Symbol.toPrimitive](hint) {
        conversions++; check(prefix + ' string hint', hint === 'string'); return raw;
      }}, both);
      target.dispatchEvent(new OtherEventCtor(raw));
      check(prefix + ' one conversion and cross realm event', conversions === 1 && calls.join(',') === 'both');
      target.removeEventListener({toString() { conversions++; return raw; }}, both);
      target.dispatchEvent(new EventCtor(raw));
      check(prefix + ' converted removal', conversions === 2 && calls.join(',') === 'both');
    }
  }
  return rows;
};
(() => {
  const child = document.querySelector('#child').contentWindow;
  const channel = new MessageChannel();
  const popup = open('about:blank');
  if (!popup) throw new Error('listener identity popup did not open');
  const detached = document.implementation.createHTMLDocument('listener types');
  const element = document.createElement('button');
  document.body.appendChild(element);
  const targets = [
    ['EventTarget', new EventTarget()], ['Window', window], ['Document', document],
    ['connected Element', element], ['windowless Document', detached],
    ['detached select', detached.createElement('select')], ['child Window', child],
    ['popup Window', popup],
    ['MessagePort', channel.port1], ['AbortSignal', new AbortController().signal],
    ['FileReader', new FileReader()], ['XMLHttpRequest', new XMLHttpRequest()],
    ['XMLHttpRequestUpload', new XMLHttpRequest().upload],
    ['MediaQueryList', matchMedia('all')], ['Performance', performance],
    ['FontFaceSet', document.fonts]
  ];
  let rows;
  try {
    rows = __probeEventListenerTypes(targets, Event, child.Event);
  } finally {
    channel.port1.close();
    channel.port2.close();
    popup.close();
    element.remove();
  }
  globalThis.__eventListenerTypeResults = {rows, passed: rows.filter(row => row.passed).length,
    total: rows.length, targets: targets.map(([name]) => name)};
  return rows.every(row => row.passed) || JSON.stringify(rows.filter(row => !row.passed));
})()
